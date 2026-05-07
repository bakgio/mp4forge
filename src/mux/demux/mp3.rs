use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::FourCc;
use crate::boxes::AnyTypeBox;
use crate::boxes::iso14496_12::{AudioSampleEntry, Btrt, SampleEntry};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{SegmentedMuxSourceSegment, StagedSample, read_exact_at_sync};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;

pub(in crate::mux) struct ParsedMp3Track {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

pub(in crate::mux) struct ParsedMp3FrameHeader {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) channel_count: u16,
    pub(in crate::mux) sample_duration: u32,
    pub(in crate::mux) frame_length: u32,
}

pub(in crate::mux) fn scan_mp3_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedMp3Track, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u32, u16, u32)>;
    while offset < file_size {
        if let Some(next_offset) = skip_id3v2_tag_sync(&mut file, file_size, offset, spec)? {
            offset = next_offset;
            continue;
        }
        if skip_trailing_id3v1_tag_offset(file_size, offset, &mut file)? {
            break;
        }
        if file_size - offset < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated MP3 frame header".to_string(),
            });
        }
        let mut header = [0_u8; 4];
        read_exact_at_sync(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated MP3 frame header",
        )?;
        if header[0] != 0xFF || header[1] & 0xE0 != 0xE0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing MP3 sync word at byte offset {offset}"),
            });
        }
        let parsed = parse_mp3_frame_header(&header, offset, spec)?;
        let frame_length = usize::try_from(parsed.frame_length)
            .map_err(|_| MuxError::LayoutOverflow("MP3 frame length"))?;
        if offset
            .checked_add(u64::try_from(frame_length).unwrap_or(u64::MAX))
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated MP3 frame at byte offset {offset}"),
            });
        }
        let descriptor = (
            parsed.sample_rate,
            parsed.channel_count,
            parsed.sample_duration,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "MP3 frames changed sample rate or channel layout mid-stream"
                        .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_length)
                .map_err(|_| MuxError::LayoutOverflow("MP3 frame size"))?,
            duration: parsed.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(
                u64::try_from(frame_length)
                    .map_err(|_| MuxError::LayoutOverflow("MP3 frame length"))?,
            )
            .ok_or(MuxError::LayoutOverflow("MP3 frame offset"))?;
    }

    let (sample_rate, channel_count, _sample_duration) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MP3 input contained no MPEG audio frames".to_string(),
        })?;
    Ok(ParsedMp3Track {
        sample_rate,
        sample_entry_box: build_mp3_sample_entry_box(
            sample_rate,
            channel_count,
            samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
        )?,
        samples,
    })
}

pub(in crate::mux) fn scan_mp3_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedMp3Track, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u32, u16, u32)>;
    while offset < total_size {
        if total_size - offset < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated MP3 frame header".to_string(),
            });
        }
        let mut header = [0_u8; 4];
        read_segmented_bytes_sync(
            file,
            segments,
            total_size,
            offset,
            &mut header,
            spec,
            "truncated MP3 frame header",
        )?;
        if header[0] != 0xFF || header[1] & 0xE0 != 0xE0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing MP3 sync word at logical byte offset {offset}"),
            });
        }
        let parsed = parse_mp3_frame_header(&header, offset, spec)?;
        let frame_length = usize::try_from(parsed.frame_length)
            .map_err(|_| MuxError::LayoutOverflow("MP3 frame length"))?;
        if offset
            .checked_add(u64::try_from(frame_length).unwrap_or(u64::MAX))
            .is_none_or(|end| end > total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated MP3 frame at logical byte offset {offset}"),
            });
        }
        let descriptor = (
            parsed.sample_rate,
            parsed.channel_count,
            parsed.sample_duration,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "MP3 frames changed sample rate or channel layout mid-stream"
                        .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_length)
                .map_err(|_| MuxError::LayoutOverflow("MP3 frame size"))?,
            duration: parsed.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(
                u64::try_from(frame_length)
                    .map_err(|_| MuxError::LayoutOverflow("MP3 frame length"))?,
            )
            .ok_or(MuxError::LayoutOverflow("MP3 frame offset"))?;
    }

    let (sample_rate, channel_count, _sample_duration) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MP3 input contained no MPEG audio frames".to_string(),
        })?;
    Ok(ParsedMp3Track {
        sample_rate,
        sample_entry_box: build_mp3_sample_entry_box(
            sample_rate,
            channel_count,
            samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
        )?,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_mp3_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedMp3Track, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u32, u16, u32)>;
    while offset < file_size {
        if let Some(next_offset) = skip_id3v2_tag_async(&mut file, file_size, offset, spec).await? {
            offset = next_offset;
            continue;
        }
        if skip_trailing_id3v1_tag_offset_async(file_size, offset, &mut file).await? {
            break;
        }
        if file_size - offset < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated MP3 frame header".to_string(),
            });
        }
        let mut header = [0_u8; 4];
        read_exact_at_async(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated MP3 frame header",
        )
        .await?;
        if header[0] != 0xFF || header[1] & 0xE0 != 0xE0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing MP3 sync word at byte offset {offset}"),
            });
        }
        let parsed = parse_mp3_frame_header(&header, offset, spec)?;
        let frame_length = usize::try_from(parsed.frame_length)
            .map_err(|_| MuxError::LayoutOverflow("MP3 frame length"))?;
        if offset
            .checked_add(u64::try_from(frame_length).unwrap_or(u64::MAX))
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated MP3 frame at byte offset {offset}"),
            });
        }
        let descriptor = (
            parsed.sample_rate,
            parsed.channel_count,
            parsed.sample_duration,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "MP3 frames changed sample rate or channel layout mid-stream"
                        .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_length)
                .map_err(|_| MuxError::LayoutOverflow("MP3 frame size"))?,
            duration: parsed.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(
                u64::try_from(frame_length)
                    .map_err(|_| MuxError::LayoutOverflow("MP3 frame length"))?,
            )
            .ok_or(MuxError::LayoutOverflow("MP3 frame offset"))?;
    }

    let (sample_rate, channel_count, _sample_duration) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MP3 input contained no MPEG audio frames".to_string(),
        })?;
    Ok(ParsedMp3Track {
        sample_rate,
        sample_entry_box: build_mp3_sample_entry_box(
            sample_rate,
            channel_count,
            samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
        )?,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_mp3_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedMp3Track, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u32, u16, u32)>;
    while offset < total_size {
        if total_size - offset < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated MP3 frame header".to_string(),
            });
        }
        let mut header = [0_u8; 4];
        read_segmented_bytes_async(
            file,
            segments,
            total_size,
            offset,
            &mut header,
            spec,
            "truncated MP3 frame header",
        )
        .await?;
        if header[0] != 0xFF || header[1] & 0xE0 != 0xE0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing MP3 sync word at logical byte offset {offset}"),
            });
        }
        let parsed = parse_mp3_frame_header(&header, offset, spec)?;
        let frame_length = usize::try_from(parsed.frame_length)
            .map_err(|_| MuxError::LayoutOverflow("MP3 frame length"))?;
        if offset
            .checked_add(u64::try_from(frame_length).unwrap_or(u64::MAX))
            .is_none_or(|end| end > total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated MP3 frame at logical byte offset {offset}"),
            });
        }
        let descriptor = (
            parsed.sample_rate,
            parsed.channel_count,
            parsed.sample_duration,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "MP3 frames changed sample rate or channel layout mid-stream"
                        .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_length)
                .map_err(|_| MuxError::LayoutOverflow("MP3 frame size"))?,
            duration: parsed.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(
                u64::try_from(frame_length)
                    .map_err(|_| MuxError::LayoutOverflow("MP3 frame length"))?,
            )
            .ok_or(MuxError::LayoutOverflow("MP3 frame offset"))?;
    }

    let (sample_rate, channel_count, _sample_duration) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MP3 input contained no MPEG audio frames".to_string(),
        })?;
    Ok(ParsedMp3Track {
        sample_rate,
        sample_entry_box: build_mp3_sample_entry_box(
            sample_rate,
            channel_count,
            samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
        )?,
        samples,
    })
}

pub(in crate::mux) fn build_mp3_sample_entry_box<I>(
    sample_rate: u32,
    channel_count: u16,
    samples: I,
) -> Result<Vec<u8>, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    let mut sample_entry = AudioSampleEntry::default();
    sample_entry.set_box_type(FourCc::from_bytes(*b".mp3"));
    sample_entry.sample_entry = SampleEntry {
        box_type: FourCc::from_bytes(*b".mp3"),
        data_reference_index: 1,
    };
    sample_entry.channel_count = channel_count;
    sample_entry.sample_size = 16;
    sample_entry.sample_rate = sample_rate << 16;

    let btrt = build_mp3_btrt(samples, sample_rate)?;
    let children = super::super::mp4::encode_typed_box(&btrt, &[])?;
    super::super::mp4::encode_typed_box(&sample_entry, &children)
}

fn build_mp3_btrt<I>(samples: I, sample_rate: u32) -> Result<Btrt, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    if sample_rate == 0 {
        return Ok(Btrt::default());
    }

    let mut saw_sample = false;
    let mut buffer_size_db = 0_u32;
    let mut total_payload_bytes = 0_u64;
    let mut total_duration = 0_u64;
    let mut max_window_payload_bytes = 0_u64;
    let mut current_window_payload_bytes = 0_u64;
    let mut window_start_decode_time = 0_u64;
    let mut sample_decode_time = 0_u64;
    for (data_size, duration) in samples {
        saw_sample = true;
        buffer_size_db = buffer_size_db.max(data_size);
        total_payload_bytes = total_payload_bytes
            .checked_add(u64::from(data_size))
            .ok_or(MuxError::LayoutOverflow("MP3 total payload bytes"))?;
        total_duration = total_duration
            .checked_add(u64::from(duration))
            .ok_or(MuxError::LayoutOverflow("MP3 total duration"))?;
        current_window_payload_bytes = current_window_payload_bytes
            .checked_add(u64::from(data_size))
            .ok_or(MuxError::LayoutOverflow("MP3 bitrate window payload"))?;
        if sample_decode_time > window_start_decode_time.saturating_add(u64::from(sample_rate)) {
            max_window_payload_bytes = max_window_payload_bytes.max(current_window_payload_bytes);
            window_start_decode_time = sample_decode_time;
            current_window_payload_bytes = 0;
        }
        sample_decode_time = sample_decode_time
            .checked_add(u64::from(duration))
            .ok_or(MuxError::LayoutOverflow("MP3 decode time"))?;
    }
    if !saw_sample {
        return Ok(Btrt::default());
    }
    if total_duration == 0 {
        return Ok(Btrt::default());
    }

    let avg_bitrate = total_payload_bytes
        .checked_mul(8)
        .and_then(|bits| bits.checked_mul(u64::from(sample_rate)))
        .ok_or(MuxError::LayoutOverflow("MP3 average bitrate"))?
        / total_duration;
    let avg_bitrate = avg_bitrate & !7;

    let max_bitrate = if max_window_payload_bytes == 0 {
        avg_bitrate
    } else {
        max_window_payload_bytes
            .checked_mul(8)
            .ok_or(MuxError::LayoutOverflow("MP3 maximum bitrate"))?
    };

    Ok(Btrt {
        buffer_size_db,
        max_bitrate: u32::try_from(max_bitrate)
            .map_err(|_| MuxError::LayoutOverflow("MP3 maximum bitrate"))?,
        avg_bitrate: u32::try_from(avg_bitrate)
            .map_err(|_| MuxError::LayoutOverflow("MP3 average bitrate"))?,
    })
}

pub(in crate::mux) fn parse_mp3_frame_header(
    header: &[u8; 4],
    offset: u64,
    spec: &str,
) -> Result<ParsedMp3FrameHeader, MuxError> {
    if header[0] != 0xFF || header[1] & 0xE0 != 0xE0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("missing MP3 sync word at byte offset {offset}"),
        });
    }
    let version_id = (header[1] >> 3) & 0x03;
    if version_id == 0x01 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("reserved MP3 MPEG version at byte offset {offset}"),
        });
    }
    let layer = (header[1] >> 1) & 0x03;
    if layer != 0x01 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "the current raw MP3 mux importer only supports MPEG Layer III frames"
                .to_string(),
        });
    }
    let bitrate_index = (header[2] >> 4) & 0x0F;
    if bitrate_index == 0 || bitrate_index == 0x0F {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported MP3 bitrate index {bitrate_index}"),
        });
    }
    let sample_rate_index = (header[2] >> 2) & 0x03;
    let sample_rate = mp3_sample_rate(version_id, sample_rate_index).ok_or_else(|| {
        MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported MP3 sample-rate index {sample_rate_index}"),
        }
    })?;
    let bitrate_bps = mp3_bitrate_bps(version_id, bitrate_index).ok_or_else(|| {
        MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported MP3 bitrate index {bitrate_index}"),
        }
    })?;
    let padding = u32::from((header[2] >> 1) & 0x01);
    let channel_count = if (header[3] >> 6) == 0x03 { 1 } else { 2 };
    let sample_duration = if version_id == 0x03 { 1152 } else { 576 };
    let frame_length = if version_id == 0x03 {
        ((144_u32 * bitrate_bps) / sample_rate).saturating_add(padding)
    } else {
        ((72_u32 * bitrate_bps) / sample_rate).saturating_add(padding)
    };
    if frame_length < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MP3 frame length underflowed the header size".to_string(),
        });
    }
    Ok(ParsedMp3FrameHeader {
        sample_rate,
        channel_count,
        sample_duration,
        frame_length,
    })
}

fn skip_id3v2_tag(header: &[u8], spec: &str) -> Result<Option<usize>, MuxError> {
    if header.len() < 10 {
        return Ok(None);
    }
    if &header[..3] != b"ID3" {
        return Ok(None);
    }
    if header[6..10].iter().any(|byte| byte & 0x80 != 0) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "ID3v2 tag uses a non-synchsafe size field".to_string(),
        });
    }
    let tag_size = (usize::from(header[6]) << 21)
        | (usize::from(header[7]) << 14)
        | (usize::from(header[8]) << 7)
        | usize::from(header[9]);
    let footer_size = if header[5] & 0x10 != 0 { 10 } else { 0 };
    let total_size = 10_usize
        .checked_add(tag_size)
        .and_then(|size| size.checked_add(footer_size))
        .ok_or(MuxError::LayoutOverflow("ID3 tag size"))?;
    Ok(Some(total_size))
}

fn skip_id3v2_tag_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<Option<u64>, MuxError> {
    if file_size - offset < 10 {
        return Ok(None);
    }
    let mut header = [0_u8; 10];
    read_exact_at_sync(
        file,
        offset,
        &mut header,
        spec,
        "truncated ID3v2 tag ahead of MPEG audio frames",
    )?;
    skip_id3v2_tag(&header, spec)?
        .map(|size| {
            offset
                .checked_add(
                    u64::try_from(size).map_err(|_| MuxError::LayoutOverflow("ID3 tag size"))?,
                )
                .ok_or(MuxError::LayoutOverflow("ID3 tag offset"))
        })
        .transpose()
}

#[cfg(feature = "async")]
async fn skip_id3v2_tag_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<Option<u64>, MuxError> {
    if file_size - offset < 10 {
        return Ok(None);
    }
    let mut header = [0_u8; 10];
    read_exact_at_async(
        file,
        offset,
        &mut header,
        spec,
        "truncated ID3v2 tag ahead of MPEG audio frames",
    )
    .await?;
    skip_id3v2_tag(&header, spec)?
        .map(|size| {
            offset
                .checked_add(
                    u64::try_from(size).map_err(|_| MuxError::LayoutOverflow("ID3 tag size"))?,
                )
                .ok_or(MuxError::LayoutOverflow("ID3 tag offset"))
        })
        .transpose()
}

fn skip_trailing_id3v1_tag(header: &[u8]) -> bool {
    header.len() == 128 && &header[..3] == b"TAG"
}

fn skip_trailing_id3v1_tag_offset(
    file_size: u64,
    offset: u64,
    file: &mut File,
) -> Result<bool, MuxError> {
    if offset + 128 != file_size {
        return Ok(false);
    }
    let mut tag = [0_u8; 128];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut tag)?;
    Ok(skip_trailing_id3v1_tag(&tag))
}

#[cfg(feature = "async")]
async fn skip_trailing_id3v1_tag_offset_async(
    file_size: u64,
    offset: u64,
    file: &mut TokioFile,
) -> Result<bool, MuxError> {
    if offset + 128 != file_size {
        return Ok(false);
    }
    let mut tag = [0_u8; 128];
    file.seek(SeekFrom::Start(offset)).await?;
    file.read_exact(&mut tag).await?;
    Ok(skip_trailing_id3v1_tag(&tag))
}

const fn mp3_sample_rate(version_id: u8, sample_rate_index: u8) -> Option<u32> {
    let base = match sample_rate_index {
        0 => 44_100,
        1 => 48_000,
        2 => 32_000,
        _ => return None,
    };
    match version_id {
        0x03 => Some(base),
        0x02 => Some(base / 2),
        0x00 => Some(base / 4),
        _ => None,
    }
}

const fn mp3_bitrate_bps(version_id: u8, bitrate_index: u8) -> Option<u32> {
    let kbps = match version_id {
        0x03 => match bitrate_index {
            1 => 32,
            2 => 40,
            3 => 48,
            4 => 56,
            5 => 64,
            6 => 80,
            7 => 96,
            8 => 112,
            9 => 128,
            10 => 160,
            11 => 192,
            12 => 224,
            13 => 256,
            14 => 320,
            _ => return None,
        },
        0x02 | 0x00 => match bitrate_index {
            1 => 8,
            2 => 16,
            3 => 24,
            4 => 32,
            5 => 40,
            6 => 48,
            7 => 56,
            8 => 64,
            9 => 80,
            10 => 96,
            11 => 112,
            12 => 128,
            13 => 144,
            14 => 160,
            _ => return None,
        },
        _ => return None,
    };
    Some(kbps * 1_000)
}
