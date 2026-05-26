//! ISMA Cryp protection-related box definitions.

use std::io::Write;

#[cfg(feature = "async")]
use crate::async_io::AsyncReadSeek;
use crate::boxes::BoxRegistry;
#[cfg(feature = "async")]
use crate::codec::CodecFuture;
use crate::codec::{
    CodecBox, CodecError, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead,
    FieldValueWrite, ImmutableBox, MutableBox, ReadSeek, StringFieldMode,
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

fn decode_utf8_string(field_name: &'static str, bytes: &[u8]) -> Result<String, FieldValueError> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| invalid_value(field_name, "value is not valid UTF-8"))
}

fn write_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
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

macro_rules! impl_leaf_box {
    ($name:ident, $box_type:expr) => {
        impl ImmutableBox for $name {
            fn box_type(&self) -> FourCc {
                FourCc::from_bytes($box_type)
            }
        }

        impl MutableBox for $name {}
    };
}

/// Key-management-system box carried under `schi` for IAEC-protected sample entries.
///
/// Version `0` stores only a trailing null-terminated URI. Version `1` additionally prefixes a
/// KMS identifier and KMS version before the URI payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ikms {
    full_box: FullBoxState,
    /// Optional KMS identifier carried by version-1 payloads.
    pub kms_id: u32,
    /// Optional KMS version carried by version-1 payloads.
    pub kms_version: u32,
    /// Key-management-system URI string.
    pub kms_uri: String,
}

impl FieldHooks for Ikms {}
impl_full_box!(Ikms, *b"iKMS");

impl FieldValueRead for Ikms {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "KmsId" => Ok(FieldValue::Unsigned(u64::from(self.kms_id))),
            "KmsVersion" => Ok(FieldValue::Unsigned(u64::from(self.kms_version))),
            "KmsUri" => Ok(FieldValue::String(self.kms_uri.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Ikms {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("KmsId", FieldValue::Unsigned(value)) => {
                self.kms_id = u32::try_from(value)
                    .map_err(|_| invalid_value(field_name, "value does not fit in u32"))?;
                Ok(())
            }
            ("KmsVersion", FieldValue::Unsigned(value)) => {
                self.kms_version = u32::try_from(value)
                    .map_err(|_| invalid_value(field_name, "value does not fit in u32"))?;
                Ok(())
            }
            ("KmsUri", FieldValue::String(value)) => {
                if value.as_bytes().contains(&0) {
                    return Err(invalid_value(
                        field_name,
                        "string value must not contain embedded null bytes",
                    ));
                }
                self.kms_uri = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Ikms {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("KmsId", 2, with_bit_width(32), as_unsigned()),
        codec_field!("KmsVersion", 3, with_bit_width(32), as_unsigned()),
        codec_field!(
            "KmsUri",
            4,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.version() > 1 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }
        if self.kms_uri.as_bytes().contains(&0) {
            return Err(invalid_value(
                "KmsUri",
                "string value must not contain embedded null bytes",
            )
            .into());
        }

        let mut payload =
            Vec::with_capacity(5 + self.kms_uri.len() + if self.version() == 1 { 8 } else { 0 });
        payload.push(self.version());
        payload.extend_from_slice(&self.flags().to_be_bytes()[1..]);
        if self.version() == 1 {
            write_u32(&mut payload, self.kms_id);
            write_u32(&mut payload, self.kms_version);
        }
        payload.extend_from_slice(self.kms_uri.as_bytes());
        payload.push(0);
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        if payload_size < 4 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
        }
        let mut fixed = [0_u8; 4];
        reader.read_exact(&mut fixed)?;

        self.set_version(fixed[0]);
        if self.version() > 1 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }
        self.set_flags(u32::from_be_bytes([0, fixed[1], fixed[2], fixed[3]]));

        let mut remaining = payload_size - 4;
        if self.version() == 1 && remaining >= 8 {
            let mut version_fields = [0_u8; 8];
            reader.read_exact(&mut version_fields)?;
            self.kms_id = u32::from_be_bytes(version_fields[..4].try_into().unwrap());
            self.kms_version = u32::from_be_bytes(version_fields[4..].try_into().unwrap());
            remaining -= 8;
        } else {
            self.kms_id = 0;
            self.kms_version = 0;
        }

        if remaining != 0 {
            let mut uri_bytes = vec![
                0_u8;
                usize::try_from(remaining).map_err(|_| invalid_value(
                    "KmsUri",
                    "payload size does not fit in usize",
                ))?
            ];
            reader.read_exact(&mut uri_bytes)?;
            let uri_bytes = uri_bytes.strip_suffix(&[0]).unwrap_or(&uri_bytes);
            self.kms_uri = decode_utf8_string("KmsUri", uri_bytes)?;
        } else {
            self.kms_uri.clear();
        }

        Ok(Some(payload_size))
    }

    #[cfg(feature = "async")]
    fn custom_unmarshal_async<'a>(
        &'a mut self,
        reader: &'a mut dyn AsyncReadSeek,
        payload_size: u64,
    ) -> CodecFuture<'a, Result<Option<u64>, CodecError>> {
        Box::pin(async move {
            if payload_size < 4 {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
            }
            let mut fixed = [0_u8; 4];
            tokio::io::AsyncReadExt::read_exact(reader, &mut fixed).await?;

            self.set_version(fixed[0]);
            if self.version() > 1 {
                return Err(CodecError::UnsupportedVersion {
                    box_type: self.box_type(),
                    version: self.version(),
                });
            }
            self.set_flags(u32::from_be_bytes([0, fixed[1], fixed[2], fixed[3]]));

            let mut remaining = payload_size - 4;
            if self.version() == 1 && remaining >= 8 {
                let mut version_fields = [0_u8; 8];
                tokio::io::AsyncReadExt::read_exact(reader, &mut version_fields).await?;
                self.kms_id = u32::from_be_bytes(version_fields[..4].try_into().unwrap());
                self.kms_version = u32::from_be_bytes(version_fields[4..].try_into().unwrap());
                remaining -= 8;
            } else {
                self.kms_id = 0;
                self.kms_version = 0;
            }

            if remaining != 0 {
                let mut uri_bytes = vec![
                    0_u8;
                    usize::try_from(remaining).map_err(|_| invalid_value(
                        "KmsUri",
                        "payload size does not fit in usize",
                    ))?
                ];
                tokio::io::AsyncReadExt::read_exact(reader, &mut uri_bytes).await?;
                let uri_bytes = uri_bytes.strip_suffix(&[0]).unwrap_or(&uri_bytes);
                self.kms_uri = decode_utf8_string("KmsUri", uri_bytes)?;
            } else {
                self.kms_uri.clear();
            }

            Ok(Some(payload_size))
        })
    }
}

/// IAEC sample-format box that describes selective-encryption, key-indicator, and IV widths.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Isfm {
    full_box: FullBoxState,
    /// Whether sample payloads carry the leading selective-encryption flag byte.
    pub selective_encryption: bool,
    /// Number of bytes reserved for the key-indicator field inside encrypted samples.
    pub key_indicator_length: u8,
    /// Number of bytes reserved for the per-sample IV field inside encrypted samples.
    pub iv_length: u8,
}

impl FieldHooks for Isfm {}
impl_full_box!(Isfm, *b"iSFM");

impl FieldValueRead for Isfm {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SelectiveEncryption" => Ok(FieldValue::Unsigned(u64::from(u8::from(
                self.selective_encryption,
            )))),
            "KeyIndicatorLength" => Ok(FieldValue::Unsigned(u64::from(self.key_indicator_length))),
            "IvLength" => Ok(FieldValue::Unsigned(u64::from(self.iv_length))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Isfm {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SelectiveEncryption", FieldValue::Unsigned(value)) => {
                self.selective_encryption = match value {
                    0 => false,
                    1 => true,
                    _ => {
                        return Err(invalid_value(field_name, "value must be either 0 or 1"));
                    }
                };
                Ok(())
            }
            ("KeyIndicatorLength", FieldValue::Unsigned(value)) => {
                self.key_indicator_length = u8::try_from(value)
                    .map_err(|_| invalid_value(field_name, "value does not fit in u8"))?;
                Ok(())
            }
            ("IvLength", FieldValue::Unsigned(value)) => {
                self.iv_length = u8::try_from(value)
                    .map_err(|_| invalid_value(field_name, "value does not fit in u8"))?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Isfm {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("SelectiveEncryption", 2, with_bit_width(8), as_unsigned()),
        codec_field!("KeyIndicatorLength", 3, with_bit_width(8), as_unsigned()),
        codec_field!("IvLength", 4, with_bit_width(8), as_unsigned()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.version() != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }

        let payload = [
            self.version(),
            self.flags().to_be_bytes()[1],
            self.flags().to_be_bytes()[2],
            self.flags().to_be_bytes()[3],
            if self.selective_encryption {
                0x80
            } else {
                0x00
            },
            self.key_indicator_length,
            self.iv_length,
        ];
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        if payload_size != 7 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
        }
        let mut payload = [0_u8; 7];
        reader.read_exact(&mut payload)?;

        self.set_version(payload[0]);
        if self.version() != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }
        self.set_flags(u32::from_be_bytes([0, payload[1], payload[2], payload[3]]));
        self.selective_encryption = (payload[4] & 0x80) != 0;
        self.key_indicator_length = payload[5];
        self.iv_length = payload[6];
        Ok(Some(payload_size))
    }

    #[cfg(feature = "async")]
    fn custom_unmarshal_async<'a>(
        &'a mut self,
        reader: &'a mut dyn AsyncReadSeek,
        payload_size: u64,
    ) -> CodecFuture<'a, Result<Option<u64>, CodecError>> {
        Box::pin(async move {
            if payload_size != 7 {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
            }
            let mut payload = [0_u8; 7];
            tokio::io::AsyncReadExt::read_exact(reader, &mut payload).await?;

            self.set_version(payload[0]);
            if self.version() != 0 {
                return Err(CodecError::UnsupportedVersion {
                    box_type: self.box_type(),
                    version: self.version(),
                });
            }
            self.set_flags(u32::from_be_bytes([0, payload[1], payload[2], payload[3]]));
            self.selective_encryption = (payload[4] & 0x80) != 0;
            self.key_indicator_length = payload[5];
            self.iv_length = payload[6];
            Ok(Some(payload_size))
        })
    }
}

/// IAEC salt box carried under `schi`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Islt {
    /// Eight-byte salt prefix that seeds the stream cipher IV.
    pub salt: [u8; 8],
}

impl FieldHooks for Islt {}
impl_leaf_box!(Islt, *b"iSLT");

impl FieldValueRead for Islt {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Salt" => Ok(FieldValue::Bytes(self.salt.to_vec())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Islt {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Salt", FieldValue::Bytes(value)) => {
                self.salt = value
                    .as_slice()
                    .try_into()
                    .map_err(|_| invalid_value(field_name, "value must be exactly 8 bytes"))?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Islt {
    const FIELD_TABLE: FieldTable =
        FieldTable::new(&[codec_field!("Salt", 0, with_bit_width(8), as_bytes())]);

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        writer.write_all(&self.salt)?;
        Ok(Some(self.salt.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        if payload_size != 8 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
        }
        reader.read_exact(&mut self.salt)?;
        Ok(Some(payload_size))
    }

    #[cfg(feature = "async")]
    fn custom_unmarshal_async<'a>(
        &'a mut self,
        reader: &'a mut dyn AsyncReadSeek,
        payload_size: u64,
    ) -> CodecFuture<'a, Result<Option<u64>, CodecError>> {
        Box::pin(async move {
            if payload_size != 8 {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
            }
            tokio::io::AsyncReadExt::read_exact(reader, &mut self.salt).await?;
            Ok(Some(payload_size))
        })
    }
}

/// Registers the landed ISMA Cryp box families in the supplied registry.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register::<Ikms>(FourCc::from_bytes(*b"iKMS"));
    registry.register::<Isfm>(FourCc::from_bytes(*b"iSFM"));
    registry.register::<Islt>(FourCc::from_bytes(*b"iSLT"));
}
