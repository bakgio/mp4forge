//! 3GPP metadata and sample-entry child boxes.

use crate::boxes::{AnyTypeBox, BoxLookupContext, BoxRegistry};
use crate::codec::{
    CodecBox, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead, FieldValueWrite,
    ImmutableBox, MutableBox,
};
use crate::{FourCc, codec_field};

const TITL: FourCc = FourCc::from_bytes(*b"titl");
const DSCP: FourCc = FourCc::from_bytes(*b"dscp");
const CPRT: FourCc = FourCc::from_bytes(*b"cprt");
const PERF: FourCc = FourCc::from_bytes(*b"perf");
const AUTH: FourCc = FourCc::from_bytes(*b"auth");
const GNRE: FourCc = FourCc::from_bytes(*b"gnre");
const DAMR: FourCc = FourCc::from_bytes(*b"damr");
const DQCP: FourCc = FourCc::from_bytes(*b"dqcp");
const DEVC: FourCc = FourCc::from_bytes(*b"devc");
const DSMV: FourCc = FourCc::from_bytes(*b"dsmv");
const D263_BOX: FourCc = FourCc::from_bytes(*b"d263");

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

fn quote_bytes(bytes: &[u8]) -> String {
    format!("\"{}\"", escape_bytes(bytes))
}

fn escape_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| escape_char(char::from(*byte)))
        .collect::<String>()
}

fn escape_char(value: char) -> char {
    if value.is_control() || (!value.is_ascii_graphic() && value != ' ') {
        '.'
    } else {
        value
    }
}

fn is_under_udta(context: BoxLookupContext) -> bool {
    context.under_udta
}

/// 3GPP `udta` string leaf that carries a language tag and arbitrary string bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Udta3gppString {
    box_type: FourCc,
    full_box: FullBoxState,
    pub pad: bool,
    pub language: [u8; 3],
    pub data: Vec<u8>,
}

/// AMR-family decoder configuration carried by `damr` child boxes under `samr` and `sawb`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damr {
    /// Vendor identifier carried by the sample-entry child box.
    pub vendor: u32,
    /// Decoder version carried by the sample-entry child box.
    pub decoder_version: u8,
    /// Bitmask of AMR or AMR-WB frame types present in the stream.
    pub mode_set: u16,
    /// Mode-change cadence carried by the sample-entry child box.
    pub mode_change_period: u8,
    /// Number of codec frames stored in each MP4 sample.
    pub frames_per_sample: u8,
}

/// QCELP decoder configuration carried by `dqcp` child boxes under `sqcp`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dqcp {
    /// Vendor identifier carried by the sample-entry child box.
    pub vendor: u32,
    /// Decoder version carried by the sample-entry child box.
    pub decoder_version: u8,
    /// Number of codec frames stored in each MP4 sample.
    pub frames_per_sample: u8,
}

/// EVRC decoder configuration carried by `devc` child boxes under `sevc`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Devc {
    /// Vendor identifier carried by the sample-entry child box.
    pub vendor: u32,
    /// Decoder version carried by the sample-entry child box.
    pub decoder_version: u8,
    /// Number of codec frames stored in each MP4 sample.
    pub frames_per_sample: u8,
}

/// SMV decoder configuration carried by `dsmv` child boxes under `ssmv`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dsmv {
    /// Vendor identifier carried by the sample-entry child box.
    pub vendor: u32,
    /// Decoder version carried by the sample-entry child box.
    pub decoder_version: u8,
    /// Number of codec frames stored in each MP4 sample.
    pub frames_per_sample: u8,
}

/// H.263 decoder configuration carried by `d263` child boxes under `s263`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct D263 {
    /// Vendor identifier carried by the sample-entry child box.
    pub vendor: u32,
    /// Decoder version carried by the sample-entry child box.
    pub decoder_version: u8,
    /// H.263 level carried by the sample-entry child box.
    pub h263_level: u8,
    /// H.263 profile carried by the sample-entry child box.
    pub h263_profile: u8,
}

impl FieldHooks for Damr {}

impl ImmutableBox for Damr {
    fn box_type(&self) -> FourCc {
        DAMR
    }
}

impl MutableBox for Damr {}

impl FieldValueRead for Damr {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Vendor" => Ok(FieldValue::Unsigned(u64::from(self.vendor))),
            "DecoderVersion" => Ok(FieldValue::Unsigned(u64::from(self.decoder_version))),
            "ModeSet" => Ok(FieldValue::Unsigned(u64::from(self.mode_set))),
            "ModeChangePeriod" => Ok(FieldValue::Unsigned(u64::from(self.mode_change_period))),
            "FramesPerSample" => Ok(FieldValue::Unsigned(u64::from(self.frames_per_sample))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Damr {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Vendor", FieldValue::Unsigned(value)) => {
                self.vendor = u32::try_from(value)
                    .map_err(|_| invalid_value(field_name, "value does not fit in u32"))?;
                Ok(())
            }
            ("DecoderVersion", FieldValue::Unsigned(value)) => {
                self.decoder_version = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ModeSet", FieldValue::Unsigned(value)) => {
                self.mode_set = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ModeChangePeriod", FieldValue::Unsigned(value)) => {
                self.mode_change_period = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FramesPerSample", FieldValue::Unsigned(value)) => {
                self.frames_per_sample = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Damr {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Vendor", 0, with_bit_width(32)),
        codec_field!("DecoderVersion", 1, with_bit_width(8)),
        codec_field!("ModeSet", 2, with_bit_width(16), as_hex()),
        codec_field!("ModeChangePeriod", 3, with_bit_width(8)),
        codec_field!("FramesPerSample", 4, with_bit_width(8)),
    ]);
}

macro_rules! impl_voice_decoder_config_box {
    ($type_name:ident, $box_type:ident) => {
        impl FieldHooks for $type_name {}

        impl ImmutableBox for $type_name {
            fn box_type(&self) -> FourCc {
                $box_type
            }
        }

        impl MutableBox for $type_name {}

        impl FieldValueRead for $type_name {
            fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
                match field_name {
                    "Vendor" => Ok(FieldValue::Unsigned(u64::from(self.vendor))),
                    "DecoderVersion" => Ok(FieldValue::Unsigned(u64::from(self.decoder_version))),
                    "FramesPerSample" => {
                        Ok(FieldValue::Unsigned(u64::from(self.frames_per_sample)))
                    }
                    _ => Err(missing_field(field_name)),
                }
            }
        }

        impl FieldValueWrite for $type_name {
            fn set_field_value(
                &mut self,
                field_name: &'static str,
                value: FieldValue,
            ) -> Result<(), FieldValueError> {
                match (field_name, value) {
                    ("Vendor", FieldValue::Unsigned(value)) => {
                        self.vendor = u32::try_from(value)
                            .map_err(|_| invalid_value(field_name, "value does not fit in u32"))?;
                        Ok(())
                    }
                    ("DecoderVersion", FieldValue::Unsigned(value)) => {
                        self.decoder_version = u8_from_unsigned(field_name, value)?;
                        Ok(())
                    }
                    ("FramesPerSample", FieldValue::Unsigned(value)) => {
                        self.frames_per_sample = u8_from_unsigned(field_name, value)?;
                        Ok(())
                    }
                    (field_name, value) => Err(unexpected_field(field_name, value)),
                }
            }
        }

        impl CodecBox for $type_name {
            const FIELD_TABLE: FieldTable = FieldTable::new(&[
                codec_field!("Vendor", 0, with_bit_width(32)),
                codec_field!("DecoderVersion", 1, with_bit_width(8)),
                codec_field!("FramesPerSample", 2, with_bit_width(8)),
            ]);
        }
    };
}

impl_voice_decoder_config_box!(Dqcp, DQCP);
impl_voice_decoder_config_box!(Devc, DEVC);
impl_voice_decoder_config_box!(Dsmv, DSMV);

impl FieldHooks for D263 {}

impl ImmutableBox for D263 {
    fn box_type(&self) -> FourCc {
        D263_BOX
    }
}

impl MutableBox for D263 {}

impl FieldValueRead for D263 {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Vendor" => Ok(FieldValue::Unsigned(u64::from(self.vendor))),
            "DecoderVersion" => Ok(FieldValue::Unsigned(u64::from(self.decoder_version))),
            "H263Level" => Ok(FieldValue::Unsigned(u64::from(self.h263_level))),
            "H263Profile" => Ok(FieldValue::Unsigned(u64::from(self.h263_profile))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for D263 {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Vendor", FieldValue::Unsigned(value)) => {
                self.vendor = u32::try_from(value)
                    .map_err(|_| invalid_value(field_name, "value does not fit in u32"))?;
                Ok(())
            }
            ("DecoderVersion", FieldValue::Unsigned(value)) => {
                self.decoder_version = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("H263Level", FieldValue::Unsigned(value)) => {
                self.h263_level = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("H263Profile", FieldValue::Unsigned(value)) => {
                self.h263_profile = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for D263 {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Vendor", 0, with_bit_width(32)),
        codec_field!("DecoderVersion", 1, with_bit_width(8)),
        codec_field!("H263Level", 2, with_bit_width(8)),
        codec_field!("H263Profile", 3, with_bit_width(8)),
    ]);
}

impl Default for Udta3gppString {
    fn default() -> Self {
        Self {
            box_type: TITL,
            full_box: FullBoxState::default(),
            pad: false,
            language: [0; 3],
            data: Vec::new(),
        }
    }
}

impl FieldHooks for Udta3gppString {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Data" => Some(quote_bytes(&self.data)),
            _ => None,
        }
    }
}

impl ImmutableBox for Udta3gppString {
    fn box_type(&self) -> FourCc {
        self.box_type
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Udta3gppString {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl AnyTypeBox for Udta3gppString {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.box_type = box_type;
    }
}

impl FieldValueRead for Udta3gppString {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Pad" => Ok(FieldValue::Boolean(self.pad)),
            "Language" => Ok(FieldValue::UnsignedArray(
                self.language.iter().copied().map(u64::from).collect(),
            )),
            "Data" => Ok(FieldValue::Bytes(self.data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Udta3gppString {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Pad", FieldValue::Boolean(value)) => {
                self.pad = value;
                Ok(())
            }
            ("Language", FieldValue::UnsignedArray(values)) => {
                if values.len() != 3 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 3 elements",
                    ));
                }
                self.language = [
                    u8_from_unsigned(field_name, values[0])?,
                    u8_from_unsigned(field_name, values[1])?,
                    u8_from_unsigned(field_name, values[2])?,
                ];
                Ok(())
            }
            ("Data", FieldValue::Bytes(value)) => {
                self.data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Udta3gppString {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Pad", 2, with_bit_width(1), as_boolean(), as_hidden()),
        codec_field!(
            "Language",
            3,
            with_bit_width(5),
            with_length(3),
            as_iso639_2()
        ),
        codec_field!("Data", 4, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Registers the flat-registry-safe 3GPP `udta` metadata types in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    for box_type in [TITL, DSCP, PERF, AUTH] {
        registry.register_any::<Udta3gppString>(box_type);
    }

    registry.register_contextual_any::<Udta3gppString>(CPRT, is_under_udta);
    registry.register_contextual_any::<Udta3gppString>(GNRE, is_under_udta);
    registry.register::<Damr>(DAMR);
    registry.register::<Dqcp>(DQCP);
    registry.register::<Devc>(DEVC);
    registry.register::<Dsmv>(DSMV);
    registry.register::<D263>(D263_BOX);
}
