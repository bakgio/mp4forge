use std::fs::File;
use std::io::Cursor;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::bitio::BitReader;
use crate::boxes::avs3::Av3c;

use super::super::MuxError;
use super::super::import::{
    SegmentedMuxSourceSegment, StagedSample, build_btrt_from_sample_sizes,
    build_visual_sample_entry_box,
};
use super::annexb_common::{
    read_bit_labeled, read_bits_u8_labeled, read_bits_u16_labeled, skip_bits_labeled,
};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;

const SAMPLE_ENTRY_AVS3: FourCc = FourCc::from_bytes(*b"avs3");
const AVS3_DESCRIPTOR_CONFIG_SIZE: usize = 10;
const AVS3_SEQUENCE_HEADER_PREFIX: [u8; 4] = [0x00, 0x00, 0x01, 0xB0];
const AVS3_INTRA_PICTURE_PREFIX: [u8; 4] = [0x00, 0x00, 0x01, 0xB3];
const AVS3_INTER_PICTURE_PREFIX: [u8; 4] = [0x00, 0x00, 0x01, 0xB6];
const AVS3_PREFIX_SCAN_BYTES: usize = 128;

pub(in crate::mux) struct ParsedTransportAvs3Track {
    pub(in crate::mux) timescale: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

struct ParsedAvs3SequenceHeader {
    timescale: u32,
    sample_duration: u32,
    low_delay: bool,
}

pub(in crate::mux) fn scan_transport_avs3_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    sample_offsets: &[u64],
    avs3_config: &[u8],
    spec: &str,
) -> Result<ParsedTransportAvs3Track, MuxError> {
    let config = parse_transport_avs3_config_bytes(avs3_config, spec)?;
    let sample_bounds = build_transport_avs3_sample_bounds(sample_offsets, total_size, spec)?;
    let prefix_cap =
        usize::try_from(total_size.min(u64::try_from(AVS3_PREFIX_SCAN_BYTES).unwrap()))
            .map_err(|_| MuxError::LayoutOverflow("transport-stream AVS3 prefix read length"))?;
    let mut sequence_header = None::<ParsedAvs3SequenceHeader>;
    let mut samples = Vec::with_capacity(sample_bounds.len());

    for (sample_offset, data_size) in sample_bounds {
        let prefix_len = prefix_cap.min(usize::try_from(data_size).unwrap_or(prefix_cap));
        let mut prefix = vec![0_u8; prefix_len];
        read_segmented_bytes_sync(
            file,
            segments,
            total_size,
            sample_offset,
            &mut prefix,
            spec,
            "transport-stream AVS3 sample prefix is truncated",
        )?;
        if sequence_header.is_none()
            && let Some(sequence_start) =
                find_prefixed_start_code(&prefix, AVS3_SEQUENCE_HEADER_PREFIX)
        {
            sequence_header = Some(parse_avs3_sequence_header(spec, &prefix[sequence_start..])?);
        }
        samples.push(StagedSample {
            data_offset: sample_offset,
            data_size,
            duration: 0,
            composition_time_offset: 0,
            is_sync_sample: classify_transport_avs3_sample(spec, &prefix)?,
        });
    }

    finalize_transport_avs3_track(spec, config, sequence_header, samples)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_transport_avs3_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    sample_offsets: &[u64],
    avs3_config: &[u8],
    spec: &str,
) -> Result<ParsedTransportAvs3Track, MuxError> {
    let config = parse_transport_avs3_config_bytes(avs3_config, spec)?;
    let sample_bounds = build_transport_avs3_sample_bounds(sample_offsets, total_size, spec)?;
    let prefix_cap =
        usize::try_from(total_size.min(u64::try_from(AVS3_PREFIX_SCAN_BYTES).unwrap()))
            .map_err(|_| MuxError::LayoutOverflow("transport-stream AVS3 prefix read length"))?;
    let mut sequence_header = None::<ParsedAvs3SequenceHeader>;
    let mut samples = Vec::with_capacity(sample_bounds.len());

    for (sample_offset, data_size) in sample_bounds {
        let prefix_len = prefix_cap.min(usize::try_from(data_size).unwrap_or(prefix_cap));
        let mut prefix = vec![0_u8; prefix_len];
        read_segmented_bytes_async(
            file,
            segments,
            total_size,
            sample_offset,
            &mut prefix,
            spec,
            "transport-stream AVS3 sample prefix is truncated",
        )
        .await?;
        if sequence_header.is_none()
            && let Some(sequence_start) =
                find_prefixed_start_code(&prefix, AVS3_SEQUENCE_HEADER_PREFIX)
        {
            sequence_header = Some(parse_avs3_sequence_header(spec, &prefix[sequence_start..])?);
        }
        samples.push(StagedSample {
            data_offset: sample_offset,
            data_size,
            duration: 0,
            composition_time_offset: 0,
            is_sync_sample: classify_transport_avs3_sample(spec, &prefix)?,
        });
    }

    finalize_transport_avs3_track(spec, config, sequence_header, samples)
}

fn finalize_transport_avs3_track(
    spec: &str,
    config: Av3c,
    sequence_header: Option<ParsedAvs3SequenceHeader>,
    mut samples: Vec<StagedSample>,
) -> Result<ParsedTransportAvs3Track, MuxError> {
    let sequence_header = sequence_header.ok_or(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message:
            "transport-stream AVS3 carriage did not expose a sequence header on the native direct-ingest path"
                .to_string(),
    })?;
    if !sequence_header.low_delay {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "transport-stream AVS3 carriage with reordered presentation is not supported on the native direct-ingest path yet"
                    .to_string(),
        });
    }
    for sample in &mut samples {
        sample.duration = sequence_header.sample_duration;
    }
    let config_box = super::super::mp4::encode_typed_box(&config, &[])?;
    let btrt_box = super::super::mp4::encode_typed_box(
        &build_btrt_from_sample_sizes(
            samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
            sequence_header.timescale,
        )?,
        &[],
    )?;
    Ok(ParsedTransportAvs3Track {
        timescale: sequence_header.timescale,
        sample_entry_box: build_visual_sample_entry_box(
            SAMPLE_ENTRY_AVS3,
            0,
            0,
            &[config_box, btrt_box],
        )?,
        samples,
    })
}

fn build_transport_avs3_sample_bounds(
    sample_offsets: &[u64],
    total_size: u64,
    spec: &str,
) -> Result<Vec<(u64, u32)>, MuxError> {
    if sample_offsets.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "transport-stream AVS3 carriage did not contain any PES payload units"
                .to_string(),
        });
    }
    let mut bounds = Vec::with_capacity(sample_offsets.len());
    for (index, &sample_offset) in sample_offsets.iter().enumerate() {
        let next_offset = sample_offsets.get(index + 1).copied().unwrap_or(total_size);
        if next_offset <= sample_offset {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "transport-stream AVS3 PES payload units must advance monotonically"
                    .to_string(),
            });
        }
        bounds.push((
            sample_offset,
            u32::try_from(next_offset - sample_offset)
                .map_err(|_| MuxError::LayoutOverflow("transport-stream AVS3 sample size"))?,
        ));
    }
    Ok(bounds)
}

fn parse_transport_avs3_config_bytes(config_bytes: &[u8], spec: &str) -> Result<Av3c, MuxError> {
    if config_bytes.len() != AVS3_DESCRIPTOR_CONFIG_SIZE {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "transport-stream AVS3 registration descriptor did not carry the expected 10-byte decoder configuration payload"
                    .to_string(),
        });
    }
    let sequence_header_length = u16::from_be_bytes([config_bytes[1], config_bytes[2]]);
    let expected_size =
        usize::from(sequence_header_length)
            .checked_add(4)
            .ok_or(MuxError::LayoutOverflow(
                "transport-stream AVS3 decoder config size",
            ))?;
    if expected_size != config_bytes.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "transport-stream AVS3 decoder configuration length does not match its carried payload"
                    .to_string(),
        });
    }
    let sequence_header_end = 3 + usize::from(sequence_header_length);
    let sequence_header = config_bytes[3..sequence_header_end].to_vec();
    if !sequence_header.starts_with(&AVS3_SEQUENCE_HEADER_PREFIX) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "transport-stream AVS3 decoder configuration did not begin with a sequence-header prefix"
                    .to_string(),
        });
    }
    Ok(Av3c {
        configuration_version: config_bytes[0],
        sequence_header_length,
        sequence_header,
        library_dependency_idc: config_bytes[config_bytes.len() - 1] & 0x03,
    })
}

fn parse_avs3_sequence_header(
    spec: &str,
    bytes: &[u8],
) -> Result<ParsedAvs3SequenceHeader, MuxError> {
    if bytes.len() < 17 || !bytes.starts_with(&AVS3_SEQUENCE_HEADER_PREFIX) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "transport-stream AVS3 sequence header is truncated".to_string(),
        });
    }
    let mut reader = BitReader::new(Cursor::new(&bytes[4..]));
    let _profile = read_bits_u8_labeled(&mut reader, 8, spec, "transport AVS3 profile")?;
    let _level = read_bits_u8_labeled(&mut reader, 8, spec, "transport AVS3 level")?;
    let _progressive = read_bit_labeled(&mut reader, spec, "transport AVS3 progressive flag")?;
    let _field = read_bit_labeled(&mut reader, spec, "transport AVS3 field flag")?;
    let _library_stream =
        read_bits_u8_labeled(&mut reader, 2, spec, "transport AVS3 library-stream flag")?;
    skip_bits_labeled(&mut reader, 1, spec, "transport AVS3 reserved bit")?;
    let width = read_bits_u16_labeled(&mut reader, 14, spec, "transport AVS3 width")?;
    skip_bits_labeled(&mut reader, 1, spec, "transport AVS3 reserved width bit")?;
    let height = read_bits_u16_labeled(&mut reader, 14, spec, "transport AVS3 height")?;
    skip_bits_labeled(&mut reader, 2, spec, "transport AVS3 chroma format")?;
    skip_bits_labeled(&mut reader, 3, spec, "transport AVS3 sample precision")?;
    skip_bits_labeled(&mut reader, 1, spec, "transport AVS3 reserved aspect bit")?;
    skip_bits_labeled(&mut reader, 4, spec, "transport AVS3 aspect ratio")?;
    let frame_rate_code =
        read_bits_u8_labeled(&mut reader, 4, spec, "transport AVS3 frame-rate code")?;
    skip_bits_labeled(&mut reader, 1, spec, "transport AVS3 reserved bitrate bit")?;
    skip_bits_labeled(&mut reader, 18, spec, "transport AVS3 bitrate low bits")?;
    skip_bits_labeled(
        &mut reader,
        1,
        spec,
        "transport AVS3 reserved high-bitrate bit",
    )?;
    skip_bits_labeled(&mut reader, 12, spec, "transport AVS3 bitrate high bits")?;
    let low_delay = read_bit_labeled(&mut reader, spec, "transport AVS3 low-delay flag")?;
    if width == 0 || height == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "transport-stream AVS3 sequence header declared a zero coded picture size"
                .to_string(),
        });
    }
    let (timescale, sample_duration) = avs3_frame_rate_from_code(frame_rate_code, spec)?;
    Ok(ParsedAvs3SequenceHeader {
        timescale,
        sample_duration,
        low_delay,
    })
}

fn avs3_frame_rate_from_code(frame_rate_code: u8, spec: &str) -> Result<(u32, u32), MuxError> {
    match frame_rate_code {
        0x01 => Ok((24_000, 1_001)),
        0x02 => Ok((24, 1)),
        0x03 => Ok((25, 1)),
        0x04 => Ok((30_000, 1_001)),
        0x05 => Ok((30, 1)),
        0x06 => Ok((50, 1)),
        0x07 => Ok((60_000, 1_001)),
        0x08 => Ok((60, 1)),
        0x09 => Ok((100, 1)),
        0x0A => Ok((120, 1)),
        0x0B => Ok((200, 1)),
        0x0C => Ok((240, 1)),
        0x0D => Ok((300, 1)),
        _ => Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "transport-stream AVS3 sequence header used unsupported frame-rate code 0x{frame_rate_code:02X}"
            ),
        }),
    }
}

fn classify_transport_avs3_sample(spec: &str, bytes: &[u8]) -> Result<bool, MuxError> {
    if find_prefixed_start_code(bytes, AVS3_INTRA_PICTURE_PREFIX).is_some() {
        return Ok(true);
    }
    if find_prefixed_start_code(bytes, AVS3_INTER_PICTURE_PREFIX).is_some() {
        return Ok(false);
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "transport-stream AVS3 sample did not contain a supported picture-start prefix"
            .to_string(),
    })
}

fn find_prefixed_start_code(bytes: &[u8], needle: [u8; 4]) -> Option<usize> {
    bytes.windows(4).position(|window| window == needle)
}
