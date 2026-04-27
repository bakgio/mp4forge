//! ISO/IEC 23001-7 common-encryption box definitions.

use std::io::{SeekFrom, Write};

use crate::boxes::BoxRegistry;
use crate::codec::{
    CodecBox, CodecError, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead,
    FieldValueWrite, ImmutableBox, MutableBox, ReadSeek, read_exact_vec_untrusted,
    untrusted_prealloc_hint,
};
use crate::{FourCc, codec_field};

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

fn u32_from_unsigned(field_name: &'static str, value: u64) -> Result<u32, FieldValueError> {
    u32::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u32"))
}

fn bytes_to_uuid(field_name: &'static str, bytes: Vec<u8>) -> Result<[u8; 16], FieldValueError> {
    bytes
        .try_into()
        .map_err(|_| invalid_value(field_name, "value must be exactly 16 bytes"))
}

fn parse_pssh_kids(
    field_name: &'static str,
    bytes: Vec<u8>,
) -> Result<Vec<PsshKid>, FieldValueError> {
    let chunks = bytes.chunks_exact(16);
    if !chunks.remainder().is_empty() {
        return Err(invalid_value(
            field_name,
            "value does not align with 16-byte KID entries",
        ));
    }

    Ok(chunks
        .map(|chunk| PsshKid {
            kid: chunk.try_into().unwrap(),
        })
        .collect())
}

fn pssh_kids_to_bytes(kids: &[PsshKid]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(kids.len() * 16);
    for kid in kids {
        bytes.extend_from_slice(&kid.kid);
    }
    bytes
}

fn render_array(values: impl IntoIterator<Item = String>) -> String {
    let values = values.into_iter().collect::<Vec<_>>();
    format!("[{}]", values.join(", "))
}

fn render_uuid(value: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        value[0],
        value[1],
        value[2],
        value[3],
        value[4],
        value[5],
        value[6],
        value[7],
        value[8],
        value[9],
        value[10],
        value[11],
        value[12],
        value[13],
        value[14],
        value[15]
    )
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn render_hex_bytes(bytes: &[u8]) -> String {
    render_array(bytes.iter().map(|byte| format!("0x{:x}", byte)))
}

fn render_senc_subsamples(subsamples: &[SencSubsample]) -> String {
    render_array(subsamples.iter().map(|subsample| {
        format!(
            "{{BytesOfClearData={} BytesOfProtectedData={}}}",
            subsample.bytes_of_clear_data, subsample.bytes_of_protected_data
        )
    }))
}

pub(crate) fn render_senc_samples_display(samples: &[SencSample]) -> String {
    render_array(samples.iter().map(|sample| {
        let mut parts = vec![format!(
            "InitializationVector={}",
            render_hex_bytes(&sample.initialization_vector)
        )];
        if !sample.subsamples.is_empty() {
            parts.push(format!(
                "Subsamples={}",
                render_senc_subsamples(&sample.subsamples)
            ));
        }
        format!("{{{}}}", parts.join(" "))
    }))
}

fn require_senc_sample_count(
    field_name: &'static str,
    sample_count: u32,
    actual_count: usize,
) -> Result<(), CodecError> {
    if usize::try_from(sample_count).ok() != Some(actual_count) {
        return Err(CodecError::InvalidLength {
            field_name,
            expected: usize::try_from(sample_count).unwrap_or(usize::MAX),
            actual: actual_count,
        });
    }

    Ok(())
}

pub(crate) fn encode_senc_payload(senc: &Senc) -> Result<Vec<u8>, CodecError> {
    if !senc.is_supported_version(senc.version()) {
        return Err(CodecError::UnsupportedVersion {
            box_type: senc.box_type(),
            version: senc.version(),
        });
    }
    validate_senc_flags(senc.flags())?;
    require_senc_sample_count("Samples", senc.sample_count, senc.samples.len())?;

    let mut payload = Vec::new();
    payload.push(senc.version());
    payload.extend_from_slice(&(senc.flags() & 0x00ff_ffff).to_be_bytes()[1..]);
    payload.extend_from_slice(&senc.sample_count.to_be_bytes());
    payload.extend_from_slice(&encode_senc_samples(
        "Samples",
        &senc.samples,
        senc.uses_subsample_encryption(),
    )?);
    Ok(payload)
}

pub(crate) fn decode_senc_payload(payload: &[u8]) -> Result<Senc, CodecError> {
    if payload.len() < 8 {
        return Err(invalid_value("Payload", "payload is too short").into());
    }

    let version = payload[0];
    let flags = u32::from_be_bytes([0, payload[1], payload[2], payload[3]]);
    let mut senc = Senc::default();
    if !senc.is_supported_version(version) {
        return Err(CodecError::UnsupportedVersion {
            box_type: senc.box_type(),
            version,
        });
    }
    validate_senc_flags(flags)?;

    let sample_count = read_u32(payload, 4);
    let samples = parse_senc_samples(
        "Samples",
        &payload[8..],
        sample_count,
        flags & SENC_USE_SUBSAMPLE_ENCRYPTION != 0,
    )?;

    senc.set_version(version);
    senc.set_flags(flags);
    senc.sample_count = sample_count;
    senc.samples = samples;
    Ok(senc)
}

#[cfg(feature = "decrypt")]
pub(crate) fn decode_senc_payload_with_iv_size(
    payload: &[u8],
    iv_size: usize,
) -> Result<Senc, CodecError> {
    if payload.len() < 8 {
        return Err(invalid_value("Payload", "payload is too short").into());
    }

    let version = payload[0];
    let flags = u32::from_be_bytes([0, payload[1], payload[2], payload[3]]);
    let mut senc = Senc::default();
    if !senc.is_supported_version(version) {
        return Err(CodecError::UnsupportedVersion {
            box_type: senc.box_type(),
            version,
        });
    }
    validate_senc_flags(flags)?;

    let sample_count = read_u32(payload, 4);
    let sample_count_usize = usize::try_from(sample_count)
        .map_err(|_| invalid_value("SampleCount", "sample count does not fit in usize"))?;
    let samples = try_parse_senc_samples_with_iv_size(
        &payload[8..],
        sample_count_usize,
        iv_size,
        flags & SENC_USE_SUBSAMPLE_ENCRYPTION != 0,
    )
    .ok_or_else(|| {
        invalid_value(
            "Samples",
            "payload does not match the forced sample IV size",
        )
    })?;

    senc.set_version(version);
    senc.set_flags(flags);
    senc.sample_count = sample_count;
    senc.samples = samples;
    Ok(senc)
}

fn resolve_senc_iv_size(
    field_name: &'static str,
    samples: &[SencSample],
) -> Result<u8, FieldValueError> {
    let Some(first) = samples.first() else {
        return Ok(0);
    };

    let iv_size = u8::try_from(first.initialization_vector.len()).map_err(|_| {
        invalid_value(
            field_name,
            "initialization vector length does not fit in u8",
        )
    })?;
    if samples
        .iter()
        .any(|sample| sample.initialization_vector.len() != usize::from(iv_size))
    {
        return Err(invalid_value(
            field_name,
            "sample IV lengths do not match across entries",
        ));
    }

    Ok(iv_size)
}

fn encode_senc_samples(
    field_name: &'static str,
    samples: &[SencSample],
    use_subsample_encryption: bool,
) -> Result<Vec<u8>, FieldValueError> {
    let iv_size = usize::from(resolve_senc_iv_size(field_name, samples)?);
    let mut bytes = Vec::new();

    for sample in samples {
        if sample.initialization_vector.len() != iv_size {
            return Err(invalid_value(
                field_name,
                "sample IV lengths do not match across entries",
            ));
        }

        bytes.extend_from_slice(&sample.initialization_vector);
        if use_subsample_encryption {
            let subsample_count = u16::try_from(sample.subsamples.len())
                .map_err(|_| invalid_value(field_name, "subsample count does not fit in u16"))?;
            bytes.extend_from_slice(&subsample_count.to_be_bytes());
            for subsample in &sample.subsamples {
                bytes.extend_from_slice(&subsample.bytes_of_clear_data.to_be_bytes());
                bytes.extend_from_slice(&subsample.bytes_of_protected_data.to_be_bytes());
            }
        } else if !sample.subsamples.is_empty() {
            return Err(invalid_value(
                field_name,
                "subsample records require the UseSubSampleEncryption flag",
            ));
        }
    }

    Ok(bytes)
}

fn try_parse_senc_samples_with_iv_size(
    bytes: &[u8],
    sample_count: usize,
    iv_size: usize,
    use_subsample_encryption: bool,
) -> Option<Vec<SencSample>> {
    let mut cursor = 0_usize;
    let mut samples = Vec::with_capacity(untrusted_prealloc_hint(sample_count));

    for _ in 0..sample_count {
        if bytes.len().saturating_sub(cursor) < iv_size {
            return None;
        }

        let initialization_vector = bytes[cursor..cursor + iv_size].to_vec();
        cursor += iv_size;

        let mut subsamples = Vec::new();
        if use_subsample_encryption {
            if bytes.len().saturating_sub(cursor) < 2 {
                return None;
            }

            let subsample_count =
                usize::from(u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]));
            cursor += 2;

            let subsample_bytes = subsample_count.checked_mul(6)?;
            if bytes.len().saturating_sub(cursor) < subsample_bytes {
                return None;
            }

            subsamples = Vec::with_capacity(untrusted_prealloc_hint(subsample_count));
            for _ in 0..subsample_count {
                subsamples.push(SencSubsample {
                    bytes_of_clear_data: u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]),
                    bytes_of_protected_data: u32::from_be_bytes([
                        bytes[cursor + 2],
                        bytes[cursor + 3],
                        bytes[cursor + 4],
                        bytes[cursor + 5],
                    ]),
                });
                cursor += 6;
            }
        }

        samples.push(SencSample {
            initialization_vector,
            subsamples,
        });
    }

    (cursor == bytes.len()).then_some(samples)
}

fn parse_senc_samples(
    field_name: &'static str,
    bytes: &[u8],
    sample_count: u32,
    use_subsample_encryption: bool,
) -> Result<Vec<SencSample>, FieldValueError> {
    let sample_count = usize::try_from(sample_count)
        .map_err(|_| invalid_value(field_name, "sample count does not fit in usize"))?;
    if sample_count == 0 {
        if !bytes.is_empty() {
            return Err(invalid_value(
                field_name,
                "sample payload must be empty when sample count is zero",
            ));
        }
        return Ok(Vec::new());
    }

    let max_iv_size = bytes.len().min(usize::from(u8::MAX));
    let mut matched = None;
    for iv_size in 0..=max_iv_size {
        let Some(samples) = try_parse_senc_samples_with_iv_size(
            bytes,
            sample_count,
            iv_size,
            use_subsample_encryption,
        ) else {
            continue;
        };

        if matched.is_some() {
            return Err(invalid_value(
                field_name,
                "sample IV size is ambiguous without external encryption parameters",
            ));
        }
        matched = Some(samples);
    }

    matched.ok_or_else(|| {
        invalid_value(
            field_name,
            "sample IV size cannot be inferred from the payload",
        )
    })
}

fn validate_senc_flags(flags: u32) -> Result<(), FieldValueError> {
    if flags & !SENC_USE_SUBSAMPLE_ENCRYPTION != 0 {
        return Err(invalid_value(
            "Flags",
            "unsupported SampleEncryptionBox flags are set",
        ));
    }

    Ok(())
}

/// `senc` flag indicating that per-sample subsample encryption records are present.
pub const SENC_USE_SUBSAMPLE_ENCRYPTION: u32 = 0x000002;

/// Protection-system-specific header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pssh {
    full_box: FullBoxState,
    pub system_id: [u8; 16],
    pub kid_count: u32,
    pub kids: Vec<PsshKid>,
    pub data_size: u32,
    pub data: Vec<u8>,
}

/// One key identifier carried by a version `1` `pssh` box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PsshKid {
    pub kid: [u8; 16],
}

impl FieldHooks for Pssh {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "KIDs" => self.kid_count.checked_mul(16),
            "Data" => Some(self.data_size),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "KIDs" => Some(render_array(
                self.kids.iter().map(|kid| render_uuid(&kid.kid)),
            )),
            _ => None,
        }
    }
}

impl ImmutableBox for Pssh {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"pssh")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Pssh {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl FieldValueRead for Pssh {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SystemID" => Ok(FieldValue::Bytes(self.system_id.to_vec())),
            "KIDCount" => Ok(FieldValue::Unsigned(u64::from(self.kid_count))),
            "KIDs" => Ok(FieldValue::Bytes(pssh_kids_to_bytes(&self.kids))),
            "DataSize" => Ok(FieldValue::Unsigned(u64::from(self.data_size))),
            "Data" => Ok(FieldValue::Bytes(self.data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Pssh {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SystemID", FieldValue::Bytes(bytes)) => {
                self.system_id = bytes_to_uuid(field_name, bytes)?;
                Ok(())
            }
            ("KIDCount", FieldValue::Unsigned(value)) => {
                self.kid_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("KIDs", FieldValue::Bytes(bytes)) => {
                self.kids = parse_pssh_kids(field_name, bytes)?;
                Ok(())
            }
            ("DataSize", FieldValue::Unsigned(value)) => {
                self.data_size = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Data", FieldValue::Bytes(bytes)) => {
                self.data = bytes;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Pssh {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "SystemID",
            2,
            with_bit_width(8),
            with_length(16),
            as_bytes(),
            as_uuid()
        ),
        codec_field!("KIDCount", 3, with_bit_width(32), with_version(1)),
        codec_field!(
            "KIDs",
            4,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes(),
            with_version(1)
        ),
        codec_field!("DataSize", 5, with_bit_width(32)),
        codec_field!(
            "Data",
            6,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Track-encryption defaults carried under a protected sample entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tenc {
    full_box: FullBoxState,
    pub reserved: u8,
    pub default_crypt_byte_block: u8,
    pub default_skip_byte_block: u8,
    pub default_is_protected: u8,
    pub default_per_sample_iv_size: u8,
    pub default_kid: [u8; 16],
    pub default_constant_iv_size: u8,
    pub default_constant_iv: Vec<u8>,
}

impl FieldHooks for Tenc {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "DefaultConstantIV" => Some(u32::from(self.default_constant_iv_size)),
            _ => None,
        }
    }

    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            // The constant-IV tail exists only for protected tracks whose IV size is signaled as zero.
            "DefaultConstantIVSize" | "DefaultConstantIV" => {
                Some(self.default_is_protected == 1 && self.default_per_sample_iv_size == 0)
            }
            _ => None,
        }
    }
}

impl ImmutableBox for Tenc {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"tenc")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Tenc {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl FieldValueRead for Tenc {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Reserved" => Ok(FieldValue::Unsigned(u64::from(self.reserved))),
            "DefaultCryptByteBlock" => Ok(FieldValue::Unsigned(u64::from(
                self.default_crypt_byte_block,
            ))),
            "DefaultSkipByteBlock" => Ok(FieldValue::Unsigned(u64::from(
                self.default_skip_byte_block,
            ))),
            "DefaultIsProtected" => Ok(FieldValue::Unsigned(u64::from(self.default_is_protected))),
            "DefaultPerSampleIVSize" => Ok(FieldValue::Unsigned(u64::from(
                self.default_per_sample_iv_size,
            ))),
            "DefaultKID" => Ok(FieldValue::Bytes(self.default_kid.to_vec())),
            "DefaultConstantIVSize" => Ok(FieldValue::Unsigned(u64::from(
                self.default_constant_iv_size,
            ))),
            "DefaultConstantIV" => Ok(FieldValue::Bytes(self.default_constant_iv.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Tenc {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Reserved", FieldValue::Unsigned(value)) => {
                self.reserved = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultCryptByteBlock", FieldValue::Unsigned(value)) => {
                self.default_crypt_byte_block = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSkipByteBlock", FieldValue::Unsigned(value)) => {
                self.default_skip_byte_block = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultIsProtected", FieldValue::Unsigned(value)) => {
                self.default_is_protected = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultPerSampleIVSize", FieldValue::Unsigned(value)) => {
                self.default_per_sample_iv_size = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultKID", FieldValue::Bytes(bytes)) => {
                self.default_kid = bytes_to_uuid(field_name, bytes)?;
                Ok(())
            }
            ("DefaultConstantIVSize", FieldValue::Unsigned(value)) => {
                self.default_constant_iv_size = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultConstantIV", FieldValue::Bytes(bytes)) => {
                self.default_constant_iv = bytes;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Tenc {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Reserved", 2, with_bit_width(8)),
        codec_field!("DefaultCryptByteBlock", 3, with_bit_width(4)),
        codec_field!("DefaultSkipByteBlock", 4, with_bit_width(4)),
        codec_field!("DefaultIsProtected", 5, with_bit_width(8)),
        codec_field!("DefaultPerSampleIVSize", 6, with_bit_width(8)),
        codec_field!(
            "DefaultKID",
            7,
            with_bit_width(8),
            with_length(16),
            as_bytes(),
            as_uuid()
        ),
        codec_field!(
            "DefaultConstantIVSize",
            8,
            with_bit_width(8),
            with_dynamic_presence()
        ),
        codec_field!(
            "DefaultConstantIV",
            9,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes(),
            with_dynamic_presence()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// One clear/protected byte-range pair carried by a subsample-encrypted sample.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SencSubsample {
    pub bytes_of_clear_data: u16,
    pub bytes_of_protected_data: u32,
}

/// One sample-specific encryption record carried by `senc`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SencSample {
    pub initialization_vector: Vec<u8>,
    pub subsamples: Vec<SencSubsample>,
}

/// Sample encryption box that stores per-sample IVs and optional subsample ranges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Senc {
    full_box: FullBoxState,
    pub sample_count: u32,
    pub samples: Vec<SencSample>,
}

impl FieldHooks for Senc {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Samples" => Some(render_senc_samples_display(&self.samples)),
            _ => None,
        }
    }
}

impl ImmutableBox for Senc {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"senc")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Senc {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Senc {
    /// Returns `true` when the payload carries per-sample subsample encryption data.
    pub fn uses_subsample_encryption(&self) -> bool {
        self.flags() & SENC_USE_SUBSAMPLE_ENCRYPTION != 0
    }

    /// Returns the shared per-sample IV size when every sample record uses the same width.
    pub fn per_sample_iv_size(&self) -> Option<u8> {
        (!self.samples.is_empty())
            .then(|| resolve_senc_iv_size("Samples", &self.samples).ok())
            .flatten()
    }
}

impl FieldValueRead for Senc {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SampleCount" => Ok(FieldValue::Unsigned(u64::from(self.sample_count))),
            "Samples" => {
                if usize::try_from(self.sample_count).ok() != Some(self.samples.len()) {
                    return Err(invalid_value(
                        field_name,
                        "sample count does not match the number of sample records",
                    ));
                }
                Ok(FieldValue::Bytes(encode_senc_samples(
                    field_name,
                    &self.samples,
                    self.uses_subsample_encryption(),
                )?))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Senc {
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
            ("Samples", FieldValue::Bytes(bytes)) => {
                validate_senc_flags(self.flags())?;
                if !self.is_supported_version(self.version()) {
                    return Err(invalid_value(
                        field_name,
                        "unsupported SampleEncryptionBox version",
                    ));
                }
                self.samples = parse_senc_samples(
                    field_name,
                    &bytes,
                    self.sample_count,
                    self.uses_subsample_encryption(),
                )?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Senc {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("SampleCount", 2, with_bit_width(32)),
        codec_field!("Samples", 3, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        let payload = encode_senc_payload(self)?;
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
            *self = decode_senc_payload(&payload)?;
            Ok(())
        })();

        if let Err(error) = parse_result {
            reader.seek(SeekFrom::Start(start))?;
            return Err(error);
        }

        Ok(Some(payload_size))
    }
}

/// Registers the currently implemented ISO/IEC 23001-7 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register::<Pssh>(FourCc::from_bytes(*b"pssh"));
    registry.register::<Senc>(FourCc::from_bytes(*b"senc"));
    registry.register::<Tenc>(FourCc::from_bytes(*b"tenc"));
}
