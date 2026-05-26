use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::iso14496_12::Btrt;
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
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;

const SAMPLE_ENTRY_MP4V: FourCc = FourCc::from_bytes(*b"mp4v");
const DIRECT_COMPRESSOR_NAME: &[u8] = b"";
const SCAN_CHUNK_SIZE: usize = 16 * 1024;
const MIN_HEADER_SIZE: usize = 8;
const MPEG2_VISUAL_OBJECT_TYPE_SIMPLE: u8 = 0x60;
const MPEG2_VISUAL_OBJECT_TYPE_MAIN: u8 = 0x61;
const MPEG2_VISUAL_OBJECT_TYPE_SNR: u8 = 0x62;
const MPEG2_VISUAL_OBJECT_TYPE_SPATIAL: u8 = 0x63;
const MPEG2_VISUAL_OBJECT_TYPE_HIGH: u8 = 0x64;
const MPEG2_VISUAL_OBJECT_TYPE_422: u8 = 0x65;
const MPEG1_VISUAL_OBJECT_TYPE: u8 = 0x6A;
const SEQUENCE_START_CODE: u8 = 0xB3;
const SEQUENCE_END_START_CODE: u8 = 0xB7;
const EXTENSION_START_CODE: u8 = 0xB5;
const PICTURE_START_CODE: u8 = 0x00;
const FRAME_RATE_23_976: (u32, u32) = (24_000, 1_001);
const FRAME_RATE_24: (u32, u32) = (24_000, 1_000);
const FRAME_RATE_25: (u32, u32) = (25_000, 1_000);
const FRAME_RATE_29_97: (u32, u32) = (30_000, 1_001);
const FRAME_RATE_30: (u32, u32) = (30_000, 1_000);
const FRAME_RATE_50: (u32, u32) = (50_000, 1_000);
const FRAME_RATE_59_94: (u32, u32) = (60_000, 1_001);
const FRAME_RATE_60: (u32, u32) = (60_000, 1_000);

pub(in crate::mux) struct ParsedMpeg2VideoTrack {
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) timescale: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) decoder_specific_info: Vec<u8>,
    pub(in crate::mux) object_type_indication: u8,
    pub(in crate::mux) pixel_aspect_ratio: Option<(u32, u32)>,
    pub(in crate::mux) eof_terminated_trailing_sample: bool,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

struct Mpeg2ScanState {
    sequence_start: Option<u64>,
    first_picture_start: Option<u64>,
    current_sample_start: Option<u64>,
    current_sync_sample: bool,
    samples: Vec<StagedSample>,
}

struct ParsedMpeg2DecoderConfig {
    width: u16,
    height: u16,
    timescale: u32,
    sample_duration: u32,
    object_type_indication: u8,
    pixel_aspect_ratio: Option<(u32, u32)>,
}

pub(in crate::mux) struct ProgramStreamMpeg2vSampleEntryConfig<'a> {
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) decoder_specific_info: &'a [u8],
    pub(in crate::mux) object_type_indication: u8,
    pub(in crate::mux) timescale: u32,
    pub(in crate::mux) leading_media_time: u64,
    pub(in crate::mux) pixel_aspect_ratio: Option<(u32, u32)>,
}

pub(in crate::mux) fn scan_mpeg2v_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedMpeg2VideoTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_mpeg2v_stream_sync(file_size, spec, |offset, buf, message| {
        read_exact_at_sync(&mut file, offset, buf, spec, message)
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_mpeg2v_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedMpeg2VideoTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_mpeg2v_stream_file_async(&mut file, file_size, spec).await
}

pub(in crate::mux) fn scan_mpeg2v_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedMpeg2VideoTrack, MuxError> {
    parse_mpeg2v_stream_sync(total_size, spec, |offset, buf, message| {
        read_segmented_bytes_sync(file, segments, total_size, offset, buf, spec, message)
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_mpeg2v_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedMpeg2VideoTrack, MuxError> {
    parse_mpeg2v_segmented_stream_async(file, segments, total_size, spec).await
}

fn parse_mpeg2v_stream_sync<F>(
    logical_size: u64,
    spec: &str,
    mut read_exact: F,
) -> Result<ParsedMpeg2VideoTrack, MuxError>
where
    F: FnMut(u64, &mut [u8], &'static str) -> Result<(), MuxError>,
{
    if logical_size < u64::try_from(MIN_HEADER_SIZE).unwrap() {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video input is truncated before the first start code",
        ));
    }
    let scan = scan_mpeg2v_boundaries_sync(logical_size, spec, &mut read_exact)?;
    finalize_mpeg2v_track_sync(logical_size, spec, scan, read_exact)
}

#[cfg(feature = "async")]
async fn parse_mpeg2v_stream_file_async(
    file: &mut TokioFile,
    logical_size: u64,
    spec: &str,
) -> Result<ParsedMpeg2VideoTrack, MuxError> {
    if logical_size < u64::try_from(MIN_HEADER_SIZE).unwrap() {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video input is truncated before the first start code",
        ));
    }
    let scan = scan_mpeg2v_boundaries_file_async(file, logical_size, spec).await?;
    finalize_mpeg2v_track_file_async(file, logical_size, spec, scan).await
}

#[cfg(feature = "async")]
async fn parse_mpeg2v_segmented_stream_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    logical_size: u64,
    spec: &str,
) -> Result<ParsedMpeg2VideoTrack, MuxError> {
    if logical_size < u64::try_from(MIN_HEADER_SIZE).unwrap() {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video input is truncated before the first start code",
        ));
    }
    let scan = scan_mpeg2v_boundaries_segmented_async(file, segments, logical_size, spec).await?;
    finalize_mpeg2v_track_segmented_async(file, segments, logical_size, spec, scan).await
}

fn scan_mpeg2v_boundaries_sync<F>(
    logical_size: u64,
    spec: &str,
    read_exact: &mut F,
) -> Result<Mpeg2ScanState, MuxError>
where
    F: FnMut(u64, &mut [u8], &'static str) -> Result<(), MuxError>,
{
    let mut samples = Vec::new();
    let mut carry = Vec::new();
    let mut offset = 0_u64;
    let mut sequence_start = None::<u64>;
    let mut first_picture_start = None::<u64>;
    let mut current_sample_start = None::<u64>;
    let mut pending_sample_start = None::<u64>;
    let mut current_sync_sample = false;

    while offset < logical_size {
        let read_len =
            usize::try_from((logical_size - offset).min(u64::try_from(SCAN_CHUNK_SIZE).unwrap()))
                .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video scan chunk length"))?;
        let mut chunk = vec![0_u8; read_len];
        read_exact(offset, &mut chunk, "MPEG-2 video scan chunk is truncated")?;

        let combined_offset = offset
            .checked_sub(u64::try_from(carry.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow(
                "MPEG-2 video combined scan offset",
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
                    .ok_or(MuxError::LayoutOverflow("MPEG-2 video start-code offset"))?;
                if start_code == SEQUENCE_START_CODE {
                    sequence_start.get_or_insert(start_offset);
                    pending_sample_start = Some(start_offset);
                    continue;
                }
                if start_code == SEQUENCE_END_START_CODE {
                    if let Some(sample_start) = current_sample_start.take()
                        && start_offset > sample_start
                    {
                        samples.push(StagedSample {
                            data_offset: sample_start,
                            data_size: u32::try_from(start_offset - sample_start)
                                .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video frame size"))?,
                            duration: 0,
                            composition_time_offset: 0,
                            is_sync_sample: current_sync_sample,
                        });
                    }
                    pending_sample_start = None;
                    current_sync_sample = false;
                    continue;
                }
                if start_code != PICTURE_START_CODE {
                    continue;
                }
                let is_sync_sample = mpeg2v_picture_is_sync_sample_sync(
                    read_exact,
                    logical_size,
                    start_offset,
                    spec,
                )?;
                let Some(sample_start) = current_sample_start else {
                    first_picture_start = Some(start_offset);
                    current_sample_start = Some(
                        pending_sample_start
                            .take()
                            .or(sequence_start)
                            .unwrap_or(start_offset),
                    );
                    current_sync_sample = is_sync_sample;
                    continue;
                };
                let next_sample_start = pending_sample_start.take().unwrap_or(start_offset);
                if next_sample_start <= sample_start {
                    continue;
                }
                samples.push(StagedSample {
                    data_offset: sample_start,
                    data_size: u32::try_from(next_sample_start - sample_start)
                        .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video frame size"))?,
                    duration: 0,
                    composition_time_offset: 0,
                    is_sync_sample: current_sync_sample,
                });
                current_sample_start = Some(next_sample_start);
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
            .ok_or(MuxError::LayoutOverflow("MPEG-2 video scan offset"))?;
    }

    Ok(Mpeg2ScanState {
        sequence_start,
        first_picture_start,
        current_sample_start,
        current_sync_sample,
        samples,
    })
}

#[cfg(feature = "async")]
async fn scan_mpeg2v_boundaries_file_async(
    file: &mut TokioFile,
    logical_size: u64,
    spec: &str,
) -> Result<Mpeg2ScanState, MuxError> {
    let mut samples = Vec::new();
    let mut carry = Vec::new();
    let mut offset = 0_u64;
    let mut sequence_start = None::<u64>;
    let mut first_picture_start = None::<u64>;
    let mut current_sample_start = None::<u64>;
    let mut pending_sample_start = None::<u64>;
    let mut current_sync_sample = false;

    while offset < logical_size {
        let read_len =
            usize::try_from((logical_size - offset).min(u64::try_from(SCAN_CHUNK_SIZE).unwrap()))
                .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video scan chunk length"))?;
        let mut chunk = vec![0_u8; read_len];
        read_exact_at_async(
            file,
            offset,
            &mut chunk,
            spec,
            "MPEG-2 video scan chunk is truncated",
        )
        .await?;

        let combined_offset = offset
            .checked_sub(u64::try_from(carry.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow(
                "MPEG-2 video combined scan offset",
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
                    .ok_or(MuxError::LayoutOverflow("MPEG-2 video start-code offset"))?;
                if start_code == SEQUENCE_START_CODE {
                    sequence_start.get_or_insert(start_offset);
                    pending_sample_start = Some(start_offset);
                    continue;
                }
                if start_code == SEQUENCE_END_START_CODE {
                    if let Some(sample_start) = current_sample_start.take()
                        && start_offset > sample_start
                    {
                        samples.push(StagedSample {
                            data_offset: sample_start,
                            data_size: u32::try_from(start_offset - sample_start)
                                .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video frame size"))?,
                            duration: 0,
                            composition_time_offset: 0,
                            is_sync_sample: current_sync_sample,
                        });
                    }
                    pending_sample_start = None;
                    current_sync_sample = false;
                    continue;
                }
                if start_code != PICTURE_START_CODE {
                    continue;
                }
                let is_sync_sample = mpeg2v_picture_is_sync_sample_file_async(
                    file,
                    logical_size,
                    start_offset,
                    spec,
                )
                .await?;
                let Some(sample_start) = current_sample_start else {
                    first_picture_start = Some(start_offset);
                    current_sample_start = Some(
                        pending_sample_start
                            .take()
                            .or(sequence_start)
                            .unwrap_or(start_offset),
                    );
                    current_sync_sample = is_sync_sample;
                    continue;
                };
                let next_sample_start = pending_sample_start.take().unwrap_or(start_offset);
                if next_sample_start <= sample_start {
                    continue;
                }
                samples.push(StagedSample {
                    data_offset: sample_start,
                    data_size: u32::try_from(next_sample_start - sample_start)
                        .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video frame size"))?,
                    duration: 0,
                    composition_time_offset: 0,
                    is_sync_sample: current_sync_sample,
                });
                current_sample_start = Some(next_sample_start);
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
            .ok_or(MuxError::LayoutOverflow("MPEG-2 video scan offset"))?;
    }

    Ok(Mpeg2ScanState {
        sequence_start,
        first_picture_start,
        current_sample_start,
        current_sync_sample,
        samples,
    })
}

#[cfg(feature = "async")]
async fn scan_mpeg2v_boundaries_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    logical_size: u64,
    spec: &str,
) -> Result<Mpeg2ScanState, MuxError> {
    let mut samples = Vec::new();
    let mut carry = Vec::new();
    let mut offset = 0_u64;
    let mut sequence_start = None::<u64>;
    let mut first_picture_start = None::<u64>;
    let mut current_sample_start = None::<u64>;
    let mut pending_sample_start = None::<u64>;
    let mut current_sync_sample = false;

    while offset < logical_size {
        let read_len =
            usize::try_from((logical_size - offset).min(u64::try_from(SCAN_CHUNK_SIZE).unwrap()))
                .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video scan chunk length"))?;
        let mut chunk = vec![0_u8; read_len];
        read_segmented_bytes_async(
            file,
            segments,
            logical_size,
            offset,
            &mut chunk,
            spec,
            "MPEG-2 video scan chunk is truncated",
        )
        .await?;

        let combined_offset = offset
            .checked_sub(u64::try_from(carry.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow(
                "MPEG-2 video combined scan offset",
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
                    .ok_or(MuxError::LayoutOverflow("MPEG-2 video start-code offset"))?;
                if start_code == SEQUENCE_START_CODE {
                    sequence_start.get_or_insert(start_offset);
                    pending_sample_start = Some(start_offset);
                    continue;
                }
                if start_code == SEQUENCE_END_START_CODE {
                    if let Some(sample_start) = current_sample_start.take()
                        && start_offset > sample_start
                    {
                        samples.push(StagedSample {
                            data_offset: sample_start,
                            data_size: u32::try_from(start_offset - sample_start)
                                .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video frame size"))?,
                            duration: 0,
                            composition_time_offset: 0,
                            is_sync_sample: current_sync_sample,
                        });
                    }
                    pending_sample_start = None;
                    current_sync_sample = false;
                    continue;
                }
                if start_code != PICTURE_START_CODE {
                    continue;
                }
                let is_sync_sample = mpeg2v_picture_is_sync_sample_segmented_async(
                    file,
                    segments,
                    logical_size,
                    start_offset,
                    spec,
                )
                .await?;
                let Some(sample_start) = current_sample_start else {
                    first_picture_start = Some(start_offset);
                    current_sample_start = Some(
                        pending_sample_start
                            .take()
                            .or(sequence_start)
                            .unwrap_or(start_offset),
                    );
                    current_sync_sample = is_sync_sample;
                    continue;
                };
                let next_sample_start = pending_sample_start.take().unwrap_or(start_offset);
                if next_sample_start <= sample_start {
                    continue;
                }
                samples.push(StagedSample {
                    data_offset: sample_start,
                    data_size: u32::try_from(next_sample_start - sample_start)
                        .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video frame size"))?,
                    duration: 0,
                    composition_time_offset: 0,
                    is_sync_sample: current_sync_sample,
                });
                current_sample_start = Some(next_sample_start);
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
            .ok_or(MuxError::LayoutOverflow("MPEG-2 video scan offset"))?;
    }

    Ok(Mpeg2ScanState {
        sequence_start,
        first_picture_start,
        current_sample_start,
        current_sync_sample,
        samples,
    })
}

fn finalize_mpeg2v_track_sync<F>(
    logical_size: u64,
    spec: &str,
    scan: Mpeg2ScanState,
    mut read_exact: F,
) -> Result<ParsedMpeg2VideoTrack, MuxError>
where
    F: FnMut(u64, &mut [u8], &'static str) -> Result<(), MuxError>,
{
    let sequence_start = scan.sequence_start.ok_or_else(|| {
        invalid_mpeg2v(
            spec,
            "MPEG-2 video input did not contain a sequence header before the first picture",
        )
    })?;
    let first_picture_start = scan.first_picture_start.ok_or_else(|| {
        invalid_mpeg2v(
            spec,
            "MPEG-2 video input did not contain any picture start codes",
        )
    })?;
    if first_picture_start <= sequence_start {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video decoder config did not precede the first picture sample",
        ));
    }
    let decoder_specific_info_size = usize::try_from(first_picture_start - sequence_start)
        .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video decoder config size"))?;
    let mut decoder_specific_info = vec![0_u8; decoder_specific_info_size];
    read_exact(
        sequence_start,
        &mut decoder_specific_info,
        "MPEG-2 video decoder config is truncated",
    )?;
    let parsed_config = parse_mpeg2_decoder_specific_info(&decoder_specific_info, spec)?;

    let eof_terminated_trailing_sample = scan.current_sample_start.is_some();
    let mut samples = scan.samples;
    if let Some(current_sample_start) = scan.current_sample_start {
        samples.push(StagedSample {
            data_offset: current_sample_start,
            data_size: u32::try_from(logical_size - current_sample_start)
                .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video trailing frame size"))?,
            duration: parsed_config.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: scan.current_sync_sample,
        });
    }
    if samples.is_empty() {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video input did not contain any complete picture samples",
        ));
    }
    for sample in &mut samples {
        if sample.duration == 0 {
            sample.duration = parsed_config.sample_duration;
        }
    }

    let sample_entry_box = build_mpeg2v_sample_entry_box(
        parsed_config.width,
        parsed_config.height,
        &decoder_specific_info,
        parsed_config.object_type_indication,
        parsed_config.timescale,
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        None,
    )?;
    Ok(ParsedMpeg2VideoTrack {
        width: parsed_config.width,
        height: parsed_config.height,
        timescale: parsed_config.timescale,
        decoder_specific_info,
        object_type_indication: parsed_config.object_type_indication,
        sample_entry_box,
        pixel_aspect_ratio: parsed_config.pixel_aspect_ratio,
        eof_terminated_trailing_sample,
        samples,
    })
}

#[cfg(feature = "async")]
async fn finalize_mpeg2v_track_file_async(
    file: &mut TokioFile,
    logical_size: u64,
    spec: &str,
    scan: Mpeg2ScanState,
) -> Result<ParsedMpeg2VideoTrack, MuxError> {
    let sequence_start = scan.sequence_start.ok_or_else(|| {
        invalid_mpeg2v(
            spec,
            "MPEG-2 video input did not contain a sequence header before the first picture",
        )
    })?;
    let first_picture_start = scan.first_picture_start.ok_or_else(|| {
        invalid_mpeg2v(
            spec,
            "MPEG-2 video input did not contain any picture start codes",
        )
    })?;
    if first_picture_start <= sequence_start {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video decoder config did not precede the first picture sample",
        ));
    }
    let decoder_specific_info_size = usize::try_from(first_picture_start - sequence_start)
        .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video decoder config size"))?;
    let mut decoder_specific_info = vec![0_u8; decoder_specific_info_size];
    read_exact_at_async(
        file,
        sequence_start,
        &mut decoder_specific_info,
        spec,
        "MPEG-2 video decoder config is truncated",
    )
    .await?;
    let parsed_config = parse_mpeg2_decoder_specific_info(&decoder_specific_info, spec)?;

    let eof_terminated_trailing_sample = scan.current_sample_start.is_some();
    let mut samples = scan.samples;
    if let Some(current_sample_start) = scan.current_sample_start {
        samples.push(StagedSample {
            data_offset: current_sample_start,
            data_size: u32::try_from(logical_size - current_sample_start)
                .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video trailing frame size"))?,
            duration: parsed_config.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: scan.current_sync_sample,
        });
    }
    if samples.is_empty() {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video input did not contain any complete picture samples",
        ));
    }
    for sample in &mut samples {
        if sample.duration == 0 {
            sample.duration = parsed_config.sample_duration;
        }
    }

    let sample_entry_box = build_mpeg2v_sample_entry_box(
        parsed_config.width,
        parsed_config.height,
        &decoder_specific_info,
        parsed_config.object_type_indication,
        parsed_config.timescale,
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        None,
    )?;
    Ok(ParsedMpeg2VideoTrack {
        width: parsed_config.width,
        height: parsed_config.height,
        timescale: parsed_config.timescale,
        decoder_specific_info,
        object_type_indication: parsed_config.object_type_indication,
        sample_entry_box,
        pixel_aspect_ratio: parsed_config.pixel_aspect_ratio,
        eof_terminated_trailing_sample,
        samples,
    })
}

#[cfg(feature = "async")]
async fn finalize_mpeg2v_track_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    logical_size: u64,
    spec: &str,
    scan: Mpeg2ScanState,
) -> Result<ParsedMpeg2VideoTrack, MuxError> {
    let sequence_start = scan.sequence_start.ok_or_else(|| {
        invalid_mpeg2v(
            spec,
            "MPEG-2 video input did not contain a sequence header before the first picture",
        )
    })?;
    let first_picture_start = scan.first_picture_start.ok_or_else(|| {
        invalid_mpeg2v(
            spec,
            "MPEG-2 video input did not contain any picture start codes",
        )
    })?;
    if first_picture_start <= sequence_start {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video decoder config did not precede the first picture sample",
        ));
    }
    let decoder_specific_info_size = usize::try_from(first_picture_start - sequence_start)
        .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video decoder config size"))?;
    let mut decoder_specific_info = vec![0_u8; decoder_specific_info_size];
    read_segmented_bytes_async(
        file,
        segments,
        logical_size,
        sequence_start,
        &mut decoder_specific_info,
        spec,
        "MPEG-2 video decoder config is truncated",
    )
    .await?;
    let parsed_config = parse_mpeg2_decoder_specific_info(&decoder_specific_info, spec)?;

    let eof_terminated_trailing_sample = scan.current_sample_start.is_some();
    let mut samples = scan.samples;
    if let Some(current_sample_start) = scan.current_sample_start {
        samples.push(StagedSample {
            data_offset: current_sample_start,
            data_size: u32::try_from(logical_size - current_sample_start)
                .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video trailing frame size"))?,
            duration: parsed_config.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: scan.current_sync_sample,
        });
    }
    if samples.is_empty() {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video input did not contain any complete picture samples",
        ));
    }
    for sample in &mut samples {
        if sample.duration == 0 {
            sample.duration = parsed_config.sample_duration;
        }
    }

    let sample_entry_box = build_mpeg2v_sample_entry_box(
        parsed_config.width,
        parsed_config.height,
        &decoder_specific_info,
        parsed_config.object_type_indication,
        parsed_config.timescale,
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        None,
    )?;
    Ok(ParsedMpeg2VideoTrack {
        width: parsed_config.width,
        height: parsed_config.height,
        timescale: parsed_config.timescale,
        decoder_specific_info,
        object_type_indication: parsed_config.object_type_indication,
        sample_entry_box,
        pixel_aspect_ratio: parsed_config.pixel_aspect_ratio,
        eof_terminated_trailing_sample,
        samples,
    })
}

pub(in crate::mux) fn build_mpeg2v_sample_entry_box<I>(
    width: u16,
    height: u16,
    decoder_specific_info: &[u8],
    object_type_indication: u8,
    timescale: u32,
    samples: I,
    pixel_aspect_ratio: Option<(u32, u32)>,
) -> Result<Vec<u8>, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    let decoder_bitrates = build_btrt_from_sample_sizes(samples, timescale)?;
    encode_mpeg2v_sample_entry_box(
        width,
        height,
        decoder_specific_info,
        object_type_indication,
        decoder_bitrates,
        pixel_aspect_ratio,
        false,
    )
}

pub(in crate::mux) fn build_transport_mpeg2v_sample_entry_box<I>(
    width: u16,
    height: u16,
    decoder_specific_info: &[u8],
    object_type_indication: u8,
    timescale: u32,
    samples: I,
    pixel_aspect_ratio: Option<(u32, u32)>,
) -> Result<Vec<u8>, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    let decoder_bitrates = build_btrt_from_sample_sizes(samples, timescale)?;
    encode_mpeg2v_sample_entry_box(
        width,
        height,
        decoder_specific_info,
        object_type_indication,
        decoder_bitrates,
        pixel_aspect_ratio,
        true,
    )
}

pub(in crate::mux) fn build_program_stream_mpeg2v_sample_entry_box<I>(
    config: ProgramStreamMpeg2vSampleEntryConfig<'_>,
    samples: I,
) -> Result<Vec<u8>, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    let decoder_bitrates =
        build_program_stream_mpeg2v_btrt(samples, config.timescale, config.leading_media_time)?;
    encode_mpeg2v_sample_entry_box(
        config.width,
        config.height,
        config.decoder_specific_info,
        config.object_type_indication,
        decoder_bitrates,
        config.pixel_aspect_ratio,
        true,
    )
}

fn encode_mpeg2v_sample_entry_box(
    width: u16,
    height: u16,
    decoder_specific_info: &[u8],
    object_type_indication: u8,
    decoder_bitrates: Btrt,
    pixel_aspect_ratio: Option<(u32, u32)>,
    force_pixel_aspect_ratio_box: bool,
) -> Result<Vec<u8>, MuxError> {
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
                object_type_indication,
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
                .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video decoder config size"))?,
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
        .map_err(|_| MuxError::LayoutOverflow("MPEG-2 video esds"))?;
    let mut child_boxes = vec![super::super::mp4::encode_typed_box(&esds, &[])?];
    if let Some((h_spacing, v_spacing)) = pixel_aspect_ratio
        && (force_pixel_aspect_ratio_box || !(h_spacing == 1 && v_spacing == 1))
    {
        child_boxes.push(super::super::mp4::encode_typed_box(
            &Pasp {
                h_spacing,
                v_spacing,
            },
            &[],
        )?);
    }
    build_visual_sample_entry_box_with_compressor_name(
        SAMPLE_ENTRY_MP4V,
        width,
        height,
        DIRECT_COMPRESSOR_NAME,
        &child_boxes,
    )
}

fn build_program_stream_mpeg2v_btrt<I>(
    samples: I,
    timescale: u32,
    leading_media_time: u64,
) -> Result<Btrt, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    if timescale == 0 {
        return Ok(Btrt::default());
    }
    let mut saw_sample = false;
    let mut buffer_size_db = 0_u32;
    let mut total_payload_bytes = 0_u64;
    let mut total_duration = 0_u64;
    for (data_size, duration) in samples {
        saw_sample = true;
        buffer_size_db = buffer_size_db.max(data_size);
        total_payload_bytes = total_payload_bytes
            .checked_add(u64::from(data_size))
            .ok_or(MuxError::LayoutOverflow(
                "program stream MPEG-2 video total payload bytes",
            ))?;
        total_duration =
            total_duration
                .checked_add(u64::from(duration))
                .ok_or(MuxError::LayoutOverflow(
                    "program stream MPEG-2 video total duration",
                ))?;
    }
    if !saw_sample || total_duration == 0 {
        return Ok(Btrt::default());
    }
    total_duration =
        total_duration
            .checked_add(leading_media_time)
            .ok_or(MuxError::LayoutOverflow(
                "program stream MPEG-2 video total duration",
            ))?;
    let avg_bitrate = total_payload_bytes
        .checked_mul(8)
        .and_then(|bits| bits.checked_mul(u64::from(timescale)))
        .ok_or(MuxError::LayoutOverflow(
            "program stream MPEG-2 video average bitrate",
        ))?
        / total_duration;
    let avg_bitrate = avg_bitrate & !7;
    Ok(Btrt {
        buffer_size_db,
        max_bitrate: u32::try_from(avg_bitrate)
            .map_err(|_| MuxError::LayoutOverflow("program stream MPEG-2 video maximum bitrate"))?,
        avg_bitrate: u32::try_from(avg_bitrate)
            .map_err(|_| MuxError::LayoutOverflow("program stream MPEG-2 video average bitrate"))?,
    })
}

fn parse_mpeg2_decoder_specific_info(
    decoder_specific_info: &[u8],
    spec: &str,
) -> Result<ParsedMpeg2DecoderConfig, MuxError> {
    let sequence_start = find_mpeg2_start_code(decoder_specific_info, SEQUENCE_START_CODE)
        .ok_or_else(|| {
            invalid_mpeg2v(
                spec,
                "MPEG-2 video decoder config did not contain a sequence header start code",
            )
        })?;
    if decoder_specific_info.len() < sequence_start + 8 {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video sequence header is truncated",
        ));
    }

    let header = &decoder_specific_info[sequence_start + 4..sequence_start + 8];
    let mut width = (u16::from(header[0]) << 4) | u16::from(header[1] >> 4);
    let mut height = (u16::from(header[1] & 0x0F) << 8) | u16::from(header[2]);
    let aspect_ratio_code = header[3] >> 4;
    let frame_rate_code = header[3] & 0x0F;
    let (timescale, sample_duration) =
        frame_rate_code_to_timing(frame_rate_code).ok_or_else(|| {
            invalid_mpeg2v(
                spec,
                "MPEG-2 video sequence header carried an unsupported frame-rate code",
            )
        })?;
    let pixel_aspect_ratio =
        aspect_ratio_code_to_pixel_aspect_ratio(aspect_ratio_code, width, height);

    let mut object_type_indication = MPEG1_VISUAL_OBJECT_TYPE;
    if let Some(extension_start) =
        find_mpeg2_sequence_extension_start(decoder_specific_info, sequence_start + 8)
    {
        if decoder_specific_info.len() < extension_start + 10 {
            return Err(invalid_mpeg2v(
                spec,
                "MPEG-2 video sequence extension is truncated",
            ));
        }
        let ext = &decoder_specific_info[extension_start + 4..extension_start + 10];
        let extension_id = ext[0] >> 4;
        if extension_id != 0x01 {
            return Err(invalid_mpeg2v(
                spec,
                "MPEG-2 video decoder config did not contain a sequence extension before the first picture",
            ));
        }
        let profile_and_level_indication = ((ext[0] & 0x0F) << 4) | (ext[1] >> 4);
        object_type_indication = map_mpeg2_profile_to_object_type(profile_and_level_indication)
            .ok_or_else(|| {
                invalid_mpeg2v(
                    spec,
                    "MPEG-2 video sequence extension carried an unsupported profile indication",
                )
            })?;
        let horizontal_size_extension = ((ext[1] & 0x01) << 1) | (ext[2] >> 7);
        let vertical_size_extension = (ext[2] >> 5) & 0x03;
        width |= u16::from(horizontal_size_extension) << 12;
        height |= u16::from(vertical_size_extension) << 12;
    }

    if width == 0 || height == 0 {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video decoder config carried a zero coded dimension",
        ));
    }

    Ok(ParsedMpeg2DecoderConfig {
        width,
        height,
        timescale,
        sample_duration,
        object_type_indication,
        pixel_aspect_ratio,
    })
}

fn aspect_ratio_code_to_pixel_aspect_ratio(
    aspect_ratio_code: u8,
    width: u16,
    height: u16,
) -> Option<(u32, u32)> {
    match aspect_ratio_code {
        1 => Some((1, 1)),
        2 => reduce_ratio(u64::from(height) * 4, u64::from(width) * 3),
        3 => reduce_ratio(u64::from(height) * 16, u64::from(width) * 9),
        4 => reduce_ratio(u64::from(height) * 221, u64::from(width) * 100),
        _ => None,
    }
}

fn reduce_ratio(numerator: u64, denominator: u64) -> Option<(u32, u32)> {
    if numerator == 0 || denominator == 0 {
        return None;
    }
    let divisor = gcd_u64(numerator, denominator);
    let reduced_numerator = numerator / divisor;
    let reduced_denominator = denominator / divisor;
    Some((
        u32::try_from(reduced_numerator).ok()?,
        u32::try_from(reduced_denominator).ok()?,
    ))
}

fn gcd_u64(mut lhs: u64, mut rhs: u64) -> u64 {
    while rhs != 0 {
        let remainder = lhs % rhs;
        lhs = rhs;
        rhs = remainder;
    }
    lhs
}

fn find_mpeg2_start_code(bytes: &[u8], start_code: u8) -> Option<usize> {
    let mut index = 0usize;
    while index + 4 <= bytes.len() {
        if bytes[index..].starts_with(&[0x00, 0x00, 0x01, start_code]) {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn find_mpeg2_sequence_extension_start(bytes: &[u8], from: usize) -> Option<usize> {
    let mut index = from;
    while index + 5 <= bytes.len() {
        if bytes[index..].starts_with(&[0x00, 0x00, 0x01, EXTENSION_START_CODE]) {
            return Some(index);
        }
        if bytes[index..].starts_with(&[0x00, 0x00, 0x01, PICTURE_START_CODE]) {
            return None;
        }
        index += 1;
    }
    None
}

fn frame_rate_code_to_timing(frame_rate_code: u8) -> Option<(u32, u32)> {
    match frame_rate_code {
        0x01 => Some(FRAME_RATE_23_976),
        0x02 => Some(FRAME_RATE_24),
        0x03 => Some(FRAME_RATE_25),
        0x04 => Some(FRAME_RATE_29_97),
        0x05 => Some(FRAME_RATE_30),
        0x06 => Some(FRAME_RATE_50),
        0x07 => Some(FRAME_RATE_59_94),
        0x08 => Some(FRAME_RATE_60),
        _ => None,
    }
}

fn map_mpeg2_profile_to_object_type(profile_and_level_indication: u8) -> Option<u8> {
    let escape_bit = profile_and_level_indication >> 7;
    let profile = (profile_and_level_indication >> 4) & 0x07;
    if escape_bit != 0 && profile == 0 {
        return Some(MPEG2_VISUAL_OBJECT_TYPE_422);
    }
    match profile {
        0x05 => Some(MPEG2_VISUAL_OBJECT_TYPE_SIMPLE),
        0x04 => Some(MPEG2_VISUAL_OBJECT_TYPE_MAIN),
        0x03 => Some(MPEG2_VISUAL_OBJECT_TYPE_SNR),
        0x02 => Some(MPEG2_VISUAL_OBJECT_TYPE_SPATIAL),
        0x01 => Some(MPEG2_VISUAL_OBJECT_TYPE_HIGH),
        _ => None,
    }
}

fn mpeg2v_picture_is_sync_sample_sync<F>(
    read_exact: &mut F,
    logical_size: u64,
    sample_start: u64,
    spec: &str,
) -> Result<bool, MuxError>
where
    F: FnMut(u64, &mut [u8], &'static str) -> Result<(), MuxError>,
{
    if sample_start
        .checked_add(6)
        .is_none_or(|end| end > logical_size)
    {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video picture header is truncated",
        ));
    }
    let mut header = [0_u8; 2];
    read_exact(
        sample_start + 4,
        &mut header,
        "MPEG-2 video picture coding-type header is truncated",
    )?;
    Ok(mpeg2v_picture_type(header) == 0x01)
}

#[cfg(feature = "async")]
async fn mpeg2v_picture_is_sync_sample_file_async(
    file: &mut TokioFile,
    logical_size: u64,
    sample_start: u64,
    spec: &str,
) -> Result<bool, MuxError> {
    if sample_start
        .checked_add(6)
        .is_none_or(|end| end > logical_size)
    {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video picture header is truncated",
        ));
    }
    let mut header = [0_u8; 2];
    read_exact_at_async(
        file,
        sample_start + 4,
        &mut header,
        spec,
        "MPEG-2 video picture coding-type header is truncated",
    )
    .await?;
    Ok(mpeg2v_picture_type(header) == 0x01)
}

#[cfg(feature = "async")]
async fn mpeg2v_picture_is_sync_sample_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    logical_size: u64,
    sample_start: u64,
    spec: &str,
) -> Result<bool, MuxError> {
    if sample_start
        .checked_add(6)
        .is_none_or(|end| end > logical_size)
    {
        return Err(invalid_mpeg2v(
            spec,
            "MPEG-2 video picture header is truncated",
        ));
    }
    let mut header = [0_u8; 2];
    read_segmented_bytes_async(
        file,
        segments,
        logical_size,
        sample_start + 4,
        &mut header,
        spec,
        "MPEG-2 video picture coding-type header is truncated",
    )
    .await?;
    Ok(mpeg2v_picture_type(header) == 0x01)
}

fn mpeg2v_picture_type(header: [u8; 2]) -> u8 {
    ((u16::from_be_bytes(header) >> 3) & 0x07) as u8
}

fn invalid_mpeg2v(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_mpeg2_decoder_specific_info;

    #[test]
    fn parse_mpeg2_decoder_specific_info_reads_sequence_extension_size_bits() {
        let decoder_specific_info = [
            0x00, 0x00, 0x01, 0xB3, 0x14, 0x00, 0xB4, 0x33, 0xFF, 0xFF, 0xE0, 0x18, 0x00, 0x00,
            0x01, 0xB5, 0x14, 0x8A, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0xB8, 0x00, 0x08,
            0x00, 0x40,
        ];
        let parsed = parse_mpeg2_decoder_specific_info(&decoder_specific_info, "test").unwrap();

        assert_eq!(parsed.width, 320);
        assert_eq!(parsed.height, 180);
        assert_eq!(parsed.pixel_aspect_ratio, Some((1, 1)));
        assert_eq!(parsed.object_type_indication, 0x61);
    }
}
