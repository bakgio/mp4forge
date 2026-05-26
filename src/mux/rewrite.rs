//! Public elementary sample rewrite helpers built on the landed mux codec logic.
//!
//! These helpers convert extracted MP4 sample payloads back into one stable elementary-stream
//! shape without depending on crate-private mux internals. They are useful when callers have
//! already extracted sample bytes through [`crate::mux::sample_reader`] or another MP4-side path
//! and want one stable library helper for elementary-stream export.

use std::error::Error;
use std::fmt;

use crate::boxes::iso14496_12::{AVCDecoderConfiguration, HEVCDecoderConfiguration};
use crate::boxes::iso14496_15::VVCDecoderConfiguration;

/// Errors returned by the public Annex B sample rewrite helpers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnnexBRewriteError {
    /// The supplied decoder configuration record was missing bytes needed to derive the NAL length
    /// field width.
    MissingConfigurationRecord {
        /// Stable codec-family label used in user-facing errors.
        codec: &'static str,
    },
    /// The decoder configuration record carried one invalid NAL length field width.
    InvalidLengthFieldWidth {
        /// Stable codec-family label used in user-facing errors.
        codec: &'static str,
        /// Invalid number of bytes claimed by the configuration record.
        width: u8,
    },
    /// The sample ended before one complete NAL length field could be read.
    TruncatedLengthField {
        /// Stable codec-family label used in user-facing errors.
        codec: &'static str,
        /// Byte offset where the truncated length field started.
        offset: usize,
        /// Expected length-field width in bytes.
        width: usize,
    },
    /// One declared NAL payload was empty.
    EmptyNalUnit {
        /// Stable codec-family label used in user-facing errors.
        codec: &'static str,
        /// Byte offset where the empty NAL length field started.
        offset: usize,
    },
    /// The sample ended before the full declared NAL payload was available.
    TruncatedNalUnit {
        /// Stable codec-family label used in user-facing errors.
        codec: &'static str,
        /// Byte offset where the NAL payload was expected to begin.
        offset: usize,
        /// NAL payload size declared by the sample.
        declared_size: usize,
        /// Remaining bytes available from `offset` onward.
        remaining_size: usize,
    },
}

impl fmt::Display for AnnexBRewriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingConfigurationRecord { codec } => write!(
                f,
                "{codec} decoder configuration record is missing the bytes needed to derive the NAL length field width"
            ),
            Self::InvalidLengthFieldWidth { codec, width } => write!(
                f,
                "{codec} decoder configuration declared an unsupported NAL length field width of {width} bytes"
            ),
            Self::TruncatedLengthField {
                codec,
                offset,
                width,
            } => write!(
                f,
                "{codec} sample ended while reading the {width}-byte NAL length field at byte offset {offset}"
            ),
            Self::EmptyNalUnit { codec, offset } => write!(
                f,
                "{codec} sample declared one empty NAL unit at byte offset {offset}"
            ),
            Self::TruncatedNalUnit {
                codec,
                offset,
                declared_size,
                remaining_size,
            } => write!(
                f,
                "{codec} sample declared one {declared_size}-byte NAL unit at byte offset {offset}, but only {remaining_size} payload bytes remained"
            ),
        }
    }
}

impl Error for AnnexBRewriteError {}

/// Errors returned by the public AV1 Annex B temporal-unit rewrite helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Av1AnnexBRewriteError {
    /// One OBU header was missing bytes before the full header could be parsed.
    TruncatedObuHeader {
        /// Byte offset where the truncated OBU header started.
        offset: usize,
    },
    /// One OBU header used a forbidden or reserved bit pattern that the helper rejects.
    InvalidObuHeader {
        /// Byte offset where the invalid OBU header started.
        offset: usize,
        /// Human-readable validation detail.
        message: &'static str,
    },
    /// One OBU omitted its internal size field, which is required on the public helper path.
    MissingObuSizeField {
        /// Byte offset where the OBU header started.
        offset: usize,
    },
    /// One leb128-encoded OBU size field was truncated or did not terminate.
    TruncatedObuSizeField {
        /// Byte offset where the size field started.
        offset: usize,
    },
    /// One OBU declared a payload larger than the remaining sample bytes.
    TruncatedObuPayload {
        /// Byte offset where the OBU payload was expected to begin.
        offset: usize,
        /// OBU payload size declared by the sample.
        declared_size: usize,
        /// Remaining bytes available from `offset` onward.
        remaining_size: usize,
    },
}

impl fmt::Display for Av1AnnexBRewriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedObuHeader { offset } => {
                write!(
                    f,
                    "AV1 sample ended while reading the OBU header at byte offset {offset}"
                )
            }
            Self::InvalidObuHeader { offset, message } => {
                write!(f, "AV1 OBU header at byte offset {offset} {message}")
            }
            Self::MissingObuSizeField { offset } => write!(
                f,
                "AV1 OBU at byte offset {offset} omitted the internal size field required for MP4 sample export"
            ),
            Self::TruncatedObuSizeField { offset } => write!(
                f,
                "AV1 sample ended while reading the leb128 OBU size field at byte offset {offset}"
            ),
            Self::TruncatedObuPayload {
                offset,
                declared_size,
                remaining_size,
            } => write!(
                f,
                "AV1 sample declared one {declared_size}-byte OBU payload at byte offset {offset}, but only {remaining_size} payload bytes remained"
            ),
        }
    }
}

impl Error for Av1AnnexBRewriteError {}

/// Errors returned by the public AAC ADTS rewrite helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdtsRewriteError {
    /// The supplied AudioSpecificConfig ended before the required first two bytes.
    TruncatedAudioSpecificConfig,
    /// The supplied AudioSpecificConfig used one unsupported audio object type.
    UnsupportedAudioObjectType {
        /// Parsed AAC audio object type.
        audio_object_type: u8,
    },
    /// The supplied AudioSpecificConfig used one reserved or unsupported sample-rate index.
    UnsupportedSamplingFrequencyIndex {
        /// Parsed sampling-frequency index.
        sampling_frequency_index: u8,
    },
    /// The supplied AudioSpecificConfig used one invalid channel configuration.
    InvalidChannelConfiguration {
        /// Parsed channel configuration.
        channel_configuration: u8,
    },
    /// The supplied AAC payload would not fit in one seven-byte ADTS frame.
    FrameTooLarge {
        /// AAC payload size in bytes.
        payload_size: usize,
    },
}

impl fmt::Display for AdtsRewriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedAudioSpecificConfig => write!(
                f,
                "AAC AudioSpecificConfig is truncated before the required first two bytes"
            ),
            Self::UnsupportedAudioObjectType { audio_object_type } => write!(
                f,
                "AAC AudioSpecificConfig declared unsupported audio object type {audio_object_type} for ADTS export"
            ),
            Self::UnsupportedSamplingFrequencyIndex {
                sampling_frequency_index,
            } => write!(
                f,
                "AAC AudioSpecificConfig declared unsupported sampling-frequency index {sampling_frequency_index} for ADTS export"
            ),
            Self::InvalidChannelConfiguration {
                channel_configuration,
            } => write!(
                f,
                "AAC AudioSpecificConfig declared invalid channel configuration {channel_configuration} for ADTS export"
            ),
            Self::FrameTooLarge { payload_size } => write!(
                f,
                "AAC payload size {payload_size} does not fit in one 13-bit ADTS frame length"
            ),
        }
    }
}

impl Error for AdtsRewriteError {}

/// Errors returned by the public MHAS stream export helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MhasRewriteError {
    /// The supplied sample list was empty.
    EmptySampleList,
    /// One sample ended before a complete MHAS packet header could be parsed.
    TruncatedPacketHeader {
        /// Index of the sample containing the truncated packet header.
        sample_index: usize,
        /// Byte offset within the sample where the truncated packet header started.
        offset: usize,
    },
    /// One sample declared a packet payload larger than the remaining sample bytes.
    TruncatedPacketPayload {
        /// Index of the sample containing the truncated packet payload.
        sample_index: usize,
        /// Byte offset within the sample where the payload was expected to begin.
        offset: usize,
        /// Packet payload size declared by the packet header.
        declared_size: usize,
        /// Remaining bytes available from `offset` onward.
        remaining_size: usize,
    },
    /// One packet type is not currently accepted by the public helper.
    UnsupportedPacketType {
        /// Index of the sample containing the packet.
        sample_index: usize,
        /// Byte offset within the sample where the packet started.
        offset: usize,
        /// Parsed packet type.
        packet_type: u32,
    },
    /// The leading stream packet was not the required sync packet.
    MissingLeadingSyncPacket,
    /// The leading sync packet did not use marker `0xA5`.
    InvalidLeadingSyncMarker {
        /// Marker byte read from the leading sync packet payload.
        marker: u8,
    },
    /// One frame packet appeared before any configuration packet.
    FrameBeforeConfig {
        /// Index of the sample containing the frame packet.
        sample_index: usize,
        /// Byte offset within the sample where the frame packet started.
        offset: usize,
    },
    /// One truncation packet requested active sample trimming, which is not exported yet.
    ActiveTruncationUnsupported {
        /// Index of the sample containing the truncation packet.
        sample_index: usize,
        /// Byte offset within the sample where the truncation packet started.
        offset: usize,
    },
    /// The supplied samples did not contain any frame packet payloads.
    MissingFramePacket,
}

impl fmt::Display for MhasRewriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySampleList => {
                write!(
                    f,
                    "MHAS stream export requires at least one packetized sample"
                )
            }
            Self::TruncatedPacketHeader {
                sample_index,
                offset,
            } => write!(
                f,
                "MHAS sample {sample_index} ended while reading the packet header at byte offset {offset}"
            ),
            Self::TruncatedPacketPayload {
                sample_index,
                offset,
                declared_size,
                remaining_size,
            } => write!(
                f,
                "MHAS sample {sample_index} declared one {declared_size}-byte packet payload at byte offset {offset}, but only {remaining_size} payload bytes remained"
            ),
            Self::UnsupportedPacketType {
                sample_index,
                offset,
                packet_type,
            } => write!(
                f,
                "MHAS sample {sample_index} used unsupported packet type {packet_type} at byte offset {offset}"
            ),
            Self::MissingLeadingSyncPacket => write!(
                f,
                "MHAS stream export requires the first packet to be the leading sync packet"
            ),
            Self::InvalidLeadingSyncMarker { marker } => write!(
                f,
                "MHAS leading sync packet used marker 0x{marker:02X}, but the exported stream requires 0xA5"
            ),
            Self::FrameBeforeConfig {
                sample_index,
                offset,
            } => write!(
                f,
                "MHAS sample {sample_index} carried one frame packet before any configuration packet at byte offset {offset}"
            ),
            Self::ActiveTruncationUnsupported {
                sample_index,
                offset,
            } => write!(
                f,
                "MHAS sample {sample_index} used one active truncation packet at byte offset {offset}, which is not supported for public stream export"
            ),
            Self::MissingFramePacket => {
                write!(f, "MHAS stream export requires at least one frame packet")
            }
        }
    }
}

impl Error for MhasRewriteError {}

/// Rewrites one length-prefixed AVC sample payload into Annex B start-code form.
///
/// The supplied `sample` is expected to contain one MP4-style length-prefixed access unit that
/// uses the NAL length-field width declared by `avcc`. The returned bytes replace each length
/// field with one four-byte Annex B start code and preserve NAL payload order exactly.
pub fn rewrite_avc_sample_to_annex_b(
    sample: &[u8],
    avcc: &AVCDecoderConfiguration,
) -> Result<Vec<u8>, AnnexBRewriteError> {
    rewrite_length_prefixed_sample_to_annex_b(sample, avcc_length_field_size(avcc)?, "AVC")
}

/// Rewrites one length-prefixed HEVC sample payload into Annex B start-code form.
///
/// The supplied `sample` is expected to contain one MP4-style length-prefixed access unit that
/// uses the NAL length-field width declared by `hvcc`. The returned bytes replace each length
/// field with one four-byte Annex B start code and preserve NAL payload order exactly.
pub fn rewrite_hevc_sample_to_annex_b(
    sample: &[u8],
    hvcc: &HEVCDecoderConfiguration,
) -> Result<Vec<u8>, AnnexBRewriteError> {
    rewrite_length_prefixed_sample_to_annex_b(sample, hevc_length_field_size(hvcc)?, "HEVC")
}

/// Rewrites one length-prefixed VVC sample payload into Annex B start-code form.
///
/// The supplied `sample` is expected to contain one MP4-style length-prefixed access unit that
/// uses the NAL length-field width declared by `vvcc`. The returned bytes replace each length
/// field with one four-byte Annex B start code and preserve NAL payload order exactly.
pub fn rewrite_vvc_sample_to_annex_b(
    sample: &[u8],
    vvcc: &VVCDecoderConfiguration,
) -> Result<Vec<u8>, AnnexBRewriteError> {
    rewrite_length_prefixed_sample_to_annex_b(sample, vvc_length_field_size(vvcc)?, "VVC")
}

/// Rewrites one MP4-style AV1 sample payload into one AV1 Annex B temporal unit.
///
/// The supplied `sample` is expected to contain one MP4-style AV1 sample payload made of
/// Section 5 OBUs with explicit internal size fields. The returned bytes wrap the sample as a
/// single Annex B temporal unit with one frame unit while preserving OBU order exactly.
pub fn rewrite_av1_sample_to_annex_b(sample: &[u8]) -> Result<Vec<u8>, Av1AnnexBRewriteError> {
    if sample.is_empty() {
        return Ok(Vec::new());
    }

    let mut frame_unit_payload = Vec::with_capacity(sample.len().saturating_add(16));
    let mut offset = 0usize;
    while offset < sample.len() {
        let obu_start = offset;
        let header = *sample
            .get(offset)
            .ok_or(Av1AnnexBRewriteError::TruncatedObuHeader { offset })?;
        if header >> 7 != 0 {
            return Err(Av1AnnexBRewriteError::InvalidObuHeader {
                offset,
                message: "used a non-zero forbidden bit",
            });
        }
        if header & 0x01 != 0 {
            return Err(Av1AnnexBRewriteError::InvalidObuHeader {
                offset,
                message: "used a non-zero reserved bit",
            });
        }
        offset += 1;

        let extension_flag = (header >> 2) & 0x01 != 0;
        let has_size_field = (header >> 1) & 0x01 != 0;
        if extension_flag {
            if sample.get(offset).is_none() {
                return Err(Av1AnnexBRewriteError::TruncatedObuHeader { offset });
            }
            offset += 1;
        }
        if !has_size_field {
            return Err(Av1AnnexBRewriteError::MissingObuSizeField { offset: obu_start });
        }
        let (payload_size, leb_size) = read_leb128(sample, offset)?;
        offset += leb_size;
        let payload_end =
            offset
                .checked_add(payload_size)
                .ok_or(Av1AnnexBRewriteError::TruncatedObuPayload {
                    offset,
                    declared_size: payload_size,
                    remaining_size: sample.len().saturating_sub(offset),
                })?;
        if payload_end > sample.len() {
            return Err(Av1AnnexBRewriteError::TruncatedObuPayload {
                offset,
                declared_size: payload_size,
                remaining_size: sample.len() - offset,
            });
        }
        let obu = &sample[obu_start..payload_end];
        frame_unit_payload
            .extend_from_slice(&encode_leb128(u32::try_from(obu.len()).unwrap_or(u32::MAX)));
        frame_unit_payload.extend_from_slice(obu);
        offset = payload_end;
    }

    let mut temporal_unit = Vec::with_capacity(frame_unit_payload.len().saturating_add(16));
    temporal_unit.extend_from_slice(&encode_leb128(
        u32::try_from(frame_unit_payload.len()).unwrap_or(u32::MAX),
    ));
    temporal_unit.extend_from_slice(&frame_unit_payload);

    let mut annex_b = Vec::with_capacity(temporal_unit.len().saturating_add(8));
    annex_b.extend_from_slice(&encode_leb128(
        u32::try_from(temporal_unit.len()).unwrap_or(u32::MAX),
    ));
    annex_b.extend_from_slice(&temporal_unit);
    Ok(annex_b)
}

/// Rewrites one raw AAC sample payload into one seven-byte-header ADTS frame.
///
/// The supplied `audio_specific_config` is expected to contain the standard MPEG-4 AudioSpecificConfig
/// prefix carried by MP4 AAC sample entries. The helper currently exports ADTS for audio object
/// types `1` through `4` only.
pub fn rewrite_aac_sample_to_adts(
    sample: &[u8],
    audio_specific_config: &[u8],
) -> Result<Vec<u8>, AdtsRewriteError> {
    let Some((&first, rest)) = audio_specific_config.split_first() else {
        return Err(AdtsRewriteError::TruncatedAudioSpecificConfig);
    };
    let Some(&second) = rest.first() else {
        return Err(AdtsRewriteError::TruncatedAudioSpecificConfig);
    };

    let audio_object_type = (first >> 3) & 0x1F;
    if !(1..=4).contains(&audio_object_type) {
        return Err(AdtsRewriteError::UnsupportedAudioObjectType { audio_object_type });
    }
    let sampling_frequency_index = ((first & 0x07) << 1) | ((second >> 7) & 0x01);
    if matches!(sampling_frequency_index, 13..=15) {
        return Err(AdtsRewriteError::UnsupportedSamplingFrequencyIndex {
            sampling_frequency_index,
        });
    }
    let channel_configuration = (second >> 3) & 0x0F;
    if channel_configuration == 0 || channel_configuration > 7 {
        return Err(AdtsRewriteError::InvalidChannelConfiguration {
            channel_configuration,
        });
    }

    let frame_length = sample.len().saturating_add(7);
    if frame_length > 0x1FFF {
        return Err(AdtsRewriteError::FrameTooLarge {
            payload_size: sample.len(),
        });
    }

    let profile = audio_object_type - 1;
    let mut header = [0_u8; 7];
    header[0] = 0xFF;
    header[1] = 0xF1;
    header[2] =
        (profile << 6) | (sampling_frequency_index << 2) | ((channel_configuration >> 2) & 0x01);
    header[3] =
        ((channel_configuration & 0x03) << 6) | u8::try_from((frame_length >> 11) & 0x03).unwrap();
    header[4] = u8::try_from((frame_length >> 3) & 0xFF).unwrap();
    header[5] = (u8::try_from(frame_length & 0x07).unwrap() << 5) | 0x1F;
    header[6] = 0xFC;

    let mut frame = Vec::with_capacity(frame_length);
    frame.extend_from_slice(&header);
    frame.extend_from_slice(sample);
    Ok(frame)
}

/// Validates and concatenates packetized MHAS sample payloads back into one elementary stream.
///
/// The first supplied sample is expected to preserve the required leading sync and configuration
/// packets before the first frame packet. Later samples may contain additional frame or inactive
/// truncation packets. The returned bytes preserve packet order exactly.
pub fn rewrite_mhas_samples_to_stream(samples: &[&[u8]]) -> Result<Vec<u8>, MhasRewriteError> {
    if samples.is_empty() {
        return Err(MhasRewriteError::EmptySampleList);
    }

    let mut output = Vec::new();
    let mut saw_leading_sync = false;
    let mut saw_config = false;
    let mut saw_frame = false;

    for (sample_index, sample) in samples.iter().enumerate() {
        let mut offset = 0usize;
        while offset < sample.len() {
            let packet_offset = offset;
            let header = parse_mhas_packet_header(sample, &mut offset, sample_index)?;
            let payload_end = offset.checked_add(header.payload_size).ok_or(
                MhasRewriteError::TruncatedPacketPayload {
                    sample_index,
                    offset,
                    declared_size: header.payload_size,
                    remaining_size: sample.len().saturating_sub(offset),
                },
            )?;
            if payload_end > sample.len() {
                return Err(MhasRewriteError::TruncatedPacketPayload {
                    sample_index,
                    offset,
                    declared_size: header.payload_size,
                    remaining_size: sample.len() - offset,
                });
            }

            match header.packet_type {
                6 => {
                    if !saw_leading_sync {
                        saw_leading_sync = true;
                    }
                    if offset == payload_end {
                        return Err(MhasRewriteError::TruncatedPacketPayload {
                            sample_index,
                            offset,
                            declared_size: 1,
                            remaining_size: 0,
                        });
                    }
                    let marker = sample[offset];
                    if marker != 0xA5 {
                        return Err(MhasRewriteError::InvalidLeadingSyncMarker { marker });
                    }
                }
                1 => {
                    if !saw_leading_sync {
                        return Err(MhasRewriteError::MissingLeadingSyncPacket);
                    }
                    saw_config = true;
                }
                2 => {
                    if !saw_config {
                        return Err(MhasRewriteError::FrameBeforeConfig {
                            sample_index,
                            offset: packet_offset,
                        });
                    }
                    saw_frame = true;
                }
                17 => {
                    if !mhas_truncation_packet_is_inactive(
                        &sample[offset..payload_end],
                        sample_index,
                        packet_offset,
                    )? {
                        return Err(MhasRewriteError::ActiveTruncationUnsupported {
                            sample_index,
                            offset: packet_offset,
                        });
                    }
                }
                packet_type => {
                    return Err(MhasRewriteError::UnsupportedPacketType {
                        sample_index,
                        offset: packet_offset,
                        packet_type,
                    });
                }
            }

            output.extend_from_slice(&sample[packet_offset..payload_end]);
            offset = payload_end;
        }
    }

    if !saw_leading_sync {
        return Err(MhasRewriteError::MissingLeadingSyncPacket);
    }
    if !saw_frame {
        return Err(MhasRewriteError::MissingFramePacket);
    }
    Ok(output)
}

fn avcc_length_field_size(avcc: &AVCDecoderConfiguration) -> Result<usize, AnnexBRewriteError> {
    length_field_size_from_minus_one("AVC", avcc.length_size_minus_one)
}

fn hevc_length_field_size(hvcc: &HEVCDecoderConfiguration) -> Result<usize, AnnexBRewriteError> {
    length_field_size_from_minus_one("HEVC", hvcc.length_size_minus_one)
}

fn vvc_length_field_size(vvcc: &VVCDecoderConfiguration) -> Result<usize, AnnexBRewriteError> {
    let Some(&first_byte) = vvcc.decoder_configuration_record.first() else {
        return Err(AnnexBRewriteError::MissingConfigurationRecord { codec: "VVC" });
    };
    Ok(usize::from(((first_byte >> 1) & 0x03) + 1))
}

fn length_field_size_from_minus_one(
    codec: &'static str,
    length_size_minus_one: u8,
) -> Result<usize, AnnexBRewriteError> {
    if length_size_minus_one > 0x03 {
        return Err(AnnexBRewriteError::InvalidLengthFieldWidth {
            codec,
            width: length_size_minus_one.saturating_add(1),
        });
    }
    Ok(usize::from(length_size_minus_one) + 1)
}

fn rewrite_length_prefixed_sample_to_annex_b(
    sample: &[u8],
    length_field_size: usize,
    codec: &'static str,
) -> Result<Vec<u8>, AnnexBRewriteError> {
    if sample.is_empty() {
        return Ok(Vec::new());
    }
    let mut output = Vec::with_capacity(sample.len().saturating_add(16));
    let mut offset = 0usize;
    while offset < sample.len() {
        if sample.len() - offset < length_field_size {
            return Err(AnnexBRewriteError::TruncatedLengthField {
                codec,
                offset,
                width: length_field_size,
            });
        }
        let length_offset = offset;
        let nal_size = read_length_field(
            &sample[offset..offset + length_field_size],
            length_field_size,
        );
        offset += length_field_size;
        if nal_size == 0 {
            return Err(AnnexBRewriteError::EmptyNalUnit {
                codec,
                offset: length_offset,
            });
        }
        let remaining_size = sample.len() - offset;
        if remaining_size < nal_size {
            return Err(AnnexBRewriteError::TruncatedNalUnit {
                codec,
                offset,
                declared_size: nal_size,
                remaining_size,
            });
        }
        output.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        output.extend_from_slice(&sample[offset..offset + nal_size]);
        offset += nal_size;
    }
    Ok(output)
}

fn read_length_field(field: &[u8], width: usize) -> usize {
    match width {
        1 => usize::from(field[0]),
        2 => usize::from(u16::from_be_bytes([field[0], field[1]])),
        3 => (usize::from(field[0]) << 16) | (usize::from(field[1]) << 8) | usize::from(field[2]),
        4 => usize::try_from(u32::from_be_bytes([field[0], field[1], field[2], field[3]])).unwrap(),
        _ => unreachable!("validated length field width"),
    }
}

fn read_leb128(bytes: &[u8], offset: usize) -> Result<(usize, usize), Av1AnnexBRewriteError> {
    let mut value = 0usize;
    let mut shift = 0usize;
    for (index, byte) in bytes
        .get(offset..)
        .unwrap_or_default()
        .iter()
        .copied()
        .enumerate()
    {
        value |= usize::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
        shift += 7;
        if shift >= usize::BITS as usize {
            break;
        }
    }
    Err(Av1AnnexBRewriteError::TruncatedObuSizeField { offset })
}

fn encode_leb128(mut value: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = u8::try_from(value & 0x7F).unwrap();
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if value == 0 {
            return bytes;
        }
    }
}

#[derive(Clone, Copy)]
struct ParsedMhasHeader {
    packet_type: u32,
    payload_size: usize,
}

fn parse_mhas_packet_header(
    sample: &[u8],
    offset: &mut usize,
    sample_index: usize,
) -> Result<ParsedMhasHeader, MhasRewriteError> {
    let mut cursor = MhasBitCursor::new(sample, *offset, sample_index);
    let packet_type = u32::try_from(cursor.read_escaped_value(3, 8, 8)?).unwrap();
    let _label = cursor.read_escaped_value(2, 8, 32)?;
    let payload_size = usize::try_from(cursor.read_escaped_value(11, 24, 24)?).unwrap();
    *offset = cursor.bytes_consumed();
    Ok(ParsedMhasHeader {
        packet_type,
        payload_size,
    })
}

fn mhas_truncation_packet_is_inactive(
    payload: &[u8],
    sample_index: usize,
    packet_offset: usize,
) -> Result<bool, MhasRewriteError> {
    let mut cursor = MhasBitCursor::new(payload, 0, sample_index);
    let is_active = cursor.read_bool()?;
    let _reserved = cursor.read_bool()?;
    let _trunc_from_begin = cursor.read_bool()?;
    let _trunc_samples = cursor.read_escaped_value(13, 24, 24)?;
    if is_active {
        return Ok(false);
    }
    let _ = packet_offset;
    Ok(true)
}

struct MhasBitCursor<'a> {
    data: &'a [u8],
    bit_offset: usize,
    sample_index: usize,
}

impl<'a> MhasBitCursor<'a> {
    fn new(data: &'a [u8], byte_offset: usize, sample_index: usize) -> Self {
        Self {
            data,
            bit_offset: byte_offset.saturating_mul(8),
            sample_index,
        }
    }

    fn bytes_consumed(&self) -> usize {
        self.bit_offset.div_ceil(8)
    }

    fn read_bits(&mut self, width: usize) -> Result<u64, MhasRewriteError> {
        let end =
            self.bit_offset
                .checked_add(width)
                .ok_or(MhasRewriteError::TruncatedPacketHeader {
                    sample_index: self.sample_index,
                    offset: self.bytes_consumed(),
                })?;
        if end > self.data.len() * 8 {
            return Err(MhasRewriteError::TruncatedPacketHeader {
                sample_index: self.sample_index,
                offset: self.bytes_consumed(),
            });
        }
        let mut value = 0_u64;
        for _ in 0..width {
            let byte = self.data[self.bit_offset / 8];
            let shift = 7 - (self.bit_offset % 8);
            value = (value << 1) | u64::from((byte >> shift) & 0x01);
            self.bit_offset += 1;
        }
        Ok(value)
    }

    fn read_bool(&mut self) -> Result<bool, MhasRewriteError> {
        Ok(self.read_bits(1)? != 0)
    }

    fn read_escaped_value(
        &mut self,
        first_width: usize,
        escape_width: usize,
        final_width: usize,
    ) -> Result<u64, MhasRewriteError> {
        let value = self.read_bits(first_width)?;
        let max_first = (1_u64 << first_width) - 1;
        if value != max_first {
            return Ok(value);
        }
        let escape = self.read_bits(escape_width)?;
        let max_escape = (1_u64 << escape_width) - 1;
        if escape != max_escape {
            return value
                .checked_add(escape)
                .ok_or(MhasRewriteError::TruncatedPacketHeader {
                    sample_index: self.sample_index,
                    offset: self.bytes_consumed(),
                });
        }
        let final_value = self.read_bits(final_width)?;
        value
            .checked_add(escape)
            .and_then(|prefix| prefix.checked_add(final_value))
            .ok_or(MhasRewriteError::TruncatedPacketHeader {
                sample_index: self.sample_index,
                offset: self.bytes_consumed(),
            })
    }
}
