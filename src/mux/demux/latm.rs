use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    Esds,
};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData, SegmentedMuxSourceSpec, StagedSample,
    build_generic_audio_sample_entry_box, read_exact_at_sync,
};

const MP4A: FourCc = FourCc::from_bytes(*b"mp4a");
const LATM_SYNC_BYTE: u8 = 0x56;
const LATM_SYNC_HIGH_BITS: u8 = 0x07;
const LATM_SAMPLE_DURATION: u32 = 1024;
const MPEG4_AUDIO_OBJECT_TYPE_INDICATION: u8 = 0x40;
const AAC_LC_AUDIO_OBJECT_TYPE: u8 = 2;
const USAC_AUDIO_OBJECT_TYPE: u8 = 42;
const AAC_SAMPLE_RATE_TABLE: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

pub(in crate::mux) struct ParsedLatmTrack {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) segmented_source: SegmentedMuxSourceSpec,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedLatmConfig {
    audio_object_type: u8,
    sample_rate: u32,
    channel_count: u16,
    audio_specific_config: Vec<u8>,
}

struct ParsedLatmAudioSpecificConfig {
    audio_object_type: u8,
    sample_rate: u32,
    channel_count: u16,
}

struct ParsedLatmFrame {
    config: ParsedLatmConfig,
    payload: Vec<u8>,
}

struct LatmBitCursor<'a> {
    data: &'a [u8],
    bit_offset: usize,
}

impl<'a> LatmBitCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_offset: 0,
        }
    }

    fn read_bits(&mut self, width: usize, spec: &str, context: &str) -> Result<u32, MuxError> {
        if width > 32 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("LATM parser requested invalid bit width {width} for {context}"),
            });
        }
        let end = self
            .bit_offset
            .checked_add(width)
            .ok_or(MuxError::LayoutOverflow("LATM bit reader position"))?;
        if end > self.data.len() * 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated LATM frame while reading {context}"),
            });
        }

        let mut value = 0_u32;
        for _ in 0..width {
            let byte = self.data[self.bit_offset / 8];
            let shift = 7 - (self.bit_offset % 8);
            value = (value << 1) | u32::from((byte >> shift) & 0x01);
            self.bit_offset += 1;
        }
        Ok(value)
    }

    fn read_bool(&mut self, spec: &str, context: &str) -> Result<bool, MuxError> {
        Ok(self.read_bits(1, spec, context)? != 0)
    }

    fn bit_offset(&self) -> usize {
        self.bit_offset
    }
}

pub(in crate::mux) fn scan_latm_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedLatmTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_latm_stream_sync(&mut file, file_size, path, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_latm_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedLatmTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_latm_stream_async(&mut file, file_size, path, spec).await
}

fn parse_latm_stream_sync(
    file: &mut File,
    file_size: u64,
    path: &Path,
    spec: &str,
) -> Result<ParsedLatmTrack, MuxError> {
    let mut offset = 0_u64;
    let mut config = None::<ParsedLatmConfig>;
    let mut transformed_segments = Vec::new();
    let mut samples = Vec::new();
    let mut logical_offset = 0_u64;

    while offset < file_size {
        let frame = read_latm_frame_sync(file, file_size, offset, spec)?;
        let parsed = parse_latm_frame(&frame, spec, config.as_ref(), offset, file_size)?;
        if let Some(existing) = &config {
            if existing != &parsed.config {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "LATM input changed sample rate, channel layout, or AudioSpecificConfig bytes mid-stream".to_string(),
                });
            }
        } else {
            config = Some(parsed.config.clone());
        }

        let data_size = u32::try_from(parsed.payload.len())
            .map_err(|_| MuxError::LayoutOverflow("LATM payload size"))?;
        transformed_segments.push(SegmentedMuxSourceSegment {
            logical_offset,
            data: SegmentedMuxSourceSegmentData::Bytes(parsed.payload),
        });
        samples.push(StagedSample {
            data_offset: logical_offset,
            data_size,
            duration: LATM_SAMPLE_DURATION,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        logical_offset = logical_offset
            .checked_add(u64::from(data_size))
            .ok_or(MuxError::LayoutOverflow("LATM transformed logical size"))?;
        offset = offset
            .checked_add(u64::try_from(frame.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow("LATM frame offset"))?;
    }

    finalize_latm_track(
        path,
        spec,
        config,
        transformed_segments,
        samples,
        logical_offset,
    )
}

#[cfg(feature = "async")]
async fn parse_latm_stream_async(
    file: &mut TokioFile,
    file_size: u64,
    path: &Path,
    spec: &str,
) -> Result<ParsedLatmTrack, MuxError> {
    let mut offset = 0_u64;
    let mut config = None::<ParsedLatmConfig>;
    let mut transformed_segments = Vec::new();
    let mut samples = Vec::new();
    let mut logical_offset = 0_u64;

    while offset < file_size {
        let frame = read_latm_frame_async(file, file_size, offset, spec).await?;
        let parsed = parse_latm_frame(&frame, spec, config.as_ref(), offset, file_size)?;
        if let Some(existing) = &config {
            if existing != &parsed.config {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "LATM input changed sample rate, channel layout, or AudioSpecificConfig bytes mid-stream".to_string(),
                });
            }
        } else {
            config = Some(parsed.config.clone());
        }

        let data_size = u32::try_from(parsed.payload.len())
            .map_err(|_| MuxError::LayoutOverflow("LATM payload size"))?;
        transformed_segments.push(SegmentedMuxSourceSegment {
            logical_offset,
            data: SegmentedMuxSourceSegmentData::Bytes(parsed.payload),
        });
        samples.push(StagedSample {
            data_offset: logical_offset,
            data_size,
            duration: LATM_SAMPLE_DURATION,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        logical_offset = logical_offset
            .checked_add(u64::from(data_size))
            .ok_or(MuxError::LayoutOverflow("LATM transformed logical size"))?;
        offset = offset
            .checked_add(u64::try_from(frame.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow("LATM frame offset"))?;
    }

    finalize_latm_track(
        path,
        spec,
        config,
        transformed_segments,
        samples,
        logical_offset,
    )
}

fn finalize_latm_track(
    path: &Path,
    spec: &str,
    config: Option<ParsedLatmConfig>,
    transformed_segments: Vec<SegmentedMuxSourceSegment>,
    samples: Vec<StagedSample>,
    logical_offset: u64,
) -> Result<ParsedLatmTrack, MuxError> {
    let config = config.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "LATM input contained no frames".to_string(),
    })?;
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "LATM input contained no AAC access units".to_string(),
        });
    }

    Ok(ParsedLatmTrack {
        sample_rate: config.sample_rate,
        sample_entry_box: build_latm_sample_entry_box(&config)?,
        segmented_source: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: transformed_segments,
            total_size: logical_offset,
        },
        samples,
    })
}

fn read_latm_frame_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    if file_size.saturating_sub(offset) < 3 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("truncated LATM sync header at byte offset {offset}"),
        });
    }
    let mut header = [0_u8; 3];
    read_exact_at_sync(
        file,
        offset,
        &mut header,
        spec,
        "truncated LATM sync header",
    )?;
    validate_latm_sync_header(&header, spec, offset)?;
    let mux_size = latm_mux_size(&header);
    let frame_size = 3_u64
        .checked_add(mux_size)
        .ok_or(MuxError::LayoutOverflow("LATM frame size"))?;
    if offset
        .checked_add(frame_size)
        .is_none_or(|end| end > file_size)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("truncated LATM frame at byte offset {offset}"),
        });
    }
    let mut frame = vec![0_u8; usize::try_from(frame_size).unwrap()];
    read_exact_at_sync(file, offset, &mut frame, spec, "truncated LATM frame")?;
    Ok(frame)
}

#[cfg(feature = "async")]
async fn read_latm_frame_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    if file_size.saturating_sub(offset) < 3 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("truncated LATM sync header at byte offset {offset}"),
        });
    }
    let mut header = [0_u8; 3];
    read_exact_at_async(
        file,
        offset,
        &mut header,
        spec,
        "truncated LATM sync header",
    )
    .await?;
    validate_latm_sync_header(&header, spec, offset)?;
    let mux_size = latm_mux_size(&header);
    let frame_size = 3_u64
        .checked_add(mux_size)
        .ok_or(MuxError::LayoutOverflow("LATM frame size"))?;
    if offset
        .checked_add(frame_size)
        .is_none_or(|end| end > file_size)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("truncated LATM frame at byte offset {offset}"),
        });
    }
    let mut frame = vec![0_u8; usize::try_from(frame_size).unwrap()];
    read_exact_at_async(file, offset, &mut frame, spec, "truncated LATM frame").await?;
    Ok(frame)
}

fn validate_latm_sync_header(header: &[u8; 3], spec: &str, offset: u64) -> Result<(), MuxError> {
    if header[0] != LATM_SYNC_BYTE || (header[1] >> 5) != LATM_SYNC_HIGH_BITS {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("missing LATM sync header at byte offset {offset}"),
        });
    }
    Ok(())
}

fn latm_mux_size(header: &[u8; 3]) -> u64 {
    u64::from((u16::from(header[1] & 0x1F) << 8) | u16::from(header[2]))
}

fn parse_latm_frame(
    frame: &[u8],
    spec: &str,
    expected_config: Option<&ParsedLatmConfig>,
    frame_offset: u64,
    file_size: u64,
) -> Result<ParsedLatmFrame, MuxError> {
    let mut bits = LatmBitCursor::new(&frame[3..]);
    let use_same_stream_mux = bits.read_bool(spec, "LATM useSameStreamMux")?;

    let config = if use_same_stream_mux {
        expected_config
            .cloned()
            .ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "LATM input used same-stream-mux before any StreamMuxConfig was available"
                    .to_string(),
            })?
    } else {
        parse_latm_stream_mux_config(&mut bits, spec)?
    };

    let payload_size = read_latm_payload_length(&mut bits, spec)?;
    if payload_size == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "LATM frame at byte offset {frame_offset} declared a zero AAC payload"
            ),
        });
    }
    let payload = extract_packed_bit_slice(
        &frame[3..],
        bits.bit_offset(),
        usize::try_from(payload_size)
            .map_err(|_| MuxError::LayoutOverflow("LATM payload size"))?
            .checked_mul(8)
            .ok_or(MuxError::LayoutOverflow("LATM payload bits"))?,
        spec,
        "LATM AAC payload",
    )?;
    let frame_size = u64::try_from(frame.len()).unwrap();
    let frame_end = frame_offset
        .checked_add(frame_size)
        .ok_or(MuxError::LayoutOverflow("LATM frame end"))?;
    if frame_end > file_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("truncated LATM frame at byte offset {frame_offset}"),
        });
    }

    Ok(ParsedLatmFrame { config, payload })
}

fn parse_latm_stream_mux_config(
    bits: &mut LatmBitCursor<'_>,
    spec: &str,
) -> Result<ParsedLatmConfig, MuxError> {
    let audio_mux_version = bits.read_bool(spec, "LATM audioMuxVersion")?;
    if audio_mux_version {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "LATM direct input currently supports audioMuxVersion 0 only".to_string(),
        });
    }
    let all_streams_same_time_framing = bits.read_bool(spec, "LATM allStreamsSameTimeFraming")?;
    if !all_streams_same_time_framing {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "LATM direct input requires allStreamsSameTimeFraming".to_string(),
        });
    }
    let num_sub_frames = bits.read_bits(6, spec, "LATM numSubFrames")?;
    if num_sub_frames != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "LATM direct input currently supports numSubFrames = 0 only".to_string(),
        });
    }
    let num_program = bits.read_bits(4, spec, "LATM numProgram")?;
    if num_program != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "LATM direct input currently supports one program only".to_string(),
        });
    }
    let num_layer = bits.read_bits(3, spec, "LATM numLayer")?;
    if num_layer != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "LATM direct input currently supports one layer only".to_string(),
        });
    }

    let audio_specific_config_start = bits.bit_offset();
    let parsed_audio_config = parse_audio_specific_config(bits, spec)?;
    let audio_specific_config_end = bits.bit_offset();
    let audio_specific_config = extract_packed_bit_slice(
        bits.data,
        audio_specific_config_start,
        audio_specific_config_end - audio_specific_config_start,
        spec,
        "LATM AudioSpecificConfig",
    )?;

    let frame_length_type = bits.read_bits(3, spec, "LATM frameLengthType")?;
    if frame_length_type != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "LATM direct input currently supports frameLengthType 0 only, found {frame_length_type}"
            ),
        });
    }
    let _latm_buffer_fullness = bits.read_bits(8, spec, "LATM latmBufferFullness")?;
    let other_data_present = bits.read_bool(spec, "LATM otherDataPresent")?;
    if other_data_present {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "LATM direct input currently rejects otherDataPresent streams".to_string(),
        });
    }
    let crc_check_present = bits.read_bool(spec, "LATM crcCheckPresent")?;
    if crc_check_present {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "LATM direct input currently rejects crcCheckPresent streams".to_string(),
        });
    }

    Ok(ParsedLatmConfig {
        audio_object_type: parsed_audio_config.audio_object_type,
        sample_rate: parsed_audio_config.sample_rate,
        channel_count: parsed_audio_config.channel_count,
        audio_specific_config,
    })
}

fn parse_audio_specific_config(
    bits: &mut LatmBitCursor<'_>,
    spec: &str,
) -> Result<ParsedLatmAudioSpecificConfig, MuxError> {
    let audio_object_type = read_audio_object_type(bits, spec)?;
    let mut sample_rate = read_aac_sample_rate(bits, spec, "LATM AudioSpecificConfig sample rate")?;
    let channel_configuration =
        u8::try_from(bits.read_bits(4, spec, "LATM AudioSpecificConfig channel configuration")?)
            .unwrap();
    let mut core_audio_object_type = audio_object_type;
    if matches!(audio_object_type, 5 | 29) {
        sample_rate = read_aac_sample_rate(bits, spec, "LATM SBR extension sample rate")?;
        core_audio_object_type = read_audio_object_type(bits, spec)?;
    }
    if !matches!(
        core_audio_object_type,
        AAC_LC_AUDIO_OBJECT_TYPE | USAC_AUDIO_OBJECT_TYPE
    ) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "LATM direct input currently supports AAC LC or USAC style AudioSpecificConfig only, found audio object type {core_audio_object_type}"
            ),
        });
    }

    let channel_count = aac_channel_count(channel_configuration).ok_or_else(|| {
        MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "LATM direct input currently rejects AAC channel configuration {channel_configuration}"
            ),
        }
    })?;
    Ok(ParsedLatmAudioSpecificConfig {
        audio_object_type: core_audio_object_type,
        sample_rate,
        channel_count,
    })
}

fn read_audio_object_type(bits: &mut LatmBitCursor<'_>, spec: &str) -> Result<u8, MuxError> {
    let audio_object_type = u8::try_from(bits.read_bits(5, spec, "LATM audioObjectType")?).unwrap();
    if audio_object_type == 31 {
        let extended =
            u8::try_from(bits.read_bits(6, spec, "LATM extended audioObjectType")?).unwrap();
        return Ok(32 + extended);
    }
    Ok(audio_object_type)
}

fn read_aac_sample_rate(
    bits: &mut LatmBitCursor<'_>,
    spec: &str,
    context: &str,
) -> Result<u32, MuxError> {
    let sample_rate_index = u8::try_from(bits.read_bits(4, spec, context)?).unwrap();
    if sample_rate_index == 0x0F {
        return bits.read_bits(24, spec, context);
    }
    AAC_SAMPLE_RATE_TABLE
        .get(usize::from(sample_rate_index))
        .copied()
        .ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "LATM direct input used unsupported AAC sample-rate index {sample_rate_index}"
            ),
        })
}

fn aac_channel_count(channel_configuration: u8) -> Option<u16> {
    match channel_configuration {
        1 => Some(1),
        2 => Some(2),
        3 => Some(3),
        4 => Some(4),
        5 => Some(5),
        6 => Some(6),
        7 => Some(8),
        _ => None,
    }
}

fn read_latm_payload_length(bits: &mut LatmBitCursor<'_>, spec: &str) -> Result<u32, MuxError> {
    let mut size = 0_u32;
    loop {
        let value = bits.read_bits(8, spec, "LATM payload length")?;
        size = size
            .checked_add(value)
            .ok_or(MuxError::LayoutOverflow("LATM payload length"))?;
        if value != 255 {
            break;
        }
    }
    Ok(size)
}

fn extract_packed_bit_slice(
    data: &[u8],
    bit_offset: usize,
    bit_len: usize,
    spec: &str,
    context: &str,
) -> Result<Vec<u8>, MuxError> {
    let end = bit_offset
        .checked_add(bit_len)
        .ok_or(MuxError::LayoutOverflow("LATM packed bit slice"))?;
    if end > data.len() * 8 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("truncated LATM frame while extracting {context}"),
        });
    }
    let byte_len = bit_len.div_ceil(8);
    let mut output = vec![0_u8; byte_len];
    for index in 0..bit_len {
        let source_bit = bit_offset + index;
        let source_byte = data[source_bit / 8];
        let source_shift = 7 - (source_bit % 8);
        let bit = (source_byte >> source_shift) & 0x01;
        if bit != 0 {
            let output_bit = index;
            let output_byte_index = output_bit / 8;
            let output_shift = 7 - (output_bit % 8);
            output[output_byte_index] |= 1 << output_shift;
        }
    }
    Ok(output)
}

fn build_latm_sample_entry_box(config: &ParsedLatmConfig) -> Result<Vec<u8>, MuxError> {
    let esds =
        super::super::mp4::encode_typed_box(&build_latm_esds(&config.audio_specific_config), &[])?;
    build_generic_audio_sample_entry_box(
        MP4A,
        config.sample_rate,
        config.channel_count,
        16,
        &[esds],
    )
}

fn build_latm_esds(audio_specific_config: &[u8]) -> Esds {
    let mut esds = Esds::default();
    esds.descriptors = vec![
        Descriptor {
            tag: DECODER_CONFIG_DESCRIPTOR_TAG,
            size: 13,
            decoder_config_descriptor: Some(DecoderConfigDescriptor {
                object_type_indication: MPEG4_AUDIO_OBJECT_TYPE_INDICATION,
                stream_type: 5,
                reserved: true,
                ..DecoderConfigDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_SPECIFIC_INFO_TAG,
            size: audio_specific_config.len() as u32,
            data: audio_specific_config.to_vec(),
            ..Descriptor::default()
        },
    ];
    esds
}
