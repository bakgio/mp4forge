//! Core ISO BMFF timing and structure boxes.

use std::io::{SeekFrom, Write};

use crate::boxes::{AnyTypeBox, BoxLookupContext, BoxRegistry};
use crate::codec::{
    CodecBox, CodecError, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead,
    FieldValueWrite, ImmutableBox, MutableBox, ReadSeek, StringFieldMode, read_exact_vec_untrusted,
    untrusted_prealloc_hint,
};
use crate::{FourCc, codec_field};

const URL_SELF_CONTAINED: u32 = 0x000001;
const URN_SELF_CONTAINED: u32 = 0x000001;
const AUX_INFO_TYPE_PRESENT: u32 = 0x000001;
const SCHEME_URI_PRESENT: u32 = 0x000001;

const COLR_NCLX: FourCc = FourCc::from_bytes(*b"nclx");
const COLR_RICC: FourCc = FourCc::from_bytes(*b"rICC");
const COLR_PROF: FourCc = FourCc::from_bytes(*b"prof");

/// `tfhd` flag indicating that `base_data_offset` is present.
pub const TFHD_BASE_DATA_OFFSET_PRESENT: u32 = 0x000001;
/// `tfhd` flag indicating that `sample_description_index` is present.
pub const TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT: u32 = 0x000002;
/// `tfhd` flag indicating that `default_sample_duration` is present.
pub const TFHD_DEFAULT_SAMPLE_DURATION_PRESENT: u32 = 0x000008;
/// `tfhd` flag indicating that `default_sample_size` is present.
pub const TFHD_DEFAULT_SAMPLE_SIZE_PRESENT: u32 = 0x000010;
/// `tfhd` flag indicating that `default_sample_flags` is present.
pub const TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT: u32 = 0x000020;
/// `tfhd` flag indicating that the fragment duration is empty.
pub const TFHD_DURATION_IS_EMPTY: u32 = 0x010000;
/// `tfhd` flag indicating that the default base is the containing `moof`.
pub const TFHD_DEFAULT_BASE_IS_MOOF: u32 = 0x020000;

/// `trun` flag indicating that `data_offset` is present.
pub const TRUN_DATA_OFFSET_PRESENT: u32 = 0x000001;
/// `trun` flag indicating that `first_sample_flags` is present.
pub const TRUN_FIRST_SAMPLE_FLAGS_PRESENT: u32 = 0x000004;
/// `trun` flag indicating that each entry carries `sample_duration`.
pub const TRUN_SAMPLE_DURATION_PRESENT: u32 = 0x000100;
/// `trun` flag indicating that each entry carries `sample_size`.
pub const TRUN_SAMPLE_SIZE_PRESENT: u32 = 0x000200;
/// `trun` flag indicating that each entry carries `sample_flags`.
pub const TRUN_SAMPLE_FLAGS_PRESENT: u32 = 0x000400;
/// `trun` flag indicating that each entry carries a composition time offset.
pub const TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT: u32 = 0x000800;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FullBoxState {
    version: u8,
    flags: u32,
}

fn missing_field(field_name: &'static str) -> FieldValueError {
    FieldValueError::MissingField { field_name }
}

fn unexpected_field(field_name: &'static str, value: FieldValue) -> FieldValueError {
    FieldValueError::UnexpectedType {
        field_name,
        expected: "matching codec field value",
        actual: value.kind_name(),
    }
}

fn invalid_value(field_name: &'static str, reason: &'static str) -> FieldValueError {
    FieldValueError::InvalidValue { field_name, reason }
}

fn u8_from_unsigned(field_name: &'static str, value: u64) -> Result<u8, FieldValueError> {
    u8::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u8"))
}

fn u16_from_unsigned(field_name: &'static str, value: u64) -> Result<u16, FieldValueError> {
    u16::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u16"))
}

fn u32_from_unsigned(field_name: &'static str, value: u64) -> Result<u32, FieldValueError> {
    u32::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u32"))
}

fn i16_from_signed(field_name: &'static str, value: i64) -> Result<i16, FieldValueError> {
    i16::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in i16"))
}

fn i32_from_signed(field_name: &'static str, value: i64) -> Result<i32, FieldValueError> {
    i32::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in i32"))
}

fn i64_from_signed(field_name: &'static str, value: i64) -> Result<i64, FieldValueError> {
    let _ = field_name;
    Ok(value)
}

fn bytes_to_fourcc(field_name: &'static str, bytes: Vec<u8>) -> Result<FourCc, FieldValueError> {
    let array: [u8; 4] = bytes
        .try_into()
        .map_err(|_| invalid_value(field_name, "value must be exactly 4 bytes"))?;
    Ok(FourCc::from_bytes(array))
}

fn bytes_to_zeroes(
    field_name: &'static str,
    bytes: &[u8],
    expected_len: usize,
) -> Result<(), FieldValueError> {
    if bytes.len() != expected_len {
        return Err(invalid_value(
            field_name,
            "value has an unexpected reserved-byte length",
        ));
    }
    if bytes.iter().any(|byte| *byte != 0) {
        return Err(invalid_value(field_name, "reserved bytes must be zero"));
    }
    Ok(())
}

fn bytes_to_fourcc_vec(
    field_name: &'static str,
    bytes: Vec<u8>,
) -> Result<Vec<FourCc>, FieldValueError> {
    parse_fixed_chunks(field_name, &bytes, 4, |chunk| {
        FourCc::from_bytes(chunk.try_into().unwrap())
    })
}

fn fourcc_vec_to_bytes(values: &[FourCc]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(value.as_bytes());
    }
    bytes
}

fn parse_fixed_chunks<T, F>(
    field_name: &'static str,
    bytes: &[u8],
    chunk_size: usize,
    parse: F,
) -> Result<Vec<T>, FieldValueError>
where
    F: FnMut(&[u8]) -> T,
{
    let chunks = bytes.chunks_exact(chunk_size);
    if !chunks.remainder().is_empty() {
        return Err(invalid_value(
            field_name,
            "value does not align with entry size",
        ));
    }

    Ok(chunks.map(parse).collect())
}

fn field_len_bytes(count: usize, bytes_per_entry: usize) -> Option<u32> {
    count
        .checked_mul(bytes_per_entry)
        .and_then(|len| u32::try_from(len).ok())
}

fn render_array(values: impl IntoIterator<Item = String>) -> String {
    let items = values.into_iter().collect::<Vec<_>>();
    format!("[{}]", items.join(", "))
}

fn render_hex_bytes(bytes: &[u8]) -> String {
    render_array(bytes.iter().map(|byte| format!("0x{:x}", byte)))
}

fn quoted_fourcc(value: FourCc) -> String {
    format!("\"{value}\"")
}

fn quote_bytes(bytes: &[u8]) -> String {
    format!("\"{}\"", escape_bytes(bytes))
}

fn escape_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| escape_display_char(char::from(*byte)))
        .collect()
}

fn escape_display_char(value: char) -> char {
    if value.is_control() || !value.is_ascii_graphic() && value != ' ' {
        '.'
    } else {
        value
    }
}

fn format_fixed_16_16_signed(value: i32) -> String {
    if value & 0xffff == 0 {
        return (value >> 16).to_string();
    }
    format!("{:.5}", f64::from(value) / 65536.0)
}

fn format_fixed_16_16_unsigned(value: u32) -> String {
    if value & 0xffff == 0 {
        return (value >> 16).to_string();
    }
    format!("{:.5}", f64::from(value) / 65536.0)
}

fn format_fixed_8_8_signed(value: i16) -> String {
    if value & 0xff == 0 {
        return (value >> 8).to_string();
    }
    format!("{:.3}", f32::from(value) / 256.0)
}

fn read_i16(bytes: &[u8], offset: usize) -> i16 {
    i16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn read_i64(bytes: &[u8], offset: usize) -> i64 {
    i64::from_be_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn read_uint(bytes: &[u8], offset: usize, width_bytes: usize) -> u64 {
    let mut value = 0_u64;
    for byte in &bytes[offset..offset + width_bytes] {
        value = (value << 8) | u64::from(*byte);
    }
    value
}

fn push_uint(
    field_name: &'static str,
    bytes: &mut Vec<u8>,
    width_bytes: usize,
    value: u64,
) -> Result<(), FieldValueError> {
    let max_value = if width_bytes == 8 {
        u64::MAX
    } else {
        (1_u64 << (width_bytes * 8)) - 1
    };
    if value > max_value {
        return Err(invalid_value(
            field_name,
            "value does not fit in the configured byte width",
        ));
    }

    for shift in (0..width_bytes).rev() {
        bytes.push((value >> (shift * 8)) as u8);
    }
    Ok(())
}

fn field_len_from_count(count: u32, bytes_per_entry: usize) -> Option<u32> {
    usize::try_from(count)
        .ok()
        .and_then(|count| field_len_bytes(count, bytes_per_entry))
}

fn encode_avc_parameter_sets(
    field_name: &'static str,
    parameter_sets: &[AVCParameterSet],
) -> Result<Vec<u8>, FieldValueError> {
    let mut bytes = Vec::new();
    for parameter_set in parameter_sets {
        let actual_len = u16::try_from(parameter_set.nal_unit.len())
            .map_err(|_| invalid_value(field_name, "parameter set length does not fit in u16"))?;
        if parameter_set.length != actual_len {
            return Err(invalid_value(
                field_name,
                "parameter set length does not match the NAL unit size",
            ));
        }
        bytes.extend_from_slice(&parameter_set.length.to_be_bytes());
        bytes.extend_from_slice(&parameter_set.nal_unit);
    }
    Ok(bytes)
}

fn encoded_avc_parameter_sets_len(
    field_name: &'static str,
    parameter_sets: &[AVCParameterSet],
) -> Result<u32, FieldValueError> {
    let bytes = encode_avc_parameter_sets(field_name, parameter_sets)?;
    u32::try_from(bytes.len()).map_err(|_| {
        invalid_value(
            field_name,
            "parameter-set payload length does not fit in u32",
        )
    })
}

fn parse_avc_parameter_sets(
    field_name: &'static str,
    bytes: &[u8],
    expected_count: u8,
) -> Result<Vec<AVCParameterSet>, FieldValueError> {
    let mut parameter_sets =
        Vec::with_capacity(untrusted_prealloc_hint(usize::from(expected_count)));
    let mut offset = 0_usize;
    for _ in 0..expected_count {
        if bytes.len().saturating_sub(offset) < 2 {
            return Err(invalid_value(
                field_name,
                "parameter-set payload length does not match the entry count",
            ));
        }

        let length = read_u16(bytes, offset);
        offset += 2;
        let end = offset + usize::from(length);
        if end > bytes.len() {
            return Err(invalid_value(
                field_name,
                "parameter-set payload length does not match the entry count",
            ));
        }

        parameter_sets.push(AVCParameterSet {
            length,
            nal_unit: bytes[offset..end].to_vec(),
        });
        offset = end;
    }

    if offset != bytes.len() {
        return Err(invalid_value(
            field_name,
            "parameter-set payload length does not match the entry count",
        ));
    }

    Ok(parameter_sets)
}

fn render_avc_parameter_sets(parameter_sets: &[AVCParameterSet]) -> String {
    render_array(parameter_sets.iter().map(|parameter_set| {
        format!(
            "{{Length={} NALUnit={}}}",
            parameter_set.length,
            render_hex_bytes(&parameter_set.nal_unit)
        )
    }))
}

fn pack_hevc_profile_compatibility(values: &[bool; 32]) -> [u8; 4] {
    let mut bytes = [0_u8; 4];
    for (index, value) in values.iter().copied().enumerate() {
        if value {
            bytes[index / 8] |= 1 << (7 - (index % 8));
        }
    }
    bytes
}

fn unpack_hevc_profile_compatibility(bytes: &[u8; 4]) -> [bool; 32] {
    let mut values = [false; 32];
    for (index, value) in values.iter_mut().enumerate() {
        *value = bytes[index / 8] & (1 << (7 - (index % 8))) != 0;
    }
    values
}

fn encode_hevc_nalus(
    field_name: &'static str,
    nalus: &[HEVCNalu],
) -> Result<Vec<u8>, FieldValueError> {
    let mut bytes = Vec::new();
    for nalu in nalus {
        let actual_len = u16::try_from(nalu.nal_unit.len())
            .map_err(|_| invalid_value(field_name, "NAL unit length does not fit in u16"))?;
        if nalu.length != actual_len {
            return Err(invalid_value(
                field_name,
                "NAL unit length does not match the NAL unit size",
            ));
        }
        bytes.extend_from_slice(&nalu.length.to_be_bytes());
        bytes.extend_from_slice(&nalu.nal_unit);
    }
    Ok(bytes)
}

fn render_hevc_nalus(nalus: &[HEVCNalu]) -> String {
    render_array(nalus.iter().map(|nalu| {
        format!(
            "{{Length={} NALUnit={}}}",
            nalu.length,
            render_hex_bytes(&nalu.nal_unit)
        )
    }))
}

fn encode_hevc_nalu_arrays(
    field_name: &'static str,
    arrays: &[HEVCNaluArray],
) -> Result<Vec<u8>, FieldValueError> {
    let mut bytes = Vec::new();
    for array in arrays {
        if array.nalu_type > 0x3f {
            return Err(invalid_value("NaluType", "value does not fit in 6 bits"));
        }
        require_count("NumNalus", u32::from(array.num_nalus), array.nalus.len())?;
        let nalus = encode_hevc_nalus("Nalus", &array.nalus)?;
        bytes.push(
            (u8::from(array.completeness) << 7) | (u8::from(array.reserved) << 6) | array.nalu_type,
        );
        bytes.extend_from_slice(&array.num_nalus.to_be_bytes());
        bytes.extend_from_slice(&nalus);
    }

    let _ = field_name;
    Ok(bytes)
}

fn encoded_hevc_nalu_arrays_len(
    field_name: &'static str,
    arrays: &[HEVCNaluArray],
) -> Result<u32, FieldValueError> {
    let bytes = encode_hevc_nalu_arrays(field_name, arrays)?;
    u32::try_from(bytes.len())
        .map_err(|_| invalid_value(field_name, "NAL-array payload length does not fit in u32"))
}

fn parse_hevc_nalu_arrays(
    field_name: &'static str,
    bytes: &[u8],
    expected_count: u8,
) -> Result<Vec<HEVCNaluArray>, FieldValueError> {
    let mut arrays = Vec::with_capacity(untrusted_prealloc_hint(usize::from(expected_count)));
    let mut offset = 0_usize;

    for _ in 0..expected_count {
        if bytes.len().saturating_sub(offset) < 3 {
            return Err(invalid_value(
                field_name,
                "NAL-array payload length does not match the entry count",
            ));
        }

        let header = bytes[offset];
        let completeness = header & 0x80 != 0;
        let reserved = header & 0x40 != 0;
        let nalu_type = header & 0x3f;
        offset += 1;

        let num_nalus = read_u16(bytes, offset);
        offset += 2;

        let mut nalus = Vec::with_capacity(untrusted_prealloc_hint(usize::from(num_nalus)));
        for _ in 0..num_nalus {
            if bytes.len().saturating_sub(offset) < 2 {
                return Err(invalid_value(
                    field_name,
                    "NAL-array payload length does not match the entry count",
                ));
            }

            let length = read_u16(bytes, offset);
            offset += 2;
            let end = offset + usize::from(length);
            if end > bytes.len() {
                return Err(invalid_value(
                    field_name,
                    "NAL-array payload length does not match the entry count",
                ));
            }

            nalus.push(HEVCNalu {
                length,
                nal_unit: bytes[offset..end].to_vec(),
            });
            offset = end;
        }

        arrays.push(HEVCNaluArray {
            completeness,
            reserved,
            nalu_type,
            num_nalus,
            nalus,
        });
    }

    if offset != bytes.len() {
        return Err(invalid_value(
            field_name,
            "NAL-array payload length does not match the entry count",
        ));
    }

    Ok(arrays)
}

fn render_hevc_nalu_arrays(arrays: &[HEVCNaluArray]) -> String {
    render_array(arrays.iter().map(|array| {
        format!(
            "{{Completeness={} Reserved={} NaluType=0x{:x} NumNalus={} Nalus={}}}",
            array.completeness,
            array.reserved,
            array.nalu_type,
            array.num_nalus,
            render_hevc_nalus(&array.nalus)
        )
    }))
}

fn avc_profile_supports_extensions(profile: u8) -> bool {
    matches!(profile, 100 | 110 | 122 | 144)
}

fn require_count(
    field_name: &'static str,
    expected_count: u32,
    actual_count: usize,
) -> Result<(), FieldValueError> {
    if usize::try_from(expected_count).ok() != Some(actual_count) {
        return Err(invalid_value(
            field_name,
            "entry count does not match the parsed payload",
        ));
    }
    Ok(())
}

macro_rules! impl_leaf_box {
    ($name:ident, $box_type:expr) => {
        impl FieldHooks for $name {}

        impl ImmutableBox for $name {
            fn box_type(&self) -> FourCc {
                FourCc::from_bytes($box_type)
            }
        }

        impl MutableBox for $name {}
    };
}

macro_rules! impl_full_box {
    ($name:ident, $box_type:expr) => {
        impl ImmutableBox for $name {
            fn box_type(&self) -> FourCc {
                FourCc::from_bytes($box_type)
            }

            fn version(&self) -> u8 {
                self.full_box.version
            }

            fn flags(&self) -> u32 {
                self.full_box.flags
            }
        }

        impl MutableBox for $name {
            fn set_version(&mut self, version: u8) {
                self.full_box.version = version;
            }

            fn set_flags(&mut self, flags: u32) {
                self.full_box.flags = flags;
            }
        }
    };
}

macro_rules! empty_hooks {
    ($($name:ident),* $(,)?) => {
        $(
            impl FieldHooks for $name {}
        )*
    };
}

macro_rules! empty_box_codec {
    ($name:ident) => {
        impl FieldValueRead for $name {
            fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
                Err(missing_field(field_name))
            }
        }

        impl FieldValueWrite for $name {
            fn set_field_value(
                &mut self,
                field_name: &'static str,
                value: FieldValue,
            ) -> Result<(), FieldValueError> {
                Err(unexpected_field(field_name, value))
            }
        }

        impl CodecBox for $name {
            const FIELD_TABLE: FieldTable = FieldTable::new(&[]);
        }
    };
}

macro_rules! simple_container_box {
    ($name:ident, $box_type:expr) => {
        #[doc = "Container box with no direct payload fields."]
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $name;

        impl_leaf_box!($name, $box_type);
        empty_box_codec!($name);
    };
}

macro_rules! raw_data_box {
    ($name:ident, $box_type:expr) => {
        #[doc = "Raw-data box that preserves its payload bytes verbatim."]
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $name {
            pub data: Vec<u8>,
        }

        impl_leaf_box!($name, $box_type);

        impl FieldValueRead for $name {
            fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
                match field_name {
                    "Data" => Ok(FieldValue::Bytes(self.data.clone())),
                    _ => Err(missing_field(field_name)),
                }
            }
        }

        impl FieldValueWrite for $name {
            fn set_field_value(
                &mut self,
                field_name: &'static str,
                value: FieldValue,
            ) -> Result<(), FieldValueError> {
                match (field_name, value) {
                    ("Data", FieldValue::Bytes(data)) => {
                        self.data = data;
                        Ok(())
                    }
                    (field_name, value) => Err(unexpected_field(field_name, value)),
                }
            }
        }

        impl CodecBox for $name {
            const FIELD_TABLE: FieldTable =
                FieldTable::new(&[codec_field!("Data", 0, with_bit_width(8), as_bytes())]);
        }
    };
}

simple_container_box!(Dinf, *b"dinf");
simple_container_box!(Edts, *b"edts");
simple_container_box!(Mdia, *b"mdia");
simple_container_box!(Minf, *b"minf");
simple_container_box!(Moof, *b"moof");
simple_container_box!(Moov, *b"moov");
simple_container_box!(Mvex, *b"mvex");
simple_container_box!(Mfra, *b"mfra");
simple_container_box!(Stbl, *b"stbl");
simple_container_box!(Traf, *b"traf");
simple_container_box!(Trak, *b"trak");
simple_container_box!(Udta, *b"udta");

raw_data_box!(Free, *b"free");
raw_data_box!(Skip, *b"skip");
raw_data_box!(Mdat, *b"mdat");

/// File type and compatibility declaration box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ftyp {
    pub major_brand: FourCc,
    pub minor_version: u32,
    pub compatible_brands: Vec<FourCc>,
}

impl Default for Ftyp {
    fn default() -> Self {
        Self {
            major_brand: FourCc::ANY,
            minor_version: 0,
            compatible_brands: Vec::new(),
        }
    }
}

impl Ftyp {
    /// Adds `brand` if it is not already listed as compatible.
    pub fn add_compatible_brand(&mut self, brand: FourCc) {
        if !self.has_compatible_brand(brand) {
            self.compatible_brands.push(brand);
        }
    }

    /// Removes `brand` from the compatibility list.
    pub fn remove_compatible_brand(&mut self, brand: FourCc) {
        self.compatible_brands
            .retain(|candidate| *candidate != brand);
    }

    /// Returns `true` when `brand` is present in the compatibility list.
    pub fn has_compatible_brand(&self, brand: FourCc) -> bool {
        self.compatible_brands.contains(&brand)
    }
}

impl FieldHooks for Ftyp {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "CompatibleBrands" => field_len_bytes(self.compatible_brands.len(), 4),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "MajorBrand" => Some(quoted_fourcc(self.major_brand)),
            "CompatibleBrands" => {
                Some(render_array(self.compatible_brands.iter().map(|brand| {
                    format!("{{CompatibleBrand={}}}", quoted_fourcc(*brand))
                })))
            }
            _ => None,
        }
    }
}

impl ImmutableBox for Ftyp {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"ftyp")
    }
}

impl MutableBox for Ftyp {}

impl FieldValueRead for Ftyp {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "MajorBrand" => Ok(FieldValue::Bytes(self.major_brand.as_bytes().to_vec())),
            "MinorVersion" => Ok(FieldValue::Unsigned(u64::from(self.minor_version))),
            "CompatibleBrands" => Ok(FieldValue::Bytes(fourcc_vec_to_bytes(
                &self.compatible_brands,
            ))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Ftyp {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("MajorBrand", FieldValue::Bytes(bytes)) => {
                self.major_brand = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("MinorVersion", FieldValue::Unsigned(value)) => {
                self.minor_version = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CompatibleBrands", FieldValue::Bytes(bytes)) => {
                self.compatible_brands = bytes_to_fourcc_vec(field_name, bytes)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Ftyp {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!(
            "MajorBrand",
            0,
            with_bit_width(8),
            with_length(4),
            as_bytes()
        ),
        codec_field!("MinorVersion", 1, with_bit_width(32)),
        codec_field!("CompatibleBrands", 2, with_bit_width(8), as_bytes()),
    ]);
}

/// Segment type and compatibility declaration box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Styp {
    pub major_brand: FourCc,
    pub minor_version: u32,
    pub compatible_brands: Vec<FourCc>,
}

impl Default for Styp {
    fn default() -> Self {
        Self {
            major_brand: FourCc::ANY,
            minor_version: 0,
            compatible_brands: Vec::new(),
        }
    }
}

impl FieldHooks for Styp {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "CompatibleBrands" => field_len_bytes(self.compatible_brands.len(), 4),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "MajorBrand" => Some(quoted_fourcc(self.major_brand)),
            "CompatibleBrands" => {
                Some(render_array(self.compatible_brands.iter().map(|brand| {
                    format!("{{CompatibleBrand={}}}", quoted_fourcc(*brand))
                })))
            }
            _ => None,
        }
    }
}

impl ImmutableBox for Styp {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"styp")
    }
}

impl MutableBox for Styp {}

impl FieldValueRead for Styp {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "MajorBrand" => Ok(FieldValue::Bytes(self.major_brand.as_bytes().to_vec())),
            "MinorVersion" => Ok(FieldValue::Unsigned(u64::from(self.minor_version))),
            "CompatibleBrands" => Ok(FieldValue::Bytes(fourcc_vec_to_bytes(
                &self.compatible_brands,
            ))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Styp {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("MajorBrand", FieldValue::Bytes(bytes)) => {
                self.major_brand = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("MinorVersion", FieldValue::Unsigned(value)) => {
                self.minor_version = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CompatibleBrands", FieldValue::Bytes(bytes)) => {
                self.compatible_brands = bytes_to_fourcc_vec(field_name, bytes)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Styp {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!(
            "MajorBrand",
            0,
            with_bit_width(8),
            with_length(4),
            as_bytes()
        ),
        codec_field!("MinorVersion", 1, with_bit_width(32)),
        codec_field!("CompatibleBrands", 2, with_bit_width(8), as_bytes()),
    ]);
}

empty_hooks!(
    Dref, Url, Urn, Mfhd, Mfro, Mehd, Mdhd, Tfdt, Tfhd, Trep, Trex, Vmhd, Stsd, Cslg
);

/// Data reference box that counts child data-entry boxes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dref {
    full_box: FullBoxState,
    pub entry_count: u32,
}

impl_full_box!(Dref, *b"dref");

impl FieldValueRead for Dref {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Dref {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Dref {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// URL data-entry box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Url {
    full_box: FullBoxState,
    pub location: String,
}

impl_full_box!(Url, *b"url ");

impl FieldValueRead for Url {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Location" => Ok(FieldValue::String(self.location.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Url {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Location", FieldValue::String(value)) => {
                self.location = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Url {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "Location",
            2,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_forbidden_flags(URL_SELF_CONTAINED)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// URN data-entry box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Urn {
    full_box: FullBoxState,
    pub name: String,
    pub location: String,
}

impl_full_box!(Urn, *b"urn ");

impl FieldValueRead for Urn {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Name" => Ok(FieldValue::String(self.name.clone())),
            "Location" => Ok(FieldValue::String(self.location.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Urn {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Name", FieldValue::String(value)) => {
                self.name = value;
                Ok(())
            }
            ("Location", FieldValue::String(value)) => {
                self.location = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Urn {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "Name",
            2,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_forbidden_flags(URN_SELF_CONTAINED)
        ),
        codec_field!(
            "Location",
            3,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_forbidden_flags(URN_SELF_CONTAINED)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Movie fragment header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mfhd {
    full_box: FullBoxState,
    pub sequence_number: u32,
}

impl_full_box!(Mfhd, *b"mfhd");

impl FieldValueRead for Mfhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SequenceNumber" => Ok(FieldValue::Unsigned(u64::from(self.sequence_number))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Mfhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SequenceNumber", FieldValue::Unsigned(value)) => {
                self.sequence_number = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Mfhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("SequenceNumber", 2, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Movie fragment random access offset box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mfro {
    full_box: FullBoxState,
    pub size: u32,
}

impl_full_box!(Mfro, *b"mfro");

impl FieldValueRead for Mfro {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Size" => Ok(FieldValue::Unsigned(u64::from(self.size))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Mfro {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Size", FieldValue::Unsigned(value)) => {
                self.size = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Mfro {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Size", 2, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Movie extends header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mehd {
    full_box: FullBoxState,
    pub fragment_duration_v0: u32,
    pub fragment_duration_v1: u64,
}

impl_full_box!(Mehd, *b"mehd");

impl Mehd {
    /// Returns the active fragment duration for the current box version.
    pub fn fragment_duration(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.fragment_duration_v0),
            1 => self.fragment_duration_v1,
            _ => 0,
        }
    }
}

impl FieldValueRead for Mehd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "FragmentDurationV0" => Ok(FieldValue::Unsigned(u64::from(self.fragment_duration_v0))),
            "FragmentDurationV1" => Ok(FieldValue::Unsigned(self.fragment_duration_v1)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Mehd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("FragmentDurationV0", FieldValue::Unsigned(value)) => {
                self.fragment_duration_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FragmentDurationV1", FieldValue::Unsigned(value)) => {
                self.fragment_duration_v1 = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Mehd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("FragmentDurationV0", 2, with_bit_width(32), with_version(0)),
        codec_field!("FragmentDurationV1", 3, with_bit_width(64), with_version(1)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Media header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mdhd {
    full_box: FullBoxState,
    pub creation_time_v0: u32,
    pub modification_time_v0: u32,
    pub creation_time_v1: u64,
    pub modification_time_v1: u64,
    pub timescale: u32,
    pub duration_v0: u32,
    pub duration_v1: u64,
    pub pad: bool,
    pub language: [u8; 3],
    pub pre_defined: u16,
}

impl_full_box!(Mdhd, *b"mdhd");

impl Mdhd {
    /// Returns the active media creation time for the current box version.
    pub fn creation_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.creation_time_v0),
            1 => self.creation_time_v1,
            _ => 0,
        }
    }

    /// Returns the active media modification time for the current box version.
    pub fn modification_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.modification_time_v0),
            1 => self.modification_time_v1,
            _ => 0,
        }
    }

    /// Returns the active media duration for the current box version.
    pub fn duration(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.duration_v0),
            1 => self.duration_v1,
            _ => 0,
        }
    }
}

impl FieldValueRead for Mdhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CreationTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.creation_time_v0))),
            "ModificationTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.modification_time_v0))),
            "CreationTimeV1" => Ok(FieldValue::Unsigned(self.creation_time_v1)),
            "ModificationTimeV1" => Ok(FieldValue::Unsigned(self.modification_time_v1)),
            "Timescale" => Ok(FieldValue::Unsigned(u64::from(self.timescale))),
            "DurationV0" => Ok(FieldValue::Unsigned(u64::from(self.duration_v0))),
            "DurationV1" => Ok(FieldValue::Unsigned(self.duration_v1)),
            "Pad" => Ok(FieldValue::Boolean(self.pad)),
            "Language" => Ok(FieldValue::UnsignedArray(
                self.language.iter().copied().map(u64::from).collect(),
            )),
            "PreDefined" => Ok(FieldValue::Unsigned(u64::from(self.pre_defined))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Mdhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CreationTimeV0", FieldValue::Unsigned(value)) => {
                self.creation_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ModificationTimeV0", FieldValue::Unsigned(value)) => {
                self.modification_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CreationTimeV1", FieldValue::Unsigned(value)) => {
                self.creation_time_v1 = value;
                Ok(())
            }
            ("ModificationTimeV1", FieldValue::Unsigned(value)) => {
                self.modification_time_v1 = value;
                Ok(())
            }
            ("Timescale", FieldValue::Unsigned(value)) => {
                self.timescale = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DurationV0", FieldValue::Unsigned(value)) => {
                self.duration_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DurationV1", FieldValue::Unsigned(value)) => {
                self.duration_v1 = value;
                Ok(())
            }
            ("Pad", FieldValue::Boolean(value)) => {
                self.pad = value;
                Ok(())
            }
            ("Language", FieldValue::UnsignedArray(values)) => {
                if values.len() != 3 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 3 elements",
                    ));
                }
                self.language = [
                    u8_from_unsigned(field_name, values[0])?,
                    u8_from_unsigned(field_name, values[1])?,
                    u8_from_unsigned(field_name, values[2])?,
                ];
                Ok(())
            }
            ("PreDefined", FieldValue::Unsigned(value)) => {
                self.pre_defined = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Mdhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("CreationTimeV0", 2, with_bit_width(32), with_version(0)),
        codec_field!("ModificationTimeV0", 3, with_bit_width(32), with_version(0)),
        codec_field!("CreationTimeV1", 4, with_bit_width(64), with_version(1)),
        codec_field!("ModificationTimeV1", 5, with_bit_width(64), with_version(1)),
        codec_field!("Timescale", 6, with_bit_width(32)),
        codec_field!("DurationV0", 7, with_bit_width(32), with_version(0)),
        codec_field!("DurationV1", 8, with_bit_width(64), with_version(1)),
        codec_field!("Pad", 9, with_bit_width(1), as_boolean(), as_hidden()),
        codec_field!(
            "Language",
            10,
            with_bit_width(5),
            with_length(3),
            as_iso639_2()
        ),
        codec_field!("PreDefined", 11, with_bit_width(16)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Movie header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mvhd {
    full_box: FullBoxState,
    pub creation_time_v0: u32,
    pub modification_time_v0: u32,
    pub creation_time_v1: u64,
    pub modification_time_v1: u64,
    pub timescale: u32,
    pub duration_v0: u32,
    pub duration_v1: u64,
    pub rate: i32,
    pub volume: i16,
    pub matrix: [i32; 9],
    pub pre_defined: [i32; 6],
    pub next_track_id: u32,
}

impl_full_box!(Mvhd, *b"mvhd");

impl FieldHooks for Mvhd {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Rate" => Some(format_fixed_16_16_signed(self.rate)),
            _ => None,
        }
    }
}

impl Mvhd {
    /// Returns the active movie creation time for the current box version.
    pub fn creation_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.creation_time_v0),
            1 => self.creation_time_v1,
            _ => 0,
        }
    }

    /// Returns the active movie modification time for the current box version.
    pub fn modification_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.modification_time_v0),
            1 => self.modification_time_v1,
            _ => 0,
        }
    }

    /// Returns the active movie duration for the current box version.
    pub fn duration(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.duration_v0),
            1 => self.duration_v1,
            _ => 0,
        }
    }

    /// Returns the playback rate as a signed 16.16 fixed-point value.
    pub fn rate_value(&self) -> f64 {
        f64::from(self.rate) / 65536.0
    }

    /// Returns the integer component of the playback rate.
    pub fn rate_int(&self) -> i16 {
        (self.rate >> 16) as i16
    }
}

impl FieldValueRead for Mvhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CreationTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.creation_time_v0))),
            "ModificationTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.modification_time_v0))),
            "CreationTimeV1" => Ok(FieldValue::Unsigned(self.creation_time_v1)),
            "ModificationTimeV1" => Ok(FieldValue::Unsigned(self.modification_time_v1)),
            "Timescale" => Ok(FieldValue::Unsigned(u64::from(self.timescale))),
            "DurationV0" => Ok(FieldValue::Unsigned(u64::from(self.duration_v0))),
            "DurationV1" => Ok(FieldValue::Unsigned(self.duration_v1)),
            "Rate" => Ok(FieldValue::Signed(i64::from(self.rate))),
            "Volume" => Ok(FieldValue::Signed(i64::from(self.volume))),
            "Reserved2" => Ok(FieldValue::Bytes(vec![0; 8])),
            "Matrix" => Ok(FieldValue::SignedArray(
                self.matrix.iter().copied().map(i64::from).collect(),
            )),
            "PreDefined" => Ok(FieldValue::SignedArray(
                self.pre_defined.iter().copied().map(i64::from).collect(),
            )),
            "NextTrackID" => Ok(FieldValue::Unsigned(u64::from(self.next_track_id))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Mvhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CreationTimeV0", FieldValue::Unsigned(value)) => {
                self.creation_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ModificationTimeV0", FieldValue::Unsigned(value)) => {
                self.modification_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CreationTimeV1", FieldValue::Unsigned(value)) => {
                self.creation_time_v1 = value;
                Ok(())
            }
            ("ModificationTimeV1", FieldValue::Unsigned(value)) => {
                self.modification_time_v1 = value;
                Ok(())
            }
            ("Timescale", FieldValue::Unsigned(value)) => {
                self.timescale = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DurationV0", FieldValue::Unsigned(value)) => {
                self.duration_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DurationV1", FieldValue::Unsigned(value)) => {
                self.duration_v1 = value;
                Ok(())
            }
            ("Rate", FieldValue::Signed(value)) => {
                self.rate = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("Volume", FieldValue::Signed(value)) => {
                self.volume = i16_from_signed(field_name, value)?;
                Ok(())
            }
            ("Reserved2", FieldValue::Bytes(bytes)) => bytes_to_zeroes(field_name, &bytes, 8),
            ("Matrix", FieldValue::SignedArray(values)) => {
                if values.len() != 9 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 9 elements",
                    ));
                }
                for (slot, value) in self.matrix.iter_mut().zip(values.into_iter()) {
                    *slot = i32_from_signed(field_name, value)?;
                }
                Ok(())
            }
            ("PreDefined", FieldValue::SignedArray(values)) => {
                if values.len() != 6 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 6 elements",
                    ));
                }
                for (slot, value) in self.pre_defined.iter_mut().zip(values.into_iter()) {
                    *slot = i32_from_signed(field_name, value)?;
                }
                Ok(())
            }
            ("NextTrackID", FieldValue::Unsigned(value)) => {
                self.next_track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Mvhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("CreationTimeV0", 2, with_bit_width(32), with_version(0)),
        codec_field!("ModificationTimeV0", 3, with_bit_width(32), with_version(0)),
        codec_field!("CreationTimeV1", 4, with_bit_width(64), with_version(1)),
        codec_field!("ModificationTimeV1", 5, with_bit_width(64), with_version(1)),
        codec_field!("Timescale", 6, with_bit_width(32)),
        codec_field!("DurationV0", 7, with_bit_width(32), with_version(0)),
        codec_field!("DurationV1", 8, with_bit_width(64), with_version(1)),
        codec_field!("Rate", 9, with_bit_width(32), as_signed()),
        codec_field!("Volume", 10, with_bit_width(16), as_signed()),
        codec_field!("Reserved", 11, with_bit_width(16), with_constant("0")),
        codec_field!(
            "Reserved2",
            12,
            with_bit_width(8),
            with_length(8),
            as_bytes(),
            as_hidden()
        ),
        codec_field!(
            "Matrix",
            13,
            with_bit_width(32),
            with_length(9),
            as_signed(),
            as_hex()
        ),
        codec_field!(
            "PreDefined",
            14,
            with_bit_width(32),
            with_length(6),
            as_signed()
        ),
        codec_field!("NextTrackID", 15, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Track fragment decode time box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tfdt {
    full_box: FullBoxState,
    pub base_media_decode_time_v0: u32,
    pub base_media_decode_time_v1: u64,
}

impl_full_box!(Tfdt, *b"tfdt");

impl Tfdt {
    /// Returns the active base media decode time for the current box version.
    pub fn base_media_decode_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.base_media_decode_time_v0),
            1 => self.base_media_decode_time_v1,
            _ => 0,
        }
    }
}

impl FieldValueRead for Tfdt {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "BaseMediaDecodeTimeV0" => Ok(FieldValue::Unsigned(u64::from(
                self.base_media_decode_time_v0,
            ))),
            "BaseMediaDecodeTimeV1" => Ok(FieldValue::Unsigned(self.base_media_decode_time_v1)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Tfdt {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("BaseMediaDecodeTimeV0", FieldValue::Unsigned(value)) => {
                self.base_media_decode_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BaseMediaDecodeTimeV1", FieldValue::Unsigned(value)) => {
                self.base_media_decode_time_v1 = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Tfdt {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "BaseMediaDecodeTimeV0",
            2,
            with_bit_width(32),
            with_version(0)
        ),
        codec_field!(
            "BaseMediaDecodeTimeV1",
            3,
            with_bit_width(64),
            with_version(1)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Track fragment header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tfhd {
    full_box: FullBoxState,
    pub track_id: u32,
    pub base_data_offset: u64,
    pub sample_description_index: u32,
    pub default_sample_duration: u32,
    pub default_sample_size: u32,
    pub default_sample_flags: u32,
}

impl_full_box!(Tfhd, *b"tfhd");

impl FieldValueRead for Tfhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "TrackID" => Ok(FieldValue::Unsigned(u64::from(self.track_id))),
            "BaseDataOffset" => Ok(FieldValue::Unsigned(self.base_data_offset)),
            "SampleDescriptionIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.sample_description_index,
            ))),
            "DefaultSampleDuration" => Ok(FieldValue::Unsigned(u64::from(
                self.default_sample_duration,
            ))),
            "DefaultSampleSize" => Ok(FieldValue::Unsigned(u64::from(self.default_sample_size))),
            "DefaultSampleFlags" => Ok(FieldValue::Unsigned(u64::from(self.default_sample_flags))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Tfhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("TrackID", FieldValue::Unsigned(value)) => {
                self.track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BaseDataOffset", FieldValue::Unsigned(value)) => {
                self.base_data_offset = value;
                Ok(())
            }
            ("SampleDescriptionIndex", FieldValue::Unsigned(value)) => {
                self.sample_description_index = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleDuration", FieldValue::Unsigned(value)) => {
                self.default_sample_duration = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleSize", FieldValue::Unsigned(value)) => {
                self.default_sample_size = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleFlags", FieldValue::Unsigned(value)) => {
                self.default_sample_flags = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Tfhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("TrackID", 2, with_bit_width(32)),
        codec_field!(
            "BaseDataOffset",
            3,
            with_bit_width(64),
            with_required_flags(TFHD_BASE_DATA_OFFSET_PRESENT)
        ),
        codec_field!(
            "SampleDescriptionIndex",
            4,
            with_bit_width(32),
            with_required_flags(TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT)
        ),
        codec_field!(
            "DefaultSampleDuration",
            5,
            with_bit_width(32),
            with_required_flags(TFHD_DEFAULT_SAMPLE_DURATION_PRESENT)
        ),
        codec_field!(
            "DefaultSampleSize",
            6,
            with_bit_width(32),
            with_required_flags(TFHD_DEFAULT_SAMPLE_SIZE_PRESENT)
        ),
        codec_field!(
            "DefaultSampleFlags",
            7,
            with_bit_width(32),
            with_required_flags(TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT),
            as_hex()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Track header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tkhd {
    full_box: FullBoxState,
    pub creation_time_v0: u32,
    pub modification_time_v0: u32,
    pub creation_time_v1: u64,
    pub modification_time_v1: u64,
    pub track_id: u32,
    pub duration_v0: u32,
    pub duration_v1: u64,
    pub layer: i16,
    pub alternate_group: i16,
    pub volume: i16,
    pub matrix: [i32; 9],
    pub width: u32,
    pub height: u32,
}

impl FieldHooks for Tkhd {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Width" => Some(format_fixed_16_16_unsigned(self.width)),
            "Height" => Some(format_fixed_16_16_unsigned(self.height)),
            _ => None,
        }
    }
}

impl ImmutableBox for Tkhd {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"tkhd")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Tkhd {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Tkhd {
    /// Returns the active track creation time for the current box version.
    pub fn creation_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.creation_time_v0),
            1 => self.creation_time_v1,
            _ => 0,
        }
    }

    /// Returns the active track modification time for the current box version.
    pub fn modification_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.modification_time_v0),
            1 => self.modification_time_v1,
            _ => 0,
        }
    }

    /// Returns the active track duration for the current box version.
    pub fn duration(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.duration_v0),
            1 => self.duration_v1,
            _ => 0,
        }
    }

    /// Returns the track width as an unsigned 16.16 fixed-point value.
    pub fn width_value(&self) -> f64 {
        f64::from(self.width) / 65536.0
    }

    /// Returns the integer component of the track width.
    pub fn width_int(&self) -> u16 {
        (self.width >> 16) as u16
    }

    /// Returns the track height as an unsigned 16.16 fixed-point value.
    pub fn height_value(&self) -> f64 {
        f64::from(self.height) / 65536.0
    }

    /// Returns the integer component of the track height.
    pub fn height_int(&self) -> u16 {
        (self.height >> 16) as u16
    }
}

impl FieldValueRead for Tkhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CreationTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.creation_time_v0))),
            "ModificationTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.modification_time_v0))),
            "CreationTimeV1" => Ok(FieldValue::Unsigned(self.creation_time_v1)),
            "ModificationTimeV1" => Ok(FieldValue::Unsigned(self.modification_time_v1)),
            "TrackID" => Ok(FieldValue::Unsigned(u64::from(self.track_id))),
            "DurationV0" => Ok(FieldValue::Unsigned(u64::from(self.duration_v0))),
            "DurationV1" => Ok(FieldValue::Unsigned(self.duration_v1)),
            "Reserved1" => Ok(FieldValue::Bytes(vec![0; 8])),
            "Layer" => Ok(FieldValue::Signed(i64::from(self.layer))),
            "AlternateGroup" => Ok(FieldValue::Signed(i64::from(self.alternate_group))),
            "Volume" => Ok(FieldValue::Signed(i64::from(self.volume))),
            "Matrix" => Ok(FieldValue::SignedArray(
                self.matrix.iter().copied().map(i64::from).collect(),
            )),
            "Width" => Ok(FieldValue::Unsigned(u64::from(self.width))),
            "Height" => Ok(FieldValue::Unsigned(u64::from(self.height))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Tkhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CreationTimeV0", FieldValue::Unsigned(value)) => {
                self.creation_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ModificationTimeV0", FieldValue::Unsigned(value)) => {
                self.modification_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CreationTimeV1", FieldValue::Unsigned(value)) => {
                self.creation_time_v1 = value;
                Ok(())
            }
            ("ModificationTimeV1", FieldValue::Unsigned(value)) => {
                self.modification_time_v1 = value;
                Ok(())
            }
            ("TrackID", FieldValue::Unsigned(value)) => {
                self.track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DurationV0", FieldValue::Unsigned(value)) => {
                self.duration_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DurationV1", FieldValue::Unsigned(value)) => {
                self.duration_v1 = value;
                Ok(())
            }
            ("Reserved1", FieldValue::Bytes(bytes)) => bytes_to_zeroes(field_name, &bytes, 8),
            ("Layer", FieldValue::Signed(value)) => {
                self.layer = i16_from_signed(field_name, value)?;
                Ok(())
            }
            ("AlternateGroup", FieldValue::Signed(value)) => {
                self.alternate_group = i16_from_signed(field_name, value)?;
                Ok(())
            }
            ("Volume", FieldValue::Signed(value)) => {
                self.volume = i16_from_signed(field_name, value)?;
                Ok(())
            }
            ("Matrix", FieldValue::SignedArray(values)) => {
                if values.len() != 9 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 9 elements",
                    ));
                }
                for (slot, value) in self.matrix.iter_mut().zip(values.into_iter()) {
                    *slot = i32_from_signed(field_name, value)?;
                }
                Ok(())
            }
            ("Width", FieldValue::Unsigned(value)) => {
                self.width = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Height", FieldValue::Unsigned(value)) => {
                self.height = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Tkhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("CreationTimeV0", 2, with_bit_width(32), with_version(0)),
        codec_field!("ModificationTimeV0", 3, with_bit_width(32), with_version(0)),
        codec_field!("CreationTimeV1", 4, with_bit_width(64), with_version(1)),
        codec_field!("ModificationTimeV1", 5, with_bit_width(64), with_version(1)),
        codec_field!("TrackID", 6, with_bit_width(32)),
        codec_field!("Reserved0", 7, with_bit_width(32), with_constant("0")),
        codec_field!("DurationV0", 8, with_bit_width(32), with_version(0)),
        codec_field!("DurationV1", 9, with_bit_width(64), with_version(1)),
        codec_field!(
            "Reserved1",
            10,
            with_bit_width(8),
            with_length(8),
            as_bytes(),
            as_hidden()
        ),
        codec_field!("Layer", 11, with_bit_width(16), as_signed()),
        codec_field!("AlternateGroup", 12, with_bit_width(16), as_signed()),
        codec_field!("Volume", 13, with_bit_width(16), as_signed()),
        codec_field!("Reserved2", 14, with_bit_width(16), with_constant("0")),
        codec_field!(
            "Matrix",
            15,
            with_bit_width(32),
            with_length(9),
            as_signed(),
            as_hex()
        ),
        codec_field!("Width", 16, with_bit_width(32)),
        codec_field!("Height", 17, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Track extension properties box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trep {
    full_box: FullBoxState,
    pub track_id: u32,
}

impl_full_box!(Trep, *b"trep");

impl FieldValueRead for Trep {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "TrackID" => Ok(FieldValue::Unsigned(u64::from(self.track_id))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Trep {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("TrackID", FieldValue::Unsigned(value)) => {
                self.track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Trep {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("TrackID", 2, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Track extends defaults box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trex {
    full_box: FullBoxState,
    pub track_id: u32,
    pub default_sample_description_index: u32,
    pub default_sample_duration: u32,
    pub default_sample_size: u32,
    pub default_sample_flags: u32,
}

impl_full_box!(Trex, *b"trex");

impl FieldValueRead for Trex {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "TrackID" => Ok(FieldValue::Unsigned(u64::from(self.track_id))),
            "DefaultSampleDescriptionIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.default_sample_description_index,
            ))),
            "DefaultSampleDuration" => Ok(FieldValue::Unsigned(u64::from(
                self.default_sample_duration,
            ))),
            "DefaultSampleSize" => Ok(FieldValue::Unsigned(u64::from(self.default_sample_size))),
            "DefaultSampleFlags" => Ok(FieldValue::Unsigned(u64::from(self.default_sample_flags))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Trex {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("TrackID", FieldValue::Unsigned(value)) => {
                self.track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleDescriptionIndex", FieldValue::Unsigned(value)) => {
                self.default_sample_description_index = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleDuration", FieldValue::Unsigned(value)) => {
                self.default_sample_duration = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleSize", FieldValue::Unsigned(value)) => {
                self.default_sample_size = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleFlags", FieldValue::Unsigned(value)) => {
                self.default_sample_flags = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Trex {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("TrackID", 2, with_bit_width(32)),
        codec_field!("DefaultSampleDescriptionIndex", 3, with_bit_width(32)),
        codec_field!("DefaultSampleDuration", 4, with_bit_width(32)),
        codec_field!("DefaultSampleSize", 5, with_bit_width(32)),
        codec_field!("DefaultSampleFlags", 6, with_bit_width(32), as_hex()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Video media header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vmhd {
    full_box: FullBoxState,
    pub graphicsmode: u16,
    pub opcolor: [u16; 3],
}

impl_full_box!(Vmhd, *b"vmhd");

impl FieldValueRead for Vmhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Graphicsmode" => Ok(FieldValue::Unsigned(u64::from(self.graphicsmode))),
            "Opcolor" => Ok(FieldValue::UnsignedArray(
                self.opcolor.iter().copied().map(u64::from).collect(),
            )),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Vmhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Graphicsmode", FieldValue::Unsigned(value)) => {
                self.graphicsmode = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Opcolor", FieldValue::UnsignedArray(values)) => {
                if values.len() != 3 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 3 elements",
                    ));
                }
                self.opcolor = [
                    u16_from_unsigned(field_name, values[0])?,
                    u16_from_unsigned(field_name, values[1])?,
                    u16_from_unsigned(field_name, values[2])?,
                ];
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Vmhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Graphicsmode", 2, with_bit_width(16)),
        codec_field!("Opcolor", 3, with_bit_width(16), with_length(3)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Sound media header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Smhd {
    full_box: FullBoxState,
    pub balance: i16,
}

impl FieldHooks for Smhd {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Balance" => Some(format_fixed_8_8_signed(self.balance)),
            _ => None,
        }
    }
}

impl ImmutableBox for Smhd {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"smhd")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Smhd {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Smhd {
    /// Returns the balance as a signed 8.8 fixed-point value.
    pub fn balance_value(&self) -> f32 {
        f32::from(self.balance) / 256.0
    }

    /// Returns the integer component of the balance.
    pub fn balance_int(&self) -> i8 {
        (self.balance >> 8) as i8
    }
}

impl FieldValueRead for Smhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Balance" => Ok(FieldValue::Signed(i64::from(self.balance))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Smhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Balance", FieldValue::Signed(value)) => {
                self.balance = i16_from_signed(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Smhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Balance", 2, with_bit_width(16), as_signed()),
        codec_field!("Reserved", 3, with_bit_width(16), with_constant("0")),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Sample description box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stsd {
    full_box: FullBoxState,
    pub entry_count: u32,
}

impl_full_box!(Stsd, *b"stsd");

impl FieldValueRead for Stsd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Stsd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Stsd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Composition-to-decode timeline shift box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cslg {
    full_box: FullBoxState,
    pub composition_to_dts_shift_v0: i32,
    pub least_decode_to_display_delta_v0: i32,
    pub greatest_decode_to_display_delta_v0: i32,
    pub composition_start_time_v0: i32,
    pub composition_end_time_v0: i32,
    pub composition_to_dts_shift_v1: i64,
    pub least_decode_to_display_delta_v1: i64,
    pub greatest_decode_to_display_delta_v1: i64,
    pub composition_start_time_v1: i64,
    pub composition_end_time_v1: i64,
}

impl_full_box!(Cslg, *b"cslg");

impl Cslg {
    /// Returns the active composition-to-decode shift for the current box version.
    pub fn composition_to_dts_shift(&self) -> i64 {
        match self.version() {
            0 => i64::from(self.composition_to_dts_shift_v0),
            1 => self.composition_to_dts_shift_v1,
            _ => 0,
        }
    }

    /// Returns the active least decode-to-display delta for the current box version.
    pub fn least_decode_to_display_delta(&self) -> i64 {
        match self.version() {
            0 => i64::from(self.least_decode_to_display_delta_v0),
            1 => self.least_decode_to_display_delta_v1,
            _ => 0,
        }
    }

    /// Returns the active greatest decode-to-display delta for the current box version.
    pub fn greatest_decode_to_display_delta(&self) -> i64 {
        match self.version() {
            0 => i64::from(self.greatest_decode_to_display_delta_v0),
            1 => self.greatest_decode_to_display_delta_v1,
            _ => 0,
        }
    }

    /// Returns the active composition start time for the current box version.
    pub fn composition_start_time(&self) -> i64 {
        match self.version() {
            0 => i64::from(self.composition_start_time_v0),
            1 => self.composition_start_time_v1,
            _ => 0,
        }
    }

    /// Returns the active composition end time for the current box version.
    pub fn composition_end_time(&self) -> i64 {
        match self.version() {
            0 => i64::from(self.composition_end_time_v0),
            1 => self.composition_end_time_v1,
            _ => 0,
        }
    }
}

impl FieldValueRead for Cslg {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CompositionToDTSShiftV0" => Ok(FieldValue::Signed(i64::from(
                self.composition_to_dts_shift_v0,
            ))),
            "LeastDecodeToDisplayDeltaV0" => Ok(FieldValue::Signed(i64::from(
                self.least_decode_to_display_delta_v0,
            ))),
            "GreatestDecodeToDisplayDeltaV0" => Ok(FieldValue::Signed(i64::from(
                self.greatest_decode_to_display_delta_v0,
            ))),
            "CompositionStartTimeV0" => Ok(FieldValue::Signed(i64::from(
                self.composition_start_time_v0,
            ))),
            "CompositionEndTimeV0" => {
                Ok(FieldValue::Signed(i64::from(self.composition_end_time_v0)))
            }
            "CompositionToDTSShiftV1" => Ok(FieldValue::Signed(self.composition_to_dts_shift_v1)),
            "LeastDecodeToDisplayDeltaV1" => {
                Ok(FieldValue::Signed(self.least_decode_to_display_delta_v1))
            }
            "GreatestDecodeToDisplayDeltaV1" => {
                Ok(FieldValue::Signed(self.greatest_decode_to_display_delta_v1))
            }
            "CompositionStartTimeV1" => Ok(FieldValue::Signed(self.composition_start_time_v1)),
            "CompositionEndTimeV1" => Ok(FieldValue::Signed(self.composition_end_time_v1)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Cslg {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CompositionToDTSShiftV0", FieldValue::Signed(value)) => {
                self.composition_to_dts_shift_v0 = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("LeastDecodeToDisplayDeltaV0", FieldValue::Signed(value)) => {
                self.least_decode_to_display_delta_v0 = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("GreatestDecodeToDisplayDeltaV0", FieldValue::Signed(value)) => {
                self.greatest_decode_to_display_delta_v0 = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("CompositionStartTimeV0", FieldValue::Signed(value)) => {
                self.composition_start_time_v0 = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("CompositionEndTimeV0", FieldValue::Signed(value)) => {
                self.composition_end_time_v0 = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("CompositionToDTSShiftV1", FieldValue::Signed(value)) => {
                self.composition_to_dts_shift_v1 = i64_from_signed(field_name, value)?;
                Ok(())
            }
            ("LeastDecodeToDisplayDeltaV1", FieldValue::Signed(value)) => {
                self.least_decode_to_display_delta_v1 = i64_from_signed(field_name, value)?;
                Ok(())
            }
            ("GreatestDecodeToDisplayDeltaV1", FieldValue::Signed(value)) => {
                self.greatest_decode_to_display_delta_v1 = i64_from_signed(field_name, value)?;
                Ok(())
            }
            ("CompositionStartTimeV1", FieldValue::Signed(value)) => {
                self.composition_start_time_v1 = i64_from_signed(field_name, value)?;
                Ok(())
            }
            ("CompositionEndTimeV1", FieldValue::Signed(value)) => {
                self.composition_end_time_v1 = i64_from_signed(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Cslg {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "CompositionToDTSShiftV0",
            2,
            with_bit_width(32),
            with_version(0),
            as_signed()
        ),
        codec_field!(
            "LeastDecodeToDisplayDeltaV0",
            3,
            with_bit_width(32),
            with_version(0),
            as_signed()
        ),
        codec_field!(
            "GreatestDecodeToDisplayDeltaV0",
            4,
            with_bit_width(32),
            with_version(0),
            as_signed()
        ),
        codec_field!(
            "CompositionStartTimeV0",
            5,
            with_bit_width(32),
            with_version(0),
            as_signed()
        ),
        codec_field!(
            "CompositionEndTimeV0",
            6,
            with_bit_width(32),
            with_version(0),
            as_signed()
        ),
        codec_field!(
            "CompositionToDTSShiftV1",
            7,
            with_bit_width(64),
            with_version(1),
            as_signed()
        ),
        codec_field!(
            "LeastDecodeToDisplayDeltaV1",
            8,
            with_bit_width(64),
            with_version(1),
            as_signed()
        ),
        codec_field!(
            "GreatestDecodeToDisplayDeltaV1",
            9,
            with_bit_width(64),
            with_version(1),
            as_signed()
        ),
        codec_field!(
            "CompositionStartTimeV1",
            10,
            with_bit_width(64),
            with_version(1),
            as_signed()
        ),
        codec_field!(
            "CompositionEndTimeV1",
            11,
            with_bit_width(64),
            with_version(1),
            as_signed()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// One composition-offset run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CttsEntry {
    pub sample_count: u32,
    pub sample_offset_v0: u32,
    pub sample_offset_v1: i32,
}

/// Composition time to sample box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ctts {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub entries: Vec<CttsEntry>,
}

impl FieldHooks for Ctts {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "Entries" => usize::try_from(self.entry_count)
                .ok()
                .and_then(|count| field_len_bytes(count, 8)),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(
                |entry| match self.version() {
                    0 => format!(
                        "{{SampleCount={} SampleOffsetV0={}}}",
                        entry.sample_count, entry.sample_offset_v0
                    ),
                    1 => format!(
                        "{{SampleCount={} SampleOffsetV1={}}}",
                        entry.sample_count, entry.sample_offset_v1
                    ),
                    _ => String::from("{}"),
                },
            ))),
            _ => None,
        }
    }
}

impl ImmutableBox for Ctts {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"ctts")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Ctts {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Ctts {
    /// Returns the active sample offset for `index`.
    pub fn sample_offset(&self, index: usize) -> i64 {
        match self.version() {
            0 => i64::from(self.entries[index].sample_offset_v0),
            1 => i64::from(self.entries[index].sample_offset_v1),
            _ => 0,
        }
    }
}

impl FieldValueRead for Ctts {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "Entries" => {
                let mut bytes = Vec::with_capacity(self.entries.len() * 8);
                for entry in &self.entries {
                    bytes.extend_from_slice(&entry.sample_count.to_be_bytes());
                    match self.version() {
                        0 => bytes.extend_from_slice(&entry.sample_offset_v0.to_be_bytes()),
                        1 => bytes.extend_from_slice(&entry.sample_offset_v1.to_be_bytes()),
                        _ => {}
                    }
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Ctts {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                self.entries =
                    parse_fixed_chunks(field_name, &bytes, 8, |chunk| match self.version() {
                        0 => CttsEntry {
                            sample_count: read_u32(chunk, 0),
                            sample_offset_v0: read_u32(chunk, 4),
                            ..CttsEntry::default()
                        },
                        1 => CttsEntry {
                            sample_count: read_u32(chunk, 0),
                            sample_offset_v1: read_i32(chunk, 4),
                            ..CttsEntry::default()
                        },
                        _ => CttsEntry::default(),
                    })?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Ctts {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!(
            "Entries",
            3,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// One edit-list entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElstEntry {
    pub segment_duration_v0: u32,
    pub media_time_v0: i32,
    pub segment_duration_v1: u64,
    pub media_time_v1: i64,
    pub media_rate_integer: i16,
}

/// Edit list box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Elst {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub entries: Vec<ElstEntry>,
}

impl FieldHooks for Elst {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "Entries" => match self.version() {
                0 => usize::try_from(self.entry_count)
                    .ok()
                    .and_then(|count| field_len_bytes(count, 12)),
                1 => usize::try_from(self.entry_count)
                    .ok()
                    .and_then(|count| field_len_bytes(count, 20)),
                _ => Some(0),
            },
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(
                |entry| match self.version() {
                    0 => format!(
                        "{{SegmentDurationV0={} MediaTimeV0={} MediaRateInteger={}}}",
                        entry.segment_duration_v0, entry.media_time_v0, entry.media_rate_integer
                    ),
                    1 => format!(
                        "{{SegmentDurationV1={} MediaTimeV1={} MediaRateInteger={}}}",
                        entry.segment_duration_v1, entry.media_time_v1, entry.media_rate_integer
                    ),
                    _ => String::from("{}"),
                },
            ))),
            _ => None,
        }
    }
}

impl ImmutableBox for Elst {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"elst")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Elst {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Elst {
    /// Returns the active segment duration for `index`.
    pub fn segment_duration(&self, index: usize) -> u64 {
        match self.version() {
            0 => u64::from(self.entries[index].segment_duration_v0),
            1 => self.entries[index].segment_duration_v1,
            _ => 0,
        }
    }

    /// Returns the active media time for `index`.
    pub fn media_time(&self, index: usize) -> i64 {
        match self.version() {
            0 => i64::from(self.entries[index].media_time_v0),
            1 => self.entries[index].media_time_v1,
            _ => 0,
        }
    }
}

impl FieldValueRead for Elst {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "Entries" => {
                let mut bytes = Vec::new();
                for entry in &self.entries {
                    match self.version() {
                        0 => {
                            bytes.extend_from_slice(&entry.segment_duration_v0.to_be_bytes());
                            bytes.extend_from_slice(&entry.media_time_v0.to_be_bytes());
                        }
                        1 => {
                            bytes.extend_from_slice(&entry.segment_duration_v1.to_be_bytes());
                            bytes.extend_from_slice(&entry.media_time_v1.to_be_bytes());
                        }
                        _ => {}
                    }
                    bytes.extend_from_slice(&entry.media_rate_integer.to_be_bytes());
                    bytes.extend_from_slice(&0_i16.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Elst {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                self.entries = match self.version() {
                    0 => parse_fixed_chunks(field_name, &bytes, 12, |chunk| ElstEntry {
                        segment_duration_v0: read_u32(chunk, 0),
                        media_time_v0: read_i32(chunk, 4),
                        media_rate_integer: read_i16(chunk, 8),
                        ..ElstEntry::default()
                    })?,
                    1 => parse_fixed_chunks(field_name, &bytes, 20, |chunk| ElstEntry {
                        segment_duration_v1: read_u64(chunk, 0),
                        media_time_v1: read_i64(chunk, 8),
                        media_rate_integer: read_i16(chunk, 16),
                        ..ElstEntry::default()
                    })?,
                    _ => Vec::new(),
                };
                for chunk in bytes.chunks_exact(match self.version() {
                    0 => 12,
                    1 => 20,
                    _ => 1,
                }) {
                    let offset = chunk.len() - 2;
                    if read_i16(chunk, offset) != 0 {
                        return Err(invalid_value(
                            field_name,
                            "media rate fraction must be zero",
                        ));
                    }
                }
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Elst {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!(
            "Entries",
            3,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// 64-bit chunk offset box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Co64 {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub chunk_offset: Vec<u64>,
}

impl_full_box!(Co64, *b"co64");

impl FieldHooks for Co64 {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "ChunkOffset" => Some(self.entry_count),
            _ => None,
        }
    }
}

impl FieldValueRead for Co64 {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "ChunkOffset" => Ok(FieldValue::UnsignedArray(self.chunk_offset.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Co64 {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChunkOffset", FieldValue::UnsignedArray(values)) => {
                self.chunk_offset = values;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Co64 {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!("ChunkOffset", 3, with_bit_width(64), with_dynamic_length()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// 32-bit chunk offset box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stco {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub chunk_offset: Vec<u64>,
}

impl_full_box!(Stco, *b"stco");

impl FieldHooks for Stco {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "ChunkOffset" => Some(self.entry_count),
            _ => None,
        }
    }
}

impl FieldValueRead for Stco {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "ChunkOffset" => Ok(FieldValue::UnsignedArray(self.chunk_offset.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Stco {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChunkOffset", FieldValue::UnsignedArray(values)) => {
                let mut offsets = Vec::with_capacity(values.len());
                for value in values {
                    offsets.push(u64::from(u32_from_unsigned(field_name, value)?));
                }
                self.chunk_offset = offsets;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Stco {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!("ChunkOffset", 3, with_bit_width(32), with_dynamic_length()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// One sample-to-chunk entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StscEntry {
    pub first_chunk: u32,
    pub samples_per_chunk: u32,
    pub sample_description_index: u32,
}

/// Sample-to-chunk box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stsc {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub entries: Vec<StscEntry>,
}

impl FieldHooks for Stsc {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "Entries" => usize::try_from(self.entry_count)
                .ok()
                .and_then(|count| field_len_bytes(count, 12)),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(|entry| {
                format!(
                    "{{FirstChunk={} SamplesPerChunk={} SampleDescriptionIndex={}}}",
                    entry.first_chunk, entry.samples_per_chunk, entry.sample_description_index
                )
            }))),
            _ => None,
        }
    }
}

impl ImmutableBox for Stsc {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"stsc")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Stsc {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl FieldValueRead for Stsc {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "Entries" => {
                let mut bytes = Vec::with_capacity(self.entries.len() * 12);
                for entry in &self.entries {
                    bytes.extend_from_slice(&entry.first_chunk.to_be_bytes());
                    bytes.extend_from_slice(&entry.samples_per_chunk.to_be_bytes());
                    bytes.extend_from_slice(&entry.sample_description_index.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Stsc {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                self.entries = parse_fixed_chunks(field_name, &bytes, 12, |chunk| StscEntry {
                    first_chunk: read_u32(chunk, 0),
                    samples_per_chunk: read_u32(chunk, 4),
                    sample_description_index: read_u32(chunk, 8),
                })?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Stsc {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!(
            "Entries",
            3,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Sync sample box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stss {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub sample_number: Vec<u64>,
}

impl_full_box!(Stss, *b"stss");

impl FieldHooks for Stss {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "SampleNumber" => Some(self.entry_count),
            _ => None,
        }
    }
}

impl FieldValueRead for Stss {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "SampleNumber" => Ok(FieldValue::UnsignedArray(self.sample_number.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Stss {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleNumber", FieldValue::UnsignedArray(values)) => {
                let mut numbers = Vec::with_capacity(values.len());
                for value in values {
                    numbers.push(u64::from(u32_from_unsigned(field_name, value)?));
                }
                self.sample_number = numbers;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Stss {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!("SampleNumber", 3, with_bit_width(32), with_dynamic_length()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Sample size box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stsz {
    full_box: FullBoxState,
    pub sample_size: u32,
    pub sample_count: u32,
    pub entry_size: Vec<u64>,
}

impl FieldHooks for Stsz {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "EntrySize" => {
                if self.sample_size == 0 {
                    Some(self.sample_count)
                } else {
                    Some(0)
                }
            }
            _ => None,
        }
    }

    fn display_field(&self, _name: &'static str) -> Option<String> {
        None
    }
}

impl ImmutableBox for Stsz {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"stsz")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Stsz {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl FieldValueRead for Stsz {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SampleSize" => Ok(FieldValue::Unsigned(u64::from(self.sample_size))),
            "SampleCount" => Ok(FieldValue::Unsigned(u64::from(self.sample_count))),
            "EntrySize" => Ok(FieldValue::UnsignedArray(self.entry_size.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Stsz {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SampleSize", FieldValue::Unsigned(value)) => {
                self.sample_size = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleCount", FieldValue::Unsigned(value)) => {
                self.sample_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EntrySize", FieldValue::UnsignedArray(values)) => {
                let mut entry_size = Vec::with_capacity(values.len());
                for value in values {
                    entry_size.push(u64::from(u32_from_unsigned(field_name, value)?));
                }
                self.entry_size = entry_size;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Stsz {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("SampleSize", 2, with_bit_width(32)),
        codec_field!("SampleCount", 3, with_bit_width(32)),
        codec_field!("EntrySize", 4, with_bit_width(32), with_dynamic_length()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// One time-to-sample entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SttsEntry {
    pub sample_count: u32,
    pub sample_delta: u32,
}

/// Time to sample box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stts {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub entries: Vec<SttsEntry>,
}

impl FieldHooks for Stts {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "Entries" => usize::try_from(self.entry_count)
                .ok()
                .and_then(|count| field_len_bytes(count, 8)),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(|entry| {
                format!(
                    "{{SampleCount={} SampleDelta={}}}",
                    entry.sample_count, entry.sample_delta
                )
            }))),
            _ => None,
        }
    }
}

impl ImmutableBox for Stts {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"stts")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Stts {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl FieldValueRead for Stts {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "Entries" => {
                let mut bytes = Vec::with_capacity(self.entries.len() * 8);
                for entry in &self.entries {
                    bytes.extend_from_slice(&entry.sample_count.to_be_bytes());
                    bytes.extend_from_slice(&entry.sample_delta.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Stts {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                self.entries = parse_fixed_chunks(field_name, &bytes, 8, |chunk| SttsEntry {
                    sample_count: read_u32(chunk, 0),
                    sample_delta: read_u32(chunk, 4),
                })?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Stts {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!(
            "Entries",
            3,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// One track-run sample entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrunEntry {
    pub sample_duration: u32,
    pub sample_size: u32,
    pub sample_flags: u32,
    pub sample_composition_time_offset_v0: u32,
    pub sample_composition_time_offset_v1: i32,
}

/// Track run box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trun {
    full_box: FullBoxState,
    pub sample_count: u32,
    pub data_offset: i32,
    pub first_sample_flags: u32,
    pub entries: Vec<TrunEntry>,
}

impl FieldHooks for Trun {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "Entries" => {
                let mut bytes_per_entry = 0usize;
                if self.flags() & TRUN_SAMPLE_DURATION_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                if self.flags() & TRUN_SAMPLE_SIZE_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                if self.flags() & TRUN_SAMPLE_FLAGS_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                if self.flags() & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                usize::try_from(self.sample_count)
                    .ok()
                    .and_then(|count| field_len_bytes(count, bytes_per_entry))
            }
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(|entry| {
                let mut fields = Vec::new();
                if self.flags() & TRUN_SAMPLE_DURATION_PRESENT != 0 {
                    fields.push(format!("SampleDuration={}", entry.sample_duration));
                }
                if self.flags() & TRUN_SAMPLE_SIZE_PRESENT != 0 {
                    fields.push(format!("SampleSize={}", entry.sample_size));
                }
                if self.flags() & TRUN_SAMPLE_FLAGS_PRESENT != 0 {
                    fields.push(format!("SampleFlags=0x{:x}", entry.sample_flags));
                }
                if self.flags() & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0 {
                    if self.version() == 0 {
                        fields.push(format!(
                            "SampleCompositionTimeOffsetV0={}",
                            entry.sample_composition_time_offset_v0
                        ));
                    } else {
                        fields.push(format!(
                            "SampleCompositionTimeOffsetV1={}",
                            entry.sample_composition_time_offset_v1
                        ));
                    }
                }
                format!("{{{}}}", fields.join(" "))
            }))),
            _ => None,
        }
    }
}

impl ImmutableBox for Trun {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"trun")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Trun {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Trun {
    /// Returns the active composition time offset for `index`.
    pub fn sample_composition_time_offset(&self, index: usize) -> i64 {
        match self.version() {
            0 => i64::from(self.entries[index].sample_composition_time_offset_v0),
            1 => i64::from(self.entries[index].sample_composition_time_offset_v1),
            _ => 0,
        }
    }
}

impl FieldValueRead for Trun {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SampleCount" => Ok(FieldValue::Unsigned(u64::from(self.sample_count))),
            "DataOffset" => Ok(FieldValue::Signed(i64::from(self.data_offset))),
            "FirstSampleFlags" => Ok(FieldValue::Unsigned(u64::from(self.first_sample_flags))),
            "Entries" => {
                let mut bytes = Vec::new();
                for entry in &self.entries {
                    if self.flags() & TRUN_SAMPLE_DURATION_PRESENT != 0 {
                        bytes.extend_from_slice(&entry.sample_duration.to_be_bytes());
                    }
                    if self.flags() & TRUN_SAMPLE_SIZE_PRESENT != 0 {
                        bytes.extend_from_slice(&entry.sample_size.to_be_bytes());
                    }
                    if self.flags() & TRUN_SAMPLE_FLAGS_PRESENT != 0 {
                        bytes.extend_from_slice(&entry.sample_flags.to_be_bytes());
                    }
                    if self.flags() & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0 {
                        if self.version() == 0 {
                            bytes.extend_from_slice(
                                &entry.sample_composition_time_offset_v0.to_be_bytes(),
                            );
                        } else {
                            bytes.extend_from_slice(
                                &entry.sample_composition_time_offset_v1.to_be_bytes(),
                            );
                        }
                    }
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Trun {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SampleCount", FieldValue::Unsigned(value)) => {
                self.sample_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataOffset", FieldValue::Signed(value)) => {
                self.data_offset = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("FirstSampleFlags", FieldValue::Unsigned(value)) => {
                self.first_sample_flags = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                let mut bytes_per_entry = 0usize;
                if self.flags() & TRUN_SAMPLE_DURATION_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                if self.flags() & TRUN_SAMPLE_SIZE_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                if self.flags() & TRUN_SAMPLE_FLAGS_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                if self.flags() & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0 {
                    bytes_per_entry += 4;
                }

                self.entries = if bytes_per_entry == 0 {
                    Vec::new()
                } else {
                    parse_fixed_chunks(field_name, &bytes, bytes_per_entry, |chunk| {
                        let mut offset = 0;
                        let mut entry = TrunEntry::default();
                        if self.flags() & TRUN_SAMPLE_DURATION_PRESENT != 0 {
                            entry.sample_duration = read_u32(chunk, offset);
                            offset += 4;
                        }
                        if self.flags() & TRUN_SAMPLE_SIZE_PRESENT != 0 {
                            entry.sample_size = read_u32(chunk, offset);
                            offset += 4;
                        }
                        if self.flags() & TRUN_SAMPLE_FLAGS_PRESENT != 0 {
                            entry.sample_flags = read_u32(chunk, offset);
                            offset += 4;
                        }
                        if self.flags() & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0 {
                            if self.version() == 0 {
                                entry.sample_composition_time_offset_v0 = read_u32(chunk, offset);
                            } else {
                                entry.sample_composition_time_offset_v1 = read_i32(chunk, offset);
                            }
                        }
                        entry
                    })?
                };
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Trun {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("SampleCount", 2, with_bit_width(32)),
        codec_field!(
            "DataOffset",
            3,
            with_bit_width(32),
            as_signed(),
            with_required_flags(TRUN_DATA_OFFSET_PRESENT)
        ),
        codec_field!(
            "FirstSampleFlags",
            4,
            with_bit_width(32),
            with_required_flags(TRUN_FIRST_SAMPLE_FLAGS_PRESENT),
            as_hex()
        ),
        codec_field!(
            "Entries",
            5,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

simple_container_box!(Schi, *b"schi");
simple_container_box!(Sinf, *b"sinf");
simple_container_box!(Wave, *b"wave");

/// Metadata box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    full_box: FullBoxState,
    quicktime_headerless: bool,
}

impl FieldHooks for Meta {
    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "Version" | "Flags" => Some(!self.quicktime_headerless),
            _ => None,
        }
    }
}

impl ImmutableBox for Meta {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"meta")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Meta {
    fn set_version(&mut self, version: u8) {
        self.quicktime_headerless = false;
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.quicktime_headerless = false;
        self.full_box.flags = flags;
    }

    fn before_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<(), CodecError> {
        self.quicktime_headerless = false;
        if payload_size < 4 {
            return Ok(());
        }

        // Headerless metadata starts directly with the first child box type instead of the full-box prefix.
        let start = reader.stream_position()?;
        let mut prefix = [0_u8; 4];
        reader.read_exact(&mut prefix)?;
        reader.seek(SeekFrom::Start(start))?;

        if prefix.iter().any(|byte| *byte != 0) {
            self.quicktime_headerless = true;
            self.full_box.version = 0;
            self.full_box.flags = 0;
        }

        Ok(())
    }
}

impl Meta {
    /// Returns `true` when the payload omits the normal full-box header bytes.
    pub fn is_quicktime_headerless(&self) -> bool {
        self.quicktime_headerless
    }
}

impl FieldValueRead for Meta {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        Err(missing_field(field_name))
    }
}

impl FieldValueWrite for Meta {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        Err(unexpected_field(field_name, value))
    }
}

impl CodecBox for Meta {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!(
            "Version",
            0,
            with_bit_width(8),
            as_version_field(),
            with_dynamic_presence()
        ),
        codec_field!(
            "Flags",
            1,
            with_bit_width(24),
            as_flags_field(),
            with_dynamic_presence()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Handler reference box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hdlr {
    full_box: FullBoxState,
    pub pre_defined: u32,
    pub handler_type: FourCc,
    pub reserved: [u8; 12],
    pub name: String,
}

impl Default for Hdlr {
    fn default() -> Self {
        Self {
            full_box: FullBoxState::default(),
            pre_defined: 0,
            handler_type: FourCc::ANY,
            reserved: [0; 12],
            name: String::new(),
        }
    }
}

impl FieldHooks for Hdlr {
    fn is_pascal_string(
        &self,
        name: &'static str,
        _data: &[u8],
        remaining_bytes: u64,
    ) -> Option<bool> {
        match name {
            // Some files store the handler name as a Pascal string and consume the last payload byte with the length prefix.
            "Name" => Some(self.pre_defined != 0 && remaining_bytes == 0),
            _ => None,
        }
    }

    fn consume_remaining_bytes_after_string(&self, name: &'static str) -> Option<bool> {
        match name {
            // Handler names may be padded after the visible terminator, so keep consuming the declared field payload.
            "Name" => Some(true),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "HandlerType" => Some(quoted_fourcc(self.handler_type)),
            _ => None,
        }
    }
}

impl_full_box!(Hdlr, *b"hdlr");

impl FieldValueRead for Hdlr {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "PreDefined" => Ok(FieldValue::Unsigned(u64::from(self.pre_defined))),
            "HandlerType" => Ok(FieldValue::Bytes(self.handler_type.as_bytes().to_vec())),
            "Reserved" => Ok(FieldValue::Bytes(self.reserved.to_vec())),
            "Name" => Ok(FieldValue::String(self.name.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Hdlr {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("PreDefined", FieldValue::Unsigned(value)) => {
                self.pre_defined = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("HandlerType", FieldValue::Bytes(bytes)) => {
                self.handler_type = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("Reserved", FieldValue::Bytes(bytes)) => {
                if bytes.len() != 12 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 12 bytes",
                    ));
                }
                self.reserved.copy_from_slice(&bytes);
                Ok(())
            }
            ("Name", FieldValue::String(value)) => {
                self.name = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Hdlr {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("PreDefined", 2, with_bit_width(32)),
        codec_field!(
            "HandlerType",
            3,
            with_bit_width(8),
            with_length(4),
            as_bytes()
        ),
        codec_field!(
            "Reserved",
            4,
            with_bit_width(8),
            with_length(12),
            as_bytes(),
            as_hidden()
        ),
        codec_field!(
            "Name",
            5,
            with_bit_width(8),
            as_string(StringFieldMode::PascalCompatible)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Auxiliary information offsets box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saio {
    full_box: FullBoxState,
    pub aux_info_type: FourCc,
    pub aux_info_type_parameter: u32,
    pub entry_count: u32,
    pub offset_v0: Vec<u64>,
    pub offset_v1: Vec<u64>,
}

impl Default for Saio {
    fn default() -> Self {
        Self {
            full_box: FullBoxState::default(),
            aux_info_type: FourCc::ANY,
            aux_info_type_parameter: 0,
            entry_count: 0,
            offset_v0: Vec::new(),
            offset_v1: Vec::new(),
        }
    }
}

impl FieldHooks for Saio {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "OffsetV0" | "OffsetV1" => Some(self.entry_count),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "AuxInfoType" => Some(quoted_fourcc(self.aux_info_type)),
            _ => None,
        }
    }
}

impl_full_box!(Saio, *b"saio");

impl Saio {
    /// Returns the active auxiliary information offset at `index`.
    pub fn offset(&self, index: usize) -> u64 {
        match self.version() {
            0 => self.offset_v0[index],
            1 => self.offset_v1[index],
            _ => 0,
        }
    }
}

impl FieldValueRead for Saio {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "AuxInfoType" => Ok(FieldValue::Bytes(self.aux_info_type.as_bytes().to_vec())),
            "AuxInfoTypeParameter" => Ok(FieldValue::Unsigned(u64::from(
                self.aux_info_type_parameter,
            ))),
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "OffsetV0" => Ok(FieldValue::UnsignedArray(self.offset_v0.clone())),
            "OffsetV1" => Ok(FieldValue::UnsignedArray(self.offset_v1.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Saio {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("AuxInfoType", FieldValue::Bytes(bytes)) => {
                self.aux_info_type = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("AuxInfoTypeParameter", FieldValue::Unsigned(value)) => {
                self.aux_info_type_parameter = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("OffsetV0", FieldValue::UnsignedArray(values)) => {
                let mut offsets = Vec::with_capacity(values.len());
                for value in values {
                    offsets.push(u64::from(u32_from_unsigned(field_name, value)?));
                }
                self.offset_v0 = offsets;
                Ok(())
            }
            ("OffsetV1", FieldValue::UnsignedArray(values)) => {
                self.offset_v1 = values;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Saio {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "AuxInfoType",
            2,
            with_bit_width(8),
            with_length(4),
            as_bytes(),
            with_required_flags(AUX_INFO_TYPE_PRESENT)
        ),
        codec_field!(
            "AuxInfoTypeParameter",
            3,
            with_bit_width(32),
            as_hex(),
            with_required_flags(AUX_INFO_TYPE_PRESENT)
        ),
        codec_field!("EntryCount", 4, with_bit_width(32)),
        codec_field!(
            "OffsetV0",
            5,
            with_bit_width(32),
            with_dynamic_length(),
            with_version(0)
        ),
        codec_field!(
            "OffsetV1",
            6,
            with_bit_width(64),
            with_dynamic_length(),
            with_version(1)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Auxiliary information sizes box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saiz {
    full_box: FullBoxState,
    pub aux_info_type: FourCc,
    pub aux_info_type_parameter: u32,
    pub default_sample_info_size: u8,
    pub sample_count: u32,
    pub sample_info_size: Vec<u8>,
}

impl Default for Saiz {
    fn default() -> Self {
        Self {
            full_box: FullBoxState::default(),
            aux_info_type: FourCc::ANY,
            aux_info_type_parameter: 0,
            default_sample_info_size: 0,
            sample_count: 0,
            sample_info_size: Vec::new(),
        }
    }
}

impl FieldHooks for Saiz {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "SampleInfoSize" => Some(self.sample_count),
            _ => None,
        }
    }

    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "SampleInfoSize" => Some(self.default_sample_info_size == 0),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "AuxInfoType" => Some(quoted_fourcc(self.aux_info_type)),
            _ => None,
        }
    }
}

impl_full_box!(Saiz, *b"saiz");

impl FieldValueRead for Saiz {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "AuxInfoType" => Ok(FieldValue::Bytes(self.aux_info_type.as_bytes().to_vec())),
            "AuxInfoTypeParameter" => Ok(FieldValue::Unsigned(u64::from(
                self.aux_info_type_parameter,
            ))),
            "DefaultSampleInfoSize" => Ok(FieldValue::Unsigned(u64::from(
                self.default_sample_info_size,
            ))),
            "SampleCount" => Ok(FieldValue::Unsigned(u64::from(self.sample_count))),
            "SampleInfoSize" => Ok(FieldValue::UnsignedArray(
                self.sample_info_size
                    .iter()
                    .copied()
                    .map(u64::from)
                    .collect(),
            )),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Saiz {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("AuxInfoType", FieldValue::Bytes(bytes)) => {
                self.aux_info_type = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("AuxInfoTypeParameter", FieldValue::Unsigned(value)) => {
                self.aux_info_type_parameter = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleInfoSize", FieldValue::Unsigned(value)) => {
                self.default_sample_info_size = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleCount", FieldValue::Unsigned(value)) => {
                self.sample_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleInfoSize", FieldValue::UnsignedArray(values)) => {
                let mut sizes = Vec::with_capacity(values.len());
                for value in values {
                    sizes.push(u8_from_unsigned(field_name, value)?);
                }
                self.sample_info_size = sizes;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Saiz {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "AuxInfoType",
            2,
            with_bit_width(8),
            with_length(4),
            as_bytes(),
            with_required_flags(AUX_INFO_TYPE_PRESENT)
        ),
        codec_field!(
            "AuxInfoTypeParameter",
            3,
            with_bit_width(32),
            as_hex(),
            with_required_flags(AUX_INFO_TYPE_PRESENT)
        ),
        codec_field!("DefaultSampleInfoSize", 4, with_bit_width(8)),
        codec_field!("SampleCount", 5, with_bit_width(32)),
        codec_field!(
            "SampleInfoSize",
            6,
            with_bit_width(8),
            with_dynamic_length(),
            with_dynamic_presence()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// One sample-to-group entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SbgpEntry {
    pub sample_count: u32,
    pub group_description_index: u32,
}

/// Sample-to-group box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sbgp {
    full_box: FullBoxState,
    pub grouping_type: u32,
    pub grouping_type_parameter: u32,
    pub entry_count: u32,
    pub entries: Vec<SbgpEntry>,
}

impl FieldHooks for Sbgp {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(|entry| {
                format!(
                    "{{SampleCount={} GroupDescriptionIndex={}}}",
                    entry.sample_count, entry.group_description_index
                )
            }))),
            _ => None,
        }
    }
}

impl_full_box!(Sbgp, *b"sbgp");

impl FieldValueRead for Sbgp {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "GroupingType" => Ok(FieldValue::Unsigned(u64::from(self.grouping_type))),
            "GroupingTypeParameter" => Ok(FieldValue::Unsigned(u64::from(
                self.grouping_type_parameter,
            ))),
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "Entries" => {
                let mut bytes = Vec::with_capacity(self.entries.len() * 8);
                for entry in &self.entries {
                    bytes.extend_from_slice(&entry.sample_count.to_be_bytes());
                    bytes.extend_from_slice(&entry.group_description_index.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Sbgp {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("GroupingType", FieldValue::Unsigned(value)) => {
                self.grouping_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("GroupingTypeParameter", FieldValue::Unsigned(value)) => {
                self.grouping_type_parameter = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                let expected_len = field_len_from_count(self.entry_count, 8)
                    .map(|len| len as usize)
                    .unwrap_or(0);
                if bytes.len() != expected_len {
                    return Err(invalid_value(
                        field_name,
                        "entry payload length does not match the entry count",
                    ));
                }

                self.entries = parse_fixed_chunks(field_name, &bytes, 8, |chunk| SbgpEntry {
                    sample_count: read_u32(chunk, 0),
                    group_description_index: read_u32(chunk, 4),
                })?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Sbgp {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("GroupingType", 2, with_bit_width(32)),
        codec_field!(
            "GroupingTypeParameter",
            3,
            with_bit_width(32),
            with_version(1)
        ),
        codec_field!("EntryCount", 4, with_bit_width(32)),
        codec_field!("Entries", 5, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// One packed sample dependency entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SdtpSampleElem {
    pub is_leading: u8,
    pub sample_depends_on: u8,
    pub sample_is_depended_on: u8,
    pub sample_has_redundancy: u8,
}

fn encode_sdtp_sample(
    field_name: &'static str,
    sample: &SdtpSampleElem,
) -> Result<u8, FieldValueError> {
    if sample.is_leading > 0x03
        || sample.sample_depends_on > 0x03
        || sample.sample_is_depended_on > 0x03
        || sample.sample_has_redundancy > 0x03
    {
        return Err(invalid_value(
            field_name,
            "sample dependency fields must fit in 2 bits",
        ));
    }

    Ok((sample.is_leading << 6)
        | (sample.sample_depends_on << 4)
        | (sample.sample_is_depended_on << 2)
        | sample.sample_has_redundancy)
}

/// Sample dependency type box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sdtp {
    full_box: FullBoxState,
    pub samples: Vec<SdtpSampleElem>,
}

impl FieldHooks for Sdtp {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Samples" => Some(render_array(self.samples.iter().map(|sample| {
                format!(
                    "{{IsLeading=0x{:x} SampleDependsOn=0x{:x} SampleIsDependedOn=0x{:x} SampleHasRedundancy=0x{:x}}}",
                    sample.is_leading,
                    sample.sample_depends_on,
                    sample.sample_is_depended_on,
                    sample.sample_has_redundancy
                )
            }))),
            _ => None,
        }
    }
}

impl_full_box!(Sdtp, *b"sdtp");

impl FieldValueRead for Sdtp {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Samples" => {
                let mut bytes = Vec::with_capacity(self.samples.len());
                for sample in &self.samples {
                    bytes.push(encode_sdtp_sample(field_name, sample)?);
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Sdtp {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Samples", FieldValue::Bytes(bytes)) => {
                self.samples = bytes
                    .into_iter()
                    .map(|sample| SdtpSampleElem {
                        is_leading: (sample >> 6) & 0x03,
                        sample_depends_on: (sample >> 4) & 0x03,
                        sample_is_depended_on: (sample >> 2) & 0x03,
                        sample_has_redundancy: sample & 0x03,
                    })
                    .collect();
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Sdtp {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Samples", 2, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Length-prefixed roll-distance description.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RollDistanceWithLength {
    pub description_length: u32,
    pub roll_distance: i16,
}

/// Optional alternative-startup sample counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlternativeStartupEntryOpt {
    pub num_output_samples: u16,
    pub num_total_samples: u16,
}

/// Alternative-startup group description payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlternativeStartupEntry {
    pub roll_count: u16,
    pub first_output_sample: u16,
    pub sample_offset: Vec<u32>,
    pub opts: Vec<AlternativeStartupEntryOpt>,
}

/// Length-prefixed alternative-startup description.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlternativeStartupEntryL {
    pub description_length: u32,
    pub alternative_startup_entry: AlternativeStartupEntry,
}

/// Visual random-access group description payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VisualRandomAccessEntry {
    pub num_leading_samples_known: bool,
    pub num_leading_samples: u8,
}

/// Length-prefixed visual random-access description.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VisualRandomAccessEntryL {
    pub description_length: u32,
    pub visual_random_access_entry: VisualRandomAccessEntry,
}

/// Temporal-level group description payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemporalLevelEntry {
    pub level_independently_decodable: bool,
}

/// Length-prefixed temporal-level description.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemporalLevelEntryL {
    pub description_length: u32,
    pub temporal_level_entry: TemporalLevelEntry,
}

fn format_alternative_startup_opts(opts: &[AlternativeStartupEntryOpt]) -> String {
    render_array(opts.iter().map(|opt| {
        format!(
            "{{NumOutputSamples={} NumTotalSamples={}}}",
            opt.num_output_samples, opt.num_total_samples
        )
    }))
}

fn format_alternative_startup_entry(entry: &AlternativeStartupEntry) -> String {
    format!(
        "{{RollCount={} FirstOutputSample={} SampleOffset={} Opts={}}}",
        entry.roll_count,
        entry.first_output_sample,
        render_array(entry.sample_offset.iter().map(|offset| offset.to_string())),
        format_alternative_startup_opts(&entry.opts)
    )
}

fn encode_alternative_startup_entry(
    field_name: &'static str,
    entry: &AlternativeStartupEntry,
) -> Result<Vec<u8>, FieldValueError> {
    require_count(
        field_name,
        u32::from(entry.roll_count),
        entry.sample_offset.len(),
    )?;

    let mut bytes = Vec::with_capacity(4 + (entry.sample_offset.len() + entry.opts.len()) * 4);
    bytes.extend_from_slice(&entry.roll_count.to_be_bytes());
    bytes.extend_from_slice(&entry.first_output_sample.to_be_bytes());
    for sample_offset in &entry.sample_offset {
        bytes.extend_from_slice(&sample_offset.to_be_bytes());
    }
    for opt in &entry.opts {
        bytes.extend_from_slice(&opt.num_output_samples.to_be_bytes());
        bytes.extend_from_slice(&opt.num_total_samples.to_be_bytes());
    }
    Ok(bytes)
}

fn parse_alternative_startup_entry(
    field_name: &'static str,
    bytes: &[u8],
) -> Result<AlternativeStartupEntry, FieldValueError> {
    if bytes.len() < 4 {
        return Err(invalid_value(
            field_name,
            "alternative startup entry is too short",
        ));
    }

    let roll_count = read_u16(bytes, 0);
    let sample_offset_count = usize::from(roll_count);
    let sample_offset_bytes = sample_offset_count
        .checked_mul(4)
        .ok_or_else(|| invalid_value(field_name, "alternative startup entry is too large"))?;
    let minimum_len = 4_usize
        .checked_add(sample_offset_bytes)
        .ok_or_else(|| invalid_value(field_name, "alternative startup entry is too large"))?;
    if bytes.len() < minimum_len {
        return Err(invalid_value(
            field_name,
            "alternative startup entry is shorter than its roll count requires",
        ));
    }

    let trailing_len = bytes.len() - minimum_len;
    if !trailing_len.is_multiple_of(4) {
        return Err(invalid_value(
            field_name,
            "alternative startup entry options do not align to 4 bytes",
        ));
    }

    let mut sample_offset = Vec::with_capacity(untrusted_prealloc_hint(sample_offset_count));
    let mut offset = 4;
    for _ in 0..sample_offset_count {
        sample_offset.push(read_u32(bytes, offset));
        offset += 4;
    }

    let mut opts = Vec::with_capacity(untrusted_prealloc_hint(trailing_len / 4));
    while offset < bytes.len() {
        opts.push(AlternativeStartupEntryOpt {
            num_output_samples: read_u16(bytes, offset),
            num_total_samples: read_u16(bytes, offset + 2),
        });
        offset += 4;
    }

    Ok(AlternativeStartupEntry {
        roll_count,
        first_output_sample: read_u16(bytes, 2),
        sample_offset,
        opts,
    })
}

fn encode_visual_random_access_entry(
    field_name: &'static str,
    entry: &VisualRandomAccessEntry,
) -> Result<u8, FieldValueError> {
    if entry.num_leading_samples > 0x7f {
        return Err(invalid_value(
            field_name,
            "num leading samples does not fit in 7 bits",
        ));
    }

    Ok((u8::from(entry.num_leading_samples_known) << 7) | entry.num_leading_samples)
}

fn parse_visual_random_access_entry(byte: u8) -> VisualRandomAccessEntry {
    VisualRandomAccessEntry {
        num_leading_samples_known: byte & 0x80 != 0,
        num_leading_samples: byte & 0x7f,
    }
}

fn encode_temporal_level_entry(entry: &TemporalLevelEntry) -> u8 {
    u8::from(entry.level_independently_decodable) << 7
}

fn parse_temporal_level_entry(
    field_name: &'static str,
    byte: u8,
) -> Result<TemporalLevelEntry, FieldValueError> {
    if byte & 0x7f != 0 {
        return Err(invalid_value(
            field_name,
            "temporal level entry reserved bits must be zero",
        ));
    }

    Ok(TemporalLevelEntry {
        level_independently_decodable: byte & 0x80 != 0,
    })
}

/// Sample group description box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sgpd {
    full_box: FullBoxState,
    pub grouping_type: FourCc,
    pub default_length: u32,
    pub default_sample_description_index: u32,
    pub entry_count: u32,
    pub roll_distances: Vec<i16>,
    pub roll_distances_l: Vec<RollDistanceWithLength>,
    pub alternative_startup_entries: Vec<AlternativeStartupEntry>,
    pub alternative_startup_entries_l: Vec<AlternativeStartupEntryL>,
    pub visual_random_access_entries: Vec<VisualRandomAccessEntry>,
    pub visual_random_access_entries_l: Vec<VisualRandomAccessEntryL>,
    pub temporal_level_entries: Vec<TemporalLevelEntry>,
    pub temporal_level_entries_l: Vec<TemporalLevelEntryL>,
    pub unsupported: Vec<u8>,
}

impl Default for Sgpd {
    fn default() -> Self {
        Self {
            full_box: FullBoxState::default(),
            grouping_type: FourCc::ANY,
            default_length: 0,
            default_sample_description_index: 0,
            entry_count: 0,
            roll_distances: Vec::new(),
            roll_distances_l: Vec::new(),
            alternative_startup_entries: Vec::new(),
            alternative_startup_entries_l: Vec::new(),
            visual_random_access_entries: Vec::new(),
            visual_random_access_entries_l: Vec::new(),
            temporal_level_entries: Vec::new(),
            temporal_level_entries_l: Vec::new(),
            unsupported: Vec::new(),
        }
    }
}

impl Sgpd {
    fn no_default_length(&self) -> bool {
        self.version() == 1 && self.default_length == 0
    }

    fn is_roll_grouping_type(&self) -> bool {
        *self.grouping_type.as_bytes() == *b"roll" || *self.grouping_type.as_bytes() == *b"prol"
    }

    fn is_alternative_startup_grouping_type(&self) -> bool {
        *self.grouping_type.as_bytes() == *b"alst"
    }

    fn is_visual_random_access_grouping_type(&self) -> bool {
        *self.grouping_type.as_bytes() == *b"rap "
    }

    fn is_temporal_level_grouping_type(&self) -> bool {
        *self.grouping_type.as_bytes() == *b"tele"
    }
}

impl FieldHooks for Sgpd {
    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        // The active payload shape depends on both the grouping type and whether version 1 uses per-entry lengths.
        let no_default_length = self.no_default_length();
        let roll_distances = self.is_roll_grouping_type();
        let alternative_startup_entries = self.is_alternative_startup_grouping_type();
        let visual_random_access_entries = self.is_visual_random_access_grouping_type();
        let temporal_level_entries = self.is_temporal_level_grouping_type();

        match name {
            "RollDistances" => Some(roll_distances && !no_default_length),
            "RollDistancesL" => Some(roll_distances && no_default_length),
            "AlternativeStartupEntries" => Some(alternative_startup_entries && !no_default_length),
            "AlternativeStartupEntriesL" => Some(alternative_startup_entries && no_default_length),
            "VisualRandomAccessEntries" => Some(visual_random_access_entries && !no_default_length),
            "VisualRandomAccessEntriesL" => Some(visual_random_access_entries && no_default_length),
            "TemporalLevelEntries" => Some(temporal_level_entries && !no_default_length),
            "TemporalLevelEntriesL" => Some(temporal_level_entries && no_default_length),
            "Unsupported" => Some(
                !roll_distances
                    && !alternative_startup_entries
                    && !visual_random_access_entries
                    && !temporal_level_entries,
            ),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "GroupingType" => Some(quoted_fourcc(self.grouping_type)),
            "RollDistances" => Some(render_array(
                self.roll_distances.iter().map(|distance| distance.to_string()),
            )),
            "RollDistancesL" => Some(render_array(self.roll_distances_l.iter().map(|entry| {
                format!(
                    "{{DescriptionLength={} RollDistance={}}}",
                    entry.description_length, entry.roll_distance
                )
            }))),
            "AlternativeStartupEntries" => Some(render_array(
                self.alternative_startup_entries
                    .iter()
                    .map(format_alternative_startup_entry),
            )),
            "AlternativeStartupEntriesL" => Some(render_array(
                self.alternative_startup_entries_l.iter().map(|entry| {
                    format!(
                        "{{DescriptionLength={} {}}}",
                        entry.description_length,
                        format_alternative_startup_entry(&entry.alternative_startup_entry)
                            .trim_start_matches('{')
                            .trim_end_matches('}')
                    )
                }),
            )),
            "VisualRandomAccessEntries" => Some(render_array(
                self.visual_random_access_entries.iter().map(|entry| {
                    format!(
                        "{{NumLeadingSamplesKnown={} NumLeadingSamples=0x{:x}}}",
                        entry.num_leading_samples_known, entry.num_leading_samples
                    )
                }),
            )),
            "VisualRandomAccessEntriesL" => Some(render_array(
                self.visual_random_access_entries_l.iter().map(|entry| {
                    format!(
                        "{{DescriptionLength={} NumLeadingSamplesKnown={} NumLeadingSamples=0x{:x}}}",
                        entry.description_length,
                        entry.visual_random_access_entry.num_leading_samples_known,
                        entry.visual_random_access_entry.num_leading_samples
                    )
                }),
            )),
            "TemporalLevelEntries" => Some(render_array(
                self.temporal_level_entries.iter().map(|entry| {
                    format!(
                        "{{LevelIndependentlyDecodable={}}}",
                        entry.level_independently_decodable
                    )
                }),
            )),
            "TemporalLevelEntriesL" => Some(render_array(
                self.temporal_level_entries_l.iter().map(|entry| {
                    format!(
                        "{{DescriptionLength={} LevelIndependentlyDecodable={}}}",
                        entry.description_length,
                        entry.temporal_level_entry.level_independently_decodable
                    )
                }),
            )),
            _ => None,
        }
    }
}

impl_full_box!(Sgpd, *b"sgpd");

impl FieldValueRead for Sgpd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "GroupingType" => Ok(FieldValue::Bytes(self.grouping_type.as_bytes().to_vec())),
            "DefaultLength" => Ok(FieldValue::Unsigned(u64::from(self.default_length))),
            "DefaultSampleDescriptionIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.default_sample_description_index,
            ))),
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "RollDistances" => {
                require_count(field_name, self.entry_count, self.roll_distances.len())?;
                let mut bytes = Vec::with_capacity(self.roll_distances.len() * 2);
                for roll_distance in &self.roll_distances {
                    bytes.extend_from_slice(&roll_distance.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "RollDistancesL" => {
                require_count(field_name, self.entry_count, self.roll_distances_l.len())?;
                let mut bytes = Vec::with_capacity(self.roll_distances_l.len() * 6);
                for entry in &self.roll_distances_l {
                    bytes.extend_from_slice(&entry.description_length.to_be_bytes());
                    bytes.extend_from_slice(&entry.roll_distance.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "AlternativeStartupEntries" => {
                require_count(
                    field_name,
                    self.entry_count,
                    self.alternative_startup_entries.len(),
                )?;
                let mut bytes = Vec::new();
                for entry in &self.alternative_startup_entries {
                    let encoded = encode_alternative_startup_entry(field_name, entry)?;
                    if self.default_length != 0 && encoded.len() != self.default_length as usize {
                        return Err(invalid_value(
                            field_name,
                            "alternative startup entry does not match the default length",
                        ));
                    }
                    bytes.extend_from_slice(&encoded);
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "AlternativeStartupEntriesL" => {
                require_count(
                    field_name,
                    self.entry_count,
                    self.alternative_startup_entries_l.len(),
                )?;
                let mut bytes = Vec::new();
                for entry in &self.alternative_startup_entries_l {
                    let encoded = encode_alternative_startup_entry(
                        field_name,
                        &entry.alternative_startup_entry,
                    )?;
                    if encoded.len() != entry.description_length as usize {
                        return Err(invalid_value(
                            field_name,
                            "alternative startup entry length does not match the description length",
                        ));
                    }
                    bytes.extend_from_slice(&entry.description_length.to_be_bytes());
                    bytes.extend_from_slice(&encoded);
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "VisualRandomAccessEntries" => {
                require_count(
                    field_name,
                    self.entry_count,
                    self.visual_random_access_entries.len(),
                )?;
                let mut bytes = Vec::with_capacity(self.visual_random_access_entries.len());
                for entry in &self.visual_random_access_entries {
                    bytes.push(encode_visual_random_access_entry(field_name, entry)?);
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "VisualRandomAccessEntriesL" => {
                require_count(
                    field_name,
                    self.entry_count,
                    self.visual_random_access_entries_l.len(),
                )?;
                let mut bytes = Vec::new();
                for entry in &self.visual_random_access_entries_l {
                    if entry.description_length != 1 {
                        return Err(invalid_value(
                            field_name,
                            "visual random access entries with explicit lengths must be 1 byte",
                        ));
                    }
                    bytes.extend_from_slice(&entry.description_length.to_be_bytes());
                    bytes.push(encode_visual_random_access_entry(
                        field_name,
                        &entry.visual_random_access_entry,
                    )?);
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "TemporalLevelEntries" => {
                require_count(
                    field_name,
                    self.entry_count,
                    self.temporal_level_entries.len(),
                )?;
                Ok(FieldValue::Bytes(
                    self.temporal_level_entries
                        .iter()
                        .map(encode_temporal_level_entry)
                        .collect(),
                ))
            }
            "TemporalLevelEntriesL" => {
                require_count(
                    field_name,
                    self.entry_count,
                    self.temporal_level_entries_l.len(),
                )?;
                let mut bytes = Vec::new();
                for entry in &self.temporal_level_entries_l {
                    if entry.description_length != 1 {
                        return Err(invalid_value(
                            field_name,
                            "temporal level entries with explicit lengths must be 1 byte",
                        ));
                    }
                    bytes.extend_from_slice(&entry.description_length.to_be_bytes());
                    bytes.push(encode_temporal_level_entry(&entry.temporal_level_entry));
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "Unsupported" => Ok(FieldValue::Bytes(self.unsupported.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Sgpd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("GroupingType", FieldValue::Bytes(bytes)) => {
                self.grouping_type = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("DefaultLength", FieldValue::Unsigned(value)) => {
                self.default_length = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleDescriptionIndex", FieldValue::Unsigned(value)) => {
                self.default_sample_description_index = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("RollDistances", FieldValue::Bytes(bytes)) => {
                let expected_len = field_len_from_count(self.entry_count, 2)
                    .map(|len| len as usize)
                    .unwrap_or(0);
                if bytes.len() != expected_len {
                    return Err(invalid_value(
                        field_name,
                        "roll distance payload length does not match the entry count",
                    ));
                }
                self.roll_distances =
                    parse_fixed_chunks(field_name, &bytes, 2, |chunk| read_i16(chunk, 0))?;
                Ok(())
            }
            ("RollDistancesL", FieldValue::Bytes(bytes)) => {
                let expected_len = field_len_from_count(self.entry_count, 6)
                    .map(|len| len as usize)
                    .unwrap_or(0);
                if bytes.len() != expected_len {
                    return Err(invalid_value(
                        field_name,
                        "roll distance payload length does not match the entry count",
                    ));
                }
                self.roll_distances_l =
                    parse_fixed_chunks(field_name, &bytes, 6, |chunk| RollDistanceWithLength {
                        description_length: read_u32(chunk, 0),
                        roll_distance: read_i16(chunk, 4),
                    })?;
                Ok(())
            }
            ("AlternativeStartupEntries", FieldValue::Bytes(bytes)) => {
                let entry_len = usize::try_from(self.default_length)
                    .map_err(|_| invalid_value(field_name, "default length is too large"))?;
                if entry_len == 0 {
                    return Err(invalid_value(
                        field_name,
                        "default length must be non-zero for alternative startup entries",
                    ));
                }
                let expected_len = field_len_from_count(self.entry_count, entry_len)
                    .map(|len| len as usize)
                    .unwrap_or(0);
                if bytes.len() != expected_len {
                    return Err(invalid_value(
                        field_name,
                        "alternative startup payload length does not match the entry count",
                    ));
                }
                self.alternative_startup_entries = bytes
                    .chunks_exact(entry_len)
                    .map(|chunk| parse_alternative_startup_entry(field_name, chunk))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(())
            }
            ("AlternativeStartupEntriesL", FieldValue::Bytes(bytes)) => {
                let mut cursor = 0;
                let mut entries = Vec::new();
                while cursor < bytes.len() {
                    if bytes.len() - cursor < 4 {
                        return Err(invalid_value(
                            field_name,
                            "alternative startup entry length prefix is truncated",
                        ));
                    }
                    let description_length = read_u32(&bytes, cursor);
                    cursor += 4;
                    let description_len = usize::try_from(description_length).map_err(|_| {
                        invalid_value(field_name, "alternative startup description is too large")
                    })?;
                    if bytes.len() - cursor < description_len {
                        return Err(invalid_value(
                            field_name,
                            "alternative startup entry exceeds the remaining payload",
                        ));
                    }
                    let payload = &bytes[cursor..cursor + description_len];
                    cursor += description_len;
                    entries.push(AlternativeStartupEntryL {
                        description_length,
                        alternative_startup_entry: parse_alternative_startup_entry(
                            field_name, payload,
                        )?,
                    });
                }
                require_count(field_name, self.entry_count, entries.len())?;
                self.alternative_startup_entries_l = entries;
                Ok(())
            }
            ("VisualRandomAccessEntries", FieldValue::Bytes(bytes)) => {
                require_count(field_name, self.entry_count, bytes.len())?;
                self.visual_random_access_entries = bytes
                    .into_iter()
                    .map(parse_visual_random_access_entry)
                    .collect();
                Ok(())
            }
            ("VisualRandomAccessEntriesL", FieldValue::Bytes(bytes)) => {
                let mut cursor = 0;
                let mut entries = Vec::new();
                while cursor < bytes.len() {
                    if bytes.len() - cursor < 5 {
                        return Err(invalid_value(
                            field_name,
                            "visual random access entry is truncated",
                        ));
                    }
                    let description_length = read_u32(&bytes, cursor);
                    cursor += 4;
                    if description_length != 1 {
                        return Err(invalid_value(
                            field_name,
                            "visual random access entries with explicit lengths must be 1 byte",
                        ));
                    }
                    entries.push(VisualRandomAccessEntryL {
                        description_length,
                        visual_random_access_entry: parse_visual_random_access_entry(bytes[cursor]),
                    });
                    cursor += 1;
                }
                require_count(field_name, self.entry_count, entries.len())?;
                self.visual_random_access_entries_l = entries;
                Ok(())
            }
            ("TemporalLevelEntries", FieldValue::Bytes(bytes)) => {
                require_count(field_name, self.entry_count, bytes.len())?;
                self.temporal_level_entries = bytes
                    .into_iter()
                    .map(|byte| parse_temporal_level_entry(field_name, byte))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(())
            }
            ("TemporalLevelEntriesL", FieldValue::Bytes(bytes)) => {
                let mut cursor = 0;
                let mut entries = Vec::new();
                while cursor < bytes.len() {
                    if bytes.len() - cursor < 5 {
                        return Err(invalid_value(
                            field_name,
                            "temporal level entry is truncated",
                        ));
                    }
                    let description_length = read_u32(&bytes, cursor);
                    cursor += 4;
                    if description_length != 1 {
                        return Err(invalid_value(
                            field_name,
                            "temporal level entries with explicit lengths must be 1 byte",
                        ));
                    }
                    entries.push(TemporalLevelEntryL {
                        description_length,
                        temporal_level_entry: parse_temporal_level_entry(
                            field_name,
                            bytes[cursor],
                        )?,
                    });
                    cursor += 1;
                }
                require_count(field_name, self.entry_count, entries.len())?;
                self.temporal_level_entries_l = entries;
                Ok(())
            }
            ("Unsupported", FieldValue::Bytes(bytes)) => {
                self.unsupported = bytes;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Sgpd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "GroupingType",
            2,
            with_bit_width(8),
            with_length(4),
            as_bytes()
        ),
        codec_field!("DefaultLength", 3, with_bit_width(32), with_version(1)),
        codec_field!(
            "DefaultSampleDescriptionIndex",
            4,
            with_bit_width(32),
            with_version(2)
        ),
        codec_field!("EntryCount", 5, with_bit_width(32)),
        codec_field!(
            "RollDistances",
            6,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "RollDistancesL",
            7,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "AlternativeStartupEntries",
            8,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "AlternativeStartupEntriesL",
            9,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "VisualRandomAccessEntries",
            10,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "VisualRandomAccessEntriesL",
            11,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "TemporalLevelEntries",
            12,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "TemporalLevelEntriesL",
            13,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "Unsupported",
            14,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[1, 2];
}

/// One segment index reference entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidxReference {
    pub reference_type: bool,
    pub referenced_size: u32,
    pub subsegment_duration: u32,
    pub starts_with_sap: bool,
    pub sap_type: u32,
    pub sap_delta_time: u32,
}

/// Segment index box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sidx {
    full_box: FullBoxState,
    pub reference_id: u32,
    pub timescale: u32,
    pub earliest_presentation_time_v0: u32,
    pub first_offset_v0: u32,
    pub earliest_presentation_time_v1: u64,
    pub first_offset_v1: u64,
    pub reference_count: u16,
    pub references: Vec<SidxReference>,
}

impl FieldHooks for Sidx {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "References" => Some(render_array(self.references.iter().map(|entry| {
                format!(
                    "{{ReferenceType={} ReferencedSize={} SubsegmentDuration={} StartsWithSAP={} SAPType={} SAPDeltaTime={}}}",
                    entry.reference_type,
                    entry.referenced_size,
                    entry.subsegment_duration,
                    entry.starts_with_sap,
                    entry.sap_type,
                    entry.sap_delta_time
                )
            }))),
            _ => None,
        }
    }
}

impl_full_box!(Sidx, *b"sidx");

impl Sidx {
    /// Returns the active earliest presentation time for the current box version.
    pub fn earliest_presentation_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.earliest_presentation_time_v0),
            1 => self.earliest_presentation_time_v1,
            _ => 0,
        }
    }

    /// Returns the active first offset for the current box version.
    pub fn first_offset(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.first_offset_v0),
            1 => self.first_offset_v1,
            _ => 0,
        }
    }
}

impl FieldValueRead for Sidx {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ReferenceID" => Ok(FieldValue::Unsigned(u64::from(self.reference_id))),
            "Timescale" => Ok(FieldValue::Unsigned(u64::from(self.timescale))),
            "EarliestPresentationTimeV0" => Ok(FieldValue::Unsigned(u64::from(
                self.earliest_presentation_time_v0,
            ))),
            "FirstOffsetV0" => Ok(FieldValue::Unsigned(u64::from(self.first_offset_v0))),
            "EarliestPresentationTimeV1" => {
                Ok(FieldValue::Unsigned(self.earliest_presentation_time_v1))
            }
            "FirstOffsetV1" => Ok(FieldValue::Unsigned(self.first_offset_v1)),
            "ReferenceCount" => Ok(FieldValue::Unsigned(u64::from(self.reference_count))),
            "References" => {
                require_count(
                    field_name,
                    u32::from(self.reference_count),
                    self.references.len(),
                )?;
                let mut bytes = Vec::with_capacity(self.references.len() * 12);
                for entry in &self.references {
                    if entry.referenced_size > 0x7fff_ffff {
                        return Err(invalid_value(
                            field_name,
                            "referenced size does not fit in 31 bits",
                        ));
                    }
                    if entry.sap_type > 0x07 {
                        return Err(invalid_value(field_name, "SAP type does not fit in 3 bits"));
                    }
                    if entry.sap_delta_time > 0x0fff_ffff {
                        return Err(invalid_value(
                            field_name,
                            "SAP delta time does not fit in 28 bits",
                        ));
                    }

                    // The reference and SAP records pack their high-bit flags into the same 32-bit words as the payload values.
                    let reference_word =
                        (u32::from(entry.reference_type) << 31) | entry.referenced_size;
                    let sap_word = (u32::from(entry.starts_with_sap) << 31)
                        | (entry.sap_type << 28)
                        | entry.sap_delta_time;
                    bytes.extend_from_slice(&reference_word.to_be_bytes());
                    bytes.extend_from_slice(&entry.subsegment_duration.to_be_bytes());
                    bytes.extend_from_slice(&sap_word.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Sidx {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ReferenceID", FieldValue::Unsigned(value)) => {
                self.reference_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Timescale", FieldValue::Unsigned(value)) => {
                self.timescale = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EarliestPresentationTimeV0", FieldValue::Unsigned(value)) => {
                self.earliest_presentation_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FirstOffsetV0", FieldValue::Unsigned(value)) => {
                self.first_offset_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EarliestPresentationTimeV1", FieldValue::Unsigned(value)) => {
                self.earliest_presentation_time_v1 = value;
                Ok(())
            }
            ("FirstOffsetV1", FieldValue::Unsigned(value)) => {
                self.first_offset_v1 = value;
                Ok(())
            }
            ("ReferenceCount", FieldValue::Unsigned(value)) => {
                self.reference_count = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("References", FieldValue::Bytes(bytes)) => {
                let expected_len = field_len_from_count(u32::from(self.reference_count), 12)
                    .map(|len| len as usize)
                    .unwrap_or(0);
                if bytes.len() != expected_len {
                    return Err(invalid_value(
                        field_name,
                        "reference payload length does not match the reference count",
                    ));
                }

                self.references =
                    parse_fixed_chunks(field_name, &bytes, 12, |chunk| SidxReference {
                        reference_type: read_u32(chunk, 0) & 0x8000_0000 != 0,
                        referenced_size: read_u32(chunk, 0) & 0x7fff_ffff,
                        subsegment_duration: read_u32(chunk, 4),
                        starts_with_sap: read_u32(chunk, 8) & 0x8000_0000 != 0,
                        sap_type: (read_u32(chunk, 8) >> 28) & 0x07,
                        sap_delta_time: read_u32(chunk, 8) & 0x0fff_ffff,
                    })?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Sidx {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("ReferenceID", 2, with_bit_width(32)),
        codec_field!("Timescale", 3, with_bit_width(32)),
        codec_field!(
            "EarliestPresentationTimeV0",
            4,
            with_bit_width(32),
            with_version(0)
        ),
        codec_field!("FirstOffsetV0", 5, with_bit_width(32), with_version(0)),
        codec_field!(
            "EarliestPresentationTimeV1",
            6,
            with_bit_width(64),
            with_version(1)
        ),
        codec_field!("FirstOffsetV1", 7, with_bit_width(64), with_version(1)),
        codec_field!("Reserved", 8, with_bit_width(16), with_constant("0")),
        codec_field!("ReferenceCount", 9, with_bit_width(16)),
        codec_field!("References", 10, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// One track-fragment random-access entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TfraEntry {
    pub time_v0: u32,
    pub moof_offset_v0: u32,
    pub time_v1: u64,
    pub moof_offset_v1: u64,
    pub traf_number: u32,
    pub trun_number: u32,
    pub sample_number: u32,
}

/// Track fragment random access box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tfra {
    full_box: FullBoxState,
    pub track_id: u32,
    pub length_size_of_traf_num: u8,
    pub length_size_of_trun_num: u8,
    pub length_size_of_sample_num: u8,
    pub number_of_entry: u32,
    pub entries: Vec<TfraEntry>,
}

impl Tfra {
    fn entry_size_bytes(&self) -> usize {
        // Each stored length field is encoded as "size minus one", so add one byte to recover the actual width.
        let traf_bytes = usize::from(self.length_size_of_traf_num) + 1;
        let trun_bytes = usize::from(self.length_size_of_trun_num) + 1;
        let sample_bytes = usize::from(self.length_size_of_sample_num) + 1;
        match self.version() {
            0 => 8 + traf_bytes + trun_bytes + sample_bytes,
            1 => 16 + traf_bytes + trun_bytes + sample_bytes,
            _ => traf_bytes + trun_bytes + sample_bytes,
        }
    }

    /// Returns the active random-access time for `index`.
    pub fn time(&self, index: usize) -> u64 {
        match self.version() {
            0 => u64::from(self.entries[index].time_v0),
            1 => self.entries[index].time_v1,
            _ => 0,
        }
    }

    /// Returns the active `moof` offset for `index`.
    pub fn moof_offset(&self, index: usize) -> u64 {
        match self.version() {
            0 => u64::from(self.entries[index].moof_offset_v0),
            1 => self.entries[index].moof_offset_v1,
            _ => 0,
        }
    }
}

impl FieldHooks for Tfra {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(|entry| {
                if self.version() == 0 {
                    format!(
                        "{{TimeV0={} MoofOffsetV0={} TrafNumber={} TrunNumber={} SampleNumber={}}}",
                        entry.time_v0,
                        entry.moof_offset_v0,
                        entry.traf_number,
                        entry.trun_number,
                        entry.sample_number
                    )
                } else {
                    format!(
                        "{{TimeV1={} MoofOffsetV1={} TrafNumber={} TrunNumber={} SampleNumber={}}}",
                        entry.time_v1,
                        entry.moof_offset_v1,
                        entry.traf_number,
                        entry.trun_number,
                        entry.sample_number
                    )
                }
            }))),
            _ => None,
        }
    }
}

impl_full_box!(Tfra, *b"tfra");

impl FieldValueRead for Tfra {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "TrackID" => Ok(FieldValue::Unsigned(u64::from(self.track_id))),
            "LengthSizeOfTrafNum" => Ok(FieldValue::Unsigned(u64::from(
                self.length_size_of_traf_num,
            ))),
            "LengthSizeOfTrunNum" => Ok(FieldValue::Unsigned(u64::from(
                self.length_size_of_trun_num,
            ))),
            "LengthSizeOfSampleNum" => Ok(FieldValue::Unsigned(u64::from(
                self.length_size_of_sample_num,
            ))),
            "NumberOfEntry" => Ok(FieldValue::Unsigned(u64::from(self.number_of_entry))),
            "Entries" => {
                require_count(field_name, self.number_of_entry, self.entries.len())?;
                let traf_bytes = usize::from(self.length_size_of_traf_num) + 1;
                let trun_bytes = usize::from(self.length_size_of_trun_num) + 1;
                let sample_bytes = usize::from(self.length_size_of_sample_num) + 1;
                let mut bytes = Vec::with_capacity(self.entries.len() * self.entry_size_bytes());
                for entry in &self.entries {
                    if self.version() == 0 {
                        bytes.extend_from_slice(&entry.time_v0.to_be_bytes());
                        bytes.extend_from_slice(&entry.moof_offset_v0.to_be_bytes());
                    } else {
                        bytes.extend_from_slice(&entry.time_v1.to_be_bytes());
                        bytes.extend_from_slice(&entry.moof_offset_v1.to_be_bytes());
                    }
                    push_uint(
                        field_name,
                        &mut bytes,
                        traf_bytes,
                        u64::from(entry.traf_number),
                    )?;
                    push_uint(
                        field_name,
                        &mut bytes,
                        trun_bytes,
                        u64::from(entry.trun_number),
                    )?;
                    push_uint(
                        field_name,
                        &mut bytes,
                        sample_bytes,
                        u64::from(entry.sample_number),
                    )?;
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Tfra {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("TrackID", FieldValue::Unsigned(value)) => {
                self.track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LengthSizeOfTrafNum", FieldValue::Unsigned(value)) => {
                self.length_size_of_traf_num = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LengthSizeOfTrunNum", FieldValue::Unsigned(value)) => {
                self.length_size_of_trun_num = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LengthSizeOfSampleNum", FieldValue::Unsigned(value)) => {
                self.length_size_of_sample_num = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NumberOfEntry", FieldValue::Unsigned(value)) => {
                self.number_of_entry = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                let entry_size = self.entry_size_bytes();
                let expected_len = field_len_from_count(self.number_of_entry, entry_size)
                    .map(|len| len as usize)
                    .unwrap_or(0);
                if bytes.len() != expected_len {
                    return Err(invalid_value(
                        field_name,
                        "random access payload length does not match the entry count",
                    ));
                }

                let traf_bytes = usize::from(self.length_size_of_traf_num) + 1;
                let trun_bytes = usize::from(self.length_size_of_trun_num) + 1;
                let sample_bytes = usize::from(self.length_size_of_sample_num) + 1;
                self.entries = bytes
                    .chunks_exact(entry_size)
                    .map(|chunk| {
                        let mut offset = 0;
                        let mut entry = TfraEntry::default();
                        if self.version() == 0 {
                            entry.time_v0 = read_u32(chunk, offset);
                            offset += 4;
                            entry.moof_offset_v0 = read_u32(chunk, offset);
                            offset += 4;
                        } else {
                            entry.time_v1 = read_u64(chunk, offset);
                            offset += 8;
                            entry.moof_offset_v1 = read_u64(chunk, offset);
                            offset += 8;
                        }
                        entry.traf_number =
                            u32_from_unsigned(field_name, read_uint(chunk, offset, traf_bytes))?;
                        offset += traf_bytes;
                        entry.trun_number =
                            u32_from_unsigned(field_name, read_uint(chunk, offset, trun_bytes))?;
                        offset += trun_bytes;
                        entry.sample_number =
                            u32_from_unsigned(field_name, read_uint(chunk, offset, sample_bytes))?;
                        Ok(entry)
                    })
                    .collect::<Result<Vec<_>, FieldValueError>>()?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Tfra {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("TrackID", 2, with_bit_width(32)),
        codec_field!("Reserved", 3, with_bit_width(26), with_constant("0")),
        codec_field!("LengthSizeOfTrafNum", 4, with_bit_width(2), as_hex()),
        codec_field!("LengthSizeOfTrunNum", 5, with_bit_width(2), as_hex()),
        codec_field!("LengthSizeOfSampleNum", 6, with_bit_width(2), as_hex()),
        codec_field!("NumberOfEntry", 7, with_bit_width(32)),
        codec_field!("Entries", 8, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Bitrate declaration box for sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Btrt {
    pub buffer_size_db: u32,
    pub max_bitrate: u32,
    pub avg_bitrate: u32,
}

impl_leaf_box!(Btrt, *b"btrt");

impl FieldValueRead for Btrt {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "BufferSizeDB" => Ok(FieldValue::Unsigned(u64::from(self.buffer_size_db))),
            "MaxBitrate" => Ok(FieldValue::Unsigned(u64::from(self.max_bitrate))),
            "AvgBitrate" => Ok(FieldValue::Unsigned(u64::from(self.avg_bitrate))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Btrt {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("BufferSizeDB", FieldValue::Unsigned(value)) => {
                self.buffer_size_db = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MaxBitrate", FieldValue::Unsigned(value)) => {
                self.max_bitrate = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("AvgBitrate", FieldValue::Unsigned(value)) => {
                self.avg_bitrate = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Btrt {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("BufferSizeDB", 0, with_bit_width(32)),
        codec_field!("MaxBitrate", 1, with_bit_width(32)),
        codec_field!("AvgBitrate", 2, with_bit_width(32)),
    ]);
}

/// Color information leaf whose active fields depend on the stored colour type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colr {
    pub colour_type: FourCc,
    pub colour_primaries: u16,
    pub transfer_characteristics: u16,
    pub matrix_coefficients: u16,
    pub full_range_flag: bool,
    pub reserved: u8,
    pub profile: Vec<u8>,
    pub unknown: Vec<u8>,
}

impl Default for Colr {
    fn default() -> Self {
        Self {
            colour_type: FourCc::ANY,
            colour_primaries: 0,
            transfer_characteristics: 0,
            matrix_coefficients: 0,
            full_range_flag: false,
            reserved: 0,
            profile: Vec::new(),
            unknown: Vec::new(),
        }
    }
}

impl FieldHooks for Colr {
    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "ColourPrimaries"
            | "TransferCharacteristics"
            | "MatrixCoefficients"
            | "FullRangeFlag"
            | "Reserved" => Some(self.colour_type == COLR_NCLX),
            "Profile" => Some(matches!(self.colour_type, COLR_RICC | COLR_PROF)),
            "Unknown" => Some(!matches!(
                self.colour_type,
                COLR_NCLX | COLR_RICC | COLR_PROF
            )),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "ColourType" => Some(quoted_fourcc(self.colour_type)),
            _ => None,
        }
    }
}

impl ImmutableBox for Colr {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"colr")
    }
}

impl MutableBox for Colr {}

impl FieldValueRead for Colr {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ColourType" => Ok(FieldValue::Bytes(self.colour_type.as_bytes().to_vec())),
            "ColourPrimaries" => Ok(FieldValue::Unsigned(u64::from(self.colour_primaries))),
            "TransferCharacteristics" => Ok(FieldValue::Unsigned(u64::from(
                self.transfer_characteristics,
            ))),
            "MatrixCoefficients" => Ok(FieldValue::Unsigned(u64::from(self.matrix_coefficients))),
            "FullRangeFlag" => Ok(FieldValue::Boolean(self.full_range_flag)),
            "Reserved" => Ok(FieldValue::Unsigned(u64::from(self.reserved))),
            "Profile" => Ok(FieldValue::Bytes(self.profile.clone())),
            "Unknown" => Ok(FieldValue::Bytes(self.unknown.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Colr {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ColourType", FieldValue::Bytes(bytes)) => {
                self.colour_type = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("ColourPrimaries", FieldValue::Unsigned(value)) => {
                self.colour_primaries = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TransferCharacteristics", FieldValue::Unsigned(value)) => {
                self.transfer_characteristics = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MatrixCoefficients", FieldValue::Unsigned(value)) => {
                self.matrix_coefficients = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FullRangeFlag", FieldValue::Boolean(value)) => {
                self.full_range_flag = value;
                Ok(())
            }
            ("Reserved", FieldValue::Unsigned(value)) => {
                self.reserved = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Profile", FieldValue::Bytes(value)) => {
                self.profile = value;
                Ok(())
            }
            ("Unknown", FieldValue::Bytes(value)) => {
                self.unknown = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Colr {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!(
            "ColourType",
            0,
            with_bit_width(8),
            with_length(4),
            as_bytes()
        ),
        codec_field!(
            "ColourPrimaries",
            1,
            with_bit_width(16),
            with_dynamic_presence()
        ),
        codec_field!(
            "TransferCharacteristics",
            2,
            with_bit_width(16),
            with_dynamic_presence()
        ),
        codec_field!(
            "MatrixCoefficients",
            3,
            with_bit_width(16),
            with_dynamic_presence()
        ),
        codec_field!(
            "FullRangeFlag",
            4,
            with_bit_width(1),
            as_boolean(),
            with_dynamic_presence()
        ),
        codec_field!(
            "Reserved",
            5,
            with_bit_width(7),
            as_hex(),
            with_dynamic_presence()
        ),
        codec_field!(
            "Profile",
            6,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "Unknown",
            7,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
    ]);
}

/// Event-message box whose field order changes with the encoded version.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Emsg {
    full_box: FullBoxState,
    pub scheme_id_uri: String,
    pub value: String,
    pub timescale: u32,
    pub presentation_time_delta: u32,
    pub presentation_time: u64,
    pub event_duration: u32,
    pub id: u32,
    pub message_data: Vec<u8>,
}

impl FieldHooks for Emsg {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "MessageData" => Some(quote_bytes(&self.message_data)),
            _ => None,
        }
    }
}

impl_full_box!(Emsg, *b"emsg");

impl FieldValueRead for Emsg {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SchemeIdUri" => Ok(FieldValue::String(self.scheme_id_uri.clone())),
            "Value" => Ok(FieldValue::String(self.value.clone())),
            "Timescale" => Ok(FieldValue::Unsigned(u64::from(self.timescale))),
            "PresentationTimeDelta" => Ok(FieldValue::Unsigned(u64::from(
                self.presentation_time_delta,
            ))),
            "PresentationTime" => Ok(FieldValue::Unsigned(self.presentation_time)),
            "EventDuration" => Ok(FieldValue::Unsigned(u64::from(self.event_duration))),
            "Id" => Ok(FieldValue::Unsigned(u64::from(self.id))),
            "MessageData" => Ok(FieldValue::Bytes(self.message_data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Emsg {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SchemeIdUri", FieldValue::String(value)) => {
                self.scheme_id_uri = value;
                Ok(())
            }
            ("Value", FieldValue::String(value)) => {
                self.value = value;
                Ok(())
            }
            ("Timescale", FieldValue::Unsigned(value)) => {
                self.timescale = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PresentationTimeDelta", FieldValue::Unsigned(value)) => {
                self.presentation_time_delta = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PresentationTime", FieldValue::Unsigned(value)) => {
                self.presentation_time = value;
                Ok(())
            }
            ("EventDuration", FieldValue::Unsigned(value)) => {
                self.event_duration = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Id", FieldValue::Unsigned(value)) => {
                self.id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MessageData", FieldValue::Bytes(value)) => {
                self.message_data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Emsg {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "SchemeIdUri",
            2,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_version(0)
        ),
        codec_field!(
            "Value",
            3,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_version(0)
        ),
        codec_field!("Timescale", 4, with_bit_width(32)),
        codec_field!(
            "PresentationTimeDelta",
            5,
            with_bit_width(32),
            with_version(0)
        ),
        codec_field!(
            "PresentationTime",
            6,
            with_bit_width(64),
            with_version(1),
            with_display_order(5)
        ),
        codec_field!(
            "EventDuration",
            7,
            with_bit_width(32),
            with_display_order(6)
        ),
        codec_field!("Id", 8, with_bit_width(32), with_display_order(7)),
        codec_field!(
            "SchemeIdUri",
            9,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_version(1),
            with_display_order(2)
        ),
        codec_field!(
            "Value",
            10,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_version(1),
            with_display_order(3)
        ),
        codec_field!(
            "MessageData",
            11,
            with_bit_width(8),
            as_bytes(),
            with_display_order(8)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Field-ordering leaf used by some video sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fiel {
    pub field_count: u8,
    pub field_ordering: u8,
}

impl_leaf_box!(Fiel, *b"fiel");

impl FieldValueRead for Fiel {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "FieldCount" => Ok(FieldValue::Unsigned(u64::from(self.field_count))),
            "FieldOrdering" => Ok(FieldValue::Unsigned(u64::from(self.field_ordering))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Fiel {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("FieldCount", FieldValue::Unsigned(value)) => {
                self.field_count = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FieldOrdering", FieldValue::Unsigned(value)) => {
                self.field_ordering = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Fiel {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("FieldCount", 0, with_bit_width(8), as_hex()),
        codec_field!("FieldOrdering", 1, with_bit_width(8), as_hex()),
    ]);
}

/// Original-format indicator inside protection-scheme sample-entry paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frma {
    pub data_format: FourCc,
}

impl Default for Frma {
    fn default() -> Self {
        Self {
            data_format: FourCc::ANY,
        }
    }
}

impl FieldHooks for Frma {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataFormat" => Some(quoted_fourcc(self.data_format)),
            _ => None,
        }
    }
}

impl ImmutableBox for Frma {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"frma")
    }
}

impl MutableBox for Frma {}

impl FieldValueRead for Frma {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataFormat" => Ok(FieldValue::Bytes(self.data_format.as_bytes().to_vec())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Frma {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataFormat", FieldValue::Bytes(bytes)) => {
                self.data_format = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Frma {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[codec_field!(
        "DataFormat",
        0,
        with_bit_width(8),
        with_length(4),
        as_bytes()
    )]);
}

/// Pixel-aspect-ratio box carried by visual sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pasp {
    pub h_spacing: u32,
    pub v_spacing: u32,
}

impl_leaf_box!(Pasp, *b"pasp");

impl FieldValueRead for Pasp {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "HSpacing" => Ok(FieldValue::Unsigned(u64::from(self.h_spacing))),
            "VSpacing" => Ok(FieldValue::Unsigned(u64::from(self.v_spacing))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Pasp {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("HSpacing", FieldValue::Unsigned(value)) => {
                self.h_spacing = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("VSpacing", FieldValue::Unsigned(value)) => {
                self.v_spacing = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Pasp {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("HSpacing", 0, with_bit_width(32)),
        codec_field!("VSpacing", 1, with_bit_width(32)),
    ]);
}

/// Scheme-type declaration box inside a protection-scheme path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schm {
    full_box: FullBoxState,
    pub scheme_type: FourCc,
    pub scheme_version: u32,
    pub scheme_uri: String,
}

impl Default for Schm {
    fn default() -> Self {
        Self {
            full_box: FullBoxState::default(),
            scheme_type: FourCc::ANY,
            scheme_version: 0,
            scheme_uri: String::new(),
        }
    }
}

impl FieldHooks for Schm {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "SchemeType" => Some(quoted_fourcc(self.scheme_type)),
            _ => None,
        }
    }
}

impl_full_box!(Schm, *b"schm");

impl FieldValueRead for Schm {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SchemeType" => Ok(FieldValue::Bytes(self.scheme_type.as_bytes().to_vec())),
            "SchemeVersion" => Ok(FieldValue::Unsigned(u64::from(self.scheme_version))),
            "SchemeUri" => Ok(FieldValue::String(self.scheme_uri.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Schm {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SchemeType", FieldValue::Bytes(bytes)) => {
                self.scheme_type = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("SchemeVersion", FieldValue::Unsigned(value)) => {
                self.scheme_version = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SchemeUri", FieldValue::String(value)) => {
                self.scheme_uri = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Schm {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "SchemeType",
            2,
            with_bit_width(8),
            with_length(4),
            as_bytes()
        ),
        codec_field!("SchemeVersion", 3, with_bit_width(32), as_hex()),
        codec_field!(
            "SchemeUri",
            4,
            with_bit_width(8),
            as_string(StringFieldMode::RawBox),
            with_required_flags(SCHEME_URI_PRESENT)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Shared header fields carried by concrete sample-entry wrappers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleEntry {
    pub box_type: FourCc,
    pub data_reference_index: u16,
}

impl Default for SampleEntry {
    fn default() -> Self {
        Self {
            box_type: FourCc::ANY,
            data_reference_index: 0,
        }
    }
}

/// Visual sample-entry wrapper used by multiple codec-specific visual types.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VisualSampleEntry {
    pub sample_entry: SampleEntry,
    pub pre_defined: u16,
    pub pre_defined2: [u32; 3],
    pub width: u16,
    pub height: u16,
    pub horizresolution: u32,
    pub vertresolution: u32,
    pub reserved2: u32,
    pub frame_count: u16,
    pub compressorname: [u8; 32],
    pub depth: u16,
    pub pre_defined3: i16,
}

impl FieldHooks for VisualSampleEntry {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Compressorname" if self.compressorname[0] <= 31 => {
                let visible_len = usize::from(self.compressorname[0]).min(31);
                Some(quote_bytes(&self.compressorname[1..1 + visible_len]))
            }
            _ => None,
        }
    }
}

impl ImmutableBox for VisualSampleEntry {
    fn box_type(&self) -> FourCc {
        self.sample_entry.box_type
    }
}

impl MutableBox for VisualSampleEntry {}

impl AnyTypeBox for VisualSampleEntry {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.sample_entry.box_type = box_type;
    }
}

impl FieldValueRead for VisualSampleEntry {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataReferenceIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.sample_entry.data_reference_index,
            ))),
            "PreDefined" => Ok(FieldValue::Unsigned(u64::from(self.pre_defined))),
            "PreDefined2" => Ok(FieldValue::UnsignedArray(
                self.pre_defined2.iter().copied().map(u64::from).collect(),
            )),
            "Width" => Ok(FieldValue::Unsigned(u64::from(self.width))),
            "Height" => Ok(FieldValue::Unsigned(u64::from(self.height))),
            "Horizresolution" => Ok(FieldValue::Unsigned(u64::from(self.horizresolution))),
            "Vertresolution" => Ok(FieldValue::Unsigned(u64::from(self.vertresolution))),
            "Reserved2" => Ok(FieldValue::Unsigned(u64::from(self.reserved2))),
            "FrameCount" => Ok(FieldValue::Unsigned(u64::from(self.frame_count))),
            "Compressorname" => Ok(FieldValue::Bytes(self.compressorname.to_vec())),
            "Depth" => Ok(FieldValue::Unsigned(u64::from(self.depth))),
            "PreDefined3" => Ok(FieldValue::Signed(i64::from(self.pre_defined3))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for VisualSampleEntry {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataReferenceIndex", FieldValue::Unsigned(value)) => {
                self.sample_entry.data_reference_index = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PreDefined", FieldValue::Unsigned(value)) => {
                self.pre_defined = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PreDefined2", FieldValue::UnsignedArray(values)) => {
                if values.len() != 3 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 3 entries",
                    ));
                }
                self.pre_defined2 = [
                    u32_from_unsigned(field_name, values[0])?,
                    u32_from_unsigned(field_name, values[1])?,
                    u32_from_unsigned(field_name, values[2])?,
                ];
                Ok(())
            }
            ("Width", FieldValue::Unsigned(value)) => {
                self.width = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Height", FieldValue::Unsigned(value)) => {
                self.height = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Horizresolution", FieldValue::Unsigned(value)) => {
                self.horizresolution = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Vertresolution", FieldValue::Unsigned(value)) => {
                self.vertresolution = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Reserved2", FieldValue::Unsigned(value)) => {
                self.reserved2 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FrameCount", FieldValue::Unsigned(value)) => {
                self.frame_count = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Compressorname", FieldValue::Bytes(bytes)) => {
                self.compressorname = bytes
                    .try_into()
                    .map_err(|_| invalid_value(field_name, "value must be exactly 32 bytes"))?;
                Ok(())
            }
            ("Depth", FieldValue::Unsigned(value)) => {
                self.depth = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PreDefined3", FieldValue::Signed(value)) => {
                self.pre_defined3 = i16_from_signed(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for VisualSampleEntry {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Reserved0A", 0, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0B", 1, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0C", 2, with_bit_width(16), with_constant("0")),
        codec_field!("DataReferenceIndex", 3, with_bit_width(16)),
        codec_field!("PreDefined", 4, with_bit_width(16)),
        codec_field!("Reserved1", 5, with_bit_width(16), with_constant("0")),
        codec_field!("PreDefined2", 6, with_bit_width(32), with_length(3)),
        codec_field!("Width", 7, with_bit_width(16)),
        codec_field!("Height", 8, with_bit_width(16)),
        codec_field!("Horizresolution", 9, with_bit_width(32)),
        codec_field!("Vertresolution", 10, with_bit_width(32)),
        codec_field!("Reserved2", 11, with_bit_width(32), as_hidden()),
        codec_field!("FrameCount", 12, with_bit_width(16)),
        codec_field!(
            "Compressorname",
            13,
            with_bit_width(8),
            with_length(32),
            as_bytes()
        ),
        codec_field!("Depth", 14, with_bit_width(16)),
        codec_field!("PreDefined3", 15, with_bit_width(16), as_signed()),
    ]);
}

/// Audio sample-entry wrapper used by multiple codec-specific audio types.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioSampleEntry {
    pub sample_entry: SampleEntry,
    pub entry_version: u16,
    pub channel_count: u16,
    pub sample_size: u16,
    pub pre_defined: u16,
    pub sample_rate: u32,
    pub quicktime_data: Vec<u8>,
}

impl FieldHooks for AudioSampleEntry {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "QuickTimeData" => match self.entry_version {
                1 => Some(16),
                2 => Some(36),
                _ => None,
            },
            _ => None,
        }
    }

    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "QuickTimeData" => Some(matches!(self.entry_version, 1 | 2)),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "SampleRate" => Some(format_fixed_16_16_unsigned(self.sample_rate)),
            _ => None,
        }
    }
}

impl ImmutableBox for AudioSampleEntry {
    fn box_type(&self) -> FourCc {
        self.sample_entry.box_type
    }
}

impl MutableBox for AudioSampleEntry {}

impl AnyTypeBox for AudioSampleEntry {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.sample_entry.box_type = box_type;
    }
}

impl AudioSampleEntry {
    /// Returns the 16.16 fixed-point sample rate as a floating-point value.
    pub fn sample_rate_value(&self) -> f64 {
        f64::from(self.sample_rate) / 65536.0
    }

    /// Returns the integer component of the 16.16 fixed-point sample rate.
    pub fn sample_rate_int(&self) -> u16 {
        (self.sample_rate >> 16) as u16
    }
}

impl FieldValueRead for AudioSampleEntry {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataReferenceIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.sample_entry.data_reference_index,
            ))),
            "EntryVersion" => Ok(FieldValue::Unsigned(u64::from(self.entry_version))),
            "ChannelCount" => Ok(FieldValue::Unsigned(u64::from(self.channel_count))),
            "SampleSize" => Ok(FieldValue::Unsigned(u64::from(self.sample_size))),
            "PreDefined" => Ok(FieldValue::Unsigned(u64::from(self.pre_defined))),
            "SampleRate" => Ok(FieldValue::Unsigned(u64::from(self.sample_rate))),
            "QuickTimeData" => Ok(FieldValue::Bytes(self.quicktime_data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for AudioSampleEntry {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataReferenceIndex", FieldValue::Unsigned(value)) => {
                self.sample_entry.data_reference_index = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EntryVersion", FieldValue::Unsigned(value)) => {
                self.entry_version = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChannelCount", FieldValue::Unsigned(value)) => {
                self.channel_count = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleSize", FieldValue::Unsigned(value)) => {
                self.sample_size = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PreDefined", FieldValue::Unsigned(value)) => {
                self.pre_defined = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleRate", FieldValue::Unsigned(value)) => {
                self.sample_rate = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("QuickTimeData", FieldValue::Bytes(value)) => {
                self.quicktime_data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for AudioSampleEntry {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Reserved0A", 0, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0B", 1, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0C", 2, with_bit_width(16), with_constant("0")),
        codec_field!("DataReferenceIndex", 3, with_bit_width(16)),
        codec_field!("EntryVersion", 4, with_bit_width(16)),
        codec_field!("Reserved1A", 5, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved1B", 6, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved1C", 7, with_bit_width(16), with_constant("0")),
        codec_field!("ChannelCount", 8, with_bit_width(16)),
        codec_field!("SampleSize", 9, with_bit_width(16)),
        codec_field!("PreDefined", 10, with_bit_width(16)),
        codec_field!("Reserved2", 11, with_bit_width(16), with_constant("0")),
        codec_field!("SampleRate", 12, with_bit_width(32)),
        codec_field!(
            "QuickTimeData",
            13,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes(),
            with_dynamic_presence()
        ),
    ]);
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WaveAudioData {
    box_type: FourCc,
    quicktime_data: Vec<u8>,
}

impl Default for WaveAudioData {
    fn default() -> Self {
        Self {
            box_type: FourCc::ANY,
            quicktime_data: Vec::new(),
        }
    }
}

impl FieldHooks for WaveAudioData {}

impl ImmutableBox for WaveAudioData {
    fn box_type(&self) -> FourCc {
        self.box_type
    }
}

impl MutableBox for WaveAudioData {}

impl AnyTypeBox for WaveAudioData {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.box_type = box_type;
    }
}

impl FieldValueRead for WaveAudioData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "QuickTimeData" => Ok(FieldValue::Bytes(self.quicktime_data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for WaveAudioData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("QuickTimeData", FieldValue::Bytes(value)) => {
                self.quicktime_data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for WaveAudioData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[codec_field!(
        "QuickTimeData",
        0,
        with_bit_width(8),
        as_bytes()
    )]);
}

/// One length-prefixed AVC parameter-set record carried by `avcC`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AVCParameterSet {
    pub length: u16,
    pub nal_unit: Vec<u8>,
}

/// AVC decoder configuration carried by visual sample entries such as `avc1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AVCDecoderConfiguration {
    pub configuration_version: u8,
    pub profile: u8,
    pub profile_compatibility: u8,
    pub level: u8,
    pub length_size_minus_one: u8,
    pub num_of_sequence_parameter_sets: u8,
    pub sequence_parameter_sets: Vec<AVCParameterSet>,
    pub num_of_picture_parameter_sets: u8,
    pub picture_parameter_sets: Vec<AVCParameterSet>,
    pub high_profile_fields_enabled: bool,
    pub chroma_format: u8,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub num_of_sequence_parameter_set_ext: u8,
    pub sequence_parameter_sets_ext: Vec<AVCParameterSet>,
}

impl FieldHooks for AVCDecoderConfiguration {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "SequenceParameterSets" => {
                encoded_avc_parameter_sets_len(name, &self.sequence_parameter_sets).ok()
            }
            "PictureParameterSets" => {
                encoded_avc_parameter_sets_len(name, &self.picture_parameter_sets).ok()
            }
            "SequenceParameterSetsExt" => {
                encoded_avc_parameter_sets_len(name, &self.sequence_parameter_sets_ext).ok()
            }
            _ => None,
        }
    }

    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "ChromaFormat"
            | "BitDepthLumaMinus8"
            | "BitDepthChromaMinus8"
            | "NumOfSequenceParameterSetExt"
            | "SequenceParameterSetsExt" => Some(self.high_profile_fields_enabled),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "SequenceParameterSets" => {
                Some(render_avc_parameter_sets(&self.sequence_parameter_sets))
            }
            "PictureParameterSets" => Some(render_avc_parameter_sets(&self.picture_parameter_sets)),
            "SequenceParameterSetsExt" => {
                Some(render_avc_parameter_sets(&self.sequence_parameter_sets_ext))
            }
            _ => None,
        }
    }
}

impl ImmutableBox for AVCDecoderConfiguration {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"avcC")
    }
}

impl MutableBox for AVCDecoderConfiguration {}

impl FieldValueRead for AVCDecoderConfiguration {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ConfigurationVersion" => {
                Ok(FieldValue::Unsigned(u64::from(self.configuration_version)))
            }
            "Profile" => Ok(FieldValue::Unsigned(u64::from(self.profile))),
            "ProfileCompatibility" => {
                Ok(FieldValue::Unsigned(u64::from(self.profile_compatibility)))
            }
            "Level" => Ok(FieldValue::Unsigned(u64::from(self.level))),
            "LengthSizeMinusOne" => Ok(FieldValue::Unsigned(u64::from(self.length_size_minus_one))),
            "NumOfSequenceParameterSets" => Ok(FieldValue::Unsigned(u64::from(
                self.num_of_sequence_parameter_sets,
            ))),
            "SequenceParameterSets" => Ok(FieldValue::Bytes(encode_avc_parameter_sets(
                field_name,
                &self.sequence_parameter_sets,
            )?)),
            "NumOfPictureParameterSets" => Ok(FieldValue::Unsigned(u64::from(
                self.num_of_picture_parameter_sets,
            ))),
            "PictureParameterSets" => Ok(FieldValue::Bytes(encode_avc_parameter_sets(
                field_name,
                &self.picture_parameter_sets,
            )?)),
            "ChromaFormat" => Ok(FieldValue::Unsigned(u64::from(self.chroma_format))),
            "BitDepthLumaMinus8" => Ok(FieldValue::Unsigned(u64::from(self.bit_depth_luma_minus8))),
            "BitDepthChromaMinus8" => Ok(FieldValue::Unsigned(u64::from(
                self.bit_depth_chroma_minus8,
            ))),
            "NumOfSequenceParameterSetExt" => Ok(FieldValue::Unsigned(u64::from(
                self.num_of_sequence_parameter_set_ext,
            ))),
            "SequenceParameterSetsExt" => Ok(FieldValue::Bytes(encode_avc_parameter_sets(
                field_name,
                &self.sequence_parameter_sets_ext,
            )?)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for AVCDecoderConfiguration {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ConfigurationVersion", FieldValue::Unsigned(value)) => {
                self.configuration_version = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Profile", FieldValue::Unsigned(value)) => {
                self.profile = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ProfileCompatibility", FieldValue::Unsigned(value)) => {
                self.profile_compatibility = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Level", FieldValue::Unsigned(value)) => {
                self.level = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LengthSizeMinusOne", FieldValue::Unsigned(value)) => {
                self.length_size_minus_one = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NumOfSequenceParameterSets", FieldValue::Unsigned(value)) => {
                self.num_of_sequence_parameter_sets = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SequenceParameterSets", FieldValue::Bytes(value)) => {
                self.sequence_parameter_sets = parse_avc_parameter_sets(
                    field_name,
                    &value,
                    self.num_of_sequence_parameter_sets,
                )?;
                Ok(())
            }
            ("NumOfPictureParameterSets", FieldValue::Unsigned(value)) => {
                self.num_of_picture_parameter_sets = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PictureParameterSets", FieldValue::Bytes(value)) => {
                self.picture_parameter_sets = parse_avc_parameter_sets(
                    field_name,
                    &value,
                    self.num_of_picture_parameter_sets,
                )?;
                Ok(())
            }
            ("ChromaFormat", FieldValue::Unsigned(value)) => {
                self.chroma_format = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BitDepthLumaMinus8", FieldValue::Unsigned(value)) => {
                self.bit_depth_luma_minus8 = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BitDepthChromaMinus8", FieldValue::Unsigned(value)) => {
                self.bit_depth_chroma_minus8 = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NumOfSequenceParameterSetExt", FieldValue::Unsigned(value)) => {
                self.num_of_sequence_parameter_set_ext = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SequenceParameterSetsExt", FieldValue::Bytes(value)) => {
                self.sequence_parameter_sets_ext = parse_avc_parameter_sets(
                    field_name,
                    &value,
                    self.num_of_sequence_parameter_set_ext,
                )?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for AVCDecoderConfiguration {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("ConfigurationVersion", 0, with_bit_width(8), as_hex()),
        codec_field!("Profile", 1, with_bit_width(8), as_hex()),
        codec_field!("ProfileCompatibility", 2, with_bit_width(8), as_hex()),
        codec_field!("Level", 3, with_bit_width(8), as_hex()),
        codec_field!("LengthSizeMinusOne", 4, with_bit_width(8), as_hex()),
        codec_field!("NumOfSequenceParameterSets", 5, with_bit_width(8), as_hex()),
        codec_field!(
            "SequenceParameterSets",
            6,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
        codec_field!("NumOfPictureParameterSets", 7, with_bit_width(8), as_hex()),
        codec_field!(
            "PictureParameterSets",
            8,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
        codec_field!(
            "ChromaFormat",
            9,
            with_bit_width(8),
            as_hex(),
            with_dynamic_presence()
        ),
        codec_field!(
            "BitDepthLumaMinus8",
            10,
            with_bit_width(8),
            as_hex(),
            with_dynamic_presence()
        ),
        codec_field!(
            "BitDepthChromaMinus8",
            11,
            with_bit_width(8),
            as_hex(),
            with_dynamic_presence()
        ),
        codec_field!(
            "NumOfSequenceParameterSetExt",
            12,
            with_bit_width(8),
            as_hex(),
            with_dynamic_presence()
        ),
        codec_field!(
            "SequenceParameterSetsExt",
            13,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes(),
            with_dynamic_presence()
        ),
    ]);

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.length_size_minus_one > 0x03 {
            return Err(invalid_value("LengthSizeMinusOne", "value does not fit in 2 bits").into());
        }
        if self.num_of_sequence_parameter_sets > 0x1f {
            return Err(invalid_value(
                "NumOfSequenceParameterSets",
                "value does not fit in 5 bits",
            )
            .into());
        }

        require_count(
            "NumOfSequenceParameterSets",
            u32::from(self.num_of_sequence_parameter_sets),
            self.sequence_parameter_sets.len(),
        )?;
        require_count(
            "NumOfPictureParameterSets",
            u32::from(self.num_of_picture_parameter_sets),
            self.picture_parameter_sets.len(),
        )?;

        let mut payload = vec![
            self.configuration_version,
            self.profile,
            self.profile_compatibility,
            self.level,
            0xfc | self.length_size_minus_one,
            0xe0 | self.num_of_sequence_parameter_sets,
        ];
        payload.extend_from_slice(&encode_avc_parameter_sets(
            "SequenceParameterSets",
            &self.sequence_parameter_sets,
        )?);
        payload.push(self.num_of_picture_parameter_sets);
        payload.extend_from_slice(&encode_avc_parameter_sets(
            "PictureParameterSets",
            &self.picture_parameter_sets,
        )?);

        if self.high_profile_fields_enabled {
            if !avc_profile_supports_extensions(self.profile) {
                return Err(invalid_value(
                    "HighProfileFieldsEnabled",
                    "each values of Profile and HighProfileFieldsEnabled are inconsistent",
                )
                .into());
            }
            if self.chroma_format > 0x03 {
                return Err(invalid_value("ChromaFormat", "value does not fit in 2 bits").into());
            }
            if self.bit_depth_luma_minus8 > 0x07 {
                return Err(
                    invalid_value("BitDepthLumaMinus8", "value does not fit in 3 bits").into(),
                );
            }
            if self.bit_depth_chroma_minus8 > 0x07 {
                return Err(
                    invalid_value("BitDepthChromaMinus8", "value does not fit in 3 bits").into(),
                );
            }
            require_count(
                "NumOfSequenceParameterSetExt",
                u32::from(self.num_of_sequence_parameter_set_ext),
                self.sequence_parameter_sets_ext.len(),
            )?;

            payload.push(0xfc | self.chroma_format);
            payload.push(0xf8 | self.bit_depth_luma_minus8);
            payload.push(0xf8 | self.bit_depth_chroma_minus8);
            payload.push(self.num_of_sequence_parameter_set_ext);
            payload.extend_from_slice(&encode_avc_parameter_sets(
                "SequenceParameterSetsExt",
                &self.sequence_parameter_sets_ext,
            )?);
        }

        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let start = reader.stream_position()?;
        let payload_len = usize::try_from(payload_size)
            .map_err(|_| invalid_value("Payload", "payload is too large to decode"))?;
        let payload = match read_exact_vec_untrusted(reader, payload_len) {
            Ok(payload) => payload,
            Err(error) => {
                reader.seek(SeekFrom::Start(start))?;
                return Err(error.into());
            }
        };

        let parse_result = (|| -> Result<(), CodecError> {
            if payload.len() < 6 {
                return Err(invalid_value("Payload", "payload is too short").into());
            }

            let mut offset = 0_usize;
            self.configuration_version = payload[offset];
            offset += 1;
            self.profile = payload[offset];
            offset += 1;
            self.profile_compatibility = payload[offset];
            offset += 1;
            self.level = payload[offset];
            offset += 1;

            let length_size = payload[offset];
            if length_size >> 2 != 0x3f {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved",
                    constant: "63",
                });
            }
            self.length_size_minus_one = length_size & 0x03;
            offset += 1;

            let sequence_count = payload[offset];
            if sequence_count >> 5 != 0x07 {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved2",
                    constant: "7",
                });
            }
            self.num_of_sequence_parameter_sets = sequence_count & 0x1f;
            offset += 1;

            let sequence_start = offset;
            self.sequence_parameter_sets = Vec::with_capacity(untrusted_prealloc_hint(
                usize::from(self.num_of_sequence_parameter_sets),
            ));
            for _ in 0..self.num_of_sequence_parameter_sets {
                if payload.len().saturating_sub(offset) < 2 {
                    return Err(invalid_value(
                        "SequenceParameterSets",
                        "parameter-set payload length does not match the entry count",
                    )
                    .into());
                }
                let length = read_u16(&payload, offset);
                offset += 2;
                let end = offset + usize::from(length);
                if end > payload.len() {
                    return Err(invalid_value(
                        "SequenceParameterSets",
                        "parameter-set payload length does not match the entry count",
                    )
                    .into());
                }
                self.sequence_parameter_sets.push(AVCParameterSet {
                    length,
                    nal_unit: payload[offset..end].to_vec(),
                });
                offset = end;
            }
            let _ = sequence_start;

            if offset >= payload.len() {
                return Err(invalid_value("Payload", "payload is too short").into());
            }
            self.num_of_picture_parameter_sets = payload[offset];
            offset += 1;

            self.picture_parameter_sets = Vec::with_capacity(untrusted_prealloc_hint(usize::from(
                self.num_of_picture_parameter_sets,
            )));
            for _ in 0..self.num_of_picture_parameter_sets {
                if payload.len().saturating_sub(offset) < 2 {
                    return Err(invalid_value(
                        "PictureParameterSets",
                        "parameter-set payload length does not match the entry count",
                    )
                    .into());
                }
                let length = read_u16(&payload, offset);
                offset += 2;
                let end = offset + usize::from(length);
                if end > payload.len() {
                    return Err(invalid_value(
                        "PictureParameterSets",
                        "parameter-set payload length does not match the entry count",
                    )
                    .into());
                }
                self.picture_parameter_sets.push(AVCParameterSet {
                    length,
                    nal_unit: payload[offset..end].to_vec(),
                });
                offset = end;
            }

            self.high_profile_fields_enabled = false;
            self.chroma_format = 0;
            self.bit_depth_luma_minus8 = 0;
            self.bit_depth_chroma_minus8 = 0;
            self.num_of_sequence_parameter_set_ext = 0;
            self.sequence_parameter_sets_ext.clear();

            let remaining = payload.len().saturating_sub(offset);
            if avc_profile_supports_extensions(self.profile) && remaining != 0 {
                if remaining < 4 {
                    return Err(invalid_value("Payload", "payload is truncated").into());
                }

                self.high_profile_fields_enabled = true;

                let chroma_format = payload[offset];
                if chroma_format >> 2 != 0x3f {
                    return Err(CodecError::ConstantMismatch {
                        field_name: "Reserved3",
                        constant: "63",
                    });
                }
                self.chroma_format = chroma_format & 0x03;
                offset += 1;

                let bit_depth_luma = payload[offset];
                if bit_depth_luma >> 3 != 0x1f {
                    return Err(CodecError::ConstantMismatch {
                        field_name: "Reserved4",
                        constant: "31",
                    });
                }
                self.bit_depth_luma_minus8 = bit_depth_luma & 0x07;
                offset += 1;

                let bit_depth_chroma = payload[offset];
                if bit_depth_chroma >> 3 != 0x1f {
                    return Err(CodecError::ConstantMismatch {
                        field_name: "Reserved5",
                        constant: "31",
                    });
                }
                self.bit_depth_chroma_minus8 = bit_depth_chroma & 0x07;
                offset += 1;

                self.num_of_sequence_parameter_set_ext = payload[offset];
                offset += 1;

                self.sequence_parameter_sets_ext = Vec::with_capacity(untrusted_prealloc_hint(
                    usize::from(self.num_of_sequence_parameter_set_ext),
                ));
                for _ in 0..self.num_of_sequence_parameter_set_ext {
                    if payload.len().saturating_sub(offset) < 2 {
                        return Err(invalid_value(
                            "SequenceParameterSetsExt",
                            "parameter-set payload length does not match the entry count",
                        )
                        .into());
                    }
                    let length = read_u16(&payload, offset);
                    offset += 2;
                    let end = offset + usize::from(length);
                    if end > payload.len() {
                        return Err(invalid_value(
                            "SequenceParameterSetsExt",
                            "parameter-set payload length does not match the entry count",
                        )
                        .into());
                    }
                    self.sequence_parameter_sets_ext.push(AVCParameterSet {
                        length,
                        nal_unit: payload[offset..end].to_vec(),
                    });
                    offset = end;
                }
            }

            if offset != payload.len() {
                return Err(invalid_value("Payload", "payload has trailing bytes").into());
            }

            Ok(())
        })();

        if let Err(error) = parse_result {
            reader.seek(SeekFrom::Start(start))?;
            return Err(error);
        }

        Ok(Some(payload_size))
    }
}

/// One length-prefixed HEVC NAL-unit record carried by `hvcC`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HEVCNalu {
    pub length: u16,
    pub nal_unit: Vec<u8>,
}

/// One HEVC NAL-array grouping carried by `hvcC`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HEVCNaluArray {
    pub completeness: bool,
    pub reserved: bool,
    pub nalu_type: u8,
    pub num_nalus: u16,
    pub nalus: Vec<HEVCNalu>,
}

/// HEVC decoder configuration carried by visual sample entries such as `hvc1` and `hev1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HEVCDecoderConfiguration {
    pub configuration_version: u8,
    pub general_profile_space: u8,
    pub general_tier_flag: bool,
    pub general_profile_idc: u8,
    pub general_profile_compatibility: [bool; 32],
    pub general_constraint_indicator: [u8; 6],
    pub general_level_idc: u8,
    pub min_spatial_segmentation_idc: u16,
    pub parallelism_type: u8,
    pub chroma_format_idc: u8,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub avg_frame_rate: u16,
    pub constant_frame_rate: u8,
    pub num_temporal_layers: u8,
    pub temporal_id_nested: u8,
    pub length_size_minus_one: u8,
    pub num_of_nalu_arrays: u8,
    pub nalu_arrays: Vec<HEVCNaluArray>,
}

impl FieldHooks for HEVCDecoderConfiguration {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "NaluArrays" => encoded_hevc_nalu_arrays_len(name, &self.nalu_arrays).ok(),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "NaluArrays" => Some(render_hevc_nalu_arrays(&self.nalu_arrays)),
            _ => None,
        }
    }
}

impl ImmutableBox for HEVCDecoderConfiguration {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"hvcC")
    }
}

impl MutableBox for HEVCDecoderConfiguration {}

impl FieldValueRead for HEVCDecoderConfiguration {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ConfigurationVersion" => {
                Ok(FieldValue::Unsigned(u64::from(self.configuration_version)))
            }
            "GeneralProfileSpace" => {
                Ok(FieldValue::Unsigned(u64::from(self.general_profile_space)))
            }
            "GeneralTierFlag" => Ok(FieldValue::Boolean(self.general_tier_flag)),
            "GeneralProfileIdc" => Ok(FieldValue::Unsigned(u64::from(self.general_profile_idc))),
            "GeneralProfileCompatibility" => Ok(FieldValue::BooleanArray(
                self.general_profile_compatibility.to_vec(),
            )),
            "GeneralConstraintIndicator" => Ok(FieldValue::UnsignedArray(
                self.general_constraint_indicator
                    .iter()
                    .copied()
                    .map(u64::from)
                    .collect(),
            )),
            "GeneralLevelIdc" => Ok(FieldValue::Unsigned(u64::from(self.general_level_idc))),
            "MinSpatialSegmentationIdc" => Ok(FieldValue::Unsigned(u64::from(
                self.min_spatial_segmentation_idc,
            ))),
            "ParallelismType" => Ok(FieldValue::Unsigned(u64::from(self.parallelism_type))),
            "ChromaFormatIdc" => Ok(FieldValue::Unsigned(u64::from(self.chroma_format_idc))),
            "BitDepthLumaMinus8" => Ok(FieldValue::Unsigned(u64::from(self.bit_depth_luma_minus8))),
            "BitDepthChromaMinus8" => Ok(FieldValue::Unsigned(u64::from(
                self.bit_depth_chroma_minus8,
            ))),
            "AvgFrameRate" => Ok(FieldValue::Unsigned(u64::from(self.avg_frame_rate))),
            "ConstantFrameRate" => Ok(FieldValue::Unsigned(u64::from(self.constant_frame_rate))),
            "NumTemporalLayers" => Ok(FieldValue::Unsigned(u64::from(self.num_temporal_layers))),
            "TemporalIdNested" => Ok(FieldValue::Unsigned(u64::from(self.temporal_id_nested))),
            "LengthSizeMinusOne" => Ok(FieldValue::Unsigned(u64::from(self.length_size_minus_one))),
            "NumOfNaluArrays" => Ok(FieldValue::Unsigned(u64::from(self.num_of_nalu_arrays))),
            "NaluArrays" => Ok(FieldValue::Bytes(encode_hevc_nalu_arrays(
                field_name,
                &self.nalu_arrays,
            )?)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for HEVCDecoderConfiguration {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ConfigurationVersion", FieldValue::Unsigned(value)) => {
                self.configuration_version = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("GeneralProfileSpace", FieldValue::Unsigned(value)) => {
                self.general_profile_space = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("GeneralTierFlag", FieldValue::Boolean(value)) => {
                self.general_tier_flag = value;
                Ok(())
            }
            ("GeneralProfileIdc", FieldValue::Unsigned(value)) => {
                self.general_profile_idc = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("GeneralProfileCompatibility", FieldValue::BooleanArray(value)) => {
                if value.len() != 32 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 32 entries",
                    ));
                }
                let mut flags = [false; 32];
                flags.copy_from_slice(&value);
                self.general_profile_compatibility = flags;
                Ok(())
            }
            ("GeneralConstraintIndicator", FieldValue::UnsignedArray(values)) => {
                if values.len() != 6 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 6 entries",
                    ));
                }
                let mut indicator = [0_u8; 6];
                for (slot, value) in indicator.iter_mut().zip(values) {
                    *slot = u8_from_unsigned(field_name, value)?;
                }
                self.general_constraint_indicator = indicator;
                Ok(())
            }
            ("GeneralLevelIdc", FieldValue::Unsigned(value)) => {
                self.general_level_idc = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MinSpatialSegmentationIdc", FieldValue::Unsigned(value)) => {
                self.min_spatial_segmentation_idc = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ParallelismType", FieldValue::Unsigned(value)) => {
                self.parallelism_type = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChromaFormatIdc", FieldValue::Unsigned(value)) => {
                self.chroma_format_idc = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BitDepthLumaMinus8", FieldValue::Unsigned(value)) => {
                self.bit_depth_luma_minus8 = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BitDepthChromaMinus8", FieldValue::Unsigned(value)) => {
                self.bit_depth_chroma_minus8 = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("AvgFrameRate", FieldValue::Unsigned(value)) => {
                self.avg_frame_rate = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ConstantFrameRate", FieldValue::Unsigned(value)) => {
                self.constant_frame_rate = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NumTemporalLayers", FieldValue::Unsigned(value)) => {
                self.num_temporal_layers = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TemporalIdNested", FieldValue::Unsigned(value)) => {
                self.temporal_id_nested = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LengthSizeMinusOne", FieldValue::Unsigned(value)) => {
                self.length_size_minus_one = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NumOfNaluArrays", FieldValue::Unsigned(value)) => {
                self.num_of_nalu_arrays = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NaluArrays", FieldValue::Bytes(value)) => {
                self.nalu_arrays =
                    parse_hevc_nalu_arrays(field_name, &value, self.num_of_nalu_arrays)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for HEVCDecoderConfiguration {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("ConfigurationVersion", 0, with_bit_width(8), as_hex()),
        codec_field!("GeneralProfileSpace", 1, with_bit_width(2), as_hex()),
        codec_field!("GeneralTierFlag", 2, with_bit_width(1), as_boolean()),
        codec_field!("GeneralProfileIdc", 3, with_bit_width(5), as_hex()),
        codec_field!(
            "GeneralProfileCompatibility",
            4,
            with_bit_width(1),
            with_length(32)
        ),
        codec_field!(
            "GeneralConstraintIndicator",
            5,
            with_bit_width(8),
            with_length(6),
            as_hex()
        ),
        codec_field!("GeneralLevelIdc", 6, with_bit_width(8), as_hex()),
        codec_field!("MinSpatialSegmentationIdc", 7, with_bit_width(12)),
        codec_field!("ParallelismType", 8, with_bit_width(2), as_hex()),
        codec_field!("ChromaFormatIdc", 9, with_bit_width(2), as_hex()),
        codec_field!("BitDepthLumaMinus8", 10, with_bit_width(3), as_hex()),
        codec_field!("BitDepthChromaMinus8", 11, with_bit_width(3), as_hex()),
        codec_field!("AvgFrameRate", 12, with_bit_width(16)),
        codec_field!("ConstantFrameRate", 13, with_bit_width(2), as_hex()),
        codec_field!("NumTemporalLayers", 14, with_bit_width(2), as_hex()),
        codec_field!("TemporalIdNested", 15, with_bit_width(2), as_hex()),
        codec_field!("LengthSizeMinusOne", 16, with_bit_width(2), as_hex()),
        codec_field!("NumOfNaluArrays", 17, with_bit_width(8), as_hex()),
        codec_field!(
            "NaluArrays",
            18,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.general_profile_space > 0x03 {
            return Err(
                invalid_value("GeneralProfileSpace", "value does not fit in 2 bits").into(),
            );
        }
        if self.general_profile_idc > 0x1f {
            return Err(invalid_value("GeneralProfileIdc", "value does not fit in 5 bits").into());
        }
        if self.min_spatial_segmentation_idc > 0x0fff {
            return Err(invalid_value(
                "MinSpatialSegmentationIdc",
                "value does not fit in 12 bits",
            )
            .into());
        }
        if self.parallelism_type > 0x03 {
            return Err(invalid_value("ParallelismType", "value does not fit in 2 bits").into());
        }
        if self.chroma_format_idc > 0x03 {
            return Err(invalid_value("ChromaFormatIdc", "value does not fit in 2 bits").into());
        }
        if self.bit_depth_luma_minus8 > 0x07 {
            return Err(invalid_value("BitDepthLumaMinus8", "value does not fit in 3 bits").into());
        }
        if self.bit_depth_chroma_minus8 > 0x07 {
            return Err(
                invalid_value("BitDepthChromaMinus8", "value does not fit in 3 bits").into(),
            );
        }
        if self.constant_frame_rate > 0x03 {
            return Err(invalid_value("ConstantFrameRate", "value does not fit in 2 bits").into());
        }
        if self.num_temporal_layers > 0x03 {
            return Err(invalid_value("NumTemporalLayers", "value does not fit in 2 bits").into());
        }
        if self.temporal_id_nested > 0x03 {
            return Err(invalid_value("TemporalIdNested", "value does not fit in 2 bits").into());
        }
        if self.length_size_minus_one > 0x03 {
            return Err(invalid_value("LengthSizeMinusOne", "value does not fit in 2 bits").into());
        }

        require_count(
            "NumOfNaluArrays",
            u32::from(self.num_of_nalu_arrays),
            self.nalu_arrays.len(),
        )?;
        let nalu_arrays = encode_hevc_nalu_arrays("NaluArrays", &self.nalu_arrays)?;

        let mut payload = Vec::with_capacity(23 + nalu_arrays.len());
        payload.push(self.configuration_version);
        payload.push(
            (self.general_profile_space << 6)
                | (u8::from(self.general_tier_flag) << 5)
                | self.general_profile_idc,
        );
        payload.extend_from_slice(&pack_hevc_profile_compatibility(
            &self.general_profile_compatibility,
        ));
        payload.extend_from_slice(&self.general_constraint_indicator);
        payload.push(self.general_level_idc);
        payload.extend_from_slice(&(0xe000 | self.min_spatial_segmentation_idc).to_be_bytes());
        payload.push(0xfc | self.parallelism_type);
        payload.push(0xfc | self.chroma_format_idc);
        payload.push(0xf8 | self.bit_depth_luma_minus8);
        payload.push(0xf8 | self.bit_depth_chroma_minus8);
        payload.extend_from_slice(&self.avg_frame_rate.to_be_bytes());
        payload.push(
            (self.constant_frame_rate << 6)
                | (self.num_temporal_layers << 4)
                | (self.temporal_id_nested << 2)
                | self.length_size_minus_one,
        );
        payload.push(self.num_of_nalu_arrays);
        payload.extend_from_slice(&nalu_arrays);

        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let start = reader.stream_position()?;
        let payload_len = usize::try_from(payload_size)
            .map_err(|_| invalid_value("Payload", "payload is too large to decode"))?;
        let payload = match read_exact_vec_untrusted(reader, payload_len) {
            Ok(payload) => payload,
            Err(error) => {
                reader.seek(SeekFrom::Start(start))?;
                return Err(error.into());
            }
        };

        let parse_result = (|| -> Result<(), CodecError> {
            if payload.len() < 23 {
                return Err(invalid_value("Payload", "payload is too short").into());
            }

            let mut offset = 0_usize;
            self.configuration_version = payload[offset];
            offset += 1;

            let profile_header = payload[offset];
            self.general_profile_space = profile_header >> 6;
            self.general_tier_flag = profile_header & 0x20 != 0;
            self.general_profile_idc = profile_header & 0x1f;
            offset += 1;

            let profile_compatibility: [u8; 4] = payload[offset..offset + 4].try_into().unwrap();
            self.general_profile_compatibility =
                unpack_hevc_profile_compatibility(&profile_compatibility);
            offset += 4;

            self.general_constraint_indicator = payload[offset..offset + 6].try_into().unwrap();
            offset += 6;

            self.general_level_idc = payload[offset];
            offset += 1;

            let segmentation = read_u16(&payload, offset);
            if segmentation >> 12 != 0x0e {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved1",
                    constant: "14",
                });
            }
            self.min_spatial_segmentation_idc = segmentation & 0x0fff;
            offset += 2;

            let parallelism = payload[offset];
            if parallelism >> 2 != 0x3f {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved2",
                    constant: "63",
                });
            }
            self.parallelism_type = parallelism & 0x03;
            offset += 1;

            let chroma_format = payload[offset];
            if chroma_format >> 2 != 0x3f {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved3",
                    constant: "63",
                });
            }
            self.chroma_format_idc = chroma_format & 0x03;
            offset += 1;

            let bit_depth_luma = payload[offset];
            if bit_depth_luma >> 3 != 0x1f {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved4",
                    constant: "31",
                });
            }
            self.bit_depth_luma_minus8 = bit_depth_luma & 0x07;
            offset += 1;

            let bit_depth_chroma = payload[offset];
            if bit_depth_chroma >> 3 != 0x1f {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved5",
                    constant: "31",
                });
            }
            self.bit_depth_chroma_minus8 = bit_depth_chroma & 0x07;
            offset += 1;

            self.avg_frame_rate = read_u16(&payload, offset);
            offset += 2;

            let layer_header = payload[offset];
            self.constant_frame_rate = layer_header >> 6;
            self.num_temporal_layers = (layer_header >> 4) & 0x03;
            self.temporal_id_nested = (layer_header >> 2) & 0x03;
            self.length_size_minus_one = layer_header & 0x03;
            offset += 1;

            self.num_of_nalu_arrays = payload[offset];
            offset += 1;

            self.nalu_arrays =
                parse_hevc_nalu_arrays("NaluArrays", &payload[offset..], self.num_of_nalu_arrays)?;

            Ok(())
        })();

        if let Err(error) = parse_result {
            reader.seek(SeekFrom::Start(start))?;
            return Err(error);
        }

        Ok(Some(payload_size))
    }
}

/// XML subtitle sample entry that stores namespace and schema strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XMLSubtitleSampleEntry {
    pub sample_entry: SampleEntry,
    pub namespace: String,
    pub schema_location: String,
    pub auxiliary_mime_types: String,
}

impl Default for XMLSubtitleSampleEntry {
    fn default() -> Self {
        Self {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"stpp"),
                data_reference_index: 0,
            },
            namespace: String::new(),
            schema_location: String::new(),
            auxiliary_mime_types: String::new(),
        }
    }
}

impl FieldHooks for XMLSubtitleSampleEntry {}

impl ImmutableBox for XMLSubtitleSampleEntry {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"stpp")
    }
}

impl MutableBox for XMLSubtitleSampleEntry {}

impl XMLSubtitleSampleEntry {
    /// Returns the whitespace-delimited namespace entries.
    pub fn namespace_list(&self) -> Vec<&str> {
        self.namespace.split_whitespace().collect()
    }

    /// Returns the whitespace-delimited schema-location entries.
    pub fn schema_location_list(&self) -> Vec<&str> {
        self.schema_location.split_whitespace().collect()
    }

    /// Returns the whitespace-delimited auxiliary MIME type entries.
    pub fn auxiliary_mime_types_list(&self) -> Vec<&str> {
        self.auxiliary_mime_types.split_whitespace().collect()
    }
}

impl FieldValueRead for XMLSubtitleSampleEntry {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataReferenceIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.sample_entry.data_reference_index,
            ))),
            "Namespace" => Ok(FieldValue::String(self.namespace.clone())),
            "SchemaLocation" => Ok(FieldValue::String(self.schema_location.clone())),
            "AuxiliaryMIMETypes" => Ok(FieldValue::String(self.auxiliary_mime_types.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for XMLSubtitleSampleEntry {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataReferenceIndex", FieldValue::Unsigned(value)) => {
                self.sample_entry.data_reference_index = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Namespace", FieldValue::String(value)) => {
                self.namespace = value;
                Ok(())
            }
            ("SchemaLocation", FieldValue::String(value)) => {
                self.schema_location = value;
                Ok(())
            }
            ("AuxiliaryMIMETypes", FieldValue::String(value)) => {
                self.auxiliary_mime_types = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for XMLSubtitleSampleEntry {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Reserved0A", 0, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0B", 1, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0C", 2, with_bit_width(16), with_constant("0")),
        codec_field!("DataReferenceIndex", 3, with_bit_width(16)),
        codec_field!(
            "Namespace",
            4,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
        codec_field!(
            "SchemaLocation",
            5,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
        codec_field!(
            "AuxiliaryMIMETypes",
            6,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
    ]);
}

/// Text subtitle sample entry that stores an optional encoding label and MIME type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSubtitleSampleEntry {
    pub sample_entry: SampleEntry,
    pub content_encoding: String,
    pub mime_format: String,
}

impl Default for TextSubtitleSampleEntry {
    fn default() -> Self {
        Self {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"sbtt"),
                data_reference_index: 0,
            },
            content_encoding: String::new(),
            mime_format: String::new(),
        }
    }
}

impl FieldHooks for TextSubtitleSampleEntry {}

impl ImmutableBox for TextSubtitleSampleEntry {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"sbtt")
    }
}

impl MutableBox for TextSubtitleSampleEntry {}

impl FieldValueRead for TextSubtitleSampleEntry {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataReferenceIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.sample_entry.data_reference_index,
            ))),
            "ContentEncoding" => Ok(FieldValue::String(self.content_encoding.clone())),
            "MIMEFormat" => Ok(FieldValue::String(self.mime_format.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for TextSubtitleSampleEntry {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataReferenceIndex", FieldValue::Unsigned(value)) => {
                self.sample_entry.data_reference_index = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ContentEncoding", FieldValue::String(value)) => {
                self.content_encoding = value;
                Ok(())
            }
            ("MIMEFormat", FieldValue::String(value)) => {
                self.mime_format = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for TextSubtitleSampleEntry {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Reserved0A", 0, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0B", 1, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0C", 2, with_bit_width(16), with_constant("0")),
        codec_field!("DataReferenceIndex", 3, with_bit_width(16)),
        codec_field!(
            "ContentEncoding",
            4,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
        codec_field!(
            "MIMEFormat",
            5,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
    ]);
}

fn is_quicktime_wave_audio_context(context: BoxLookupContext) -> bool {
    context.is_quicktime_compatible() && context.under_wave()
}

fn matches_audio_sample_entry_context(box_type: FourCc, context: BoxLookupContext) -> bool {
    (box_type == FourCc::from_bytes(*b"enca") || box_type == FourCc::from_bytes(*b"mp4a"))
        && !is_quicktime_wave_audio_context(context)
}

/// Registers the currently implemented ISO/IEC 14496-12 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register::<AVCDecoderConfiguration>(FourCc::from_bytes(*b"avcC"));
    registry.register::<Btrt>(FourCc::from_bytes(*b"btrt"));
    registry.register::<Colr>(FourCc::from_bytes(*b"colr"));
    registry.register::<Co64>(FourCc::from_bytes(*b"co64"));
    registry.register::<Cslg>(FourCc::from_bytes(*b"cslg"));
    registry.register::<Ctts>(FourCc::from_bytes(*b"ctts"));
    registry.register::<Dinf>(FourCc::from_bytes(*b"dinf"));
    registry.register::<Dref>(FourCc::from_bytes(*b"dref"));
    registry.register::<Edts>(FourCc::from_bytes(*b"edts"));
    registry.register::<Elst>(FourCc::from_bytes(*b"elst"));
    registry.register::<Emsg>(FourCc::from_bytes(*b"emsg"));
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"avc1"));
    registry.register_contextual_any::<WaveAudioData>(
        FourCc::from_bytes(*b"enca"),
        is_quicktime_wave_audio_context,
    );
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"encv"));
    registry.register::<Fiel>(FourCc::from_bytes(*b"fiel"));
    registry.register::<Frma>(FourCc::from_bytes(*b"frma"));
    registry.register::<Free>(FourCc::from_bytes(*b"free"));
    registry.register::<Ftyp>(FourCc::from_bytes(*b"ftyp"));
    registry.register::<Hdlr>(FourCc::from_bytes(*b"hdlr"));
    registry.register::<HEVCDecoderConfiguration>(FourCc::from_bytes(*b"hvcC"));
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"hev1"));
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"hvc1"));
    registry.register::<Mdat>(FourCc::from_bytes(*b"mdat"));
    registry.register::<Mdhd>(FourCc::from_bytes(*b"mdhd"));
    registry.register::<Mdia>(FourCc::from_bytes(*b"mdia"));
    registry.register::<Mehd>(FourCc::from_bytes(*b"mehd"));
    registry.register::<Meta>(FourCc::from_bytes(*b"meta"));
    registry.register::<Mfhd>(FourCc::from_bytes(*b"mfhd"));
    registry.register::<Mfra>(FourCc::from_bytes(*b"mfra"));
    registry.register::<Mfro>(FourCc::from_bytes(*b"mfro"));
    registry.register::<Minf>(FourCc::from_bytes(*b"minf"));
    registry.register::<Moof>(FourCc::from_bytes(*b"moof"));
    registry.register::<Moov>(FourCc::from_bytes(*b"moov"));
    registry.register::<Mvex>(FourCc::from_bytes(*b"mvex"));
    registry.register::<Mvhd>(FourCc::from_bytes(*b"mvhd"));
    registry.register_contextual_any::<WaveAudioData>(
        FourCc::from_bytes(*b"mp4a"),
        is_quicktime_wave_audio_context,
    );
    registry.register_dynamic_any::<AudioSampleEntry>(matches_audio_sample_entry_context);
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"mp4v"));
    registry.register::<Pasp>(FourCc::from_bytes(*b"pasp"));
    registry.register::<Saio>(FourCc::from_bytes(*b"saio"));
    registry.register::<Saiz>(FourCc::from_bytes(*b"saiz"));
    registry.register::<Sbgp>(FourCc::from_bytes(*b"sbgp"));
    registry.register::<Schi>(FourCc::from_bytes(*b"schi"));
    registry.register::<Schm>(FourCc::from_bytes(*b"schm"));
    registry.register::<TextSubtitleSampleEntry>(FourCc::from_bytes(*b"sbtt"));
    registry.register::<Sdtp>(FourCc::from_bytes(*b"sdtp"));
    registry.register::<Sgpd>(FourCc::from_bytes(*b"sgpd"));
    registry.register::<Sidx>(FourCc::from_bytes(*b"sidx"));
    registry.register::<Sinf>(FourCc::from_bytes(*b"sinf"));
    registry.register::<Skip>(FourCc::from_bytes(*b"skip"));
    registry.register::<Smhd>(FourCc::from_bytes(*b"smhd"));
    registry.register::<Stbl>(FourCc::from_bytes(*b"stbl"));
    registry.register::<Stco>(FourCc::from_bytes(*b"stco"));
    registry.register::<Stsc>(FourCc::from_bytes(*b"stsc"));
    registry.register::<Stsd>(FourCc::from_bytes(*b"stsd"));
    registry.register::<Stss>(FourCc::from_bytes(*b"stss"));
    registry.register::<Stsz>(FourCc::from_bytes(*b"stsz"));
    registry.register::<Stts>(FourCc::from_bytes(*b"stts"));
    registry.register::<Styp>(FourCc::from_bytes(*b"styp"));
    registry.register::<Tfdt>(FourCc::from_bytes(*b"tfdt"));
    registry.register::<Tfhd>(FourCc::from_bytes(*b"tfhd"));
    registry.register::<Tfra>(FourCc::from_bytes(*b"tfra"));
    registry.register::<Traf>(FourCc::from_bytes(*b"traf"));
    registry.register::<Trak>(FourCc::from_bytes(*b"trak"));
    registry.register::<Trep>(FourCc::from_bytes(*b"trep"));
    registry.register::<Trex>(FourCc::from_bytes(*b"trex"));
    registry.register::<Trun>(FourCc::from_bytes(*b"trun"));
    registry.register::<Tkhd>(FourCc::from_bytes(*b"tkhd"));
    registry.register::<Udta>(FourCc::from_bytes(*b"udta"));
    registry.register::<Url>(FourCc::from_bytes(*b"url "));
    registry.register::<Urn>(FourCc::from_bytes(*b"urn "));
    registry.register::<Vmhd>(FourCc::from_bytes(*b"vmhd"));
    registry.register::<Wave>(FourCc::from_bytes(*b"wave"));
    registry.register::<XMLSubtitleSampleEntry>(FourCc::from_bytes(*b"stpp"));
}
