use std::fs::File;
use std::io::Cursor;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::bitio::BitReader;
use crate::boxes::iso14496_12::Pasp;
use crate::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    ES_DESCRIPTOR_TAG, EsDescriptor, Esds, SL_CONFIG_DESCRIPTOR_TAG,
};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    SegmentedMuxSourceSegment, StagedSample, build_btrt_from_sample_sizes,
    build_visual_sample_entry_box_with_compressor_name, read_exact_at_sync,
};
use super::annexb_common::{read_bit_labeled, read_bits_u8_labeled, read_bits_u16_labeled};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;

const SAMPLE_ENTRY_MP4V: FourCc = FourCc::from_bytes(*b"mp4v");
const DIRECT_TIMESCALE: u32 = 25_000;
const DEFAULT_SAMPLE_DURATION: u32 = 1_000;
const SCAN_CHUNK_SIZE: usize = 16 * 1024;
const VOS_START_CODE: u8 = 0xB0;
const USER_DATA_START_CODE: u8 = 0xB2;
const GROUP_OF_VOP_START_CODE: u8 = 0xB3;
const VO_START_CODE: u8 = 0xB5;
const VOP_START_CODE: u8 = 0xB6;

pub(in crate::mux) struct ParsedMp4vTrack {
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) timescale: u32,
    pub(in crate::mux) decoder_specific_info: Vec<u8>,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

pub(in crate::mux) fn scan_mp4v_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedMp4vTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_mp4v_stream_sync(file_size, spec, |offset, buf, message| {
        read_exact_at_sync(&mut file, offset, buf, spec, message)
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_mp4v_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedMp4vTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_mp4v_stream_file_async(&mut file, file_size, spec).await
}

pub(in crate::mux) fn scan_mp4v_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedMp4vTrack, MuxError> {
    parse_mp4v_stream_sync(total_size, spec, |offset, buf, message| {
        read_segmented_bytes_sync(file, segments, total_size, offset, buf, spec, message)
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_mp4v_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedMp4vTrack, MuxError> {
    parse_mp4v_segmented_stream_async(file, segments, total_size, spec).await
}

pub(in crate::mux) fn build_direct_mp4v_sample_entry_box<I>(
    width: u16,
    height: u16,
    decoder_specific_info: &[u8],
    timescale: u32,
    samples: I,
) -> Result<Vec<u8>, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    let decoder_bitrates = build_btrt_from_sample_sizes(samples, timescale)?;
    let mut esds = Esds::default();
    esds.descriptors = vec![
        Descriptor {
            tag: ES_DESCRIPTOR_TAG,
            es_descriptor: Some(EsDescriptor::default()),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_CONFIG_DESCRIPTOR_TAG,
            decoder_config_descriptor: Some(DecoderConfigDescriptor {
                object_type_indication: 0x20,
                stream_type: 4,
                reserved: true,
                buffer_size_db: decoder_bitrates.buffer_size_db,
                max_bitrate: decoder_bitrates.max_bitrate,
                avg_bitrate: decoder_bitrates.avg_bitrate,
                ..DecoderConfigDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_SPECIFIC_INFO_TAG,
            size: u32::try_from(decoder_specific_info.len())
                .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 decoder config size"))?,
            data: decoder_specific_info.to_vec(),
            ..Descriptor::default()
        },
        Descriptor {
            tag: SL_CONFIG_DESCRIPTOR_TAG,
            size: 1,
            data: vec![0x02],
            ..Descriptor::default()
        },
    ];
    esds.normalize_descriptor_sizes_for_mux()
        .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 esds"))?;
    let pasp = super::super::mp4::encode_typed_box(
        &Pasp {
            h_spacing: 1,
            v_spacing: 1,
        },
        &[],
    )?;
    build_visual_sample_entry_box_with_compressor_name(
        SAMPLE_ENTRY_MP4V,
        width,
        height,
        &[],
        &[super::super::mp4::encode_typed_box(&esds, &[])?, pasp],
    )
}

fn parse_mp4v_stream_sync<F>(
    logical_size: u64,
    spec: &str,
    mut read_exact: F,
) -> Result<ParsedMp4vTrack, MuxError>
where
    F: FnMut(u64, &mut [u8], &'static str) -> Result<(), MuxError>,
{
    if logical_size < 5 {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 input is truncated before the first start code",
        ));
    }

    let scan = scan_mp4v_boundaries_sync(logical_size, spec, &mut read_exact)?;
    finalize_mp4v_track_sync(logical_size, spec, scan, read_exact)
}

#[cfg(feature = "async")]
async fn parse_mp4v_stream_file_async(
    file: &mut TokioFile,
    logical_size: u64,
    spec: &str,
) -> Result<ParsedMp4vTrack, MuxError> {
    if logical_size < 5 {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 input is truncated before the first start code",
        ));
    }

    let scan = scan_mp4v_boundaries_file_async(file, logical_size, spec).await?;
    finalize_mp4v_track_file_async(file, logical_size, spec, scan).await
}

#[cfg(feature = "async")]
async fn parse_mp4v_segmented_stream_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    logical_size: u64,
    spec: &str,
) -> Result<ParsedMp4vTrack, MuxError> {
    if logical_size < 5 {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 input is truncated before the first start code",
        ));
    }

    let scan = scan_mp4v_boundaries_segmented_async(file, segments, logical_size, spec).await?;
    finalize_mp4v_track_segmented_async(file, segments, logical_size, spec, scan).await
}

struct Mp4vScanState {
    config_start: Option<u64>,
    first_vop_start: Option<u64>,
    first_sample_prefix_start: Option<u64>,
    current_sample_start: Option<u64>,
    current_sync_sample: bool,
    samples: Vec<StagedSample>,
}

fn scan_mp4v_boundaries_sync<F>(
    logical_size: u64,
    spec: &str,
    read_exact: &mut F,
) -> Result<Mp4vScanState, MuxError>
where
    F: FnMut(u64, &mut [u8], &'static str) -> Result<(), MuxError>,
{
    let mut samples = Vec::new();
    let mut carry = Vec::new();
    let mut offset = 0_u64;
    let mut config_start = None::<u64>;
    let mut first_vop_start = None::<u64>;
    let mut first_sample_prefix_start = None::<u64>;
    let mut current_sample_start = None::<u64>;
    let mut current_sync_sample = false;

    while offset < logical_size {
        let read_len =
            usize::try_from((logical_size - offset).min(u64::try_from(SCAN_CHUNK_SIZE).unwrap()))
                .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 scan chunk length"))?;
        let mut chunk = vec![0_u8; read_len];
        read_exact(offset, &mut chunk, "MPEG-4 Part 2 scan chunk is truncated")?;

        let combined_offset = offset
            .checked_sub(u64::try_from(carry.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow(
                "MPEG-4 Part 2 combined scan offset",
            ))?;
        let mut combined = carry;
        combined.extend_from_slice(&chunk);

        if combined.len() >= 4 {
            for index in 0..=combined.len() - 4 {
                if !combined[index..].starts_with(&[0x00, 0x00, 0x01]) {
                    continue;
                }
                let start_code = combined[index + 3];
                let start_offset = combined_offset
                    .checked_add(u64::try_from(index).unwrap())
                    .ok_or(MuxError::LayoutOverflow("MPEG-4 Part 2 start code offset"))?;
                if is_mp4v_config_start_code(start_code) || start_code == USER_DATA_START_CODE {
                    config_start.get_or_insert(start_offset);
                    continue;
                }
                if current_sample_start.is_none()
                    && first_vop_start.is_none()
                    && config_start.is_some()
                    && start_code == GROUP_OF_VOP_START_CODE
                {
                    first_sample_prefix_start.get_or_insert(start_offset);
                    continue;
                }
                if start_code != VOP_START_CODE {
                    continue;
                }
                config_start.get_or_insert(start_offset);
                let is_sync_sample =
                    mp4v_vop_is_sync_sample_sync(read_exact, logical_size, start_offset, spec)?;
                let Some(sample_start) = current_sample_start else {
                    first_vop_start = Some(start_offset);
                    current_sample_start = Some(first_sample_prefix_start.unwrap_or(start_offset));
                    current_sync_sample = is_sync_sample;
                    continue;
                };
                if start_offset <= sample_start {
                    continue;
                }
                samples.push(StagedSample {
                    data_offset: sample_start,
                    data_size: u32::try_from(start_offset - sample_start)
                        .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 frame size"))?,
                    duration: DEFAULT_SAMPLE_DURATION,
                    composition_time_offset: 0,
                    is_sync_sample: current_sync_sample,
                });
                current_sample_start = Some(start_offset);
                current_sync_sample = is_sync_sample;
            }
        }

        carry = if combined.len() > 3 {
            combined[combined.len() - 3..].to_vec()
        } else {
            combined
        };
        offset = offset
            .checked_add(u64::try_from(read_len).unwrap())
            .ok_or(MuxError::LayoutOverflow("MPEG-4 Part 2 scan offset"))?;
    }

    Ok(Mp4vScanState {
        config_start,
        first_vop_start,
        first_sample_prefix_start,
        current_sample_start,
        current_sync_sample,
        samples,
    })
}

#[cfg(feature = "async")]
async fn scan_mp4v_boundaries_file_async(
    file: &mut TokioFile,
    logical_size: u64,
    spec: &str,
) -> Result<Mp4vScanState, MuxError> {
    let mut samples = Vec::new();
    let mut carry = Vec::new();
    let mut offset = 0_u64;
    let mut config_start = None::<u64>;
    let mut first_vop_start = None::<u64>;
    let mut first_sample_prefix_start = None::<u64>;
    let mut current_sample_start = None::<u64>;
    let mut current_sync_sample = false;

    while offset < logical_size {
        let read_len =
            usize::try_from((logical_size - offset).min(u64::try_from(SCAN_CHUNK_SIZE).unwrap()))
                .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 scan chunk length"))?;
        let mut chunk = vec![0_u8; read_len];
        read_exact_at_async(
            file,
            offset,
            &mut chunk,
            spec,
            "MPEG-4 Part 2 scan chunk is truncated",
        )
        .await?;

        let combined_offset = offset
            .checked_sub(u64::try_from(carry.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow(
                "MPEG-4 Part 2 combined scan offset",
            ))?;
        let mut combined = carry;
        combined.extend_from_slice(&chunk);

        if combined.len() >= 4 {
            for index in 0..=combined.len() - 4 {
                if !combined[index..].starts_with(&[0x00, 0x00, 0x01]) {
                    continue;
                }
                let start_code = combined[index + 3];
                let start_offset = combined_offset
                    .checked_add(u64::try_from(index).unwrap())
                    .ok_or(MuxError::LayoutOverflow("MPEG-4 Part 2 start code offset"))?;
                if is_mp4v_config_start_code(start_code) || start_code == USER_DATA_START_CODE {
                    config_start.get_or_insert(start_offset);
                    continue;
                }
                if current_sample_start.is_none()
                    && first_vop_start.is_none()
                    && config_start.is_some()
                    && start_code == GROUP_OF_VOP_START_CODE
                {
                    first_sample_prefix_start.get_or_insert(start_offset);
                    continue;
                }
                if start_code != VOP_START_CODE {
                    continue;
                }
                config_start.get_or_insert(start_offset);
                let is_sync_sample =
                    mp4v_vop_is_sync_sample_file_async(file, logical_size, start_offset, spec)
                        .await?;
                let Some(sample_start) = current_sample_start else {
                    first_vop_start = Some(start_offset);
                    current_sample_start = Some(first_sample_prefix_start.unwrap_or(start_offset));
                    current_sync_sample = is_sync_sample;
                    continue;
                };
                if start_offset <= sample_start {
                    continue;
                }
                samples.push(StagedSample {
                    data_offset: sample_start,
                    data_size: u32::try_from(start_offset - sample_start)
                        .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 frame size"))?,
                    duration: DEFAULT_SAMPLE_DURATION,
                    composition_time_offset: 0,
                    is_sync_sample: current_sync_sample,
                });
                current_sample_start = Some(start_offset);
                current_sync_sample = is_sync_sample;
            }
        }

        carry = if combined.len() > 3 {
            combined[combined.len() - 3..].to_vec()
        } else {
            combined
        };
        offset = offset
            .checked_add(u64::try_from(read_len).unwrap())
            .ok_or(MuxError::LayoutOverflow("MPEG-4 Part 2 scan offset"))?;
    }

    Ok(Mp4vScanState {
        config_start,
        first_vop_start,
        first_sample_prefix_start,
        current_sample_start,
        current_sync_sample,
        samples,
    })
}

#[cfg(feature = "async")]
async fn scan_mp4v_boundaries_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    logical_size: u64,
    spec: &str,
) -> Result<Mp4vScanState, MuxError> {
    let mut samples = Vec::new();
    let mut carry = Vec::new();
    let mut offset = 0_u64;
    let mut config_start = None::<u64>;
    let mut first_vop_start = None::<u64>;
    let mut first_sample_prefix_start = None::<u64>;
    let mut current_sample_start = None::<u64>;
    let mut current_sync_sample = false;

    while offset < logical_size {
        let read_len =
            usize::try_from((logical_size - offset).min(u64::try_from(SCAN_CHUNK_SIZE).unwrap()))
                .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 scan chunk length"))?;
        let mut chunk = vec![0_u8; read_len];
        read_segmented_bytes_async(
            file,
            segments,
            logical_size,
            offset,
            &mut chunk,
            spec,
            "MPEG-4 Part 2 scan chunk is truncated",
        )
        .await?;

        let combined_offset = offset
            .checked_sub(u64::try_from(carry.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow(
                "MPEG-4 Part 2 combined scan offset",
            ))?;
        let mut combined = carry;
        combined.extend_from_slice(&chunk);

        if combined.len() >= 4 {
            for index in 0..=combined.len() - 4 {
                if !combined[index..].starts_with(&[0x00, 0x00, 0x01]) {
                    continue;
                }
                let start_code = combined[index + 3];
                let start_offset = combined_offset
                    .checked_add(u64::try_from(index).unwrap())
                    .ok_or(MuxError::LayoutOverflow("MPEG-4 Part 2 start code offset"))?;
                if is_mp4v_config_start_code(start_code) || start_code == USER_DATA_START_CODE {
                    config_start.get_or_insert(start_offset);
                    continue;
                }
                if current_sample_start.is_none()
                    && first_vop_start.is_none()
                    && config_start.is_some()
                    && start_code == GROUP_OF_VOP_START_CODE
                {
                    first_sample_prefix_start.get_or_insert(start_offset);
                    continue;
                }
                if start_code != VOP_START_CODE {
                    continue;
                }
                config_start.get_or_insert(start_offset);
                let is_sync_sample = mp4v_vop_is_sync_sample_segmented_async(
                    file,
                    segments,
                    logical_size,
                    start_offset,
                    spec,
                )
                .await?;
                let Some(sample_start) = current_sample_start else {
                    first_vop_start = Some(start_offset);
                    current_sample_start = Some(first_sample_prefix_start.unwrap_or(start_offset));
                    current_sync_sample = is_sync_sample;
                    continue;
                };
                if start_offset <= sample_start {
                    continue;
                }
                samples.push(StagedSample {
                    data_offset: sample_start,
                    data_size: u32::try_from(start_offset - sample_start)
                        .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 frame size"))?,
                    duration: DEFAULT_SAMPLE_DURATION,
                    composition_time_offset: 0,
                    is_sync_sample: current_sync_sample,
                });
                current_sample_start = Some(start_offset);
                current_sync_sample = is_sync_sample;
            }
        }

        carry = if combined.len() > 3 {
            combined[combined.len() - 3..].to_vec()
        } else {
            combined
        };
        offset = offset
            .checked_add(u64::try_from(read_len).unwrap())
            .ok_or(MuxError::LayoutOverflow("MPEG-4 Part 2 scan offset"))?;
    }

    Ok(Mp4vScanState {
        config_start,
        first_vop_start,
        first_sample_prefix_start,
        current_sample_start,
        current_sync_sample,
        samples,
    })
}

fn finalize_mp4v_track_sync<F>(
    logical_size: u64,
    spec: &str,
    scan: Mp4vScanState,
    mut read_exact: F,
) -> Result<ParsedMp4vTrack, MuxError>
where
    F: FnMut(u64, &mut [u8], &'static str) -> Result<(), MuxError>,
{
    let config_start = scan.config_start.ok_or_else(|| {
        invalid_mp4v(
            spec,
            "MPEG-4 Part 2 input did not expose any decoder-config start codes before the first VOP",
        )
    })?;
    let first_vop_start = scan.first_vop_start.ok_or_else(|| {
        invalid_mp4v(
            spec,
            "MPEG-4 Part 2 input did not contain any VOP start codes",
        )
    })?;
    let current_sample_start = scan.current_sample_start.ok_or_else(|| {
        invalid_mp4v(
            spec,
            "MPEG-4 Part 2 input did not contain any complete VOP samples",
        )
    })?;
    if first_vop_start <= config_start {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 decoder config did not precede the first VOP sample",
        ));
    }
    let config_end = scan.first_sample_prefix_start.unwrap_or(first_vop_start);
    let config_size = usize::try_from(config_end - config_start)
        .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 decoder config size"))?;
    let mut decoder_specific_info = vec![0_u8; config_size];
    read_exact(
        config_start,
        &mut decoder_specific_info,
        "MPEG-4 Part 2 decoder config is truncated",
    )?;
    let (width, height) = parse_mp4v_decoder_specific_info(&decoder_specific_info, spec)?;

    let mut samples = scan.samples;
    samples.push(StagedSample {
        data_offset: current_sample_start,
        data_size: u32::try_from(logical_size - current_sample_start)
            .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 trailing frame size"))?,
        duration: DEFAULT_SAMPLE_DURATION,
        composition_time_offset: 0,
        is_sync_sample: scan.current_sync_sample,
    });

    Ok(ParsedMp4vTrack {
        width,
        height,
        timescale: DIRECT_TIMESCALE,
        decoder_specific_info: decoder_specific_info.clone(),
        sample_entry_box: build_direct_mp4v_sample_entry_box(
            width,
            height,
            &decoder_specific_info,
            DIRECT_TIMESCALE,
            samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
        )?,
        samples,
    })
}

#[cfg(feature = "async")]
async fn finalize_mp4v_track_file_async(
    file: &mut TokioFile,
    logical_size: u64,
    spec: &str,
    scan: Mp4vScanState,
) -> Result<ParsedMp4vTrack, MuxError> {
    let config_start = scan.config_start.ok_or_else(|| {
        invalid_mp4v(
            spec,
            "MPEG-4 Part 2 input did not expose any decoder-config start codes before the first VOP",
        )
    })?;
    let first_vop_start = scan.first_vop_start.ok_or_else(|| {
        invalid_mp4v(
            spec,
            "MPEG-4 Part 2 input did not contain any VOP start codes",
        )
    })?;
    let current_sample_start = scan.current_sample_start.ok_or_else(|| {
        invalid_mp4v(
            spec,
            "MPEG-4 Part 2 input did not contain any complete VOP samples",
        )
    })?;
    if first_vop_start <= config_start {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 decoder config did not precede the first VOP sample",
        ));
    }
    let config_end = scan.first_sample_prefix_start.unwrap_or(first_vop_start);
    let config_size = usize::try_from(config_end - config_start)
        .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 decoder config size"))?;
    let mut decoder_specific_info = vec![0_u8; config_size];
    read_exact_at_async(
        file,
        config_start,
        &mut decoder_specific_info,
        spec,
        "MPEG-4 Part 2 decoder config is truncated",
    )
    .await?;
    let (width, height) = parse_mp4v_decoder_specific_info(&decoder_specific_info, spec)?;

    let mut samples = scan.samples;
    samples.push(StagedSample {
        data_offset: current_sample_start,
        data_size: u32::try_from(logical_size - current_sample_start)
            .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 trailing frame size"))?,
        duration: DEFAULT_SAMPLE_DURATION,
        composition_time_offset: 0,
        is_sync_sample: scan.current_sync_sample,
    });

    Ok(ParsedMp4vTrack {
        width,
        height,
        timescale: DIRECT_TIMESCALE,
        decoder_specific_info: decoder_specific_info.clone(),
        sample_entry_box: build_direct_mp4v_sample_entry_box(
            width,
            height,
            &decoder_specific_info,
            DIRECT_TIMESCALE,
            samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
        )?,
        samples,
    })
}

#[cfg(feature = "async")]
async fn finalize_mp4v_track_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    logical_size: u64,
    spec: &str,
    scan: Mp4vScanState,
) -> Result<ParsedMp4vTrack, MuxError> {
    let config_start = scan.config_start.ok_or_else(|| {
        invalid_mp4v(
            spec,
            "MPEG-4 Part 2 input did not expose any decoder-config start codes before the first VOP",
        )
    })?;
    let first_vop_start = scan.first_vop_start.ok_or_else(|| {
        invalid_mp4v(
            spec,
            "MPEG-4 Part 2 input did not contain any VOP start codes",
        )
    })?;
    let current_sample_start = scan.current_sample_start.ok_or_else(|| {
        invalid_mp4v(
            spec,
            "MPEG-4 Part 2 input did not contain any complete VOP samples",
        )
    })?;
    if first_vop_start <= config_start {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 decoder config did not precede the first VOP sample",
        ));
    }
    let config_end = scan.first_sample_prefix_start.unwrap_or(first_vop_start);
    let config_size = usize::try_from(config_end - config_start)
        .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 decoder config size"))?;
    let mut decoder_specific_info = vec![0_u8; config_size];
    read_segmented_bytes_async(
        file,
        segments,
        logical_size,
        config_start,
        &mut decoder_specific_info,
        spec,
        "MPEG-4 Part 2 decoder config is truncated",
    )
    .await?;
    let (width, height) = parse_mp4v_decoder_specific_info(&decoder_specific_info, spec)?;

    let mut samples = scan.samples;
    samples.push(StagedSample {
        data_offset: current_sample_start,
        data_size: u32::try_from(logical_size - current_sample_start)
            .map_err(|_| MuxError::LayoutOverflow("MPEG-4 Part 2 trailing frame size"))?,
        duration: DEFAULT_SAMPLE_DURATION,
        composition_time_offset: 0,
        is_sync_sample: scan.current_sync_sample,
    });

    Ok(ParsedMp4vTrack {
        width,
        height,
        timescale: DIRECT_TIMESCALE,
        decoder_specific_info: decoder_specific_info.clone(),
        sample_entry_box: build_direct_mp4v_sample_entry_box(
            width,
            height,
            &decoder_specific_info,
            DIRECT_TIMESCALE,
            samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
        )?,
        samples,
    })
}

fn mp4v_vop_is_sync_sample_sync<F>(
    read_exact: &mut F,
    logical_size: u64,
    sample_start: u64,
    spec: &str,
) -> Result<bool, MuxError>
where
    F: FnMut(u64, &mut [u8], &'static str) -> Result<(), MuxError>,
{
    if sample_start
        .checked_add(5)
        .is_none_or(|end| end > logical_size)
    {
        return Err(invalid_mp4v(spec, "MPEG-4 Part 2 VOP header is truncated"));
    }
    let mut header = [0_u8; 1];
    read_exact(
        sample_start + 4,
        &mut header,
        "MPEG-4 Part 2 VOP coding-type header is truncated",
    )?;
    Ok((header[0] >> 6) == 0)
}

#[cfg(feature = "async")]
async fn mp4v_vop_is_sync_sample_file_async(
    file: &mut TokioFile,
    logical_size: u64,
    sample_start: u64,
    spec: &str,
) -> Result<bool, MuxError> {
    if sample_start
        .checked_add(5)
        .is_none_or(|end| end > logical_size)
    {
        return Err(invalid_mp4v(spec, "MPEG-4 Part 2 VOP header is truncated"));
    }
    let mut header = [0_u8; 1];
    read_exact_at_async(
        file,
        sample_start + 4,
        &mut header,
        spec,
        "MPEG-4 Part 2 VOP coding-type header is truncated",
    )
    .await?;
    Ok((header[0] >> 6) == 0)
}

#[cfg(feature = "async")]
async fn mp4v_vop_is_sync_sample_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    logical_size: u64,
    sample_start: u64,
    spec: &str,
) -> Result<bool, MuxError> {
    if sample_start
        .checked_add(5)
        .is_none_or(|end| end > logical_size)
    {
        return Err(invalid_mp4v(spec, "MPEG-4 Part 2 VOP header is truncated"));
    }
    let mut header = [0_u8; 1];
    read_segmented_bytes_async(
        file,
        segments,
        logical_size,
        sample_start + 4,
        &mut header,
        spec,
        "MPEG-4 Part 2 VOP coding-type header is truncated",
    )
    .await?;
    Ok((header[0] >> 6) == 0)
}

pub(in crate::mux) fn parse_mp4v_decoder_specific_info(
    decoder_specific_info: &[u8],
    spec: &str,
) -> Result<(u16, u16), MuxError> {
    let Some((vol_start, vol_header_offset)) = find_mp4v_vol_start(decoder_specific_info) else {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 decoder config did not contain one video object layer start code",
        ));
    };
    let vol_end = find_next_mp4v_start_code(decoder_specific_info, vol_header_offset)
        .unwrap_or(decoder_specific_info.len());
    if vol_end <= vol_header_offset {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 video object layer header is empty",
        ));
    }
    let _ = vol_start;
    parse_mp4v_vol_header(&decoder_specific_info[vol_header_offset..vol_end], spec)
}

fn find_mp4v_vol_start(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0usize;
    while index + 4 <= bytes.len() {
        if bytes[index..].starts_with(&[0x00, 0x00, 0x01]) {
            let start_code = bytes[index + 3];
            if is_mp4v_vol_start_code(start_code) {
                return Some((index, index + 4));
            }
        }
        index += 1;
    }
    None
}

fn find_next_mp4v_start_code(bytes: &[u8], from: usize) -> Option<usize> {
    let mut index = from;
    while index + 4 <= bytes.len() {
        if bytes[index..].starts_with(&[0x00, 0x00, 0x01]) {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn parse_mp4v_vol_header(bytes: &[u8], spec: &str) -> Result<(u16, u16), MuxError> {
    let mut reader = BitReader::new(Cursor::new(bytes));
    let _random_accessible_vol = read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")?;
    let _video_object_type_indication =
        read_bits_u8_labeled(&mut reader, 8, spec, "MPEG-4 Part 2")?;
    if read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")? {
        let _verid = read_bits_u8_labeled(&mut reader, 4, spec, "MPEG-4 Part 2")?;
        let _priority = read_bits_u8_labeled(&mut reader, 3, spec, "MPEG-4 Part 2")?;
    }
    let aspect_ratio_info = read_bits_u8_labeled(&mut reader, 4, spec, "MPEG-4 Part 2")?;
    if aspect_ratio_info == 0x0F {
        let _par_width = read_bits_u8_labeled(&mut reader, 8, spec, "MPEG-4 Part 2")?;
        let _par_height = read_bits_u8_labeled(&mut reader, 8, spec, "MPEG-4 Part 2")?;
    }
    if read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")? {
        let _chroma_format = read_bits_u8_labeled(&mut reader, 2, spec, "MPEG-4 Part 2")?;
        let _low_delay = read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")?;
        if read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")? {
            let _first_half_bit_rate =
                read_bits_u16_labeled(&mut reader, 15, spec, "MPEG-4 Part 2")?;
            let _ = read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")?;
            let _latter_half_bit_rate =
                read_bits_u16_labeled(&mut reader, 15, spec, "MPEG-4 Part 2")?;
            let _ = read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")?;
            let _first_half_vbv_buffer_size =
                read_bits_u16_labeled(&mut reader, 15, spec, "MPEG-4 Part 2")?;
            let _ = read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")?;
            let _latter_half_vbv_buffer_size =
                read_bits_u8_labeled(&mut reader, 3, spec, "MPEG-4 Part 2")?;
            let _first_half_vbv_occupancy =
                read_bits_u16_labeled(&mut reader, 11, spec, "MPEG-4 Part 2")?;
            let _ = read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")?;
            let _latter_half_vbv_occupancy =
                read_bits_u16_labeled(&mut reader, 15, spec, "MPEG-4 Part 2")?;
            let _ = read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")?;
        }
    }
    let video_object_layer_shape = read_bits_u8_labeled(&mut reader, 2, spec, "MPEG-4 Part 2")?;
    if video_object_layer_shape != 0 {
        return Err(invalid_mp4v(
            spec,
            "only rectangular MPEG-4 Part 2 video object layers are supported on the native direct-ingest path",
        ));
    }
    if !read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")? {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 video object layer marker bit was not set before the time-increment resolution",
        ));
    }
    let vop_time_increment_resolution =
        read_bits_u16_labeled(&mut reader, 16, spec, "MPEG-4 Part 2")?;
    if vop_time_increment_resolution == 0 {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 video object layer declared a zero time-increment resolution",
        ));
    }
    if !read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")? {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 video object layer marker bit was not set after the time-increment resolution",
        ));
    }
    if read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")? {
        let fixed_vop_time_increment_bits =
            fixed_vop_time_increment_bit_count(vop_time_increment_resolution);
        let _fixed_vop_time_increment = read_bits_u16_labeled(
            &mut reader,
            fixed_vop_time_increment_bits,
            spec,
            "MPEG-4 Part 2",
        )?;
    }
    if !read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")? {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 video object layer marker bit was not set before coded width",
        ));
    }
    let width = read_bits_u16_labeled(&mut reader, 13, spec, "MPEG-4 Part 2")?;
    if !read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")? {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 video object layer marker bit was not set before coded height",
        ));
    }
    let height = read_bits_u16_labeled(&mut reader, 13, spec, "MPEG-4 Part 2")?;
    if !read_bit_labeled(&mut reader, spec, "MPEG-4 Part 2")? {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 video object layer marker bit was not set after coded height",
        ));
    }
    if width == 0 || height == 0 {
        return Err(invalid_mp4v(
            spec,
            "MPEG-4 Part 2 video object layer carried a zero coded dimension",
        ));
    }
    Ok((width, height))
}

fn fixed_vop_time_increment_bit_count(vop_time_increment_resolution: u16) -> usize {
    let max_value = u32::from(vop_time_increment_resolution.saturating_sub(1));
    let bits = 32 - max_value.leading_zeros();
    usize::try_from(bits.max(1)).unwrap()
}

fn is_mp4v_config_start_code(start_code: u8) -> bool {
    matches!(start_code, VOS_START_CODE | VO_START_CODE) || is_mp4v_vol_start_code(start_code)
}

fn is_mp4v_vol_start_code(start_code: u8) -> bool {
    (0x20..=0x2F).contains(&start_code)
}

fn invalid_mp4v(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}
