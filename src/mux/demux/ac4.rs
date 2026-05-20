use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::bitio::BitWriter;
use crate::boxes::AnyTypeBox;
use crate::boxes::etsi_ts_103_190::Dac4;
use crate::boxes::iso14496_12::{AudioSampleEntry, Btrt, SampleEntry};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{SegmentedMuxSourceSegment, StagedSample, read_exact_at_sync};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;
pub(in crate::mux) struct ParsedAc4Track {
    pub(in crate::mux) media_time_scale: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

#[derive(Clone, Copy, Debug)]
struct Ac4FrameHeader {
    header_size: u64,
    frame_payload_size: u64,
    total_frame_size: u64,
}

#[derive(Clone, Debug)]
struct ParsedAc4Stream {
    bitstream_version: u8,
    fs_index: u8,
    frame_rate_index: u8,
    has_program_id: bool,
    short_program_id: u16,
    program_uuid: Option<[u8; 16]>,
    bit_rate_mode: u8,
    presentations: Vec<ParsedAc4Presentation>,
    sample_rate: u32,
    sample_duration: u32,
    media_time_scale: u32,
    channel_count: u16,
}

#[derive(Clone, Debug)]
struct ParsedAc4Presentation {
    presentation_version: u8,
    mdcompat: u8,
    has_presentation_id: bool,
    presentation_id: u16,
    frame_rate_multiply_info: u8,
    frame_rate_fraction_info: u8,
    emdf_version: u8,
    key_id: u16,
    has_presentation_filter: bool,
    enable_presentation: bool,
    group_index: u32,
    group: Option<ParsedAc4SubstreamGroup>,
    pre_virtualized: bool,
    add_emdf_substreams: Vec<ParsedAc4EmdfInfo>,
    presentation_channel_mode: u8,
    presentation_channel_mask: u32,
    four_back_channels_present: bool,
    top_channel_pairs: u8,
}

#[derive(Clone, Debug)]
struct ParsedAc4SubstreamGroup {
    substreams_present: bool,
    high_sample_rate_extension: bool,
    substreams: Vec<ParsedAc4Substream>,
    content_type: Option<ParsedAc4ContentType>,
}

#[derive(Clone, Debug)]
struct ParsedAc4Substream {
    dsi_sf_multiplier: u8,
    has_substream_bitrate_indicator: bool,
    substream_bitrate_indicator: u8,
    channel_mask: u32,
    channel_mode: u8,
    four_back_channels_present: bool,
    top_channel_pairs: u8,
}

#[derive(Clone, Debug)]
struct ParsedAc4ContentType {
    classifier: u8,
    language_tag_bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
struct ParsedAc4EmdfInfo {
    version: u8,
    key_id: u16,
}

struct Ac4BitCursor<'a> {
    data: &'a [u8],
    bit_offset: usize,
}

impl<'a> Ac4BitCursor<'a> {
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
                message: format!("AC-4 parser requested invalid bit width {width} for {context}"),
            });
        }
        let end = self
            .bit_offset
            .checked_add(width)
            .ok_or(MuxError::LayoutOverflow("AC-4 bit reader position"))?;
        if end > self.data.len() * 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated AC-4 data while reading {context}"),
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

    fn skip_bits(&mut self, width: usize, spec: &str, context: &str) -> Result<(), MuxError> {
        let _ = self.read_bits(width, spec, context)?;
        Ok(())
    }
}

const AC4_SAMPLE_RATE_TABLE: [u32; 2] = [44_100, 48_000];
const AC4_SAMPLE_DELTA_TABLE_48: [u32; 14] = [
    2002, 2000, 1920, 8008, 1600, 1001, 1000, 960, 4004, 800, 480, 2002, 400, 2048,
];
const AC4_SAMPLE_DELTA_TABLE_441: [u32; 14] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2048];
const AC4_MEDIA_TIMESCALE_48: [u32; 14] = [
    48_000, 48_000, 48_000, 240_000, 48_000, 48_000, 48_000, 48_000, 240_000, 48_000, 48_000,
    240_000, 48_000, 48_000,
];
const AC4_MEDIA_TIMESCALE_441: [u32; 14] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 44_100];
const AC4_CHANNEL_MASK_BY_MODE: [u32; 17] = [
    0x2, 0x1, 0x3, 0x7, 0x47, 0x0f, 0x4f, 0x20007, 0x20047, 0x40007, 0x40047, 0x3f, 0x7f, 0x1003f,
    0x1007f, 0x2ff7f, 0,
];
pub(in crate::mux) fn scan_ac4_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedAc4Track, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let first_frame = read_ac4_frame_header_sync(&mut file, file_size, 0, spec)?;
    let first_payload = read_ac4_frame_payload_sync(&mut file, 0, first_frame, spec)?;
    let parsed_stream = parse_ac4_stream(&first_payload, spec)?;
    let first_is_sync_sample = read_ac4_frame_sync_flag(&first_payload, spec)?;
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    while offset < file_size {
        let frame = read_ac4_frame_header_sync(&mut file, file_size, offset, spec)?;
        let is_sync_sample = if offset == 0 {
            first_is_sync_sample
        } else {
            read_ac4_frame_sync_flag(
                &read_ac4_frame_payload_sync(&mut file, offset, frame, spec)?,
                spec,
            )?
        };
        samples.push(StagedSample {
            data_offset: offset
                .checked_add(frame.header_size)
                .ok_or(MuxError::LayoutOverflow("AC-4 payload offset"))?,
            data_size: u32::try_from(frame.frame_payload_size)
                .map_err(|_| MuxError::LayoutOverflow("AC-4 frame payload size"))?,
            duration: parsed_stream.sample_duration,
            composition_time_offset: 0,
            is_sync_sample,
        });
        offset = offset
            .checked_add(frame.total_frame_size)
            .ok_or(MuxError::LayoutOverflow("AC-4 frame offset"))?;
    }
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-4 input contained no syncframes".to_string(),
        });
    }
    Ok(ParsedAc4Track {
        media_time_scale: parsed_stream.media_time_scale,
        sample_entry_box: build_ac4_sample_entry_box(
            parsed_stream.channel_count,
            parsed_stream.sample_rate,
            parsed_stream.media_time_scale,
            &samples,
            &serialize_ac4_dac4(&parsed_stream, spec)?,
        )?,
        samples,
    })
}

pub(in crate::mux) fn scan_ac4_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedAc4Track, MuxError> {
    let first_frame = read_ac4_frame_header_segmented_sync(file, segments, total_size, 0, spec)?;
    let first_payload =
        read_ac4_frame_payload_segmented_sync(file, segments, total_size, 0, first_frame, spec)?;
    let parsed_stream = parse_ac4_stream(&first_payload, spec)?;
    let first_is_sync_sample = read_ac4_frame_sync_flag(&first_payload, spec)?;
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    while offset < total_size {
        let frame = read_ac4_frame_header_segmented_sync(file, segments, total_size, offset, spec)?;
        let is_sync_sample = if offset == 0 {
            first_is_sync_sample
        } else {
            read_ac4_frame_sync_flag(
                &read_ac4_frame_payload_segmented_sync(file, segments, total_size, offset, frame, spec)?,
                spec,
            )?
        };
        samples.push(StagedSample {
            data_offset: offset
                .checked_add(frame.header_size)
                .ok_or(MuxError::LayoutOverflow("AC-4 payload offset"))?,
            data_size: u32::try_from(frame.frame_payload_size)
                .map_err(|_| MuxError::LayoutOverflow("AC-4 frame payload size"))?,
            duration: parsed_stream.sample_duration,
            composition_time_offset: 0,
            is_sync_sample,
        });
        offset = offset
            .checked_add(frame.total_frame_size)
            .ok_or(MuxError::LayoutOverflow("AC-4 frame offset"))?;
    }
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-4 input contained no syncframes".to_string(),
        });
    }
    Ok(ParsedAc4Track {
        media_time_scale: parsed_stream.media_time_scale,
        sample_entry_box: build_ac4_sample_entry_box(
            parsed_stream.channel_count,
            parsed_stream.sample_rate,
            parsed_stream.media_time_scale,
            &samples,
            &serialize_ac4_dac4(&parsed_stream, spec)?,
        )?,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_ac4_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedAc4Track, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let first_frame = read_ac4_frame_header_async(&mut file, file_size, 0, spec).await?;
    let first_payload = read_ac4_frame_payload_async(&mut file, 0, first_frame, spec).await?;
    let parsed_stream = parse_ac4_stream(&first_payload, spec)?;
    let first_is_sync_sample = read_ac4_frame_sync_flag(&first_payload, spec)?;
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    while offset < file_size {
        let frame = read_ac4_frame_header_async(&mut file, file_size, offset, spec).await?;
        let is_sync_sample = if offset == 0 {
            first_is_sync_sample
        } else {
            read_ac4_frame_sync_flag(
                &read_ac4_frame_payload_async(&mut file, offset, frame, spec).await?,
                spec,
            )?
        };
        samples.push(StagedSample {
            data_offset: offset
                .checked_add(frame.header_size)
                .ok_or(MuxError::LayoutOverflow("AC-4 payload offset"))?,
            data_size: u32::try_from(frame.frame_payload_size)
                .map_err(|_| MuxError::LayoutOverflow("AC-4 frame payload size"))?,
            duration: parsed_stream.sample_duration,
            composition_time_offset: 0,
            is_sync_sample,
        });
        offset = offset
            .checked_add(frame.total_frame_size)
            .ok_or(MuxError::LayoutOverflow("AC-4 frame offset"))?;
    }
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-4 input contained no syncframes".to_string(),
        });
    }
    Ok(ParsedAc4Track {
        media_time_scale: parsed_stream.media_time_scale,
        sample_entry_box: build_ac4_sample_entry_box(
            parsed_stream.channel_count,
            parsed_stream.sample_rate,
            parsed_stream.media_time_scale,
            &samples,
            &serialize_ac4_dac4(&parsed_stream, spec)?,
        )?,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_ac4_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedAc4Track, MuxError> {
    let first_frame =
        read_ac4_frame_header_segmented_async(file, segments, total_size, 0, spec).await?;
    let first_payload =
        read_ac4_frame_payload_segmented_async(file, segments, total_size, 0, first_frame, spec)
            .await?;
    let parsed_stream = parse_ac4_stream(&first_payload, spec)?;
    let first_is_sync_sample = read_ac4_frame_sync_flag(&first_payload, spec)?;
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    while offset < total_size {
        let frame =
            read_ac4_frame_header_segmented_async(file, segments, total_size, offset, spec).await?;
        let is_sync_sample = if offset == 0 {
            first_is_sync_sample
        } else {
            read_ac4_frame_sync_flag(
                &read_ac4_frame_payload_segmented_async(file, segments, total_size, offset, frame, spec)
                    .await?,
                spec,
            )?
        };
        samples.push(StagedSample {
            data_offset: offset
                .checked_add(frame.header_size)
                .ok_or(MuxError::LayoutOverflow("AC-4 payload offset"))?,
            data_size: u32::try_from(frame.frame_payload_size)
                .map_err(|_| MuxError::LayoutOverflow("AC-4 frame payload size"))?,
            duration: parsed_stream.sample_duration,
            composition_time_offset: 0,
            is_sync_sample,
        });
        offset = offset
            .checked_add(frame.total_frame_size)
            .ok_or(MuxError::LayoutOverflow("AC-4 frame offset"))?;
    }
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-4 input contained no syncframes".to_string(),
        });
    }
    Ok(ParsedAc4Track {
        media_time_scale: parsed_stream.media_time_scale,
        sample_entry_box: build_ac4_sample_entry_box(
            parsed_stream.channel_count,
            parsed_stream.sample_rate,
            parsed_stream.media_time_scale,
            &samples,
            &serialize_ac4_dac4(&parsed_stream, spec)?,
        )?,
        samples,
    })
}

fn read_ac4_frame_header_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<Ac4FrameHeader, MuxError> {
    let mut header = [0_u8; 7];
    if file_size - offset < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated AC-4 syncframe header".to_string(),
        });
    }
    read_exact_at_sync(
        file,
        offset,
        &mut header[..4],
        spec,
        "truncated AC-4 syncframe header",
    )?;
    if u16::from_be_bytes([header[2], header[3]]) == 0xFFFF {
        if file_size - offset < 7 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated extended AC-4 syncframe header".to_string(),
            });
        }
        read_exact_at_sync(
            file,
            offset,
            &mut header,
            spec,
            "truncated extended AC-4 syncframe header",
        )?;
    }
    parse_ac4_frame_header(&header, file_size, offset, spec)
}

fn read_ac4_frame_header_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    offset: u64,
    spec: &str,
) -> Result<Ac4FrameHeader, MuxError> {
    let mut header = [0_u8; 7];
    if total_size - offset < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated AC-4 syncframe header".to_string(),
        });
    }
    read_segmented_bytes_sync(
        file,
        segments,
        total_size,
        offset,
        &mut header[..4],
        spec,
        "truncated AC-4 syncframe header",
    )?;
    if u16::from_be_bytes([header[2], header[3]]) == 0xFFFF {
        if total_size - offset < 7 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated extended AC-4 syncframe header".to_string(),
            });
        }
        read_segmented_bytes_sync(
            file,
            segments,
            total_size,
            offset,
            &mut header,
            spec,
            "truncated extended AC-4 syncframe header",
        )?;
    }
    parse_ac4_frame_header(&header, total_size, offset, spec)
}

#[cfg(feature = "async")]
async fn read_ac4_frame_header_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<Ac4FrameHeader, MuxError> {
    let mut header = [0_u8; 7];
    if file_size - offset < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated AC-4 syncframe header".to_string(),
        });
    }
    read_exact_at_async(
        file,
        offset,
        &mut header[..4],
        spec,
        "truncated AC-4 syncframe header",
    )
    .await?;
    if u16::from_be_bytes([header[2], header[3]]) == 0xFFFF {
        if file_size - offset < 7 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated extended AC-4 syncframe header".to_string(),
            });
        }
        read_exact_at_async(
            file,
            offset,
            &mut header,
            spec,
            "truncated extended AC-4 syncframe header",
        )
        .await?;
    }
    parse_ac4_frame_header(&header, file_size, offset, spec)
}

#[cfg(feature = "async")]
async fn read_ac4_frame_header_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    offset: u64,
    spec: &str,
) -> Result<Ac4FrameHeader, MuxError> {
    let mut header = [0_u8; 7];
    if total_size - offset < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated AC-4 syncframe header".to_string(),
        });
    }
    read_segmented_bytes_async(
        file,
        segments,
        total_size,
        offset,
        &mut header[..4],
        spec,
        "truncated AC-4 syncframe header",
    )
    .await?;
    if u16::from_be_bytes([header[2], header[3]]) == 0xFFFF {
        if total_size - offset < 7 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated extended AC-4 syncframe header".to_string(),
            });
        }
        read_segmented_bytes_async(
            file,
            segments,
            total_size,
            offset,
            &mut header,
            spec,
            "truncated extended AC-4 syncframe header",
        )
        .await?;
    }
    parse_ac4_frame_header(&header, total_size, offset, spec)
}

fn parse_ac4_frame_header(
    header: &[u8; 7],
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<Ac4FrameHeader, MuxError> {
    let syncword = u16::from_be_bytes([header[0], header[1]]);
    if syncword != 0xAC40 && syncword != 0xAC41 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("missing AC-4 sync word at byte offset {offset}"),
        });
    }
    let size_code = u16::from_be_bytes([header[2], header[3]]);
    let (header_size, frame_payload_size) = if size_code == 0xFFFF {
        (
            7_u64,
            u64::from(header[4]) << 16 | u64::from(header[5]) << 8 | u64::from(header[6]),
        )
    } else {
        (4_u64, u64::from(size_code))
    };
    let crc_size = if syncword == 0xAC41 { 2_u64 } else { 0_u64 };
    let mut total_frame_size = header_size
        .checked_add(frame_payload_size)
        .and_then(|size| size.checked_add(crc_size))
        .ok_or(MuxError::LayoutOverflow("AC-4 frame size"))?;
    if total_frame_size <= header_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-4 syncframes must carry payload bytes".to_string(),
        });
    }
    if offset
        .checked_add(total_frame_size)
        .is_none_or(|end| end > file_size)
    {
        if size_code != 0xFFFF {
            let alternate_frame_size = u64::from(size_code)
                .checked_add(2)
                .and_then(|size| size.checked_add(crc_size))
                .ok_or(MuxError::LayoutOverflow("AC-4 alternate frame size"))?;
            if alternate_frame_size > header_size
                && offset
                    .checked_add(alternate_frame_size)
                    .is_some_and(|end| end <= file_size)
            {
                total_frame_size = alternate_frame_size;
            } else {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!("truncated AC-4 syncframe at byte offset {offset}"),
                });
            }
        } else {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated AC-4 syncframe at byte offset {offset}"),
            });
        }
    }
    Ok(Ac4FrameHeader {
        header_size,
        frame_payload_size,
        total_frame_size,
    })
}

fn read_ac4_frame_payload_sync(
    file: &mut File,
    offset: u64,
    header: Ac4FrameHeader,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    let mut payload = vec![
        0_u8;
        usize::try_from(header.frame_payload_size)
            .map_err(|_| MuxError::LayoutOverflow("AC-4 frame payload size"))?
    ];
    read_exact_at_sync(
        file,
        offset
            .checked_add(header.header_size)
            .ok_or(MuxError::LayoutOverflow("AC-4 payload offset"))?,
        &mut payload,
        spec,
        "truncated AC-4 frame payload",
    )?;
    Ok(payload)
}

fn read_ac4_frame_payload_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    offset: u64,
    header: Ac4FrameHeader,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    let mut payload = vec![
        0_u8;
        usize::try_from(header.frame_payload_size)
            .map_err(|_| MuxError::LayoutOverflow("AC-4 frame payload size"))?
    ];
    read_segmented_bytes_sync(
        file,
        segments,
        total_size,
        offset
            .checked_add(header.header_size)
            .ok_or(MuxError::LayoutOverflow("AC-4 payload offset"))?,
        &mut payload,
        spec,
        "truncated AC-4 frame payload",
    )?;
    Ok(payload)
}

#[cfg(feature = "async")]
async fn read_ac4_frame_payload_async(
    file: &mut TokioFile,
    offset: u64,
    header: Ac4FrameHeader,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    let mut payload = vec![
        0_u8;
        usize::try_from(header.frame_payload_size)
            .map_err(|_| MuxError::LayoutOverflow("AC-4 frame payload size"))?
    ];
    read_exact_at_async(
        file,
        offset
            .checked_add(header.header_size)
            .ok_or(MuxError::LayoutOverflow("AC-4 payload offset"))?,
        &mut payload,
        spec,
        "truncated AC-4 frame payload",
    )
    .await?;
    Ok(payload)
}

#[cfg(feature = "async")]
async fn read_ac4_frame_payload_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    offset: u64,
    header: Ac4FrameHeader,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    let mut payload = vec![
        0_u8;
        usize::try_from(header.frame_payload_size)
            .map_err(|_| MuxError::LayoutOverflow("AC-4 frame payload size"))?
    ];
    read_segmented_bytes_async(
        file,
        segments,
        total_size,
        offset
            .checked_add(header.header_size)
            .ok_or(MuxError::LayoutOverflow("AC-4 payload offset"))?,
        &mut payload,
        spec,
        "truncated AC-4 frame payload",
    )
    .await?;
    Ok(payload)
}

fn parse_ac4_stream(frame_payload: &[u8], spec: &str) -> Result<ParsedAc4Stream, MuxError> {
    let mut reader = Ac4BitCursor::new(frame_payload);
    let bitstream_version = u8::try_from(read_ac4_variable_bits_prefixed(
        &mut reader,
        spec,
        "bitstream_version",
        2,
        Some(3),
    )?)
    .map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "AC-4 bitstream version does not fit in u8".to_string(),
    })?;
    if bitstream_version <= 1 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "path-only AC-4 import currently requires bitstream_version > 1".to_string(),
        });
    }

    let _sequence_counter = reader.read_bits(10, spec, "sequence_counter")?;
    let wait_frames = if reader.read_bool(spec, "b_wait_frames")? {
        let wait_frames = reader.read_bits(3, spec, "wait_frames")?;
        if wait_frames > 0 {
            reader.skip_bits(2, spec, "wait_frames reserved bits")?;
        }
        Some(wait_frames)
    } else {
        None
    };
    let fs_index = u8::try_from(reader.read_bits(1, spec, "fs_index")?).unwrap();
    let frame_rate_index = usize::try_from(reader.read_bits(4, spec, "frame_rate_index")?)
        .map_err(|_| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-4 frame_rate_index does not fit in usize".to_string(),
        })?;
    let _iframe_global = reader.read_bool(spec, "b_iframe_global")?;
    let presentation_count = if reader.read_bool(spec, "b_single_presentation")? {
        1_usize
    } else if reader.read_bool(spec, "b_more_presentations")? {
        usize::try_from(read_ac4_variable_bits(&mut reader, spec, "n_presentations", 2)? + 2)
            .map_err(|_| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "AC-4 presentation count does not fit in usize".to_string(),
            })?
    } else {
        0
    };
    if presentation_count == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-4 inputs without presentations are not supported".to_string(),
        });
    }

    if reader.read_bool(spec, "b_payload_base")? {
        let payload_base = reader.read_bits(5, spec, "payload_base_minus1")? + 1;
        if payload_base == 0x20 {
            let _ = read_ac4_variable_bits(&mut reader, spec, "payload_base extension", 3)?;
        }
    }

    let has_program_id = reader.read_bool(spec, "b_program_id")?;
    let (short_program_id, program_uuid) = if has_program_id {
        let short_program_id =
            u16::try_from(reader.read_bits(16, spec, "short_program_id")?).unwrap();
        let program_uuid = if reader.read_bool(spec, "b_program_uuid_present")? {
            let mut uuid = [0_u8; 16];
            for byte in &mut uuid {
                *byte = u8::try_from(reader.read_bits(8, spec, "program_uuid")?).unwrap();
            }
            Some(uuid)
        } else {
            None
        };
        (short_program_id, program_uuid)
    } else {
        (0, None)
    };

    let sample_rate = *AC4_SAMPLE_RATE_TABLE
        .get(usize::from(fs_index))
        .ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported AC-4 sampling frequency index {fs_index}"),
        })?;
    let (sample_duration, media_time_scale) =
        ac4_timing_from_frame_rate_index(fs_index, frame_rate_index, spec)?;
    let bit_rate_mode = match wait_frames {
        Some(0) => 1,
        Some(1..=6) => 2,
        Some(_) => 3,
        None => 0,
    };

    let mut presentations = Vec::with_capacity(presentation_count);
    for presentation_index in 0..presentation_count {
        presentations.push(parse_ac4_presentation(
            &mut reader,
            spec,
            bitstream_version,
            fs_index,
            frame_rate_index,
            presentation_index,
        )?);
    }
    let mut referenced_group_indices = Vec::new();
    for presentation in &presentations {
        if !referenced_group_indices.contains(&presentation.group_index) {
            referenced_group_indices.push(presentation.group_index);
        }
    }
    let mut parsed_groups = Vec::with_capacity(referenced_group_indices.len());
    for group_index in &referenced_group_indices {
        let group_presentation = presentations
            .iter()
            .find(|presentation| presentation.group_index == *group_index)
            .ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("AC-4 substream group {group_index} is not referenced by any presentation"),
            })?;
        let group_frame_rate_factor = match group_presentation.frame_rate_multiply_info {
            0 => 1,
            value => u32::from(value) * 2,
        };
        let parsed_group = parse_ac4_substream_group(
            &mut reader,
            spec,
            group_frame_rate_factor,
            fs_index,
            group_presentation.presentation_version,
        )?;
        parsed_groups.push(parsed_group);
    }
    let default_speaker_group_mask = presentations
        .first()
        .and_then(|presentation| {
            referenced_group_indices
                .iter()
                .position(|group_index| *group_index == presentation.group_index)
                .and_then(|position| parsed_groups.get(position))
        })
        .map(|group| {
            group.substreams.iter().fold(0_u32, |mask, substream| {
                mask | substream.channel_mask
            })
        })
        .unwrap_or(0);
    for presentation in &mut presentations {
        let group_position = referenced_group_indices
            .iter()
            .position(|group_index| *group_index == presentation.group_index)
            .ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "AC-4 presentation references unknown substream group {}",
                    presentation.group_index
                ),
            })?;
        presentation.group = Some(
            parsed_groups[group_position].clone(),
        );
        populate_ac4_presentation_channels(presentation);
        normalize_ac4_presentation_for_dsi(presentation);
    }
    append_legacy_ac4_presentations(&mut presentations);

    let presentation = presentations
        .first()
        .ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-4 input contained no supported presentations".to_string(),
        })?;
    let channel_count = if default_speaker_group_mask == 0 {
        ac4_channel_count_from_mask(presentation.presentation_channel_mask)?
    } else {
        ac4_channel_count_from_mask(default_speaker_group_mask)?
    };

    Ok(ParsedAc4Stream {
        bitstream_version,
        fs_index,
        frame_rate_index: u8::try_from(frame_rate_index).unwrap(),
        has_program_id,
        short_program_id,
        program_uuid,
        bit_rate_mode,
        presentations,
        sample_rate,
        sample_duration,
        media_time_scale,
        channel_count,
    })
}

fn read_ac4_frame_sync_flag(frame_payload: &[u8], spec: &str) -> Result<bool, MuxError> {
    let mut reader = Ac4BitCursor::new(frame_payload);
    let bitstream_version = u8::try_from(read_ac4_variable_bits_prefixed(
        &mut reader,
        spec,
        "bitstream_version",
        2,
        Some(3),
    )?)
    .map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "AC-4 bitstream version does not fit in u8".to_string(),
    })?;
    if bitstream_version <= 1 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "path-only AC-4 import currently requires bitstream_version > 1".to_string(),
        });
    }
    let _sequence_counter = reader.read_bits(10, spec, "sequence_counter")?;
    if reader.read_bool(spec, "b_wait_frames")? {
        let wait_frames = reader.read_bits(3, spec, "wait_frames")?;
        if wait_frames > 0 {
            reader.skip_bits(2, spec, "wait_frames reserved bits")?;
        }
    }
    let _fs_index = reader.read_bits(1, spec, "fs_index")?;
    let _frame_rate_index = reader.read_bits(4, spec, "frame_rate_index")?;
    reader.read_bool(spec, "b_iframe_global")
}

fn parse_ac4_presentation(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
    bitstream_version: u8,
    fs_index: u8,
    frame_rate_index: usize,
    presentation_index: usize,
) -> Result<ParsedAc4Presentation, MuxError> {
    let single_substream_group = reader.read_bool(spec, "b_single_substream_group")?;
    if !single_substream_group {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "AC-4 presentation {} uses multiple substream groups; path-only AC-4 import currently supports only single-group presentations",
                presentation_index + 1
            ),
        });
    }

    let presentation_version = parse_ac4_presentation_version(reader, spec)?;
    if presentation_version > 2 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "AC-4 presentation {} uses unsupported presentation_version {}",
                presentation_index + 1,
                presentation_version
            ),
        });
    }

    let mdcompat = u8::try_from(reader.read_bits(3, spec, "mdcompat")?).unwrap();
    let has_presentation_id = reader.read_bool(spec, "b_presentation_id")?;
    let presentation_id = if has_presentation_id {
        u16::try_from(read_ac4_variable_bits(reader, spec, "presentation_id", 2)?).unwrap()
    } else {
        0
    };
    if presentation_id > 31 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "AC-4 presentation {} uses presentation_id {} larger than the currently supported path-only DSI writer can represent",
                presentation_index + 1,
                presentation_id
            ),
        });
    }

    let frame_rate_multiply_info =
        parse_ac4_frame_rate_multiply_info(reader, spec, frame_rate_index)?;
    let frame_rate_fraction_info =
        parse_ac4_frame_rate_fraction_info(reader, spec, frame_rate_index)?;
    let emdf_info = parse_ac4_emdf_info(reader, spec)?;
    let has_presentation_filter = reader.read_bool(spec, "b_presentation_filter")?;
    let enable_presentation = if has_presentation_filter {
        reader.read_bool(spec, "b_enable_presentation")?
    } else {
        false
    };
    let _ = fs_index;
    let group_index = parse_ac4_group_index(reader, spec, bitstream_version)?;
    let mut pre_virtualized = reader.read_bool(spec, "b_pre_virtualized")?;
    if presentation_version == 2 {
        pre_virtualized = true;
    }
    let has_add_emdf_substreams = reader.read_bool(spec, "b_add_emdf_substreams")?;
    skip_ac4_presentation_substream_info(reader, spec)?;

    let mut add_emdf_substreams = Vec::new();
    if has_add_emdf_substreams {
        let count = {
            let raw = reader.read_bits(2, spec, "n_add_emdf_substreams")?;
            if raw == 0 {
                usize::try_from(
                    read_ac4_variable_bits(reader, spec, "n_add_emdf_substreams extension", 2)? + 4,
                )
                .map_err(|_| MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "AC-4 add_emdf_substreams count does not fit in usize".to_string(),
                })?
            } else {
                usize::try_from(raw).unwrap()
            }
        };
        for _ in 0..count {
            add_emdf_substreams.push(parse_ac4_emdf_info(reader, spec)?);
        }
    }

    Ok(ParsedAc4Presentation {
        presentation_version,
        mdcompat,
        has_presentation_id,
        presentation_id,
        frame_rate_multiply_info,
        frame_rate_fraction_info,
        emdf_version: emdf_info.version,
        key_id: emdf_info.key_id,
        has_presentation_filter,
        enable_presentation,
        group_index,
        group: None,
        pre_virtualized,
        add_emdf_substreams,
        presentation_channel_mode: 0,
        presentation_channel_mask: 0,
        four_back_channels_present: false,
        top_channel_pairs: 0,
    })
}

fn populate_ac4_presentation_channels(presentation: &mut ParsedAc4Presentation) {
    let Some(group) = &presentation.group else {
        return;
    };
    let mut substreams = group.substreams.iter();
    let Some(first_substream) = substreams.next() else {
        return;
    };
    let mut presentation_channel_mode = first_substream.channel_mode;
    let mut presentation_channel_mask = first_substream.channel_mask;
    let mut four_back_channels_present = first_substream.four_back_channels_present;
    let mut top_channel_pairs = first_substream.top_channel_pairs;
    for substream in substreams {
        presentation_channel_mode = presentation_channel_mode.max(substream.channel_mode);
        presentation_channel_mask |= substream.channel_mask;
        four_back_channels_present |= substream.four_back_channels_present;
        top_channel_pairs = top_channel_pairs.max(substream.top_channel_pairs);
    }
    if presentation_channel_mask == 0x03 {
        presentation_channel_mask = 0x01;
    }
    if (presentation_channel_mask & 0x30) != 0 && (presentation_channel_mask & 0x80) != 0 {
        presentation_channel_mask &= !0x80;
    }
    presentation.presentation_channel_mode = presentation_channel_mode;
    presentation.presentation_channel_mask = presentation_channel_mask;
    presentation.four_back_channels_present = four_back_channels_present;
    presentation.top_channel_pairs = top_channel_pairs;
}

fn normalize_ac4_presentation_for_dsi(presentation: &mut ParsedAc4Presentation) {
    let uses_stereo_fallback = presentation.presentation_channel_mode == 0
        && presentation.presentation_channel_mask == 0x02;
    presentation.has_presentation_filter = false;
    presentation.enable_presentation = false;
    presentation.add_emdf_substreams.clear();
    presentation.presentation_channel_mode = ac4_channel_mode_from_mask(
        presentation.presentation_channel_mask,
        presentation.presentation_channel_mode,
    );
    if presentation.top_channel_pairs == 0 && (presentation.presentation_channel_mask & 0x80) != 0
    {
        presentation.top_channel_pairs = 1;
    }
    if uses_stereo_fallback {
        presentation.presentation_channel_mode = 1;
        presentation.presentation_channel_mask = 0x01;
    }

    if let Some(group) = &mut presentation.group {
        group.substreams_present = true;
        group.high_sample_rate_extension = false;
        group.content_type = None;
        group.substreams.clear();
        group.substreams.push(ParsedAc4Substream {
            dsi_sf_multiplier: 0,
            has_substream_bitrate_indicator: false,
            substream_bitrate_indicator: 0,
            channel_mask: presentation.presentation_channel_mask,
            channel_mode: presentation.presentation_channel_mode,
            four_back_channels_present: presentation.four_back_channels_present,
            top_channel_pairs: presentation.top_channel_pairs,
        });
        if uses_stereo_fallback && group.content_type.is_none() {
            group.content_type = Some(ParsedAc4ContentType {
                classifier: 0,
                language_tag_bytes: Vec::new(),
            });
        }
    }
}

fn append_legacy_ac4_presentations(presentations: &mut Vec<ParsedAc4Presentation>) {
    if presentations
        .iter()
        .any(|presentation| presentation.presentation_version == 1)
    {
        return;
    }

    let legacy = presentations
        .iter()
        .filter(|presentation| presentation.presentation_version == 2)
        .cloned()
        .map(|mut presentation| {
            presentation.presentation_version = 1;
            presentation.pre_virtualized = false;
            presentation
        })
        .collect::<Vec<_>>();
    presentations.extend(legacy);
}

fn parse_ac4_substream_group(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
    frame_rate_factor: u32,
    fs_index: u8,
    presentation_version: u8,
) -> Result<ParsedAc4SubstreamGroup, MuxError> {
    let substreams_present = reader.read_bool(spec, "b_substreams_present")?;
    let high_sample_rate_extension = reader.read_bool(spec, "b_hsf_ext")?;
    let single_substream = reader.read_bool(spec, "b_single_substream")?;
    let lf_substream_count = if single_substream {
        1_usize
    } else {
        let count = reader.read_bits(2, spec, "n_lf_substreams_minus2")? + 2;
        if count == 5 {
            usize::try_from(
                count + read_ac4_variable_bits(reader, spec, "n_lf_substreams extension", 2)?,
            )
            .map_err(|_| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "AC-4 lf substream count does not fit in usize".to_string(),
            })?
        } else {
            usize::try_from(count).unwrap()
        }
    };
    if !reader.read_bool(spec, "b_channel_coded")? {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "path-only AC-4 import currently supports only channel-coded substreams"
                .to_string(),
        });
    }

    let mut substreams = Vec::with_capacity(lf_substream_count);
    for _ in 0..lf_substream_count {
        substreams.push(parse_ac4_channel_coded_substream(
            reader,
            spec,
            frame_rate_factor,
            substreams_present,
            fs_index,
            presentation_version,
        )?);
        if high_sample_rate_extension {
            skip_ac4_hsf_ext_substream_info(reader, spec, substreams_present)?;
        }
    }
    let content_type = parse_ac4_content_type(reader, spec)?;

    Ok(ParsedAc4SubstreamGroup {
        substreams_present,
        high_sample_rate_extension,
        substreams,
        content_type,
    })
}

fn skip_ac4_hsf_ext_substream_info(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
    substreams_present: bool,
) -> Result<(), MuxError> {
    if substreams_present {
        let substream_index = reader.read_bits(2, spec, "hsf_ext_substream_index")?;
        if substream_index == 3 {
            let _ = read_ac4_variable_bits(reader, spec, "hsf_ext_substream_index extension", 2)?;
        }
    }
    Ok(())
}

fn parse_ac4_channel_coded_substream(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
    frame_rate_factor: u32,
    substreams_present: bool,
    fs_index: u8,
    presentation_version: u8,
) -> Result<ParsedAc4Substream, MuxError> {
    let channel_mode = parse_ac4_channel_mode(reader, spec, presentation_version)?;
    let mut channel_mask = *AC4_CHANNEL_MASK_BY_MODE
        .get(usize::from(channel_mode))
        .ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported AC-4 channel mode {channel_mode}"),
        })?;
    let mut four_back_channels_present = false;
    let mut top_channel_pairs = 0_u8;
    if (11..=14).contains(&channel_mode) {
        four_back_channels_present = reader.read_bool(spec, "b_4_back_channels_present")?;
        let centre_present = reader.read_bool(spec, "b_centre_present")?;
        top_channel_pairs =
            u8::try_from(reader.read_bits(2, spec, "top_channels_present")?).unwrap();
        if !four_back_channels_present {
            channel_mask &= !0x8;
        }
        if !centre_present {
            channel_mask &= !0x2;
        }
        match top_channel_pairs {
            0 => {
                channel_mask &= !0x30;
            }
            1 | 2 => {
                channel_mask &= !0x30;
                channel_mask |= 0x80;
            }
            _ => {}
        }
    }
    let dsi_sf_multiplier = if fs_index == 1 && reader.read_bool(spec, "b_sf_multiplier")? {
        u8::try_from(reader.read_bits(1, spec, "sf_multiplier")? + 1).unwrap()
    } else {
        0
    };
    let has_substream_bitrate_indicator =
        reader.read_bool(spec, "b_substream_bitrate_indicator")?;
    let substream_bitrate_indicator = if has_substream_bitrate_indicator {
        let mut indicator =
            u8::try_from(reader.read_bits(3, spec, "substream_bitrate_indicator")?).unwrap();
        if indicator & 0x01 == 1 {
            indicator = (indicator << 2)
                | u8::try_from(reader.read_bits(
                    2,
                    spec,
                    "substream_bitrate_indicator extension",
                )?)
                .unwrap();
        }
        indicator
    } else {
        0
    };
    if (7..=10).contains(&channel_mode) {
        let _ = reader.read_bool(spec, "add_ch_base")?;
    }
    for _ in 0..frame_rate_factor {
        let _ = reader.read_bool(spec, "b_audio_ndot")?;
    }
    if substreams_present {
        let substream_index = reader.read_bits(2, spec, "substream_index")?;
        if substream_index == 3 {
            let _ = read_ac4_variable_bits(reader, spec, "substream_index extension", 2)?;
        }
    }

    Ok(ParsedAc4Substream {
        dsi_sf_multiplier,
        has_substream_bitrate_indicator,
        substream_bitrate_indicator,
        channel_mask,
        channel_mode,
        four_back_channels_present,
        top_channel_pairs,
    })
}

fn parse_ac4_channel_mode(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
    presentation_version: u8,
) -> Result<u8, MuxError> {
    let mut code = reader.read_bits(1, spec, "channel_mode")?;
    if code == 0 {
        return Ok(0);
    }
    code = (code << 1) | reader.read_bits(1, spec, "channel_mode")?;
    if code == 2 {
        return Ok(1);
    }
    code = (code << 2) | reader.read_bits(2, spec, "channel_mode")?;
    match code {
        12 => return Ok(2),
        13 => return Ok(3),
        14 => return Ok(4),
        _ => {}
    }
    code = (code << 3) | reader.read_bits(3, spec, "channel_mode")?;
    match code {
        120 if presentation_version == 2 => return Ok(1),
        121 if presentation_version == 2 => return Ok(1),
        120 => return Ok(5),
        121 => return Ok(6),
        122 => return Ok(7),
        123 => return Ok(8),
        124 => return Ok(9),
        125 => return Ok(10),
        _ => {}
    }
    code = (code << 1) | reader.read_bits(1, spec, "channel_mode")?;
    match code {
        252 => return Ok(11),
        253 => return Ok(12),
        _ => {}
    }
    code = (code << 1) | reader.read_bits(1, spec, "channel_mode")?;
    match code {
        508 => Ok(13),
        509 => Ok(14),
        510 => Ok(15),
        _ => Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "unsupported or reserved AC-4 channel mode".to_string(),
        }),
    }
}

fn parse_ac4_content_type(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
) -> Result<Option<ParsedAc4ContentType>, MuxError> {
    if !reader.read_bool(spec, "b_content_type")? {
        return Ok(None);
    }
    let classifier = u8::try_from(reader.read_bits(3, spec, "content_classifier")?).unwrap();
    let language_tag_bytes = if reader.read_bool(spec, "b_language_indicator")? {
        if reader.read_bool(spec, "b_serialized_language_tag")? {
            let _ = reader.read_bool(spec, "b_start_tag")?;
            let _ = reader.read_bits(16, spec, "language_tag_chunk")?;
            Vec::new()
        } else {
            let len = usize::try_from(reader.read_bits(6, spec, "n_language_tag_bytes")?).unwrap();
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                bytes.push(u8::try_from(reader.read_bits(8, spec, "language_tag_byte")?).unwrap());
            }
            bytes
        }
    } else {
        Vec::new()
    };
    Ok(Some(ParsedAc4ContentType {
        classifier,
        language_tag_bytes,
    }))
}

fn parse_ac4_group_index(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
    bitstream_version: u8,
) -> Result<u32, MuxError> {
    if bitstream_version == 1 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "path-only AC-4 import does not support bitstream_version 1".to_string(),
        });
    }
    let group_index = reader.read_bits(3, spec, "group_index")?;
    if group_index == 7 {
        read_ac4_variable_bits(reader, spec, "group_index extension", 2)
            .map(|value| group_index + value)
    } else {
        Ok(group_index)
    }
}

fn parse_ac4_presentation_version(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
) -> Result<u8, MuxError> {
    let mut version = 0_u8;
    while reader.read_bool(spec, "presentation_version")? {
        version = version
            .checked_add(1)
            .ok_or(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "AC-4 presentation version overflow".to_string(),
            })?;
    }
    Ok(version)
}

fn parse_ac4_frame_rate_multiply_info(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
    frame_rate_index: usize,
) -> Result<u8, MuxError> {
    let value = match frame_rate_index {
        2..=4 => {
            if reader.read_bool(spec, "b_multiplier")? {
                if reader.read_bool(spec, "multiplier_bit")? {
                    2
                } else {
                    1
                }
            } else {
                0
            }
        }
        0 | 1 | 7 | 8 | 9 => {
            if reader.read_bool(spec, "b_multiplier")? {
                1
            } else {
                0
            }
        }
        _ => 0,
    };
    Ok(value)
}

fn parse_ac4_frame_rate_fraction_info(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
    frame_rate_index: usize,
) -> Result<u8, MuxError> {
    let value = match frame_rate_index {
        5..=9 => {
            if reader.read_bool(spec, "b_frame_rate_fraction")? {
                1
            } else {
                0
            }
        }
        10..=12 => {
            if reader.read_bool(spec, "b_frame_rate_fraction")? {
                if reader.read_bool(spec, "b_frame_rate_fraction_is_4")? {
                    2
                } else {
                    1
                }
            } else {
                0
            }
        }
        _ => 0,
    };
    Ok(value)
}

fn parse_ac4_emdf_info(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
) -> Result<ParsedAc4EmdfInfo, MuxError> {
    let mut version = reader.read_bits(2, spec, "emdf_version")?;
    if version == 3 {
        version += read_ac4_variable_bits(reader, spec, "emdf_version extension", 2)?;
    }
    let mut key_id = reader.read_bits(3, spec, "key_id")?;
    if key_id == 7 {
        key_id += read_ac4_variable_bits(reader, spec, "key_id extension", 3)?;
    }
    if reader.read_bool(spec, "b_emdf_payloads_substream_info")? {
        let substream_index = reader.read_bits(2, spec, "emdf substream_index")?;
        if substream_index == 3 {
            let _ = read_ac4_variable_bits(reader, spec, "emdf substream_index extension", 2)?;
        }
    }
    skip_ac4_emdf_protection(reader, spec)?;
    Ok(ParsedAc4EmdfInfo {
        version: u8::try_from(version).unwrap_or(u8::MAX),
        key_id: u16::try_from(key_id).unwrap_or(u16::MAX),
    })
}

fn skip_ac4_emdf_protection(reader: &mut Ac4BitCursor<'_>, spec: &str) -> Result<(), MuxError> {
    for label in ["primary", "secondary"] {
        let length = reader.read_bits(2, spec, &format!("protection_length_{label}"))?;
        let bytes = match length {
            1 => 1,
            2 => 4,
            3 => 16,
            _ => 0,
        };
        for _ in 0..bytes {
            let _ = reader.read_bits(8, spec, &format!("protection_bits_{label}"))?;
        }
    }
    Ok(())
}

fn skip_ac4_presentation_substream_info(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
) -> Result<(), MuxError> {
    let _ = reader.read_bool(spec, "b_alternative")?;
    let _ = reader.read_bool(spec, "b_pres_ndot")?;
    let substream_index = reader.read_bits(2, spec, "presentation_substream_index")?;
    if substream_index == 3 {
        let _ = read_ac4_variable_bits(reader, spec, "presentation_substream_index extension", 2)?;
    }
    Ok(())
}

fn read_ac4_variable_bits(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
    context: &str,
    bit_width: usize,
) -> Result<u32, MuxError> {
    let mut value = 0_u32;
    loop {
        value = value
            .checked_add(reader.read_bits(bit_width, spec, context)?)
            .ok_or(MuxError::LayoutOverflow("AC-4 variable-width value"))?;
        let more = reader.read_bool(spec, &format!("{context} continuation"))?;
        if !more {
            return Ok(value);
        }
        value = (value << bit_width)
            .checked_add(1_u32 << bit_width)
            .ok_or(MuxError::LayoutOverflow("AC-4 variable-width continuation"))?;
    }
}

fn read_ac4_variable_bits_prefixed(
    reader: &mut Ac4BitCursor<'_>,
    spec: &str,
    context: &str,
    bit_width: usize,
    extension_trigger: Option<u32>,
) -> Result<u32, MuxError> {
    let base = reader.read_bits(bit_width, spec, context)?;
    if extension_trigger.is_some_and(|trigger| trigger == base) {
        Ok(base + read_ac4_variable_bits(reader, spec, context, bit_width)?)
    } else {
        Ok(base)
    }
}

fn ac4_timing_from_frame_rate_index(
    fs_index: u8,
    frame_rate_index: usize,
    spec: &str,
) -> Result<(u32, u32), MuxError> {
    let (durations, timescales) = if fs_index == 0 {
        (
            &AC4_SAMPLE_DELTA_TABLE_441[..],
            &AC4_MEDIA_TIMESCALE_441[..],
        )
    } else {
        (&AC4_SAMPLE_DELTA_TABLE_48[..], &AC4_MEDIA_TIMESCALE_48[..])
    };
    let sample_duration =
        *durations
            .get(frame_rate_index)
            .ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported AC-4 frame_rate_index {frame_rate_index}"),
            })?;
    let media_time_scale =
        *timescales
            .get(frame_rate_index)
            .ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported AC-4 frame_rate_index {frame_rate_index}"),
            })?;
    if sample_duration == 0 || media_time_scale == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "AC-4 frame_rate_index {frame_rate_index} is reserved for this sampling frequency"
            ),
        });
    }
    Ok((sample_duration, media_time_scale))
}

fn ac4_channel_count_from_mask(mask: u32) -> Result<u16, MuxError> {
    if mask == 0 {
        return Ok(0);
    }
    let mut count = 0_u16;
    for bit in [0_u8, 2, 3, 4, 5, 7, 8, 13, 16, 17, 18] {
        if mask & (1_u32 << bit) != 0 {
            count = count
                .checked_add(2)
                .ok_or(MuxError::LayoutOverflow("AC-4 channel count"))?;
        }
    }
    for bit in [1_u8, 6, 9, 10, 11, 12, 14, 15] {
        if mask & (1_u32 << bit) != 0 {
            count = count
                .checked_add(1)
                .ok_or(MuxError::LayoutOverflow("AC-4 channel count"))?;
        }
    }
    if (mask & 1) != 0 && (mask & 2) != 0 && count == 3 {
        count = 2;
    }
    Ok(count)
}

fn ac4_channel_mode_from_mask(mask: u32, fallback: u8) -> u8 {
    AC4_CHANNEL_MASK_BY_MODE
        .iter()
        .take(16)
        .position(|candidate| *candidate == mask)
        .map(|index| u8::try_from(index).unwrap_or(fallback))
        .unwrap_or(fallback)
}

fn serialize_ac4_dac4(parsed: &ParsedAc4Stream, spec: &str) -> Result<Vec<u8>, MuxError> {
    let mut writer = BitWriter::new(Vec::new());
    write_ac4_bits(&mut writer, 1, 3)?;
    write_ac4_bits(&mut writer, u32::from(parsed.bitstream_version), 7)?;
    write_ac4_bits(&mut writer, u32::from(parsed.fs_index), 1)?;
    write_ac4_bits(&mut writer, u32::from(parsed.frame_rate_index), 4)?;
    write_ac4_bits(
        &mut writer,
        u32::try_from(parsed.presentations.len()).map_err(|_| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "AC-4 presentation count does not fit in u32".to_string(),
            }
        })?,
        9,
    )?;
    write_ac4_bits(&mut writer, u32::from(parsed.has_program_id), 1)?;
    if parsed.has_program_id {
        write_ac4_bits(&mut writer, u32::from(parsed.short_program_id), 16)?;
        write_ac4_bits(&mut writer, u32::from(parsed.program_uuid.is_some()), 1)?;
        if let Some(program_uuid) = &parsed.program_uuid {
            for byte in program_uuid {
                write_ac4_bits(&mut writer, u32::from(*byte), 8)?;
            }
        }
    }
    write_ac4_bits(&mut writer, u32::from(parsed.bit_rate_mode), 2)?;
    write_ac4_bits(&mut writer, 0, 32)?;
    write_ac4_bits(&mut writer, u32::MAX, 32)?;
    align_ac4_writer(&mut writer)?;
    for presentation in &parsed.presentations {
        let body = serialize_ac4_presentation_body(presentation)?;
        write_ac4_bits(&mut writer, u32::from(presentation.presentation_version), 8)?;
        if body.len() < 255 {
            write_ac4_bits(&mut writer, u32::try_from(body.len()).unwrap(), 8)?;
        } else {
            write_ac4_bits(&mut writer, 255, 8)?;
            write_ac4_bits(
                &mut writer,
                u32::try_from(body.len() - 255)
                    .map_err(|_| MuxError::LayoutOverflow("AC-4 presentation body length"))?,
                16,
            )?;
        }
        for byte in body {
            write_ac4_bits(&mut writer, u32::from(byte), 8)?;
        }
    }

    writer.into_inner().map_err(MuxError::Io)
}

fn serialize_ac4_presentation_body(
    presentation: &ParsedAc4Presentation,
) -> Result<Vec<u8>, MuxError> {
    let mut writer = BitWriter::new(Vec::new());
    let group = presentation
        .group
        .as_ref()
        .ok_or(MuxError::LayoutOverflow("AC-4 presentation group"))?;
    write_ac4_bits(&mut writer, 0x1f, 5)?;
    write_ac4_bits(&mut writer, u32::from(presentation.mdcompat), 3)?;
    write_ac4_bits(&mut writer, u32::from(presentation.has_presentation_id), 1)?;
    if presentation.has_presentation_id {
        write_ac4_bits(&mut writer, u32::from(presentation.presentation_id), 5)?;
    }
    write_ac4_bits(
        &mut writer,
        u32::from(presentation.frame_rate_multiply_info),
        2,
    )?;
    write_ac4_bits(
        &mut writer,
        u32::from(presentation.frame_rate_fraction_info),
        2,
    )?;
    write_ac4_bits(&mut writer, u32::from(presentation.emdf_version), 5)?;
    write_ac4_bits(&mut writer, u32::from(presentation.key_id), 10)?;
    write_ac4_bits(&mut writer, 1, 1)?;
    write_ac4_bits(
        &mut writer,
        u32::from(presentation.presentation_channel_mode),
        5,
    )?;
    if (11..=14).contains(&presentation.presentation_channel_mode) {
        write_ac4_bits(
            &mut writer,
            u32::from(presentation.four_back_channels_present),
            1,
        )?;
        write_ac4_bits(&mut writer, u32::from(presentation.top_channel_pairs), 2)?;
    }
    write_ac4_bits(&mut writer, presentation.presentation_channel_mask, 24)?;
    write_ac4_bits(&mut writer, 0, 1)?;
    write_ac4_bits(
        &mut writer,
        u32::from(presentation.has_presentation_filter),
        1,
    )?;
    if presentation.has_presentation_filter {
        write_ac4_bits(&mut writer, u32::from(presentation.enable_presentation), 1)?;
        write_ac4_bits(&mut writer, 0, 8)?;
    }
    serialize_ac4_substream_group(&mut writer, group)?;
    write_ac4_bits(&mut writer, u32::from(presentation.pre_virtualized), 1)?;
    write_ac4_bits(
        &mut writer,
        u32::from(!presentation.add_emdf_substreams.is_empty()),
        1,
    )?;
    if !presentation.add_emdf_substreams.is_empty() {
        write_ac4_bits(
            &mut writer,
            u32::try_from(presentation.add_emdf_substreams.len()).unwrap(),
            7,
        )?;
        for emdf in &presentation.add_emdf_substreams {
            write_ac4_bits(&mut writer, u32::from(emdf.version), 5)?;
            write_ac4_bits(&mut writer, u32::from(emdf.key_id), 10)?;
        }
    }
    write_ac4_bits(&mut writer, 0, 1)?;
    write_ac4_bits(&mut writer, 0, 1)?;
    align_ac4_writer(&mut writer)?;
    write_ac4_bits(&mut writer, 1, 1)?;
    write_ac4_bits(
        &mut writer,
        u32::from(presentation.pre_virtualized || presentation.top_channel_pairs != 0),
        1,
    )?;
    write_ac4_bits(&mut writer, 0, 4)?;
    write_ac4_bits(&mut writer, 0, 1)?;
    write_ac4_bits(&mut writer, 0, 1)?;
    writer.into_inner().map_err(MuxError::Io)
}

fn serialize_ac4_substream_group(
    writer: &mut BitWriter<Vec<u8>>,
    group: &ParsedAc4SubstreamGroup,
) -> Result<(), MuxError> {
    write_ac4_bits(writer, u32::from(group.substreams_present), 1)?;
    write_ac4_bits(writer, u32::from(group.high_sample_rate_extension), 1)?;
    write_ac4_bits(writer, 1, 1)?;
    write_ac4_bits(
        writer,
        u32::try_from(group.substreams.len())
            .map_err(|_| MuxError::LayoutOverflow("AC-4 substream count"))?,
        8,
    )?;
    for substream in &group.substreams {
        serialize_ac4_substream(writer, substream)?;
    }
    if let Some(content) = &group.content_type {
        write_ac4_bits(writer, 1, 1)?;
        write_ac4_bits(writer, u32::from(content.classifier), 3)?;
        write_ac4_bits(writer, u32::from(!content.language_tag_bytes.is_empty()), 1)?;
        if !content.language_tag_bytes.is_empty() {
            write_ac4_bits(
                writer,
                u32::try_from(content.language_tag_bytes.len()).unwrap(),
                6,
            )?;
            for byte in &content.language_tag_bytes {
                write_ac4_bits(writer, u32::from(*byte), 8)?;
            }
        }
    } else {
        write_ac4_bits(writer, 0, 1)?;
    }
    Ok(())
}

fn serialize_ac4_substream(
    writer: &mut BitWriter<Vec<u8>>,
    substream: &ParsedAc4Substream,
) -> Result<(), MuxError> {
    write_ac4_bits(writer, u32::from(substream.dsi_sf_multiplier), 2)?;
    write_ac4_bits(
        writer,
        u32::from(substream.has_substream_bitrate_indicator),
        1,
    )?;
    if substream.has_substream_bitrate_indicator {
        write_ac4_bits(writer, u32::from(substream.substream_bitrate_indicator), 5)?;
    }
    write_ac4_bits(writer, substream.channel_mask, 24)?;
    Ok(())
}

fn write_ac4_bits(
    writer: &mut BitWriter<Vec<u8>>,
    value: u32,
    width: usize,
) -> Result<(), MuxError> {
    let bytes = value.to_be_bytes();
    writer.write_bits(&bytes, width).map_err(MuxError::Io)
}

fn align_ac4_writer(writer: &mut BitWriter<Vec<u8>>) -> Result<(), MuxError> {
    while !writer.is_aligned() {
        writer.write_bit(false).map_err(MuxError::Io)?;
    }
    Ok(())
}

fn build_ac4_sample_entry_box(
    _channel_count: u16,
    sample_rate: u32,
    media_time_scale: u32,
    samples: &[StagedSample],
    dac4_data: &[u8],
) -> Result<Vec<u8>, MuxError> {
    let mut sample_entry = AudioSampleEntry::default();
    sample_entry.set_box_type(FourCc::from_bytes(*b"ac-4"));
    sample_entry.sample_entry = SampleEntry {
        box_type: FourCc::from_bytes(*b"ac-4"),
        data_reference_index: 1,
    };
    // AC-4 sample entries keep the authored channel topology in `dac4`; the
    // sample-entry header itself stays on the standard two-channel default.
    sample_entry.channel_count = 2;
    sample_entry.sample_size = 16;
    sample_entry.sample_rate = sample_rate << 16;

    let dac4 = super::super::mp4::encode_typed_box(
        &Dac4 {
            data: dac4_data.to_vec(),
        },
        &[],
    )?;
    let btrt =
        super::super::mp4::encode_typed_box(&build_ac4_btrt(samples, media_time_scale)?, &[])?;
    let mut children = Vec::with_capacity(dac4.len() + btrt.len());
    children.extend_from_slice(&dac4);
    children.extend_from_slice(&btrt);

    super::super::mp4::encode_typed_box(&sample_entry, &children)
}

fn build_ac4_btrt(samples: &[StagedSample], media_time_scale: u32) -> Result<Btrt, MuxError> {
    if samples.is_empty() || media_time_scale == 0 {
        return Ok(Btrt::default());
    }

    let mut buffer_size_db = 0_u32;
    let mut total_payload_bytes = 0_u64;
    let mut total_duration = 0_u64;
    for sample in samples {
        buffer_size_db = buffer_size_db.max(sample.data_size);
        total_payload_bytes = total_payload_bytes
            .checked_add(u64::from(sample.data_size))
            .ok_or(MuxError::LayoutOverflow("AC-4 total payload bytes"))?;
        total_duration = total_duration
            .checked_add(u64::from(sample.duration))
            .ok_or(MuxError::LayoutOverflow("AC-4 total duration"))?;
    }
    if total_duration == 0 {
        return Ok(Btrt::default());
    }

    let avg_bitrate = total_payload_bytes
        .checked_mul(8)
        .and_then(|bits| bits.checked_mul(u64::from(media_time_scale)))
        .ok_or(MuxError::LayoutOverflow("AC-4 average bitrate"))?
        / total_duration;
    let avg_bitrate = avg_bitrate & !7;

    Ok(Btrt {
        buffer_size_db,
        max_bitrate: u32::try_from(avg_bitrate)
            .map_err(|_| MuxError::LayoutOverflow("AC-4 maximum bitrate"))?,
        avg_bitrate: u32::try_from(avg_bitrate)
            .map_err(|_| MuxError::LayoutOverflow("AC-4 average bitrate"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_ac4_frame_header, parse_ac4_stream, serialize_ac4_dac4};

    const TEST_AC4_FRAME_HEX: &str = concat!(
        "ac41ffff00015cbfcee7984004a7012e2c20304d805c8458d0a0c06013b58354cb613912144b0232be85",
        "4b4800025c71fd3eaacd4a86324c1498a4bd6021dfa8b016b42115ba6b684770fd34e31a264f66703f14",
        "090541b22397fd7c837ef68f05211a79862d48d5c46d87857bedd9f69bbdb26682bcf49b036bccb100ab84",
        "4568e5a54fc32e4302233b9144cb4bd0ca86c64794cf4e7eca5191e8d8c48ccef686868ae56b5f5e416097",
        "07ad77775b5bfa5b61bff5f32ed963f6caee5ac968a743e60e578f5a4892c90101e18a7246f88c51161028",
        "870564d088f0799f9d11701ecd86f202692868b8649e14e10f0304bc20f4b47d06b3ba58fcd3c950fecd1a",
        "137dd410334797b62d82ed35073d1131e2f10a02ce51c269e1248e423c299956b2c53ad26a6c5ddcb1d7cd",
        "c999265bb1954775fbc72cd8cf322a47091169f3fff19ff6aca15a5894fe68d2fa20c1f55000000000f010",
        "4a51e02094a880a3c134b5ff00",
    );

    fn decode_test_hex_bytes(hex: &str) -> Vec<u8> {
        assert!(hex.len().is_multiple_of(2));
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        for index in (0..hex.len()).step_by(2) {
            bytes.push(u8::from_str_radix(&hex[index..index + 2], 16).unwrap());
        }
        bytes
    }

    #[test]
    fn retained_ac4_frame_parses_with_expected_channel_count() {
        let mut frame = decode_test_hex_bytes(TEST_AC4_FRAME_HEX);
        frame.extend_from_slice(&[0, 0]);
        let header = parse_ac4_frame_header(
            &frame[..7].try_into().unwrap(),
            u64::try_from(frame.len()).unwrap(),
            0,
            "test.ac4",
        )
        .unwrap();
        let payload = &frame[usize::try_from(header.header_size).unwrap()
            ..usize::try_from(header.header_size + header.frame_payload_size).unwrap()];
        let parsed = parse_ac4_stream(payload, "test.ac4").unwrap();
        assert_eq!(parsed.presentations.len(), 1);
        assert_eq!(
            serialize_ac4_dac4(&parsed, "test.ac4").unwrap(),
            decode_test_hex_bytes("20a601400000001fffffffE0010ff88000004200000250100000030080"),
            "{parsed:#?}"
        );
    }

}
