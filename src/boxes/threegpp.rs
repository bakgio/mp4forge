//! 3GPP user-data metadata string boxes scoped under `udta`.

use crate::boxes::{AnyTypeBox, BoxLookupContext, BoxRegistry};
use crate::codec::{
    CodecBox, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead, FieldValueWrite,
    ImmutableBox, MutableBox,
};
use crate::{FourCc, codec_field};

const TITL: FourCc = FourCc::from_bytes(*b"titl");
const DSCP: FourCc = FourCc::from_bytes(*b"dscp");
const CPRT: FourCc = FourCc::from_bytes(*b"cprt");
const PERF: FourCc = FourCc::from_bytes(*b"perf");
const AUTH: FourCc = FourCc::from_bytes(*b"auth");
const GNRE: FourCc = FourCc::from_bytes(*b"gnre");

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

fn quote_bytes(bytes: &[u8]) -> String {
    format!("\"{}\"", escape_bytes(bytes))
}

fn escape_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| escape_char(char::from(*byte)))
        .collect::<String>()
}

fn escape_char(value: char) -> char {
    if value.is_control() || (!value.is_ascii_graphic() && value != ' ') {
        '.'
    } else {
        value
    }
}

fn is_under_udta(context: BoxLookupContext) -> bool {
    context.under_udta
}

/// 3GPP `udta` string leaf that carries a language tag and arbitrary string bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Udta3gppString {
    box_type: FourCc,
    full_box: FullBoxState,
    pub pad: bool,
    pub language: [u8; 3],
    pub data: Vec<u8>,
}

impl Default for Udta3gppString {
    fn default() -> Self {
        Self {
            box_type: TITL,
            full_box: FullBoxState::default(),
            pad: false,
            language: [0; 3],
            data: Vec::new(),
        }
    }
}

impl FieldHooks for Udta3gppString {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Data" => Some(quote_bytes(&self.data)),
            _ => None,
        }
    }
}

impl ImmutableBox for Udta3gppString {
    fn box_type(&self) -> FourCc {
        self.box_type
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Udta3gppString {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl AnyTypeBox for Udta3gppString {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.box_type = box_type;
    }
}

impl FieldValueRead for Udta3gppString {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Pad" => Ok(FieldValue::Boolean(self.pad)),
            "Language" => Ok(FieldValue::UnsignedArray(
                self.language.iter().copied().map(u64::from).collect(),
            )),
            "Data" => Ok(FieldValue::Bytes(self.data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Udta3gppString {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
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
            ("Data", FieldValue::Bytes(value)) => {
                self.data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Udta3gppString {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Pad", 2, with_bit_width(1), as_boolean(), as_hidden()),
        codec_field!(
            "Language",
            3,
            with_bit_width(5),
            with_length(3),
            as_iso639_2()
        ),
        codec_field!("Data", 4, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Registers the flat-registry-safe 3GPP `udta` metadata types in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    for box_type in [TITL, DSCP, PERF, AUTH] {
        registry.register_any::<Udta3gppString>(box_type);
    }

    registry.register_contextual_any::<Udta3gppString>(CPRT, is_under_udta);
    registry.register_contextual_any::<Udta3gppString>(GNRE, is_under_udta);
}
