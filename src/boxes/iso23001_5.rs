//! ISO/IEC 23001-5 uncompressed-audio box definitions.

use super::iso14496_12::AudioSampleEntry;
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

/// PCM configuration box carried by `ipcm` and `fpcm` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PcmC {
    full_box: FullBoxState,
    pub format_flags: u8,
    pub pcm_sample_size: u8,
}

impl FieldHooks for PcmC {}

impl ImmutableBox for PcmC {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"pcmC")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for PcmC {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl FieldValueRead for PcmC {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "FormatFlags" => Ok(FieldValue::Unsigned(u64::from(self.format_flags))),
            "PCMSampleSize" => Ok(FieldValue::Unsigned(u64::from(self.pcm_sample_size))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for PcmC {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("FormatFlags", FieldValue::Unsigned(value)) => {
                self.format_flags = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PCMSampleSize", FieldValue::Unsigned(value)) => {
                self.pcm_sample_size = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for PcmC {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("FormatFlags", 2, with_bit_width(8), as_hex()),
        codec_field!("PCMSampleSize", 3, with_bit_width(8), as_hex()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Registers the currently implemented ISO/IEC 23001-5 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"ipcm"));
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"fpcm"));
    registry.register::<PcmC>(FourCc::from_bytes(*b"pcmC"));
}
