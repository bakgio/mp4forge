//! ETSI TS 102 366 AC-3 sample-entry and decoder-configuration box definitions.

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

fn invalid_value(field_name: &'static str, reason: &'static str) -> FieldValueError {
    FieldValueError::InvalidValue { field_name, reason }
}

fn u8_from_unsigned(field_name: &'static str, value: u64) -> Result<u8, FieldValueError> {
    u8::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u8"))
}

/// AC-3 decoder configuration box carried by `ac-3` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dac3 {
    pub fscod: u8,
    pub bsid: u8,
    pub bsmod: u8,
    pub acmod: u8,
    pub lfe_on: u8,
    pub bit_rate_code: u8,
}

impl FieldHooks for Dac3 {}

impl ImmutableBox for Dac3 {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"dac3")
    }
}

impl MutableBox for Dac3 {}

impl FieldValueRead for Dac3 {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Fscod" => Ok(FieldValue::Unsigned(u64::from(self.fscod))),
            "Bsid" => Ok(FieldValue::Unsigned(u64::from(self.bsid))),
            "Bsmod" => Ok(FieldValue::Unsigned(u64::from(self.bsmod))),
            "Acmod" => Ok(FieldValue::Unsigned(u64::from(self.acmod))),
            "LfeOn" => Ok(FieldValue::Unsigned(u64::from(self.lfe_on))),
            "BitRateCode" => Ok(FieldValue::Unsigned(u64::from(self.bit_rate_code))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Dac3 {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Fscod", FieldValue::Unsigned(value)) => {
                self.fscod = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Bsid", FieldValue::Unsigned(value)) => {
                self.bsid = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Bsmod", FieldValue::Unsigned(value)) => {
                self.bsmod = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Acmod", FieldValue::Unsigned(value)) => {
                self.acmod = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LfeOn", FieldValue::Unsigned(value)) => {
                self.lfe_on = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BitRateCode", FieldValue::Unsigned(value)) => {
                self.bit_rate_code = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Dac3 {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Fscod", 0, with_bit_width(2), as_hex()),
        codec_field!("Bsid", 1, with_bit_width(5), as_hex()),
        codec_field!("Bsmod", 2, with_bit_width(3), as_hex()),
        codec_field!("Acmod", 3, with_bit_width(3), as_hex()),
        codec_field!("LfeOn", 4, with_bit_width(1), as_hex()),
        codec_field!("BitRateCode", 5, with_bit_width(5), as_hex()),
        codec_field!("Reserved", 6, with_bit_width(5), with_constant("0")),
    ]);
}

/// Registers the currently implemented ETSI TS 102 366 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"ac-3"));
    registry.register::<Dac3>(FourCc::from_bytes(*b"dac3"));
}
