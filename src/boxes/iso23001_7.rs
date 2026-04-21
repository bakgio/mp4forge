//! ISO/IEC 23001-7 common-encryption box definitions.

use crate::boxes::BoxRegistry;
use crate::codec::{
    CodecBox, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead, FieldValueWrite,
    ImmutableBox, MutableBox,
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

/// Registers the currently implemented ISO/IEC 23001-7 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register::<Pssh>(FourCc::from_bytes(*b"pssh"));
    registry.register::<Tenc>(FourCc::from_bytes(*b"tenc"));
}
