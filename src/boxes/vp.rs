//! VP8/VP9 sample-entry and codec-configuration box definitions.

use super::iso14496_12::VisualSampleEntry;
use crate::boxes::BoxRegistry;
use crate::codec::{
    CodecBox, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead, FieldValueWrite,
    ImmutableBox, MutableBox,
};
use crate::{FourCc, codec_field};

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

fn u16_from_unsigned(field_name: &'static str, value: u64) -> Result<u16, FieldValueError> {
    u16::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u16"))
}

/// VP codec-configuration box carried by `vp08` and `vp09` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VpCodecConfiguration {
    full_box: FullBoxState,
    pub profile: u8,
    pub level: u8,
    pub bit_depth: u8,
    pub chroma_subsampling: u8,
    pub video_full_range_flag: u8,
    pub colour_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
    pub codec_initialization_data_size: u16,
    pub codec_initialization_data: Vec<u8>,
}

impl FieldHooks for VpCodecConfiguration {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            // Encode and decode both honor the declared codec-init size field.
            "CodecInitializationData" => Some(u32::from(self.codec_initialization_data_size)),
            _ => None,
        }
    }
}

impl ImmutableBox for VpCodecConfiguration {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"vpcC")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for VpCodecConfiguration {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl FieldValueRead for VpCodecConfiguration {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Profile" => Ok(FieldValue::Unsigned(u64::from(self.profile))),
            "Level" => Ok(FieldValue::Unsigned(u64::from(self.level))),
            "BitDepth" => Ok(FieldValue::Unsigned(u64::from(self.bit_depth))),
            "ChromaSubsampling" => Ok(FieldValue::Unsigned(u64::from(self.chroma_subsampling))),
            "VideoFullRangeFlag" => Ok(FieldValue::Unsigned(u64::from(self.video_full_range_flag))),
            "ColourPrimaries" => Ok(FieldValue::Unsigned(u64::from(self.colour_primaries))),
            "TransferCharacteristics" => Ok(FieldValue::Unsigned(u64::from(
                self.transfer_characteristics,
            ))),
            "MatrixCoefficients" => Ok(FieldValue::Unsigned(u64::from(self.matrix_coefficients))),
            "CodecInitializationDataSize" => Ok(FieldValue::Unsigned(u64::from(
                self.codec_initialization_data_size,
            ))),
            "CodecInitializationData" => {
                Ok(FieldValue::Bytes(self.codec_initialization_data.clone()))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for VpCodecConfiguration {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Profile", FieldValue::Unsigned(value)) => {
                self.profile = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Level", FieldValue::Unsigned(value)) => {
                self.level = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BitDepth", FieldValue::Unsigned(value)) => {
                self.bit_depth = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChromaSubsampling", FieldValue::Unsigned(value)) => {
                self.chroma_subsampling = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("VideoFullRangeFlag", FieldValue::Unsigned(value)) => {
                self.video_full_range_flag = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ColourPrimaries", FieldValue::Unsigned(value)) => {
                self.colour_primaries = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TransferCharacteristics", FieldValue::Unsigned(value)) => {
                self.transfer_characteristics = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MatrixCoefficients", FieldValue::Unsigned(value)) => {
                self.matrix_coefficients = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CodecInitializationDataSize", FieldValue::Unsigned(value)) => {
                self.codec_initialization_data_size = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CodecInitializationData", FieldValue::Bytes(value)) => {
                self.codec_initialization_data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for VpCodecConfiguration {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Profile", 2, with_bit_width(8), as_hex()),
        codec_field!("Level", 3, with_bit_width(8), as_hex()),
        codec_field!("BitDepth", 4, with_bit_width(4), as_hex()),
        codec_field!("ChromaSubsampling", 5, with_bit_width(3), as_hex()),
        codec_field!("VideoFullRangeFlag", 6, with_bit_width(1), as_hex()),
        codec_field!("ColourPrimaries", 7, with_bit_width(8), as_hex()),
        codec_field!("TransferCharacteristics", 8, with_bit_width(8), as_hex()),
        codec_field!("MatrixCoefficients", 9, with_bit_width(8), as_hex()),
        codec_field!("CodecInitializationDataSize", 10, with_bit_width(16)),
        codec_field!(
            "CodecInitializationData",
            11,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
}

/// Registers the currently implemented VP boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"vp08"));
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"vp09"));
    registry.register::<VpCodecConfiguration>(FourCc::from_bytes(*b"vpcC"));
}
