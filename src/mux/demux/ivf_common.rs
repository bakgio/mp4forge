use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::FourCc;

use super::super::import::StagedSample;
use super::super::{MuxError, MuxRawCodec};

const AV01_ENTRY: FourCc = FourCc::from_bytes(*b"av01");
const VP08_ENTRY: FourCc = FourCc::from_bytes(*b"vp08");
const VP09_ENTRY: FourCc = FourCc::from_bytes(*b"vp09");
const VP10_ENTRY: FourCc = FourCc::from_bytes(*b"vp10");

pub(in crate::mux) struct ParsedIvfTrack {
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) timescale: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

#[derive(Clone, Copy)]
struct ParsedIvfHeader {
    width: u16,
    height: u16,
    timescale: u32,
    timestamp_scale: u32,
}

#[derive(Clone, Copy)]
pub(super) struct IndexedIvfSample {
    pub(super) data_offset: u64,
    pub(super) data_size: u32,
    timestamp: u64,
}

pub(super) struct IndexedIvfTrack {
    pub(super) width: u16,
    pub(super) height: u16,
    pub(super) timescale: u32,
    pub(super) sample_entry_type: FourCc,
    pub(super) first_sample_span: IndexedIvfSample,
    pub(super) samples: Vec<StagedSample>,
}

pub(super) fn read_indexed_sample_sync(
    path: &Path,
    sample: IndexedIvfSample,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(sample.data_offset))?;
    let mut bytes = vec![
        0_u8;
        usize::try_from(sample.data_size)
            .map_err(|_| MuxError::LayoutOverflow("IVF sample size"))?
    ];
    file.read_exact(&mut bytes)
        .map_err(|error| map_truncated_ivf_error(error, spec, truncated_message))?;
    Ok(bytes)
}

#[cfg(feature = "async")]
pub(super) async fn read_indexed_sample_async(
    path: &Path,
    sample: IndexedIvfSample,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let mut file = TokioFile::open(path).await?;
    file.seek(SeekFrom::Start(sample.data_offset)).await?;
    let mut bytes = vec![
        0_u8;
        usize::try_from(sample.data_size)
            .map_err(|_| MuxError::LayoutOverflow("IVF sample size"))?
    ];
    file.read_exact(&mut bytes)
        .await
        .map_err(|error| map_truncated_ivf_error(error, spec, truncated_message))?;
    Ok(bytes)
}

pub(super) fn scan_ivf_video_file_sync(
    path: &Path,
    codec: MuxRawCodec,
    spec: &str,
) -> Result<IndexedIvfTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    if file_size < 32 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IVF input is truncated before the 32-byte file header".to_string(),
        });
    }

    let mut header = [0_u8; 32];
    file.read_exact(&mut header).map_err(|error| {
        map_truncated_ivf_error(
            error,
            spec,
            "IVF input is truncated before the 32-byte file header",
        )
    })?;
    let parsed_header = parse_ivf_video_header(&header, codec, spec)?;

    let mut offset = 32_u64;
    let mut indexed_samples = Vec::new();
    while offset < file_size {
        if file_size - offset < 12 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "IVF frame header at byte offset {offset} is truncated before 12 bytes"
                ),
            });
        }
        let mut frame_header = [0_u8; 12];
        file.read_exact(&mut frame_header).map_err(|error| {
            map_truncated_ivf_error(error, spec, "IVF input ended while reading a frame header")
        })?;
        let frame_size = u32::from_le_bytes(frame_header[..4].try_into().unwrap());
        let timestamp = u64::from_le_bytes(frame_header[4..12].try_into().unwrap());
        let data_offset = offset + 12;
        let frame_end = data_offset
            .checked_add(u64::from(frame_size))
            .ok_or(MuxError::LayoutOverflow("IVF frame range"))?;
        if frame_end > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("IVF frame at byte offset {offset} overruns the input length"),
            });
        }
        indexed_samples.push(IndexedIvfSample {
            data_offset,
            data_size: frame_size,
            timestamp,
        });
        file.seek(SeekFrom::Current(i64::from(frame_size)))?;
        offset = frame_end;
    }
    if indexed_samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IVF input contained no frame payloads".to_string(),
        });
    }

    Ok(IndexedIvfTrack {
        width: parsed_header.width,
        height: parsed_header.height,
        timescale: parsed_header.timescale,
        sample_entry_type: ivf_video_sample_entry_type(codec),
        first_sample_span: indexed_samples[0],
        samples: build_ivf_staged_samples(&indexed_samples, parsed_header.timestamp_scale, spec)?,
    })
}

#[cfg(feature = "async")]
pub(super) async fn scan_ivf_video_file_async(
    path: &Path,
    codec: MuxRawCodec,
    spec: &str,
) -> Result<IndexedIvfTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    if file_size < 32 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IVF input is truncated before the 32-byte file header".to_string(),
        });
    }

    let mut header = [0_u8; 32];
    file.read_exact(&mut header).await.map_err(|error| {
        map_truncated_ivf_error(
            error,
            spec,
            "IVF input is truncated before the 32-byte file header",
        )
    })?;
    let parsed_header = parse_ivf_video_header(&header, codec, spec)?;

    let mut offset = 32_u64;
    let mut indexed_samples = Vec::new();
    while offset < file_size {
        if file_size - offset < 12 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "IVF frame header at byte offset {offset} is truncated before 12 bytes"
                ),
            });
        }
        let mut frame_header = [0_u8; 12];
        file.read_exact(&mut frame_header).await.map_err(|error| {
            map_truncated_ivf_error(error, spec, "IVF input ended while reading a frame header")
        })?;
        let frame_size = u32::from_le_bytes(frame_header[..4].try_into().unwrap());
        let timestamp = u64::from_le_bytes(frame_header[4..12].try_into().unwrap());
        let data_offset = offset + 12;
        let frame_end = data_offset
            .checked_add(u64::from(frame_size))
            .ok_or(MuxError::LayoutOverflow("IVF frame range"))?;
        if frame_end > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("IVF frame at byte offset {offset} overruns the input length"),
            });
        }
        indexed_samples.push(IndexedIvfSample {
            data_offset,
            data_size: frame_size,
            timestamp,
        });
        file.seek(SeekFrom::Current(i64::from(frame_size))).await?;
        offset = frame_end;
    }
    if indexed_samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IVF input contained no frame payloads".to_string(),
        });
    }

    Ok(IndexedIvfTrack {
        width: parsed_header.width,
        height: parsed_header.height,
        timescale: parsed_header.timescale,
        sample_entry_type: ivf_video_sample_entry_type(codec),
        first_sample_span: indexed_samples[0],
        samples: build_ivf_staged_samples(&indexed_samples, parsed_header.timestamp_scale, spec)?,
    })
}

fn parse_ivf_video_header(
    header: &[u8; 32],
    expected_codec: MuxRawCodec,
    spec: &str,
) -> Result<ParsedIvfHeader, MuxError> {
    if &header[..4] != b"DKIF" {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IVF input did not start with the `DKIF` signature".to_string(),
        });
    }
    let version = u16::from_le_bytes(header[4..6].try_into().unwrap());
    if version != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("IVF input used unsupported version {version}; expected 0"),
        });
    }
    let header_size = u16::from_le_bytes(header[6..8].try_into().unwrap());
    if header_size != 32 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "IVF input declared unsupported header size {header_size}; expected 32"
            ),
        });
    }
    let codec =
        ivf_codec_from_fourcc_bytes(header[8..12].try_into().unwrap()).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "IVF input used unsupported codec tag `{}`",
                    String::from_utf8_lossy(&header[8..12])
                ),
            }
        })?;
    if codec != expected_codec {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "IVF input codec `{}` does not match requested raw `{}` import",
                codec.prefix(),
                expected_codec.prefix()
            ),
        });
    }
    let width = u16::from_le_bytes(header[12..14].try_into().unwrap());
    let height = u16::from_le_bytes(header[14..16].try_into().unwrap());
    if width == 0 || height == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IVF input declared zero width or height".to_string(),
        });
    }
    let timescale = u32::from_le_bytes(header[16..20].try_into().unwrap());
    let timestamp_scale = u32::from_le_bytes(header[20..24].try_into().unwrap());
    if timescale == 0 || timestamp_scale == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IVF input declared a zero timebase field".to_string(),
        });
    }
    Ok(ParsedIvfHeader {
        width,
        height,
        timescale,
        timestamp_scale,
    })
}

fn ivf_codec_from_fourcc_bytes(fourcc: [u8; 4]) -> Option<MuxRawCodec> {
    match &fourcc {
        b"AV01" => Some(MuxRawCodec::Av1),
        b"VP80" => Some(MuxRawCodec::Vp8),
        b"VP90" => Some(MuxRawCodec::Vp9),
        b"VP10" => Some(MuxRawCodec::Vp10),
        _ => None,
    }
}

fn ivf_video_sample_entry_type(codec: MuxRawCodec) -> FourCc {
    match codec {
        MuxRawCodec::Av1 => AV01_ENTRY,
        MuxRawCodec::Vp8 => VP08_ENTRY,
        MuxRawCodec::Vp9 => VP09_ENTRY,
        MuxRawCodec::Vp10 => VP10_ENTRY,
        _ => unreachable!("only IVF-backed raw video codecs use this helper"),
    }
}

fn build_ivf_staged_samples(
    indexed_samples: &[IndexedIvfSample],
    timestamp_scale: u32,
    spec: &str,
) -> Result<Vec<StagedSample>, MuxError> {
    if indexed_samples.len() == 1 {
        let sample = indexed_samples[0];
        return Ok(vec![StagedSample {
            data_offset: sample.data_offset,
            data_size: sample.data_size,
            duration: 0,
            composition_time_offset: 0,
            is_sync_sample: true,
        }]);
    }

    let default_duration = timestamp_scale;
    let mut previous_duration = default_duration;
    let mut samples = Vec::with_capacity(indexed_samples.len());
    for (index, sample) in indexed_samples.iter().enumerate() {
        let duration = if let Some(next) = indexed_samples.get(index + 1) {
            if next.timestamp < sample.timestamp {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!(
                        "IVF frame timestamps must be monotonic, but frame {index} regressed from {} to {}",
                        sample.timestamp, next.timestamp
                    ),
                });
            }
            let delta = next.timestamp - sample.timestamp;
            if delta == 0 {
                previous_duration
            } else {
                let scaled = delta
                    .checked_mul(u64::from(timestamp_scale))
                    .ok_or(MuxError::LayoutOverflow("IVF sample duration"))?;
                let duration = u32::try_from(scaled)
                    .map_err(|_| MuxError::LayoutOverflow("IVF sample duration"))?;
                previous_duration = duration;
                duration
            }
        } else {
            previous_duration
        };
        samples.push(StagedSample {
            data_offset: sample.data_offset,
            data_size: sample.data_size,
            duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    Ok(samples)
}

fn map_truncated_ivf_error(
    error: std::io::Error,
    spec: &str,
    truncated_message: &'static str,
) -> MuxError {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: truncated_message.to_string(),
        }
    } else {
        MuxError::Io(error)
    }
}
