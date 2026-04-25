//! ETSI TS 103 190 AC-4 sample-entry and decoder-configuration box definitions.

use super::iso14496_12::AudioSampleEntry;
use crate::boxes::BoxRegistry;
use crate::codec::{
    CodecBox, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead, FieldValueWrite,
    ImmutableBox, MutableBox,
};
use crate::{FourCc, codec_field};

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

/// AC-4 decoder configuration box carried by `ac-4` sample entries.
///
/// The decoder-specific syntax is intentionally preserved as raw bytes so the box remains
/// roundtrip-safe while the crate grows its typed AC-4 coverage incrementally.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dac4 {
    pub data: Vec<u8>,
}

impl FieldHooks for Dac4 {}

impl ImmutableBox for Dac4 {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"dac4")
    }
}

impl MutableBox for Dac4 {}

impl FieldValueRead for Dac4 {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Data" => Ok(FieldValue::Bytes(self.data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Dac4 {
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

impl CodecBox for Dac4 {
    const FIELD_TABLE: FieldTable =
        FieldTable::new(&[codec_field!("Data", 0, with_bit_width(8), as_bytes())]);
}

/// Registers the currently implemented ETSI TS 103 190 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"ac-4"));
    registry.register::<Dac4>(FourCc::from_bytes(*b"dac4"));
}
