//! DTS sample-entry child box definitions.

use std::io::{Cursor, Write};

#[cfg(feature = "async")]
use crate::async_io::{AsyncReadSeek, AsyncWriteSeek};
use crate::bitio::{BitReader, BitWriter};
use crate::codec::{
    CodecBox, CodecError, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead,
    FieldValueWrite, ImmutableBox, MutableBox, ReadSeek, read_exact_vec_untrusted,
};
#[cfg(feature = "async")]
use crate::codec::{CodecFuture, read_exact_vec_untrusted_async};
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

fn read_bits_u8(
    reader: &mut BitReader<Cursor<&[u8]>>,
    width: usize,
    _field_name: &'static str,
) -> Result<u8, CodecError> {
    Ok(reader.read_bits(width)?[0])
}

/// DTS core-specific configuration box carried by DTS sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ddts {
    /// Decoder sample rate.
    pub sampling_frequency: u32,
    /// Declared maximum bitrate.
    pub max_bitrate: u32,
    /// Declared average bitrate.
    pub avg_bitrate: u32,
    /// Source sample depth in bits.
    pub sample_depth: u8,
    /// Two-bit DTS frame-duration code.
    pub frame_duration: u8,
    /// Five-bit DTS stream-construction code.
    pub stream_construction: u8,
    /// Whether the core stream carries an LFE channel.
    pub core_lfe_present: bool,
    /// Six-bit DTS core-layout code.
    pub core_layout: u8,
    /// Fourteen-bit DTS core-size value.
    pub core_size: u16,
    /// Whether stereo downmix is signaled.
    pub stereo_downmix: bool,
    /// Three-bit representation-type code.
    pub representation_type: u8,
    /// Sixteen-bit channel-layout mask.
    pub channel_layout: u16,
    /// Whether the stream is flagged as a multi-asset presentation.
    pub multi_asset_flag: bool,
    /// Whether low-bitrate duration modulation is present.
    pub lbr_duration_mod: bool,
}

impl FieldHooks for Ddts {}

impl ImmutableBox for Ddts {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"ddts")
    }
}

impl MutableBox for Ddts {}

impl FieldValueRead for Ddts {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SamplingFrequency" => Ok(FieldValue::Unsigned(u64::from(self.sampling_frequency))),
            "MaxBitrate" => Ok(FieldValue::Unsigned(u64::from(self.max_bitrate))),
            "AvgBitrate" => Ok(FieldValue::Unsigned(u64::from(self.avg_bitrate))),
            "SampleDepth" => Ok(FieldValue::Unsigned(u64::from(self.sample_depth))),
            "FrameDuration" => Ok(FieldValue::Unsigned(u64::from(self.frame_duration))),
            "StreamConstruction" => Ok(FieldValue::Unsigned(u64::from(self.stream_construction))),
            "CoreLFEPresent" => Ok(FieldValue::Boolean(self.core_lfe_present)),
            "CoreLayout" => Ok(FieldValue::Unsigned(u64::from(self.core_layout))),
            "CoreSize" => Ok(FieldValue::Unsigned(u64::from(self.core_size))),
            "StereoDownmix" => Ok(FieldValue::Boolean(self.stereo_downmix)),
            "RepresentationType" => Ok(FieldValue::Unsigned(u64::from(self.representation_type))),
            "ChannelLayout" => Ok(FieldValue::Unsigned(u64::from(self.channel_layout))),
            "MultiAssetFlag" => Ok(FieldValue::Boolean(self.multi_asset_flag)),
            "LbrDurationMod" => Ok(FieldValue::Boolean(self.lbr_duration_mod)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Ddts {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SamplingFrequency", FieldValue::Unsigned(value)) => {
                self.sampling_frequency = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MaxBitrate", FieldValue::Unsigned(value)) => {
                self.max_bitrate = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("AvgBitrate", FieldValue::Unsigned(value)) => {
                self.avg_bitrate = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleDepth", FieldValue::Unsigned(value)) => {
                self.sample_depth = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FrameDuration", FieldValue::Unsigned(value)) => {
                self.frame_duration = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("StreamConstruction", FieldValue::Unsigned(value)) => {
                self.stream_construction = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CoreLFEPresent", FieldValue::Boolean(value)) => {
                self.core_lfe_present = value;
                Ok(())
            }
            ("CoreLayout", FieldValue::Unsigned(value)) => {
                self.core_layout = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CoreSize", FieldValue::Unsigned(value)) => {
                self.core_size = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("StereoDownmix", FieldValue::Boolean(value)) => {
                self.stereo_downmix = value;
                Ok(())
            }
            ("RepresentationType", FieldValue::Unsigned(value)) => {
                self.representation_type = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChannelLayout", FieldValue::Unsigned(value)) => {
                self.channel_layout = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MultiAssetFlag", FieldValue::Boolean(value)) => {
                self.multi_asset_flag = value;
                Ok(())
            }
            ("LbrDurationMod", FieldValue::Boolean(value)) => {
                self.lbr_duration_mod = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Ddts {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("SamplingFrequency", 0, with_bit_width(32)),
        codec_field!("MaxBitrate", 1, with_bit_width(32)),
        codec_field!("AvgBitrate", 2, with_bit_width(32)),
        codec_field!("SampleDepth", 3, with_bit_width(8)),
        codec_field!("FrameDuration", 4, with_bit_width(2)),
        codec_field!("StreamConstruction", 5, with_bit_width(5)),
        codec_field!("CoreLFEPresent", 6, with_bit_width(1), as_boolean()),
        codec_field!("CoreLayout", 7, with_bit_width(6)),
        codec_field!("CoreSize", 8, with_bit_width(14)),
        codec_field!("StereoDownmix", 9, with_bit_width(1), as_boolean()),
        codec_field!("RepresentationType", 10, with_bit_width(3)),
        codec_field!("ChannelLayout", 11, with_bit_width(16)),
        codec_field!("MultiAssetFlag", 12, with_bit_width(1), as_boolean()),
        codec_field!("LbrDurationMod", 13, with_bit_width(1), as_boolean()),
    ]);

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.frame_duration > 0x03 {
            return Err(invalid_value("FrameDuration", "value does not fit in 2 bits").into());
        }
        if self.stream_construction > 0x1f {
            return Err(invalid_value("StreamConstruction", "value does not fit in 5 bits").into());
        }
        if self.core_layout > 0x3f {
            return Err(invalid_value("CoreLayout", "value does not fit in 6 bits").into());
        }
        if self.core_size > 0x3fff {
            return Err(invalid_value("CoreSize", "value does not fit in 14 bits").into());
        }
        if self.representation_type > 0x07 {
            return Err(invalid_value("RepresentationType", "value does not fit in 3 bits").into());
        }

        writer.write_all(&self.sampling_frequency.to_be_bytes())?;
        writer.write_all(&self.max_bitrate.to_be_bytes())?;
        writer.write_all(&self.avg_bitrate.to_be_bytes())?;
        writer.write_all(&[self.sample_depth])?;
        let mut bit_writer = BitWriter::new(Vec::new());
        bit_writer.write_bits(&[self.frame_duration << 6], 2)?;
        bit_writer.write_bits(&[self.stream_construction << 3], 5)?;
        bit_writer.write_bit(self.core_lfe_present)?;
        bit_writer.write_bits(&[self.core_layout << 2], 6)?;
        bit_writer.write_bits(&self.core_size.to_be_bytes(), 14)?;
        bit_writer.write_bit(self.stereo_downmix)?;
        bit_writer.write_bits(&[self.representation_type << 5], 3)?;
        bit_writer.write_bits(&self.channel_layout.to_be_bytes(), 16)?;
        bit_writer.write_bit(self.multi_asset_flag)?;
        bit_writer.write_bit(self.lbr_duration_mod)?;
        bit_writer.write_bits(&[0u8], 6)?;
        let bits = bit_writer.into_inner()?;
        writer.write_all(&bits)?;
        Ok(Some(20))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        if payload_size != 20 {
            return Err(invalid_value("Ddts", "payload size must be exactly 20 bytes").into());
        }
        let mut fixed = [0u8; 20];
        std::io::Read::read_exact(reader, &mut fixed)?;
        self.sampling_frequency = u32::from_be_bytes(fixed[0..4].try_into().unwrap());
        self.max_bitrate = u32::from_be_bytes(fixed[4..8].try_into().unwrap());
        self.avg_bitrate = u32::from_be_bytes(fixed[8..12].try_into().unwrap());
        self.sample_depth = fixed[12];
        let mut bit_reader = BitReader::new(Cursor::new(&fixed[13..20]));
        self.frame_duration = read_bits_u8(&mut bit_reader, 2, "FrameDuration")?;
        self.stream_construction = read_bits_u8(&mut bit_reader, 5, "StreamConstruction")?;
        self.core_lfe_present = bit_reader.read_bit()?;
        self.core_layout = read_bits_u8(&mut bit_reader, 6, "CoreLayout")?;
        self.core_size = u16::from_be_bytes({
            let bits = bit_reader.read_bits(14)?;
            [bits[0], bits[1]]
        }) & 0x3fff;
        self.stereo_downmix = bit_reader.read_bit()?;
        self.representation_type = read_bits_u8(&mut bit_reader, 3, "RepresentationType")?;
        self.channel_layout = u16::from_be_bytes(bit_reader.read_bits(16)?.try_into().unwrap());
        self.multi_asset_flag = bit_reader.read_bit()?;
        self.lbr_duration_mod = bit_reader.read_bit()?;
        let _ = bit_reader.read_bits(6)?;
        Ok(Some(20))
    }
}

/// DTS-UHD-specific configuration box carried by DTS sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Udts {
    /// Six-bit decoder-profile code.
    pub decoder_profile_code: u8,
    /// Two-bit frame-duration code.
    pub frame_duration_code: u8,
    /// Three-bit max-payload code.
    pub max_payload_code: u8,
    /// Five-bit number-of-presentations code.
    pub num_presentations_code: u8,
    /// Thirty-two-bit channel mask.
    pub channel_mask: u32,
    /// One-bit base-sampling-frequency code.
    pub base_sampling_frequency_code: bool,
    /// Two-bit sample-rate modifier.
    pub sample_rate_mod: u8,
    /// Three-bit representation-type code.
    pub representation_type: u8,
    /// Three-bit stream index.
    pub stream_index: u8,
    /// Whether expansion-box bytes are present.
    pub expansion_box_present: bool,
    /// One-bit per-presentation ID-tag-presence flags.
    pub id_tag_present: Vec<bool>,
    /// Packed presentation-ID tag bytes.
    pub presentation_id_tag_data: Vec<u8>,
    /// Opaque expansion-box bytes.
    pub expansion_box_data: Vec<u8>,
}

impl FieldHooks for Udts {}

impl ImmutableBox for Udts {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"udts")
    }
}

impl MutableBox for Udts {}

impl FieldValueRead for Udts {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DecoderProfileCode" => Ok(FieldValue::Unsigned(u64::from(self.decoder_profile_code))),
            "FrameDurationCode" => Ok(FieldValue::Unsigned(u64::from(self.frame_duration_code))),
            "MaxPayloadCode" => Ok(FieldValue::Unsigned(u64::from(self.max_payload_code))),
            "NumPresentationsCode" => {
                Ok(FieldValue::Unsigned(u64::from(self.num_presentations_code)))
            }
            "ChannelMask" => Ok(FieldValue::Unsigned(u64::from(self.channel_mask))),
            "BaseSamplingFrequencyCode" => {
                Ok(FieldValue::Boolean(self.base_sampling_frequency_code))
            }
            "SampleRateMod" => Ok(FieldValue::Unsigned(u64::from(self.sample_rate_mod))),
            "RepresentationType" => Ok(FieldValue::Unsigned(u64::from(self.representation_type))),
            "StreamIndex" => Ok(FieldValue::Unsigned(u64::from(self.stream_index))),
            "ExpansionBoxPresent" => Ok(FieldValue::Boolean(self.expansion_box_present)),
            "PresentationIdTagData" => Ok(FieldValue::Bytes(self.presentation_id_tag_data.clone())),
            "ExpansionBoxData" => Ok(FieldValue::Bytes(self.expansion_box_data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Udts {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DecoderProfileCode", FieldValue::Unsigned(value)) => {
                self.decoder_profile_code = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FrameDurationCode", FieldValue::Unsigned(value)) => {
                self.frame_duration_code = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MaxPayloadCode", FieldValue::Unsigned(value)) => {
                self.max_payload_code = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NumPresentationsCode", FieldValue::Unsigned(value)) => {
                self.num_presentations_code = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChannelMask", FieldValue::Unsigned(value)) => {
                self.channel_mask = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BaseSamplingFrequencyCode", FieldValue::Boolean(value)) => {
                self.base_sampling_frequency_code = value;
                Ok(())
            }
            ("SampleRateMod", FieldValue::Unsigned(value)) => {
                self.sample_rate_mod = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("RepresentationType", FieldValue::Unsigned(value)) => {
                self.representation_type = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("StreamIndex", FieldValue::Unsigned(value)) => {
                self.stream_index = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ExpansionBoxPresent", FieldValue::Boolean(value)) => {
                self.expansion_box_present = value;
                Ok(())
            }
            ("PresentationIdTagData", FieldValue::Bytes(value)) => {
                self.presentation_id_tag_data = value;
                Ok(())
            }
            ("ExpansionBoxData", FieldValue::Bytes(value)) => {
                self.expansion_box_data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Udts {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DecoderProfileCode", 0, with_bit_width(6)),
        codec_field!("FrameDurationCode", 1, with_bit_width(2)),
        codec_field!("MaxPayloadCode", 2, with_bit_width(3)),
        codec_field!("NumPresentationsCode", 3, with_bit_width(5)),
        codec_field!("ChannelMask", 4, with_bit_width(32)),
        codec_field!(
            "BaseSamplingFrequencyCode",
            5,
            with_bit_width(1),
            as_boolean()
        ),
        codec_field!("SampleRateMod", 6, with_bit_width(2)),
        codec_field!("RepresentationType", 7, with_bit_width(3)),
        codec_field!("StreamIndex", 8, with_bit_width(3)),
        codec_field!("ExpansionBoxPresent", 9, with_bit_width(1), as_boolean()),
        codec_field!("PresentationIdTagData", 10, with_bit_width(8), as_bytes()),
        codec_field!("ExpansionBoxData", 11, with_bit_width(8), as_bytes()),
    ]);

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        let expected_flags = usize::from(self.num_presentations_code) + 1;
        if self.id_tag_present.len() != expected_flags {
            return Err(invalid_value(
                "NumPresentationsCode",
                "id_tag_present length must equal NumPresentationsCode + 1",
            )
            .into());
        }
        if self.id_tag_present.iter().filter(|flag| **flag).count() * 16
            != self.presentation_id_tag_data.len()
        {
            return Err(invalid_value(
                "PresentationIdTagData",
                "payload length must equal 16 bytes for each enabled ID tag",
            )
            .into());
        }
        let mut bit_writer = BitWriter::new(Vec::new());
        bit_writer.write_bits(&[self.decoder_profile_code << 2], 6)?;
        bit_writer.write_bits(&[self.frame_duration_code << 6], 2)?;
        bit_writer.write_bits(&[self.max_payload_code << 5], 3)?;
        bit_writer.write_bits(&[self.num_presentations_code << 3], 5)?;
        bit_writer.write_bits(&self.channel_mask.to_be_bytes(), 32)?;
        bit_writer.write_bit(self.base_sampling_frequency_code)?;
        bit_writer.write_bits(&[self.sample_rate_mod << 6], 2)?;
        bit_writer.write_bits(&[self.representation_type << 5], 3)?;
        bit_writer.write_bits(&[self.stream_index << 5], 3)?;
        bit_writer.write_bit(self.expansion_box_present)?;
        for flag in &self.id_tag_present {
            bit_writer.write_bit(*flag)?;
        }
        bit_writer.flush()?;
        let mut bytes = bit_writer.into_inner()?;
        bytes.extend_from_slice(&self.presentation_id_tag_data);
        bytes.extend_from_slice(&self.expansion_box_data);
        writer.write_all(&bytes)?;
        Ok(Some(u64::try_from(bytes.len()).unwrap()))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let payload = read_exact_vec_untrusted(
            reader,
            usize::try_from(payload_size).map_err(|_| {
                invalid_value("Udts", "payload size exceeds the supported in-memory range")
            })?,
        )?;
        if payload.len() < 6 {
            return Err(invalid_value("Udts", "payload size must be at least 6 bytes").into());
        }
        let mut bit_reader = BitReader::new(Cursor::new(payload.as_slice()));
        self.decoder_profile_code = read_bits_u8(&mut bit_reader, 6, "DecoderProfileCode")?;
        self.frame_duration_code = read_bits_u8(&mut bit_reader, 2, "FrameDurationCode")?;
        self.max_payload_code = read_bits_u8(&mut bit_reader, 3, "MaxPayloadCode")?;
        self.num_presentations_code = read_bits_u8(&mut bit_reader, 5, "NumPresentationsCode")?;
        self.channel_mask = u32::from_be_bytes(bit_reader.read_bits(32)?.try_into().unwrap());
        self.base_sampling_frequency_code = bit_reader.read_bit()?;
        self.sample_rate_mod = read_bits_u8(&mut bit_reader, 2, "SampleRateMod")?;
        self.representation_type = read_bits_u8(&mut bit_reader, 3, "RepresentationType")?;
        self.stream_index = read_bits_u8(&mut bit_reader, 3, "StreamIndex")?;
        self.expansion_box_present = bit_reader.read_bit()?;
        self.id_tag_present.clear();
        for _ in 0..=self.num_presentations_code {
            self.id_tag_present.push(bit_reader.read_bit()?);
        }
        let mut consumed = (58usize + self.id_tag_present.len()).div_ceil(8);
        let presentation_id_len = self.id_tag_present.iter().filter(|flag| **flag).count() * 16;
        if payload.len().saturating_sub(consumed) < presentation_id_len {
            return Err(invalid_value(
                "PresentationIdTagData",
                "payload is truncated before the declared ID-tag data",
            )
            .into());
        }
        self.presentation_id_tag_data = payload[consumed..consumed + presentation_id_len].to_vec();
        consumed += presentation_id_len;
        if self.expansion_box_present {
            self.expansion_box_data = payload[consumed..].to_vec();
        } else {
            self.expansion_box_data.clear();
        }
        Ok(Some(payload_size))
    }

    #[cfg(feature = "async")]
    fn custom_marshal_async<'a>(
        &'a self,
        writer: &'a mut dyn AsyncWriteSeek,
    ) -> CodecFuture<'a, Result<Option<u64>, CodecError>> {
        Box::pin(async move {
            let mut bytes = Vec::new();
            let written = self.custom_marshal(&mut bytes)?.unwrap_or(0);
            tokio::io::AsyncWriteExt::write_all(writer, &bytes).await?;
            Ok(Some(written))
        })
    }

    #[cfg(feature = "async")]
    fn custom_unmarshal_async<'a>(
        &'a mut self,
        reader: &'a mut dyn AsyncReadSeek,
        payload_size: u64,
    ) -> CodecFuture<'a, Result<Option<u64>, CodecError>> {
        Box::pin(async move {
            let payload = read_exact_vec_untrusted_async(
                reader,
                usize::try_from(payload_size).map_err(|_| {
                    invalid_value("Udts", "payload size exceeds the supported in-memory range")
                })?,
            )
            .await?;
            let mut cursor = Cursor::new(payload);
            let read = self.custom_unmarshal(&mut cursor, payload_size)?;
            Ok(read)
        })
    }
}
