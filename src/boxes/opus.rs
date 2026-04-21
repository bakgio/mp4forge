//! Opus sample-entry and decoder-configuration box definitions.

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

fn u16_from_unsigned(field_name: &'static str, value: u64) -> Result<u16, FieldValueError> {
    u16::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u16"))
}

fn u32_from_unsigned(field_name: &'static str, value: u64) -> Result<u32, FieldValueError> {
    u32::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u32"))
}

fn i16_from_signed(field_name: &'static str, value: i64) -> Result<i16, FieldValueError> {
    i16::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in i16"))
}

/// Opus decoder setup box carried by `Opus` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DOps {
    pub version: u8,
    pub output_channel_count: u8,
    pub pre_skip: u16,
    pub input_sample_rate: u32,
    pub output_gain: i16,
    pub channel_mapping_family: u8,
    pub stream_count: u8,
    pub coupled_count: u8,
    pub channel_mapping: Vec<u8>,
}

impl FieldHooks for DOps {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "ChannelMapping" => Some(u32::from(self.output_channel_count)),
            _ => None,
        }
    }

    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "StreamCount" | "CoupledCount" | "ChannelMapping" => {
                Some(self.channel_mapping_family != 0)
            }
            _ => None,
        }
    }
}

impl ImmutableBox for DOps {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"dOps")
    }
}

impl MutableBox for DOps {}

impl FieldValueRead for DOps {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Version" => Ok(FieldValue::Unsigned(u64::from(self.version))),
            "OutputChannelCount" => Ok(FieldValue::Unsigned(u64::from(self.output_channel_count))),
            "PreSkip" => Ok(FieldValue::Unsigned(u64::from(self.pre_skip))),
            "InputSampleRate" => Ok(FieldValue::Unsigned(u64::from(self.input_sample_rate))),
            "OutputGain" => Ok(FieldValue::Signed(i64::from(self.output_gain))),
            "ChannelMappingFamily" => {
                Ok(FieldValue::Unsigned(u64::from(self.channel_mapping_family)))
            }
            "StreamCount" => Ok(FieldValue::Unsigned(u64::from(self.stream_count))),
            "CoupledCount" => Ok(FieldValue::Unsigned(u64::from(self.coupled_count))),
            "ChannelMapping" => Ok(FieldValue::Bytes(self.channel_mapping.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for DOps {
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
            ("OutputChannelCount", FieldValue::Unsigned(value)) => {
                self.output_channel_count = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PreSkip", FieldValue::Unsigned(value)) => {
                self.pre_skip = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("InputSampleRate", FieldValue::Unsigned(value)) => {
                self.input_sample_rate = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("OutputGain", FieldValue::Signed(value)) => {
                self.output_gain = i16_from_signed(field_name, value)?;
                Ok(())
            }
            ("ChannelMappingFamily", FieldValue::Unsigned(value)) => {
                self.channel_mapping_family = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("StreamCount", FieldValue::Unsigned(value)) => {
                self.stream_count = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CoupledCount", FieldValue::Unsigned(value)) => {
                self.coupled_count = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChannelMapping", FieldValue::Bytes(value)) => {
                self.channel_mapping = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for DOps {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8)),
        codec_field!("OutputChannelCount", 1, with_bit_width(8), as_hex()),
        codec_field!("PreSkip", 2, with_bit_width(16)),
        codec_field!("InputSampleRate", 3, with_bit_width(32)),
        codec_field!("OutputGain", 4, with_bit_width(16), as_signed()),
        codec_field!("ChannelMappingFamily", 5, with_bit_width(8), as_hex()),
        codec_field!(
            "StreamCount",
            6,
            with_bit_width(8),
            as_hex(),
            with_dynamic_presence()
        ),
        codec_field!(
            "CoupledCount",
            7,
            with_bit_width(8),
            as_hex(),
            with_dynamic_presence()
        ),
        codec_field!(
            "ChannelMapping",
            8,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes(),
            with_dynamic_presence()
        ),
    ]);
}

/// Registers the currently implemented Opus boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"Opus"));
    registry.register::<DOps>(FourCc::from_bytes(*b"dOps"));
}
