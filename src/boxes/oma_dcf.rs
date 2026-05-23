//! OMA DCF decryption-related box definitions.

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

/// `ohdr` encryption-method value for already-clear payloads.
pub const OHDR_ENCRYPTION_METHOD_NULL: u8 = 0;
/// `ohdr` encryption-method value for AES-CBC protected payloads.
pub const OHDR_ENCRYPTION_METHOD_AES_CBC: u8 = 1;
/// `ohdr` encryption-method value for AES-CTR protected payloads.
pub const OHDR_ENCRYPTION_METHOD_AES_CTR: u8 = 2;

/// `ohdr` padding-scheme value for unpadded payloads.
pub const OHDR_PADDING_SCHEME_NONE: u8 = 0;
/// `ohdr` padding-scheme value for RFC 2630 block padding.
pub const OHDR_PADDING_SCHEME_RFC_2630: u8 = 1;

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

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(bytes[offset..offset + 8].try_into().unwrap())
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

fn validate_len_prefixed_string(
    field_name: &'static str,
    value: &str,
    max_len: usize,
) -> Result<(), FieldValueError> {
    if value.len() > max_len {
        return Err(invalid_value(
            field_name,
            "string length exceeds the field capacity",
        ));
    }
    Ok(())
}

fn decode_utf8_string(field_name: &'static str, bytes: &[u8]) -> Result<String, FieldValueError> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| invalid_value(field_name, "value is not valid UTF-8"))
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

macro_rules! empty_full_box_codec {
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
            const FIELD_TABLE: FieldTable = FieldTable::new(&[
                codec_field!("Version", 0, with_bit_width(8), as_version_field()),
                codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
            ]);
            const SUPPORTED_VERSIONS: &'static [u8] = &[0];
        }
    };
}

macro_rules! simple_container_box {
    ($name:ident, $box_type:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $name;

        impl_leaf_box!($name, $box_type);
        impl FieldHooks for $name {}
        empty_box_codec!($name);
    };
}

macro_rules! simple_full_container_box {
    ($name:ident, $box_type:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $name {
            full_box: FullBoxState,
        }

        impl FieldHooks for $name {}
        impl_full_box!($name, $box_type);
        empty_full_box_codec!($name);
    };
}

simple_container_box!(
    Odrm,
    *b"odrm",
    "Container box that wraps top-level OMA DRM metadata and encrypted payload atoms."
);
simple_full_container_box!(
    Odkm,
    *b"odkm",
    "Scheme-specific OMA DRM container carried under `schi`."
);

/// OMA DRM header container that carries the protected content type and nested header metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Odhe {
    full_box: FullBoxState,
    pub content_type: String,
}

impl FieldHooks for Odhe {}
impl_full_box!(Odhe, *b"odhe");

impl FieldValueRead for Odhe {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ContentType" => Ok(FieldValue::String(self.content_type.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Odhe {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ContentType", FieldValue::String(value)) => {
                validate_len_prefixed_string(field_name, &value, usize::from(u8::MAX))?;
                self.content_type = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Odhe {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "ContentType",
            2,
            with_bit_width(8),
            as_string(StringFieldMode::PascalCompatible)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        validate_len_prefixed_string("ContentType", &self.content_type, usize::from(u8::MAX))?;
        if self.version() != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }

        let mut payload = Vec::with_capacity(5 + self.content_type.len());
        payload.push(self.version());
        push_uint("Flags", &mut payload, 3, u64::from(self.flags()))?;
        payload.push(self.content_type.len() as u8);
        payload.extend_from_slice(self.content_type.as_bytes());
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        if payload_size < 5 {
            return Err(invalid_value("Payload", "payload is too short").into());
        }
        let mut header = [0_u8; 5];
        reader.read_exact(&mut header).map_err(CodecError::Io)?;

        let version = header[0];
        if version != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version,
            });
        }

        let content_type_len = usize::from(header[4]);
        let fixed_len = 5 + content_type_len;
        if payload_size < u64::try_from(fixed_len).unwrap() {
            return Err(invalid_value("Payload", "content-type bytes are truncated").into());
        }
        let mut content_type_bytes = vec![0_u8; content_type_len];
        reader
            .read_exact(&mut content_type_bytes)
            .map_err(CodecError::Io)?;

        self.full_box = FullBoxState {
            version,
            flags: ((header[1] as u32) << 16) | ((header[2] as u32) << 8) | u32::from(header[3]),
        };
        self.content_type = decode_utf8_string("ContentType", &content_type_bytes)?;
        Ok(Some(fixed_len as u64))
    }

    #[cfg(feature = "async")]
    fn custom_unmarshal_async<'a>(
        &'a mut self,
        reader: &'a mut dyn AsyncReadSeek,
        payload_size: u64,
    ) -> CodecFuture<'a, Result<Option<u64>, CodecError>> {
        Box::pin(async move {
            if payload_size < 5 {
                return Err(invalid_value("Payload", "payload is too short").into());
            }
            let mut header = [0_u8; 5];
            tokio::io::AsyncReadExt::read_exact(reader, &mut header)
                .await
                .map_err(CodecError::Io)?;

            let version = header[0];
            if version != 0 {
                return Err(CodecError::UnsupportedVersion {
                    box_type: self.box_type(),
                    version,
                });
            }

            let content_type_len = usize::from(header[4]);
            let fixed_len = 5 + content_type_len;
            if payload_size < u64::try_from(fixed_len).unwrap() {
                return Err(invalid_value("Payload", "content-type bytes are truncated").into());
            }
            let mut content_type_bytes = vec![0_u8; content_type_len];
            tokio::io::AsyncReadExt::read_exact(reader, &mut content_type_bytes)
                .await
                .map_err(CodecError::Io)?;

            self.full_box = FullBoxState {
                version,
                flags: ((header[1] as u32) << 16)
                    | ((header[2] as u32) << 8)
                    | u32::from(header[3]),
            };
            self.content_type = decode_utf8_string("ContentType", &content_type_bytes)?;
            Ok(Some(fixed_len as u64))
        })
    }
}

/// OMA DRM information box that carries encryption parameters, content identifiers, and nested metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ohdr {
    full_box: FullBoxState,
    pub encryption_method: u8,
    pub padding_scheme: u8,
    pub plaintext_length: u64,
    pub content_id: String,
    pub rights_issuer_url: String,
    pub textual_headers: Vec<u8>,
}

impl FieldHooks for Ohdr {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "TextualHeaders" => u32::try_from(self.textual_headers.len()).ok(),
            _ => None,
        }
    }
}
impl_full_box!(Ohdr, *b"ohdr");

impl FieldValueRead for Ohdr {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EncryptionMethod" => Ok(FieldValue::Unsigned(u64::from(self.encryption_method))),
            "PaddingScheme" => Ok(FieldValue::Unsigned(u64::from(self.padding_scheme))),
            "PlaintextLength" => Ok(FieldValue::Unsigned(self.plaintext_length)),
            "ContentId" => Ok(FieldValue::String(self.content_id.clone())),
            "RightsIssuerUrl" => Ok(FieldValue::String(self.rights_issuer_url.clone())),
            "TextualHeaders" => Ok(FieldValue::Bytes(self.textual_headers.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Ohdr {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EncryptionMethod", FieldValue::Unsigned(value)) => {
                self.encryption_method = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PaddingScheme", FieldValue::Unsigned(value)) => {
                self.padding_scheme = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PlaintextLength", FieldValue::Unsigned(value)) => {
                self.plaintext_length = value;
                Ok(())
            }
            ("ContentId", FieldValue::String(value)) => {
                validate_len_prefixed_string(field_name, &value, usize::from(u16::MAX))?;
                self.content_id = value;
                Ok(())
            }
            ("RightsIssuerUrl", FieldValue::String(value)) => {
                validate_len_prefixed_string(field_name, &value, usize::from(u16::MAX))?;
                self.rights_issuer_url = value;
                Ok(())
            }
            ("TextualHeaders", FieldValue::Bytes(value)) => {
                if value.len() > usize::from(u16::MAX) {
                    return Err(invalid_value(
                        field_name,
                        "payload length exceeds the field capacity",
                    ));
                }
                self.textual_headers = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Ohdr {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EncryptionMethod", 2, with_bit_width(8)),
        codec_field!("PaddingScheme", 3, with_bit_width(8)),
        codec_field!("PlaintextLength", 4, with_bit_width(64)),
        codec_field!(
            "ContentId",
            5,
            with_bit_width(8),
            as_string(StringFieldMode::PascalCompatible)
        ),
        codec_field!(
            "RightsIssuerUrl",
            6,
            with_bit_width(8),
            as_string(StringFieldMode::PascalCompatible)
        ),
        codec_field!(
            "TextualHeaders",
            7,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        validate_len_prefixed_string("ContentId", &self.content_id, usize::from(u16::MAX))?;
        validate_len_prefixed_string(
            "RightsIssuerUrl",
            &self.rights_issuer_url,
            usize::from(u16::MAX),
        )?;
        if self.textual_headers.len() > usize::from(u16::MAX) {
            return Err(invalid_value(
                "TextualHeaders",
                "payload length exceeds the field capacity",
            )
            .into());
        }
        if self.version() != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }

        let mut payload = Vec::with_capacity(
            20 + self.content_id.len() + self.rights_issuer_url.len() + self.textual_headers.len(),
        );
        payload.push(self.version());
        push_uint("Flags", &mut payload, 3, u64::from(self.flags()))?;
        payload.push(self.encryption_method);
        payload.push(self.padding_scheme);
        payload.extend_from_slice(&self.plaintext_length.to_be_bytes());
        payload.extend_from_slice(&(self.content_id.len() as u16).to_be_bytes());
        payload.extend_from_slice(&(self.rights_issuer_url.len() as u16).to_be_bytes());
        payload.extend_from_slice(&(self.textual_headers.len() as u16).to_be_bytes());
        payload.extend_from_slice(self.content_id.as_bytes());
        payload.extend_from_slice(self.rights_issuer_url.as_bytes());
        payload.extend_from_slice(&self.textual_headers);
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        if payload_size < 20 {
            return Err(invalid_value("Payload", "payload is too short").into());
        }
        let mut header = [0_u8; 20];
        reader.read_exact(&mut header).map_err(CodecError::Io)?;

        let version = header[0];
        if version != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version,
            });
        }

        let content_id_len = usize::from(read_u16(&header, 14));
        let rights_issuer_url_len = usize::from(read_u16(&header, 16));
        let textual_headers_len = usize::from(read_u16(&header, 18));
        let fixed_len = 20 + content_id_len + rights_issuer_url_len + textual_headers_len;
        if payload_size < u64::try_from(fixed_len).unwrap() {
            return Err(invalid_value("Payload", "header strings are truncated").into());
        }
        let mut content_id_bytes = vec![0_u8; content_id_len];
        let mut rights_issuer_url_bytes = vec![0_u8; rights_issuer_url_len];
        let mut textual_headers = vec![0_u8; textual_headers_len];
        reader
            .read_exact(&mut content_id_bytes)
            .map_err(CodecError::Io)?;
        reader
            .read_exact(&mut rights_issuer_url_bytes)
            .map_err(CodecError::Io)?;
        reader
            .read_exact(&mut textual_headers)
            .map_err(CodecError::Io)?;

        self.full_box = FullBoxState {
            version,
            flags: ((header[1] as u32) << 16) | ((header[2] as u32) << 8) | u32::from(header[3]),
        };
        self.encryption_method = header[4];
        self.padding_scheme = header[5];
        self.plaintext_length = read_u64(&header, 6);
        self.content_id = decode_utf8_string("ContentId", &content_id_bytes)?;
        self.rights_issuer_url = decode_utf8_string("RightsIssuerUrl", &rights_issuer_url_bytes)?;
        self.textual_headers = textual_headers;
        Ok(Some(fixed_len as u64))
    }

    #[cfg(feature = "async")]
    fn custom_unmarshal_async<'a>(
        &'a mut self,
        reader: &'a mut dyn AsyncReadSeek,
        payload_size: u64,
    ) -> CodecFuture<'a, Result<Option<u64>, CodecError>> {
        Box::pin(async move {
            if payload_size < 20 {
                return Err(invalid_value("Payload", "payload is too short").into());
            }
            let mut header = [0_u8; 20];
            tokio::io::AsyncReadExt::read_exact(reader, &mut header)
                .await
                .map_err(CodecError::Io)?;

            let version = header[0];
            if version != 0 {
                return Err(CodecError::UnsupportedVersion {
                    box_type: self.box_type(),
                    version,
                });
            }

            let content_id_len = usize::from(read_u16(&header, 14));
            let rights_issuer_url_len = usize::from(read_u16(&header, 16));
            let textual_headers_len = usize::from(read_u16(&header, 18));
            let fixed_len = 20 + content_id_len + rights_issuer_url_len + textual_headers_len;
            if payload_size < u64::try_from(fixed_len).unwrap() {
                return Err(invalid_value("Payload", "header strings are truncated").into());
            }
            let mut content_id_bytes = vec![0_u8; content_id_len];
            let mut rights_issuer_url_bytes = vec![0_u8; rights_issuer_url_len];
            let mut textual_headers = vec![0_u8; textual_headers_len];
            tokio::io::AsyncReadExt::read_exact(reader, &mut content_id_bytes)
                .await
                .map_err(CodecError::Io)?;
            tokio::io::AsyncReadExt::read_exact(reader, &mut rights_issuer_url_bytes)
                .await
                .map_err(CodecError::Io)?;
            tokio::io::AsyncReadExt::read_exact(reader, &mut textual_headers)
                .await
                .map_err(CodecError::Io)?;

            self.full_box = FullBoxState {
                version,
                flags: ((header[1] as u32) << 16)
                    | ((header[2] as u32) << 8)
                    | u32::from(header[3]),
            };
            self.encryption_method = header[4];
            self.padding_scheme = header[5];
            self.plaintext_length = read_u64(&header, 6);
            self.content_id = decode_utf8_string("ContentId", &content_id_bytes)?;
            self.rights_issuer_url =
                decode_utf8_string("RightsIssuerUrl", &rights_issuer_url_bytes)?;
            self.textual_headers = textual_headers;
            Ok(Some(fixed_len as u64))
        })
    }
}

/// OMA access-unit format box carried under `odkm`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Odaf {
    full_box: FullBoxState,
    pub selective_encryption: bool,
    pub key_indicator_length: u8,
    pub iv_length: u8,
}

impl FieldHooks for Odaf {}
impl_full_box!(Odaf, *b"odaf");

impl FieldValueRead for Odaf {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SelectiveEncryption" => Ok(FieldValue::Boolean(self.selective_encryption)),
            "KeyIndicatorLength" => Ok(FieldValue::Unsigned(u64::from(self.key_indicator_length))),
            "IvLength" => Ok(FieldValue::Unsigned(u64::from(self.iv_length))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Odaf {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SelectiveEncryption", FieldValue::Boolean(value)) => {
                self.selective_encryption = value;
                Ok(())
            }
            ("KeyIndicatorLength", FieldValue::Unsigned(value)) => {
                self.key_indicator_length = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("IvLength", FieldValue::Unsigned(value)) => {
                self.iv_length = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Odaf {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("SelectiveEncryption", 2, with_bit_width(1), as_boolean()),
        codec_field!("KeyIndicatorLength", 3, with_bit_width(8)),
        codec_field!("IvLength", 4, with_bit_width(8)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.version() != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }

        let mut payload = Vec::with_capacity(7);
        payload.push(self.version());
        push_uint("Flags", &mut payload, 3, u64::from(self.flags()))?;
        payload.push(if self.selective_encryption {
            0x80
        } else {
            0x00
        });
        payload.push(self.key_indicator_length);
        payload.push(self.iv_length);
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        if payload_size != 7 {
            return Err(invalid_value("Payload", "payload length must be exactly 7 bytes").into());
        }
        let mut payload = [0_u8; 7];
        reader.read_exact(&mut payload).map_err(CodecError::Io)?;

        let version = payload[0];
        if version != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version,
            });
        }

        self.full_box = FullBoxState {
            version,
            flags: ((payload[1] as u32) << 16) | ((payload[2] as u32) << 8) | u32::from(payload[3]),
        };
        self.selective_encryption = payload[4] & 0x80 != 0;
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
                return Err(
                    invalid_value("Payload", "payload length must be exactly 7 bytes").into(),
                );
            }
            let mut payload = [0_u8; 7];
            tokio::io::AsyncReadExt::read_exact(reader, &mut payload)
                .await
                .map_err(CodecError::Io)?;

            let version = payload[0];
            if version != 0 {
                return Err(CodecError::UnsupportedVersion {
                    box_type: self.box_type(),
                    version,
                });
            }

            self.full_box = FullBoxState {
                version,
                flags: ((payload[1] as u32) << 16)
                    | ((payload[2] as u32) << 8)
                    | u32::from(payload[3]),
            };
            self.selective_encryption = payload[4] & 0x80 != 0;
            self.key_indicator_length = payload[5];
            self.iv_length = payload[6];
            Ok(Some(payload_size))
        })
    }
}

/// OMA encrypted-payload box that stores an explicit payload length followed by encrypted bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Odda {
    full_box: FullBoxState,
    pub encrypted_payload: Vec<u8>,
}

impl FieldHooks for Odda {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "EncryptedPayload" => u32::try_from(self.encrypted_payload.len()).ok(),
            _ => None,
        }
    }
}
impl_full_box!(Odda, *b"odda");

impl Odda {
    /// Returns the explicit encrypted-data length that will be written into the payload prefix.
    pub fn encrypted_data_length(&self) -> u64 {
        self.encrypted_payload.len() as u64
    }
}

impl FieldValueRead for Odda {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EncryptedPayload" => Ok(FieldValue::Bytes(self.encrypted_payload.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Odda {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EncryptedPayload", FieldValue::Bytes(value)) => {
                self.encrypted_payload = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Odda {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "EncryptedPayload",
            2,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.version() != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }

        let mut payload = Vec::with_capacity(12 + self.encrypted_payload.len());
        payload.push(self.version());
        push_uint("Flags", &mut payload, 3, u64::from(self.flags()))?;
        payload.extend_from_slice(&self.encrypted_data_length().to_be_bytes());
        payload.extend_from_slice(&self.encrypted_payload);
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        if payload_size < 12 {
            return Err(invalid_value("Payload", "payload is too short").into());
        }
        let mut header = [0_u8; 12];
        reader.read_exact(&mut header).map_err(CodecError::Io)?;

        let version = header[0];
        if version != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version,
            });
        }

        let encrypted_data_length = usize::try_from(read_u64(&header, 4)).map_err(|_| {
            invalid_value("EncryptedPayload", "payload length does not fit in usize")
        })?;
        if payload_size != 12 + u64::try_from(encrypted_data_length).unwrap() {
            return Err(invalid_value(
                "EncryptedPayload",
                "explicit payload length does not match the actual bytes",
            )
            .into());
        }
        let mut encrypted_payload = vec![0_u8; encrypted_data_length];
        reader
            .read_exact(&mut encrypted_payload)
            .map_err(CodecError::Io)?;

        self.full_box = FullBoxState {
            version,
            flags: ((header[1] as u32) << 16) | ((header[2] as u32) << 8) | u32::from(header[3]),
        };
        self.encrypted_payload = encrypted_payload;
        Ok(Some(payload_size))
    }

    #[cfg(feature = "async")]
    fn custom_unmarshal_async<'a>(
        &'a mut self,
        reader: &'a mut dyn AsyncReadSeek,
        payload_size: u64,
    ) -> CodecFuture<'a, Result<Option<u64>, CodecError>> {
        Box::pin(async move {
            if payload_size < 12 {
                return Err(invalid_value("Payload", "payload is too short").into());
            }
            let mut header = [0_u8; 12];
            tokio::io::AsyncReadExt::read_exact(reader, &mut header)
                .await
                .map_err(CodecError::Io)?;

            let version = header[0];
            if version != 0 {
                return Err(CodecError::UnsupportedVersion {
                    box_type: self.box_type(),
                    version,
                });
            }

            let encrypted_data_length = usize::try_from(read_u64(&header, 4)).map_err(|_| {
                invalid_value("EncryptedPayload", "payload length does not fit in usize")
            })?;
            if payload_size != 12 + u64::try_from(encrypted_data_length).unwrap() {
                return Err(invalid_value(
                    "EncryptedPayload",
                    "explicit payload length does not match the actual bytes",
                )
                .into());
            }
            let mut encrypted_payload = vec![0_u8; encrypted_data_length];
            tokio::io::AsyncReadExt::read_exact(reader, &mut encrypted_payload)
                .await
                .map_err(CodecError::Io)?;

            self.full_box = FullBoxState {
                version,
                flags: ((header[1] as u32) << 16)
                    | ((header[2] as u32) << 8)
                    | u32::from(header[3]),
            };
            self.encrypted_payload = encrypted_payload;
            Ok(Some(payload_size))
        })
    }
}

/// OMA group-key box nested under `ohdr`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grpi {
    full_box: FullBoxState,
    pub key_encryption_method: u8,
    pub group_id: String,
    pub group_key: Vec<u8>,
}

impl FieldHooks for Grpi {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "GroupKey" => u32::try_from(self.group_key.len()).ok(),
            _ => None,
        }
    }
}
impl_full_box!(Grpi, *b"grpi");

impl FieldValueRead for Grpi {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "KeyEncryptionMethod" => {
                Ok(FieldValue::Unsigned(u64::from(self.key_encryption_method)))
            }
            "GroupId" => Ok(FieldValue::String(self.group_id.clone())),
            "GroupKey" => Ok(FieldValue::Bytes(self.group_key.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Grpi {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("KeyEncryptionMethod", FieldValue::Unsigned(value)) => {
                self.key_encryption_method = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("GroupId", FieldValue::String(value)) => {
                validate_len_prefixed_string(field_name, &value, usize::from(u16::MAX))?;
                self.group_id = value;
                Ok(())
            }
            ("GroupKey", FieldValue::Bytes(value)) => {
                if value.len() > usize::from(u16::MAX) {
                    return Err(invalid_value(
                        field_name,
                        "payload length exceeds the field capacity",
                    ));
                }
                self.group_key = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Grpi {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("KeyEncryptionMethod", 2, with_bit_width(8)),
        codec_field!(
            "GroupId",
            3,
            with_bit_width(8),
            as_string(StringFieldMode::PascalCompatible)
        ),
        codec_field!(
            "GroupKey",
            4,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        validate_len_prefixed_string("GroupId", &self.group_id, usize::from(u16::MAX))?;
        if self.group_key.len() > usize::from(u16::MAX) {
            return Err(
                invalid_value("GroupKey", "payload length exceeds the field capacity").into(),
            );
        }
        if self.version() != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }

        let mut payload = Vec::with_capacity(9 + self.group_id.len() + self.group_key.len());
        payload.push(self.version());
        push_uint("Flags", &mut payload, 3, u64::from(self.flags()))?;
        payload.extend_from_slice(&(self.group_id.len() as u16).to_be_bytes());
        payload.push(self.key_encryption_method);
        payload.extend_from_slice(&(self.group_key.len() as u16).to_be_bytes());
        payload.extend_from_slice(self.group_id.as_bytes());
        payload.extend_from_slice(&self.group_key);
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        if payload_size < 9 {
            return Err(invalid_value("Payload", "payload is too short").into());
        }
        let mut header = [0_u8; 9];
        reader.read_exact(&mut header).map_err(CodecError::Io)?;

        let version = header[0];
        if version != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version,
            });
        }

        let group_id_len = usize::from(read_u16(&header, 4));
        let group_key_len = usize::from(read_u16(&header, 7));
        let fixed_len = 9 + group_id_len + group_key_len;
        if payload_size != u64::try_from(fixed_len).unwrap() {
            return Err(invalid_value(
                "Payload",
                "group-id and group-key lengths do not match the payload size",
            )
            .into());
        }
        let mut group_id_bytes = vec![0_u8; group_id_len];
        let mut group_key = vec![0_u8; group_key_len];
        reader
            .read_exact(&mut group_id_bytes)
            .map_err(CodecError::Io)?;
        reader.read_exact(&mut group_key).map_err(CodecError::Io)?;

        self.full_box = FullBoxState {
            version,
            flags: ((header[1] as u32) << 16) | ((header[2] as u32) << 8) | u32::from(header[3]),
        };
        self.key_encryption_method = header[6];
        self.group_id = decode_utf8_string("GroupId", &group_id_bytes)?;
        self.group_key = group_key;
        Ok(Some(payload_size))
    }

    #[cfg(feature = "async")]
    fn custom_unmarshal_async<'a>(
        &'a mut self,
        reader: &'a mut dyn AsyncReadSeek,
        payload_size: u64,
    ) -> CodecFuture<'a, Result<Option<u64>, CodecError>> {
        Box::pin(async move {
            if payload_size < 9 {
                return Err(invalid_value("Payload", "payload is too short").into());
            }
            let mut header = [0_u8; 9];
            tokio::io::AsyncReadExt::read_exact(reader, &mut header)
                .await
                .map_err(CodecError::Io)?;

            let version = header[0];
            if version != 0 {
                return Err(CodecError::UnsupportedVersion {
                    box_type: self.box_type(),
                    version,
                });
            }

            let group_id_len = usize::from(read_u16(&header, 4));
            let group_key_len = usize::from(read_u16(&header, 7));
            let fixed_len = 9 + group_id_len + group_key_len;
            if payload_size != u64::try_from(fixed_len).unwrap() {
                return Err(invalid_value(
                    "Payload",
                    "group-id and group-key lengths do not match the payload size",
                )
                .into());
            }
            let mut group_id_bytes = vec![0_u8; group_id_len];
            let mut group_key = vec![0_u8; group_key_len];
            tokio::io::AsyncReadExt::read_exact(reader, &mut group_id_bytes)
                .await
                .map_err(CodecError::Io)?;
            tokio::io::AsyncReadExt::read_exact(reader, &mut group_key)
                .await
                .map_err(CodecError::Io)?;

            self.full_box = FullBoxState {
                version,
                flags: ((header[1] as u32) << 16)
                    | ((header[2] as u32) << 8)
                    | u32::from(header[3]),
            };
            self.key_encryption_method = header[6];
            self.group_id = decode_utf8_string("GroupId", &group_id_bytes)?;
            self.group_key = group_key;
            Ok(Some(payload_size))
        })
    }
}

/// Registers the built-in OMA DCF boxes in the supplied registry.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register::<Odrm>(FourCc::from_bytes(*b"odrm"));
    registry.register::<Odkm>(FourCc::from_bytes(*b"odkm"));
    registry.register::<Odhe>(FourCc::from_bytes(*b"odhe"));
    registry.register::<Ohdr>(FourCc::from_bytes(*b"ohdr"));
    registry.register::<Odaf>(FourCc::from_bytes(*b"odaf"));
    registry.register::<Odda>(FourCc::from_bytes(*b"odda"));
    registry.register::<Grpi>(FourCc::from_bytes(*b"grpi"));
}
