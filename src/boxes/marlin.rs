//! Marlin IPMP decryption-related box definitions and payload helpers.

use std::fmt::Write as _;

use crate::boxes::BoxRegistry;
use crate::codec::{
    CodecBox, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead, FieldValueWrite,
    ImmutableBox, MutableBox,
};
use crate::{FourCc, codec_field};

/// File-type brand used by Marlin IPMP protected MP4-family files.
pub const MARLIN_BRAND_MGSV: FourCc = FourCc::from_bytes(*b"MGSV");

/// IPMP descriptor type used by Marlin `MGSV` object-descriptor data.
pub const MARLIN_IPMPS_TYPE_MGSV: u16 = 0xA551;

/// Marlin track-key protection scheme carried by short-form `schm`.
pub const PROTECTION_SCHEME_TYPE_MARLIN_ACBC: FourCc = FourCc::from_bytes(*b"ACBC");

/// Marlin group-key protection scheme carried by short-form `schm`.
pub const PROTECTION_SCHEME_TYPE_MARLIN_ACGK: FourCc = FourCc::from_bytes(*b"ACGK");

/// Marlin `styp` value used for audio tracks inside carried `sinf` atoms.
pub const MARLIN_STYP_AUDIO: &str = "urn:marlin:organization:sne:content-type:audio";

/// Marlin `styp` value used for video tracks inside carried `sinf` atoms.
pub const MARLIN_STYP_VIDEO: &str = "urn:marlin:organization:sne:content-type:video";

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

fn render_hex_bytes(bytes: &[u8]) -> String {
    let mut rendered = String::from("[");
    let mut first = true;
    for byte in bytes {
        if !first {
            rendered.push_str(", ");
        }
        first = false;
        let _ = write!(&mut rendered, "0x{:x}", byte);
    }
    rendered.push(']');
    rendered
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

/// Container atom that groups Marlin stream-attribute records under `schi`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Satr;

impl_leaf_box!(Satr, *b"satr");
impl FieldHooks for Satr {}

impl FieldValueRead for Satr {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        Err(missing_field(field_name))
    }
}

impl FieldValueWrite for Satr {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        Err(unexpected_field(field_name, value))
    }
}

impl CodecBox for Satr {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[]);
}

/// Raw `hmac` payload carried under Marlin `schi`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hmac {
    /// Raw keyed-hash bytes.
    pub data: Vec<u8>,
}

impl_leaf_box!(Hmac, *b"hmac");

impl FieldHooks for Hmac {
    fn display_field(&self, name: &'static str) -> Option<String> {
        (name == "Data").then(|| render_hex_bytes(&self.data))
    }
}

impl FieldValueRead for Hmac {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Data" => Ok(FieldValue::Bytes(self.data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Hmac {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Data", FieldValue::Bytes(value)) => {
                self.data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Hmac {
    const FIELD_TABLE: FieldTable =
        FieldTable::new(&[codec_field!("Data", 0, with_bit_width(8), as_bytes())]);
}

/// Raw `gkey` payload carried under Marlin `schi` for group-key unwrap data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gkey {
    /// Raw wrapped group-key bytes.
    pub data: Vec<u8>,
}

impl_leaf_box!(Gkey, *b"gkey");

impl FieldHooks for Gkey {
    fn display_field(&self, name: &'static str) -> Option<String> {
        (name == "Data").then(|| render_hex_bytes(&self.data))
    }
}

impl FieldValueRead for Gkey {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Data" => Ok(FieldValue::Bytes(self.data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Gkey {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Data", FieldValue::Bytes(value)) => {
                self.data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Gkey {
    const FIELD_TABLE: FieldTable =
        FieldTable::new(&[codec_field!("Data", 0, with_bit_width(8), as_bytes())]);
}

/// Context-specific payload helper for the Marlin `styp` atom carried under `satr`.
///
/// This helper is not globally registered because `styp` already has the standard segment-type
/// meaning elsewhere in the MP4 box catalog. The decode path mirrors the current reference
/// behavior by forcing the final payload byte to NUL before extracting the string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarlinStyp {
    /// The carried Marlin stream-type URN.
    pub value: String,
}

impl MarlinStyp {
    /// Decodes one Marlin `styp` payload from the raw atom bytes that follow the box header.
    pub fn parse_payload(bytes: &[u8]) -> Result<Self, FieldValueError> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }

        let mut forced_nul = bytes.to_vec();
        *forced_nul.last_mut().unwrap() = 0;
        let string_end = forced_nul.iter().position(|byte| *byte == 0).unwrap();
        let value = String::from_utf8(forced_nul[..string_end].to_vec())
            .map_err(|_| invalid_value("Value", "value is not valid UTF-8"))?;
        Ok(Self { value })
    }

    /// Encodes one Marlin `styp` payload as a NUL-terminated byte string.
    pub fn encode_payload(&self) -> Result<Vec<u8>, FieldValueError> {
        if self.value.as_bytes().contains(&0) {
            return Err(invalid_value("Value", "string contains an embedded NUL"));
        }

        let mut payload = self.value.as_bytes().to_vec();
        payload.push(0);
        Ok(payload)
    }
}

/// Context-specific short-form `schm` payload carried inside Marlin IPMP `sinf` atoms.
///
/// This helper is not globally registered because `schm` already has the standard full-length
/// scheme-version layout elsewhere in the MP4 box catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarlinShortSchm {
    /// The carried scheme type such as `ACBC` or `ACGK`.
    pub scheme_type: FourCc,
    /// The carried 16-bit scheme version.
    pub scheme_version: u16,
}

impl Default for MarlinShortSchm {
    fn default() -> Self {
        Self {
            scheme_type: FourCc::from_bytes(*b"\0\0\0\0"),
            scheme_version: 0,
        }
    }
}

impl MarlinShortSchm {
    /// Decodes the six-byte Marlin short-form `schm` payload after the full-box header.
    pub fn parse_payload(bytes: &[u8]) -> Result<Self, FieldValueError> {
        if bytes.len() != 6 {
            return Err(invalid_value(
                "Payload",
                "expected a 6-byte Marlin short-form schm payload",
            ));
        }

        Ok(Self {
            scheme_type: FourCc::from_bytes(bytes[..4].try_into().unwrap()),
            scheme_version: u16::from_be_bytes(bytes[4..6].try_into().unwrap()),
        })
    }

    /// Encodes the six-byte Marlin short-form `schm` payload after the full-box header.
    pub fn encode_payload(&self) -> [u8; 6] {
        let mut payload = [0_u8; 6];
        payload[..4].copy_from_slice(self.scheme_type.as_bytes());
        payload[4..6].copy_from_slice(&self.scheme_version.to_be_bytes());
        payload
    }

    /// Returns whether this short-form scheme selects the track-key branch.
    pub fn uses_track_key(&self) -> bool {
        self.scheme_type == PROTECTION_SCHEME_TYPE_MARLIN_ACBC && self.scheme_version == 0x0100
    }

    /// Returns whether this short-form scheme selects the group-key branch.
    pub fn uses_group_key(&self) -> bool {
        self.scheme_type == PROTECTION_SCHEME_TYPE_MARLIN_ACGK && self.scheme_version == 0x0100
    }
}

/// Registers the currently implemented globally unique Marlin atoms in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register::<Satr>(FourCc::from_bytes(*b"satr"));
    registry.register::<Hmac>(FourCc::from_bytes(*b"hmac"));
    registry.register::<Gkey>(FourCc::from_bytes(*b"gkey"));
}
