//! AVS3 sample-entry and decoder-configuration box definitions.

use std::io::Write;

use super::iso14496_12::VisualSampleEntry;
use crate::boxes::BoxRegistry;
use crate::codec::{
    CodecBox, CodecError, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead,
    FieldValueWrite, ImmutableBox, MutableBox,
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

fn u16_from_unsigned(field_name: &'static str, value: u64) -> Result<u16, FieldValueError> {
    u16::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u16"))
}

/// AVS3 decoder-configuration box carried by `avs3` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Av3c {
    /// Decoder-configuration record version.
    pub configuration_version: u8,
    /// Declared byte length of the sequence header.
    pub sequence_header_length: u16,
    /// Opaque AVS3 sequence-header bytes.
    pub sequence_header: Vec<u8>,
    /// Two-bit library-dependency identifier.
    pub library_dependency_idc: u8,
}

impl FieldHooks for Av3c {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "SequenceHeader" => Some(u32::from(self.sequence_header_length)),
            _ => None,
        }
    }
}

impl ImmutableBox for Av3c {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"av3c")
    }
}

impl MutableBox for Av3c {}

impl FieldValueRead for Av3c {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ConfigurationVersion" => {
                Ok(FieldValue::Unsigned(u64::from(self.configuration_version)))
            }
            "SequenceHeaderLength" => {
                Ok(FieldValue::Unsigned(u64::from(self.sequence_header_length)))
            }
            "SequenceHeader" => Ok(FieldValue::Bytes(self.sequence_header.clone())),
            "LibraryDependencyIDC" => {
                Ok(FieldValue::Unsigned(u64::from(self.library_dependency_idc)))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Av3c {
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
            ("SequenceHeaderLength", FieldValue::Unsigned(value)) => {
                self.sequence_header_length = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SequenceHeader", FieldValue::Bytes(value)) => {
                self.sequence_header = value;
                Ok(())
            }
            ("LibraryDependencyIDC", FieldValue::Unsigned(value)) => {
                self.library_dependency_idc = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Av3c {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("ConfigurationVersion", 0, with_bit_width(8)),
        codec_field!("SequenceHeaderLength", 1, with_bit_width(16)),
        codec_field!(
            "SequenceHeader",
            2,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
        codec_field!(
            "Reserved",
            3,
            with_bit_width(6),
            with_constant("63"),
            as_hidden()
        ),
        codec_field!("LibraryDependencyIDC", 4, with_bit_width(2), as_hex()),
    ]);

    fn custom_marshal(&self, _writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if usize::from(self.sequence_header_length) != self.sequence_header.len() {
            return Err(invalid_value(
                "SequenceHeader",
                "length does not match SequenceHeaderLength",
            )
            .into());
        }
        if self.library_dependency_idc > 0x03 {
            return Err(
                invalid_value("LibraryDependencyIDC", "value does not fit in 2 bits").into(),
            );
        }
        Ok(None)
    }
}

/// Registers the currently implemented AVS3 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"avs3"));
    registry.register::<Av3c>(FourCc::from_bytes(*b"av3c"));
}
