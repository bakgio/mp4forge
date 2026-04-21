//! ISO/IEC 14496-30 WebVTT box definitions.

use super::iso14496_12::SampleEntry;
use crate::boxes::{AnyTypeBox, BoxRegistry};
use crate::codec::{
    CodecBox, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead, FieldValueWrite,
    ImmutableBox, MutableBox, StringFieldMode,
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

fn u16_from_unsigned(field_name: &'static str, value: u64) -> Result<u16, FieldValueError> {
    u16::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u16"))
}

fn u32_from_unsigned(field_name: &'static str, value: u64) -> Result<u32, FieldValueError> {
    u32::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u32"))
}

macro_rules! empty_box {
    ($name:ident, $box_type:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $name;

        impl FieldHooks for $name {}

        impl ImmutableBox for $name {
            fn box_type(&self) -> FourCc {
                FourCc::from_bytes($box_type)
            }
        }

        impl MutableBox for $name {}

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

/// WebVTT configuration box carried by `wvtt` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebVTTConfigurationBox {
    pub config: String,
}

impl FieldHooks for WebVTTConfigurationBox {}

impl ImmutableBox for WebVTTConfigurationBox {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"vttC")
    }
}

impl MutableBox for WebVTTConfigurationBox {}

impl FieldValueRead for WebVTTConfigurationBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Config" => Ok(FieldValue::String(self.config.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for WebVTTConfigurationBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Config", FieldValue::String(value)) => {
                self.config = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for WebVTTConfigurationBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[codec_field!(
        "Config",
        0,
        with_bit_width(8),
        as_string(StringFieldMode::RawBox)
    )]);
}

/// WebVTT source-label box carried by `wvtt` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebVTTSourceLabelBox {
    pub source_label: String,
}

impl FieldHooks for WebVTTSourceLabelBox {}

impl ImmutableBox for WebVTTSourceLabelBox {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"vlab")
    }
}

impl MutableBox for WebVTTSourceLabelBox {}

impl FieldValueRead for WebVTTSourceLabelBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SourceLabel" => Ok(FieldValue::String(self.source_label.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for WebVTTSourceLabelBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SourceLabel", FieldValue::String(value)) => {
                self.source_label = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for WebVTTSourceLabelBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[codec_field!(
        "SourceLabel",
        0,
        with_bit_width(8),
        as_string(StringFieldMode::RawBox)
    )]);
}

/// WebVTT sample-entry wrapper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WVTTSampleEntry {
    pub sample_entry: SampleEntry,
}

impl Default for WVTTSampleEntry {
    fn default() -> Self {
        Self {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"wvtt"),
                data_reference_index: 0,
            },
        }
    }
}

impl FieldHooks for WVTTSampleEntry {}

impl ImmutableBox for WVTTSampleEntry {
    fn box_type(&self) -> FourCc {
        self.sample_entry.box_type
    }
}

impl MutableBox for WVTTSampleEntry {}

impl AnyTypeBox for WVTTSampleEntry {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.sample_entry.box_type = box_type;
    }
}

impl FieldValueRead for WVTTSampleEntry {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataReferenceIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.sample_entry.data_reference_index,
            ))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for WVTTSampleEntry {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataReferenceIndex", FieldValue::Unsigned(value)) => {
                self.sample_entry.data_reference_index = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for WVTTSampleEntry {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Reserved0A", 0, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0B", 1, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0C", 2, with_bit_width(16), with_constant("0")),
        codec_field!("DataReferenceIndex", 3, with_bit_width(16)),
    ]);
}

empty_box!(VTTCueBox, *b"vttc", "WebVTT cue container box.");

/// WebVTT cue source-identifier box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CueSourceIDBox {
    pub source_id: u32,
}

impl FieldHooks for CueSourceIDBox {}

impl ImmutableBox for CueSourceIDBox {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"vsid")
    }
}

impl MutableBox for CueSourceIDBox {}

impl FieldValueRead for CueSourceIDBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SourceId" => Ok(FieldValue::Unsigned(u64::from(self.source_id))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for CueSourceIDBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SourceId", FieldValue::Unsigned(value)) => {
                self.source_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for CueSourceIDBox {
    const FIELD_TABLE: FieldTable =
        FieldTable::new(&[codec_field!("SourceId", 0, with_bit_width(32))]);
}

/// WebVTT cue current-time box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CueTimeBox {
    pub cue_current_time: String,
}

impl FieldHooks for CueTimeBox {}

impl ImmutableBox for CueTimeBox {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"ctim")
    }
}

impl MutableBox for CueTimeBox {}

impl FieldValueRead for CueTimeBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CueCurrentTime" => Ok(FieldValue::String(self.cue_current_time.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for CueTimeBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CueCurrentTime", FieldValue::String(value)) => {
                self.cue_current_time = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for CueTimeBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[codec_field!(
        "CueCurrentTime",
        0,
        with_bit_width(8),
        as_string(StringFieldMode::RawBox)
    )]);
}

/// WebVTT cue identifier box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CueIDBox {
    pub cue_id: String,
}

impl FieldHooks for CueIDBox {}

impl ImmutableBox for CueIDBox {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"iden")
    }
}

impl MutableBox for CueIDBox {}

impl FieldValueRead for CueIDBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CueId" => Ok(FieldValue::String(self.cue_id.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for CueIDBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CueId", FieldValue::String(value)) => {
                self.cue_id = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for CueIDBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[codec_field!(
        "CueId",
        0,
        with_bit_width(8),
        as_string(StringFieldMode::RawBox)
    )]);
}

/// WebVTT cue settings box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CueSettingsBox {
    pub settings: String,
}

impl FieldHooks for CueSettingsBox {}

impl ImmutableBox for CueSettingsBox {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"sttg")
    }
}

impl MutableBox for CueSettingsBox {}

impl FieldValueRead for CueSettingsBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Settings" => Ok(FieldValue::String(self.settings.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for CueSettingsBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Settings", FieldValue::String(value)) => {
                self.settings = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for CueSettingsBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[codec_field!(
        "Settings",
        0,
        with_bit_width(8),
        as_string(StringFieldMode::RawBox)
    )]);
}

/// WebVTT cue payload box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CuePayloadBox {
    pub cue_text: String,
}

impl FieldHooks for CuePayloadBox {}

impl ImmutableBox for CuePayloadBox {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"payl")
    }
}

impl MutableBox for CuePayloadBox {}

impl FieldValueRead for CuePayloadBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CueText" => Ok(FieldValue::String(self.cue_text.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for CuePayloadBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CueText", FieldValue::String(value)) => {
                self.cue_text = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for CuePayloadBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[codec_field!(
        "CueText",
        0,
        with_bit_width(8),
        as_string(StringFieldMode::RawBox)
    )]);
}

empty_box!(VTTEmptyCueBox, *b"vtte", "WebVTT empty-cue marker box.");

/// WebVTT additional-cue-text box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VTTAdditionalTextBox {
    pub cue_additional_text: String,
}

impl FieldHooks for VTTAdditionalTextBox {}

impl ImmutableBox for VTTAdditionalTextBox {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"vtta")
    }
}

impl MutableBox for VTTAdditionalTextBox {}

impl FieldValueRead for VTTAdditionalTextBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CueAdditionalText" => Ok(FieldValue::String(self.cue_additional_text.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for VTTAdditionalTextBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CueAdditionalText", FieldValue::String(value)) => {
                self.cue_additional_text = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for VTTAdditionalTextBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[codec_field!(
        "CueAdditionalText",
        0,
        with_bit_width(8),
        as_string(StringFieldMode::RawBox)
    )]);
}

/// Registers the currently implemented ISO/IEC 14496-30 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register::<WebVTTConfigurationBox>(FourCc::from_bytes(*b"vttC"));
    registry.register::<WebVTTSourceLabelBox>(FourCc::from_bytes(*b"vlab"));
    registry.register_any::<WVTTSampleEntry>(FourCc::from_bytes(*b"wvtt"));
    registry.register::<VTTCueBox>(FourCc::from_bytes(*b"vttc"));
    registry.register::<CueSourceIDBox>(FourCc::from_bytes(*b"vsid"));
    registry.register::<CueTimeBox>(FourCc::from_bytes(*b"ctim"));
    registry.register::<CueIDBox>(FourCc::from_bytes(*b"iden"));
    registry.register::<CueSettingsBox>(FourCc::from_bytes(*b"sttg"));
    registry.register::<CuePayloadBox>(FourCc::from_bytes(*b"payl"));
    registry.register::<VTTEmptyCueBox>(FourCc::from_bytes(*b"vtte"));
    registry.register::<VTTAdditionalTextBox>(FourCc::from_bytes(*b"vtta"));
}
