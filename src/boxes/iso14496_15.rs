//! ISO/IEC 14496-15 VVC sample-entry and decoder-configuration box definitions.

use super::iso14496_12::VisualSampleEntry;
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

fn u32_from_unsigned(field_name: &'static str, value: u64) -> Result<u32, FieldValueError> {
    if value > 0x00ff_ffff {
        return Err(invalid_value(field_name, "value does not fit in 24 bits"));
    }
    Ok(value as u32)
}

/// VVC decoder configuration box carried by `vvc1` and `vvi1` sample entries.
///
/// The decoder configuration record is preserved as raw bytes so typed extraction, rewriting, and
/// registry lookup can land cleanly before deeper VVC record parsing is added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VVCDecoderConfiguration {
    pub version: u8,
    pub flags: u32,
    pub decoder_configuration_record: Vec<u8>,
}

impl FieldHooks for VVCDecoderConfiguration {}

impl ImmutableBox for VVCDecoderConfiguration {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"vvcC")
    }

    fn version(&self) -> u8 {
        self.version
    }

    fn flags(&self) -> u32 {
        self.flags
    }
}

impl MutableBox for VVCDecoderConfiguration {
    fn set_version(&mut self, version: u8) {
        self.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.flags = flags;
    }
}

impl FieldValueRead for VVCDecoderConfiguration {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Version" => Ok(FieldValue::Unsigned(u64::from(self.version))),
            "Flags" => Ok(FieldValue::Unsigned(u64::from(self.flags))),
            "DecoderConfigurationRecord" => {
                Ok(FieldValue::Bytes(self.decoder_configuration_record.clone()))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for VVCDecoderConfiguration {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Version", FieldValue::Unsigned(value)) => {
                self.version = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Flags", FieldValue::Unsigned(value)) => {
                self.flags = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DecoderConfigurationRecord", FieldValue::Bytes(value)) => {
                self.decoder_configuration_record = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for VVCDecoderConfiguration {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "DecoderConfigurationRecord",
            2,
            with_bit_width(8),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Registers the currently implemented ISO/IEC 14496-15 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"vvc1"));
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"vvi1"));
    registry.register::<VVCDecoderConfiguration>(FourCc::from_bytes(*b"vvcC"));
}
