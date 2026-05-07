use std::fs::File;
use std::io::Cursor;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::bitio::BitReader;
use crate::boxes::AnyTypeBox;
use crate::boxes::iso14496_12::{AudioSampleEntry, Btrt, SampleEntry};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{StagedSample, build_btrt_from_sample_sizes, read_exact_at_sync};

const DTSC: FourCc = FourCc::from_bytes(*b"dtsc");
const DTS_SYNC_WORD: u32 = 0x7FFE_8001;
const DTS_MIN_HEADER_BYTES: u64 = 11;
const DTS_MEDIA_TIMESCALE: u32 = 90_000;
const DTS_SAMPLE_RATE_BY_CODE: [Option<u32>; 16] = [
    None,
    Some(8_000),
    Some(16_000),
    Some(32_000),
    None,
    None,
    Some(11_025),
    Some(22_050),
    Some(44_100),
    None,
    None,
    Some(12_000),
    Some(24_000),
    Some(48_000),
    None,
    None,
];
const DTS_EXT_AUDIO_ID_VALID: [bool; 8] = [true, false, true, false, false, false, true, false];
const DTS_CORE_CHANNELS_BY_AMODE: [u16; 16] = [1, 2, 2, 2, 2, 3, 3, 4, 4, 5, 6, 6, 7, 7, 7, 8];

pub(in crate::mux) struct ParsedDtsTrack {
    pub(in crate::mux) media_timescale: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DtsTrackDescriptor {
    sample_rate: u32,
    sample_duration: u32,
    channel_count: u16,
    sample_depth: u8,
}

#[derive(Clone, Copy)]
struct ParsedDtsFrame {
    descriptor: DtsTrackDescriptor,
    frame_size: u32,
}

pub(in crate::mux) fn scan_dts_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_dts_stream_sync(&mut file, file_size, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_dts_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_dts_stream_async(&mut file, file_size, spec).await
}

fn parse_dts_stream_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut descriptor = None::<DtsTrackDescriptor>;

    while offset < file_size {
        if file_size - offset < DTS_MIN_HEADER_BYTES {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated DTS frame header".to_string(),
            });
        }
        let mut header = [0_u8; DTS_MIN_HEADER_BYTES as usize];
        read_exact_at_sync(
            file,
            offset,
            &mut header,
            spec,
            "truncated DTS frame header",
        )?;
        let parsed = parse_dts_frame_header(&header, offset, spec)?;
        let frame_size_u64 = u64::from(parsed.frame_size);
        if offset
            .checked_add(frame_size_u64)
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated DTS frame at byte offset {offset}"),
            });
        }
        if let Some(current) = descriptor {
            if current != parsed.descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "DTS frames changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            descriptor = Some(parsed.descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: parsed.frame_size,
            duration: parsed.descriptor.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size_u64)
            .ok_or(MuxError::LayoutOverflow("DTS frame offset"))?;
    }

    finalize_parsed_dts_track(spec, descriptor, samples)
}

#[cfg(feature = "async")]
async fn parse_dts_stream_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut descriptor = None::<DtsTrackDescriptor>;

    while offset < file_size {
        if file_size - offset < DTS_MIN_HEADER_BYTES {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated DTS frame header".to_string(),
            });
        }
        let mut header = [0_u8; DTS_MIN_HEADER_BYTES as usize];
        read_exact_at_async(
            file,
            offset,
            &mut header,
            spec,
            "truncated DTS frame header",
        )
        .await?;
        let parsed = parse_dts_frame_header(&header, offset, spec)?;
        let frame_size_u64 = u64::from(parsed.frame_size);
        if offset
            .checked_add(frame_size_u64)
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated DTS frame at byte offset {offset}"),
            });
        }
        if let Some(current) = descriptor {
            if current != parsed.descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "DTS frames changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            descriptor = Some(parsed.descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: parsed.frame_size,
            duration: parsed.descriptor.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size_u64)
            .ok_or(MuxError::LayoutOverflow("DTS frame offset"))?;
    }

    finalize_parsed_dts_track(spec, descriptor, samples)
}

fn finalize_parsed_dts_track(
    spec: &str,
    descriptor: Option<DtsTrackDescriptor>,
    samples: Vec<StagedSample>,
) -> Result<ParsedDtsTrack, MuxError> {
    let descriptor = descriptor.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "DTS input contained no frames".to_string(),
    })?;
    let samples = samples
        .into_iter()
        .map(|sample| {
            let duration = u64::from(sample.duration)
                .checked_mul(u64::from(DTS_MEDIA_TIMESCALE))
                .ok_or(MuxError::LayoutOverflow("DTS media duration"))?
                / u64::from(descriptor.sample_rate);
            let duration = u32::try_from(duration)
                .map_err(|_| MuxError::LayoutOverflow("DTS media duration"))?;
            if duration == 0 {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "DTS frame duration underflowed after media-timescale normalization"
                        .to_string(),
                });
            }
            Ok(StagedSample { duration, ..sample })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if samples.iter().all(|sample| sample.duration == 0) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "DTS input contained frames with zero duration".to_string(),
        });
    }
    let btrt = build_btrt_from_sample_sizes(
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        DTS_MEDIA_TIMESCALE,
    )?;
    Ok(ParsedDtsTrack {
        media_timescale: DTS_MEDIA_TIMESCALE,
        sample_entry_box: build_dts_sample_entry_box(descriptor, btrt)?,
        samples,
    })
}

fn parse_dts_frame_header(
    header: &[u8; DTS_MIN_HEADER_BYTES as usize],
    offset: u64,
    spec: &str,
) -> Result<ParsedDtsFrame, MuxError> {
    let mut reader = BitReader::new(Cursor::new(header.as_slice()));
    let sync_word = u32::from_be_bytes(read_bits_exact::<4, _>(&mut reader, spec, "DTS")?);
    if sync_word != DTS_SYNC_WORD {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("missing DTS sync word at byte offset {offset}"),
        });
    }
    skip_bits_labeled(&mut reader, 1 + 5, spec, "DTS")?;
    if read_bit_labeled(&mut reader, spec, "DTS")? {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "DTS frames with CRC protection are not supported".to_string(),
        });
    }
    let blocks_per_frame_minus_one = read_bits_u8_labeled(&mut reader, 7, spec, "DTS")?;
    if blocks_per_frame_minus_one < 5 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "unsupported DTS PCM sample-block count {}",
                blocks_per_frame_minus_one + 1
            ),
        });
    }
    let frame_size_minus_one = read_bits_u16_labeled(&mut reader, 14, spec, "DTS")?;
    if frame_size_minus_one < 95 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "unsupported DTS frame size {}",
                u32::from(frame_size_minus_one) + 1
            ),
        });
    }
    let amode = read_bits_u8_labeled(&mut reader, 6, spec, "DTS")?;
    let sample_rate_code = read_bits_u8_labeled(&mut reader, 4, spec, "DTS")?;
    let sample_rate = DTS_SAMPLE_RATE_BY_CODE
        .get(usize::from(sample_rate_code))
        .and_then(|value| *value)
        .ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported DTS sample-rate code {sample_rate_code}"),
        })?;
    let bitrate_code = read_bits_u8_labeled(&mut reader, 5, spec, "DTS")?;
    if bitrate_code > 25 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported DTS bitrate code {bitrate_code}"),
        });
    }
    let reserved = read_bit_labeled(&mut reader, spec, "DTS")?;
    if reserved {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "reserved DTS header bit was set".to_string(),
        });
    }
    skip_bits_labeled(&mut reader, 1 + 1 + 1 + 1, spec, "DTS")?;
    let ext_audio_id = read_bits_u8_labeled(&mut reader, 3, spec, "DTS")?;
    if !DTS_EXT_AUDIO_ID_VALID[usize::from(ext_audio_id)] {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported DTS extension-audio descriptor flag {ext_audio_id}"),
        });
    }
    skip_bits_labeled(&mut reader, 1 + 1, spec, "DTS")?;
    let lfe_flag = read_bits_u8_labeled(&mut reader, 2, spec, "DTS")?;
    if lfe_flag == 3 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "reserved DTS low-frequency-effects flag value".to_string(),
        });
    }

    let sample_duration = u32::from(blocks_per_frame_minus_one + 1) * 32;
    dts_frame_duration_code(sample_duration).ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("unsupported DTS frame duration {sample_duration}"),
    })?;
    let channel_count = dts_channel_count(amode, lfe_flag != 0).ok_or_else(|| {
        MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported DTS channel arrangement code {amode}"),
        }
    })?;
    let frame_size = u32::from(frame_size_minus_one) + 1;
    Ok(ParsedDtsFrame {
        descriptor: DtsTrackDescriptor {
            sample_rate,
            sample_duration,
            channel_count,
            sample_depth: 16,
        },
        frame_size,
    })
}

fn build_dts_sample_entry_box(
    descriptor: DtsTrackDescriptor,
    btrt: Btrt,
) -> Result<Vec<u8>, MuxError> {
    let mut sample_entry = AudioSampleEntry::default();
    sample_entry.set_box_type(DTSC);
    sample_entry.sample_entry = SampleEntry {
        box_type: DTSC,
        data_reference_index: 1,
    };
    sample_entry.channel_count = descriptor.channel_count;
    sample_entry.sample_size = u16::from(descriptor.sample_depth);
    sample_entry.sample_rate = descriptor.sample_rate << 16;

    let btrt_bytes = super::super::mp4::encode_typed_box(&btrt, &[])?;
    super::super::mp4::encode_typed_box(&sample_entry, &btrt_bytes)
}

const fn dts_frame_duration_code(sample_duration: u32) -> Option<u8> {
    match sample_duration {
        512 => Some(0),
        1024 => Some(1),
        2048 => Some(2),
        4096 => Some(3),
        _ => None,
    }
}

const fn dts_channel_count(amode: u8, lfe_present: bool) -> Option<u16> {
    if amode > 15 {
        return None;
    }
    Some(DTS_CORE_CHANNELS_BY_AMODE[amode as usize] + if lfe_present { 1 } else { 0 })
}

fn skip_bits_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<(), MuxError>
where
    R: std::io::Read,
{
    reader
        .read_bits(width)
        .map(|_| ())
        .map_err(|error| truncated_dts_error(spec, label, error))
}

fn read_bit_labeled<R>(reader: &mut BitReader<R>, spec: &str, label: &str) -> Result<bool, MuxError>
where
    R: std::io::Read,
{
    reader
        .read_bit()
        .map_err(|error| truncated_dts_error(spec, label, error))
}

fn read_bits_u8_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u8, MuxError>
where
    R: std::io::Read,
{
    let bytes = reader
        .read_bits(width)
        .map_err(|error| truncated_dts_error(spec, label, error))?;
    Ok(bytes[0])
}

fn read_bits_u16_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u16, MuxError>
where
    R: std::io::Read,
{
    let bytes = reader
        .read_bits(width)
        .map_err(|error| truncated_dts_error(spec, label, error))?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_bits_exact<const N: usize, R>(
    reader: &mut BitReader<R>,
    spec: &str,
    label: &str,
) -> Result<[u8; N], MuxError>
where
    R: std::io::Read,
{
    let bytes = reader
        .read_bits(N * 8)
        .map_err(|error| truncated_dts_error(spec, label, error))?;
    Ok(bytes.try_into().unwrap())
}

fn truncated_dts_error(spec: &str, label: &str, error: std::io::Error) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("{label} parsing failed: {error}"),
    }
}
