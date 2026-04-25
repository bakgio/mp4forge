//! MPEG-H sample-entry and decoder-configuration box definitions.

use std::io::Write;

use super::iso14496_12::AudioSampleEntry;
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

/// MPEG-H decoder-configuration box carried by MPEG-H audio sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MhaC {
    /// Decoder-configuration record version.
    pub config_version: u8,
    /// MPEG-H 3D Audio profile-level indication.
    pub mpeg_h_3da_profile_level_indication: u8,
    /// Reference-channel-layout code.
    pub reference_channel_layout: u8,
    /// Declared byte length of the opaque MPEG-H configuration payload.
    pub mpeg_h_3da_config_length: u16,
    /// Opaque MPEG-H configuration bytes.
    pub mpeg_h_3da_config: Vec<u8>,
}

impl FieldHooks for MhaC {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "MpegH3DAConfig" => Some(u32::from(self.mpeg_h_3da_config_length)),
            _ => None,
        }
    }
}

impl ImmutableBox for MhaC {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"mhaC")
    }
}

impl MutableBox for MhaC {}

impl FieldValueRead for MhaC {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ConfigVersion" => Ok(FieldValue::Unsigned(u64::from(self.config_version))),
            "MpegH3DAProfileLevelIndication" => Ok(FieldValue::Unsigned(u64::from(
                self.mpeg_h_3da_profile_level_indication,
            ))),
            "ReferenceChannelLayout" => Ok(FieldValue::Unsigned(u64::from(
                self.reference_channel_layout,
            ))),
            "MpegH3DAConfigLength" => Ok(FieldValue::Unsigned(u64::from(
                self.mpeg_h_3da_config_length,
            ))),
            "MpegH3DAConfig" => Ok(FieldValue::Bytes(self.mpeg_h_3da_config.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for MhaC {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ConfigVersion", FieldValue::Unsigned(value)) => {
                self.config_version = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MpegH3DAProfileLevelIndication", FieldValue::Unsigned(value)) => {
                self.mpeg_h_3da_profile_level_indication = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ReferenceChannelLayout", FieldValue::Unsigned(value)) => {
                self.reference_channel_layout = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MpegH3DAConfigLength", FieldValue::Unsigned(value)) => {
                self.mpeg_h_3da_config_length = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MpegH3DAConfig", FieldValue::Bytes(value)) => {
                self.mpeg_h_3da_config = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for MhaC {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("ConfigVersion", 0, with_bit_width(8)),
        codec_field!("MpegH3DAProfileLevelIndication", 1, with_bit_width(8)),
        codec_field!("ReferenceChannelLayout", 2, with_bit_width(8)),
        codec_field!("MpegH3DAConfigLength", 3, with_bit_width(16)),
        codec_field!(
            "MpegH3DAConfig",
            4,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);

    fn custom_marshal(&self, _writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if usize::from(self.mpeg_h_3da_config_length) != self.mpeg_h_3da_config.len() {
            return Err(invalid_value(
                "MpegH3DAConfig",
                "length does not match MpegH3DAConfigLength",
            )
            .into());
        }
        Ok(None)
    }
}

/// Registers the currently implemented MPEG-H boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"mha1"));
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"mha2"));
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"mhm1"));
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"mhm2"));
    registry.register::<MhaC>(FourCc::from_bytes(*b"mhaC"));
}
