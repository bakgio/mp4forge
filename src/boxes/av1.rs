//! AV1 sample-entry and codec-configuration box definitions.

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

/// AV1 codec-configuration box carried by `av01` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AV1CodecConfiguration {
    pub seq_profile: u8,
    pub seq_level_idx_0: u8,
    pub seq_tier_0: u8,
    pub high_bitdepth: u8,
    pub twelve_bit: u8,
    pub monochrome: u8,
    pub chroma_subsampling_x: u8,
    pub chroma_subsampling_y: u8,
    pub chroma_sample_position: u8,
    pub initial_presentation_delay_present: u8,
    pub initial_presentation_delay_minus_one: u8,
    pub config_obus: Vec<u8>,
}

impl FieldHooks for AV1CodecConfiguration {}

impl ImmutableBox for AV1CodecConfiguration {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"av1C")
    }
}

impl MutableBox for AV1CodecConfiguration {}

impl FieldValueRead for AV1CodecConfiguration {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SeqProfile" => Ok(FieldValue::Unsigned(u64::from(self.seq_profile))),
            "SeqLevelIdx0" => Ok(FieldValue::Unsigned(u64::from(self.seq_level_idx_0))),
            "SeqTier0" => Ok(FieldValue::Unsigned(u64::from(self.seq_tier_0))),
            "HighBitdepth" => Ok(FieldValue::Unsigned(u64::from(self.high_bitdepth))),
            "TwelveBit" => Ok(FieldValue::Unsigned(u64::from(self.twelve_bit))),
            "Monochrome" => Ok(FieldValue::Unsigned(u64::from(self.monochrome))),
            "ChromaSubsamplingX" => Ok(FieldValue::Unsigned(u64::from(self.chroma_subsampling_x))),
            "ChromaSubsamplingY" => Ok(FieldValue::Unsigned(u64::from(self.chroma_subsampling_y))),
            "ChromaSamplePosition" => {
                Ok(FieldValue::Unsigned(u64::from(self.chroma_sample_position)))
            }
            "InitialPresentationDelayPresent" => Ok(FieldValue::Unsigned(u64::from(
                self.initial_presentation_delay_present,
            ))),
            "InitialPresentationDelayMinusOne" => Ok(FieldValue::Unsigned(u64::from(
                self.initial_presentation_delay_minus_one,
            ))),
            "ConfigOBUs" => Ok(FieldValue::Bytes(self.config_obus.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for AV1CodecConfiguration {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SeqProfile", FieldValue::Unsigned(value)) => {
                self.seq_profile = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SeqLevelIdx0", FieldValue::Unsigned(value)) => {
                self.seq_level_idx_0 = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SeqTier0", FieldValue::Unsigned(value)) => {
                self.seq_tier_0 = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("HighBitdepth", FieldValue::Unsigned(value)) => {
                self.high_bitdepth = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TwelveBit", FieldValue::Unsigned(value)) => {
                self.twelve_bit = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Monochrome", FieldValue::Unsigned(value)) => {
                self.monochrome = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChromaSubsamplingX", FieldValue::Unsigned(value)) => {
                self.chroma_subsampling_x = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChromaSubsamplingY", FieldValue::Unsigned(value)) => {
                self.chroma_subsampling_y = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChromaSamplePosition", FieldValue::Unsigned(value)) => {
                self.chroma_sample_position = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("InitialPresentationDelayPresent", FieldValue::Unsigned(value)) => {
                self.initial_presentation_delay_present = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("InitialPresentationDelayMinusOne", FieldValue::Unsigned(value)) => {
                self.initial_presentation_delay_minus_one = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ConfigOBUs", FieldValue::Bytes(value)) => {
                self.config_obus = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for AV1CodecConfiguration {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!(
            "Marker",
            0,
            with_bit_width(1),
            with_constant("1"),
            as_hidden()
        ),
        codec_field!(
            "Version",
            1,
            with_bit_width(7),
            with_constant("1"),
            as_hidden()
        ),
        codec_field!("SeqProfile", 2, with_bit_width(3), as_hex()),
        codec_field!("SeqLevelIdx0", 3, with_bit_width(5), as_hex()),
        codec_field!("SeqTier0", 4, with_bit_width(1), as_hex()),
        codec_field!("HighBitdepth", 5, with_bit_width(1), as_hex()),
        codec_field!("TwelveBit", 6, with_bit_width(1), as_hex()),
        codec_field!("Monochrome", 7, with_bit_width(1), as_hex()),
        codec_field!("ChromaSubsamplingX", 8, with_bit_width(1), as_hex()),
        codec_field!("ChromaSubsamplingY", 9, with_bit_width(1), as_hex()),
        codec_field!("ChromaSamplePosition", 10, with_bit_width(2), as_hex()),
        codec_field!(
            "Reserved",
            11,
            with_bit_width(3),
            with_constant("0"),
            as_hidden()
        ),
        codec_field!(
            "InitialPresentationDelayPresent",
            12,
            with_bit_width(1),
            as_hex()
        ),
        codec_field!(
            "InitialPresentationDelayMinusOne",
            13,
            with_bit_width(4),
            as_hex()
        ),
        codec_field!("ConfigOBUs", 14, with_bit_width(8), as_bytes()),
    ]);
}

/// Registers the currently implemented AV1 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"av01"));
    registry.register::<AV1CodecConfiguration>(FourCc::from_bytes(*b"av1C"));
}
