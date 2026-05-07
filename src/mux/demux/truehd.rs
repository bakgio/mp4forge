use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::dolby::Dmlp;
use crate::boxes::iso14496_12::{AudioSampleEntry, Btrt, SampleEntry};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{StagedSample, read_exact_at_sync};

const SAMPLE_ENTRY_MLPA: FourCc = FourCc::from_bytes(*b"mlpa");
const TRUEHD_SYNC: u32 = 0xF872_6FBA;
const TRUEHD_SIGNATURE: u16 = 0xB752;
const TRUEHD_MIN_HEADER_BYTES: usize = 20;
const AC3_MIN_HEADER_BYTES: usize = 8;
const AC3_FRAME_SIZE_WORDS: [[u16; 3]; 38] = [
    [64, 69, 96],
    [64, 70, 96],
    [80, 87, 120],
    [80, 88, 120],
    [96, 104, 144],
    [96, 105, 144],
    [112, 121, 168],
    [112, 122, 168],
    [128, 139, 192],
    [128, 140, 192],
    [160, 174, 240],
    [160, 175, 240],
    [192, 208, 288],
    [192, 209, 288],
    [224, 243, 336],
    [224, 244, 336],
    [256, 278, 384],
    [256, 279, 384],
    [320, 348, 480],
    [320, 349, 480],
    [384, 417, 576],
    [384, 418, 576],
    [448, 487, 672],
    [448, 488, 672],
    [512, 557, 768],
    [512, 558, 768],
    [640, 696, 960],
    [640, 697, 960],
    [768, 835, 1_152],
    [768, 836, 1_152],
    [896, 975, 1_344],
    [896, 976, 1_344],
    [1_024, 1_114, 1_536],
    [1_024, 1_115, 1_536],
    [1_152, 1_253, 1_728],
    [1_152, 1_254, 1_728],
    [1_280, 1_393, 1_920],
    [1_280, 1_394, 1_920],
];

pub(in crate::mux) struct ParsedTrueHdTrack {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrueHdDescriptor {
    sample_rate: u32,
    channel_count: u16,
    format_info: u32,
    peak_data_rate: u16,
    sample_duration: u32,
}

enum ParsedTrueHdUnit {
    AuxiliaryAc3 {
        frame_size: u32,
    },
    TrueHdFrame {
        descriptor: TrueHdDescriptor,
        frame_size: u32,
    },
}

pub(in crate::mux) fn scan_truehd_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedTrueHdTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_truehd_stream_sync(&mut file, file_size, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_truehd_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedTrueHdTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_truehd_stream_async(&mut file, file_size, spec).await
}

fn parse_truehd_stream_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<ParsedTrueHdTrack, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut descriptor = None::<TrueHdDescriptor>;

    while offset < file_size {
        match parse_truehd_unit_sync(file, file_size, offset, spec)? {
            ParsedTrueHdUnit::AuxiliaryAc3 { frame_size } => {
                offset =
                    offset
                        .checked_add(u64::from(frame_size))
                        .ok_or(MuxError::LayoutOverflow(
                            "TrueHD auxiliary AC-3 frame offset",
                        ))?;
            }
            ParsedTrueHdUnit::TrueHdFrame {
                descriptor: parsed_descriptor,
                frame_size,
            } => {
                if offset
                    .checked_add(u64::from(frame_size))
                    .is_none_or(|end| end > file_size)
                {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: format!("truncated TrueHD frame at byte offset {offset}"),
                    });
                }
                if let Some(current) = descriptor {
                    if current != parsed_descriptor {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message: "TrueHD frames changed decoder configuration mid-stream"
                                .to_string(),
                        });
                    }
                } else {
                    descriptor = Some(parsed_descriptor);
                }
                samples.push(StagedSample {
                    data_offset: offset,
                    data_size: frame_size,
                    duration: parsed_descriptor.sample_duration,
                    composition_time_offset: 0,
                    is_sync_sample: true,
                });
                offset = offset
                    .checked_add(u64::from(frame_size))
                    .ok_or(MuxError::LayoutOverflow("TrueHD frame offset"))?;
            }
        }
    }

    finalize_truehd_track(spec, descriptor, samples)
}

#[cfg(feature = "async")]
async fn parse_truehd_stream_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<ParsedTrueHdTrack, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut descriptor = None::<TrueHdDescriptor>;

    while offset < file_size {
        match parse_truehd_unit_async(file, file_size, offset, spec).await? {
            ParsedTrueHdUnit::AuxiliaryAc3 { frame_size } => {
                offset =
                    offset
                        .checked_add(u64::from(frame_size))
                        .ok_or(MuxError::LayoutOverflow(
                            "TrueHD auxiliary AC-3 frame offset",
                        ))?;
            }
            ParsedTrueHdUnit::TrueHdFrame {
                descriptor: parsed_descriptor,
                frame_size,
            } => {
                if offset
                    .checked_add(u64::from(frame_size))
                    .is_none_or(|end| end > file_size)
                {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: format!("truncated TrueHD frame at byte offset {offset}"),
                    });
                }
                if let Some(current) = descriptor {
                    if current != parsed_descriptor {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message: "TrueHD frames changed decoder configuration mid-stream"
                                .to_string(),
                        });
                    }
                } else {
                    descriptor = Some(parsed_descriptor);
                }
                samples.push(StagedSample {
                    data_offset: offset,
                    data_size: frame_size,
                    duration: parsed_descriptor.sample_duration,
                    composition_time_offset: 0,
                    is_sync_sample: true,
                });
                offset = offset
                    .checked_add(u64::from(frame_size))
                    .ok_or(MuxError::LayoutOverflow("TrueHD frame offset"))?;
            }
        }
    }

    finalize_truehd_track(spec, descriptor, samples)
}

fn finalize_truehd_track(
    spec: &str,
    descriptor: Option<TrueHdDescriptor>,
    samples: Vec<StagedSample>,
) -> Result<ParsedTrueHdTrack, MuxError> {
    let descriptor = descriptor.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "TrueHD input contained no TrueHD frames".to_string(),
    })?;

    Ok(ParsedTrueHdTrack {
        sample_rate: descriptor.sample_rate,
        sample_entry_box: build_truehd_sample_entry_box(descriptor)?,
        samples,
    })
}

fn parse_truehd_unit_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<ParsedTrueHdUnit, MuxError> {
    let remaining = file_size.saturating_sub(offset);
    if remaining < u64::try_from(AC3_MIN_HEADER_BYTES).unwrap() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated TrueHD frame header".to_string(),
        });
    }
    let header_len =
        usize::try_from(remaining.min(u64::try_from(TRUEHD_MIN_HEADER_BYTES).unwrap())).unwrap();
    let mut header = vec![0_u8; header_len];
    read_exact_at_sync(
        file,
        offset,
        &mut header,
        spec,
        "truncated TrueHD frame header",
    )?;
    parse_truehd_unit_header(&header, remaining, offset, spec)
}

#[cfg(feature = "async")]
async fn parse_truehd_unit_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<ParsedTrueHdUnit, MuxError> {
    let remaining = file_size.saturating_sub(offset);
    if remaining < u64::try_from(AC3_MIN_HEADER_BYTES).unwrap() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated TrueHD frame header".to_string(),
        });
    }
    let header_len =
        usize::try_from(remaining.min(u64::try_from(TRUEHD_MIN_HEADER_BYTES).unwrap())).unwrap();
    let mut header = vec![0_u8; header_len];
    read_exact_at_async(
        file,
        offset,
        &mut header,
        spec,
        "truncated TrueHD frame header",
    )
    .await?;
    parse_truehd_unit_header(&header, remaining, offset, spec)
}

fn parse_truehd_unit_header(
    header: &[u8],
    remaining: u64,
    offset: u64,
    spec: &str,
) -> Result<ParsedTrueHdUnit, MuxError> {
    if header.starts_with(&[0x0B, 0x77]) {
        let frame_size = parse_auxiliary_ac3_frame_size(header, offset, spec)?;
        if u64::from(frame_size) > remaining {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated auxiliary AC-3 frame at byte offset {offset}"),
            });
        }
        return Ok(ParsedTrueHdUnit::AuxiliaryAc3 { frame_size });
    }
    if header.len() < TRUEHD_MIN_HEADER_BYTES {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated TrueHD frame header".to_string(),
        });
    }
    let frame_size = parse_truehd_frame_size(header, offset, spec)?;
    if u64::from(frame_size) > remaining {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("truncated TrueHD frame at byte offset {offset}"),
        });
    }
    let descriptor = parse_truehd_descriptor(header, offset, spec)?;
    Ok(ParsedTrueHdUnit::TrueHdFrame {
        descriptor,
        frame_size,
    })
}

fn parse_truehd_frame_size(header: &[u8], offset: u64, spec: &str) -> Result<u32, MuxError> {
    let packed = u16::from_be_bytes([header[0], header[1]]);
    let frame_size = u32::from(packed & 0x0FFF) * 2;
    if frame_size < u32::try_from(TRUEHD_MIN_HEADER_BYTES).unwrap() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("invalid TrueHD frame size at byte offset {offset}"),
        });
    }
    Ok(frame_size)
}

fn parse_truehd_descriptor(
    header: &[u8],
    offset: u64,
    spec: &str,
) -> Result<TrueHdDescriptor, MuxError> {
    let sync = u32::from_be_bytes(header[4..8].try_into().unwrap());
    if sync != TRUEHD_SYNC {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("missing TrueHD sync marker at byte offset {offset}"),
        });
    }
    let format_info = u32::from_be_bytes(header[8..12].try_into().unwrap());
    let sample_rate = truehd_sample_rate(((format_info >> 28) & 0x0F) as u8).ok_or_else(|| {
        MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "unsupported TrueHD sample-rate code {}",
                (format_info >> 28) & 0x0F
            ),
        }
    })?;
    let signature = u16::from_be_bytes(header[12..14].try_into().unwrap());
    if signature != TRUEHD_SIGNATURE {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("missing TrueHD format signature at byte offset {offset}"),
        });
    }
    let sample_duration =
        truehd_frame_duration(sample_rate).ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported TrueHD sample rate {sample_rate}"),
        })?;
    let channel_count =
        truehd_channel_count(format_info).ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported TrueHD channel layout at byte offset {offset}"),
        })?;
    let peak_data_rate = (u16::from_be_bytes(header[2..4].try_into().unwrap()) >> 1) & 0x7FFF;
    Ok(TrueHdDescriptor {
        sample_rate,
        channel_count,
        format_info,
        peak_data_rate,
        sample_duration,
    })
}

fn truehd_sample_rate(code: u8) -> Option<u32> {
    match code {
        0 => Some(48_000),
        1 => Some(96_000),
        2 => Some(192_000),
        8 => Some(44_100),
        9 => Some(88_200),
        10 => Some(176_400),
        _ => None,
    }
}

fn truehd_frame_duration(sample_rate: u32) -> Option<u32> {
    match sample_rate {
        48_000 | 96_000 | 192_000 => Some(sample_rate / 1_200),
        44_100 | 88_200 | 176_400 => Some(sample_rate * 2 / 2_205),
        _ => None,
    }
}

fn truehd_channel_count(format_info: u32) -> Option<u16> {
    let ch_2_modif = ((format_info >> 22) & 0x03) as u8;
    let ch_6_assign = ((format_info >> 15) & 0x1F) as u8;
    let ch_8_assign = (format_info & 0x1FFF) as u16;

    let mut channel_count = if ch_2_modif == 1 { 1 } else { 2 };

    if ch_6_assign != 0 {
        channel_count = 0;
        if ch_6_assign & 0x01 != 0 {
            channel_count += 2;
        }
        if ch_6_assign & 0x02 != 0 {
            channel_count += 1;
        }
        if ch_6_assign & 0x04 != 0 {
            channel_count += 1;
        }
        if ch_6_assign & 0x08 != 0 {
            channel_count += 2;
        }
        if ch_6_assign & 0x10 != 0 {
            channel_count += 2;
        }
    }

    if ch_8_assign != 0 {
        channel_count = 0;
        if ch_8_assign & (1 << 0) != 0 {
            channel_count += 2;
        }
        if ch_8_assign & (1 << 1) != 0 {
            channel_count += 1;
        }
        if ch_8_assign & (1 << 2) != 0 {
            channel_count += 1;
        }
        if ch_8_assign & (1 << 3) != 0 {
            channel_count += 2;
        }
        if ch_8_assign & (1 << 4) != 0 {
            channel_count += 2;
        }
        if ch_8_assign & (1 << 5) != 0 {
            channel_count += 2;
        }
        if ch_8_assign & (1 << 6) != 0 {
            channel_count += 2;
        }
        if ch_8_assign & (1 << 7) != 0 {
            channel_count += 1;
        }
        if ch_8_assign & (1 << 8) != 0 {
            channel_count += 1;
        }
        if ch_8_assign & (1 << 9) != 0 {
            channel_count += 2;
        }
        if ch_8_assign & (1 << 10) != 0 {
            channel_count += 2;
        }
        if ch_8_assign & (1 << 11) != 0 {
            channel_count += 1;
        }
        if ch_8_assign & (1 << 12) != 0 {
            channel_count += 1;
        }
    }

    (channel_count != 0).then_some(channel_count)
}

fn build_truehd_sample_entry_box(descriptor: TrueHdDescriptor) -> Result<Vec<u8>, MuxError> {
    let dmlp = super::super::mp4::encode_typed_box(
        &Dmlp {
            format_info: descriptor.format_info,
            peak_data_rate: descriptor.peak_data_rate,
        },
        &[],
    )?;
    let nominal_bitrate = descriptor
        .sample_rate
        .checked_mul(u32::from(descriptor.channel_count))
        .and_then(|value| value.checked_mul(4))
        .ok_or(MuxError::LayoutOverflow("TrueHD nominal bitrate"))?;
    let btrt = super::super::mp4::encode_typed_box(
        &Btrt {
            buffer_size_db: descriptor.sample_duration,
            max_bitrate: nominal_bitrate,
            avg_bitrate: nominal_bitrate,
        },
        &[],
    )?;
    super::super::mp4::encode_typed_box(
        &AudioSampleEntry {
            sample_entry: SampleEntry {
                box_type: SAMPLE_ENTRY_MLPA,
                data_reference_index: 1,
            },
            channel_count: descriptor.channel_count,
            sample_size: 16,
            sample_rate: descriptor.sample_rate,
            ..AudioSampleEntry::default()
        },
        &[dmlp, btrt].concat(),
    )
}

fn parse_auxiliary_ac3_frame_size(header: &[u8], offset: u64, spec: &str) -> Result<u32, MuxError> {
    if header.len() < AC3_MIN_HEADER_BYTES {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated auxiliary AC-3 frame header".to_string(),
        });
    }
    if header[0] != 0x0B || header[1] != 0x77 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("missing auxiliary AC-3 sync word at byte offset {offset}"),
        });
    }
    let fscod = header[4] >> 6;
    if fscod == 0x03 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "reserved auxiliary AC-3 sample-rate code".to_string(),
        });
    }
    let frmsizecod = header[4] & 0x3F;
    let frame_size = ac3_frame_size_bytes(fscod, frmsizecod).ok_or_else(|| {
        MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported auxiliary AC-3 frame-size code {frmsizecod}"),
        }
    })?;
    let bsid = (header[5] >> 3) & 0x1F;
    if bsid > 10 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "embedded E-AC-3 frames are not currently supported in TrueHD input"
                .to_string(),
        });
    }
    Ok(frame_size)
}

fn ac3_frame_size_bytes(fscod: u8, frmsizecod: u8) -> Option<u32> {
    if frmsizecod > 37 {
        return None;
    }
    let frame_words = *AC3_FRAME_SIZE_WORDS.get(usize::from(frmsizecod))?;
    let sample_rate_index = match fscod {
        0 => 0,
        1 => 1,
        2 => 2,
        _ => return None,
    };
    Some(u32::from(frame_words[sample_rate_index]) * 2)
}
