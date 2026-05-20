use std::fs::File;
use std::io::Cursor;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::bitio::BitReader;
use crate::boxes::AnyTypeBox;
use crate::boxes::etsi_ts_102_366::Dac3;
use crate::boxes::iso14496_12::{AudioSampleEntry, Btrt, SampleEntry};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{SegmentedMuxSourceSegment, StagedSample, read_exact_at_sync};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;

pub(in crate::mux) struct ParsedAc3Track {
    pub(in crate::mux) decoder_config: Ac3DecoderConfig,
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

#[derive(Clone, Copy)]
pub(in crate::mux) struct Ac3DecoderConfig {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) channel_count: u16,
    pub(in crate::mux) fscod: u8,
    pub(in crate::mux) bsid: u8,
    pub(in crate::mux) bsmod: u8,
    pub(in crate::mux) acmod: u8,
    pub(in crate::mux) lfe_on: u8,
    pub(in crate::mux) bit_rate_code: u8,
}

pub(in crate::mux) fn scan_ac3_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedAc3Track, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<Ac3DecoderConfig>;
    while offset < file_size {
        if file_size - offset < 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated AC-3 syncframe header".to_string(),
            });
        }
        let mut header = [0_u8; 8];
        read_exact_at_sync(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated AC-3 syncframe header",
        )?;
        let (decoder_config, frame_size) = parse_ac3_frame_header(&header, offset, spec)?;
        let frame_size_u64 = u64::from(frame_size);
        if offset
            .checked_add(frame_size_u64)
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated AC-3 syncframe at byte offset {offset}"),
            });
        }
        if let Some(current) = &expected {
            if !same_ac3_config(current, &decoder_config) {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "AC-3 syncframes changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            expected = Some(decoder_config);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: frame_size,
            duration: 1536,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size_u64)
            .ok_or(MuxError::LayoutOverflow("AC-3 frame offset"))?;
    }

    let decoder_config = expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "AC-3 input contained no syncframes".to_string(),
    })?;
    Ok(ParsedAc3Track {
        decoder_config,
        sample_rate: decoder_config.sample_rate,
        sample_entry_box: build_ac3_sample_entry_box(&decoder_config, &samples)?,
        samples,
    })
}

pub(in crate::mux) fn scan_ac3_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedAc3Track, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<Ac3DecoderConfig>;
    while offset < total_size {
        if total_size - offset < 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated AC-3 syncframe header".to_string(),
            });
        }
        let mut header = [0_u8; 8];
        read_segmented_bytes_sync(
            file,
            segments,
            total_size,
            offset,
            &mut header,
            spec,
            "truncated AC-3 syncframe header",
        )?;
        let (decoder_config, frame_size) = parse_ac3_frame_header(&header, offset, spec)?;
        let frame_size_u64 = u64::from(frame_size);
        if offset
            .checked_add(frame_size_u64)
            .is_none_or(|end| end > total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated AC-3 syncframe at logical byte offset {offset}"),
            });
        }
        if let Some(current) = &expected {
            if !same_ac3_config(current, &decoder_config) {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "AC-3 syncframes changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            expected = Some(decoder_config);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: frame_size,
            duration: 1536,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size_u64)
            .ok_or(MuxError::LayoutOverflow("AC-3 frame offset"))?;
    }

    let decoder_config = expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "AC-3 input contained no syncframes".to_string(),
    })?;
    Ok(ParsedAc3Track {
        decoder_config,
        sample_rate: decoder_config.sample_rate,
        sample_entry_box: build_ac3_sample_entry_box(&decoder_config, &samples)?,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_ac3_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedAc3Track, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<Ac3DecoderConfig>;
    while offset < file_size {
        if file_size - offset < 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated AC-3 syncframe header".to_string(),
            });
        }
        let mut header = [0_u8; 8];
        read_exact_at_async(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated AC-3 syncframe header",
        )
        .await?;
        let (decoder_config, frame_size) = parse_ac3_frame_header(&header, offset, spec)?;
        let frame_size_u64 = u64::from(frame_size);
        if offset
            .checked_add(frame_size_u64)
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated AC-3 syncframe at byte offset {offset}"),
            });
        }
        if let Some(current) = &expected {
            if !same_ac3_config(current, &decoder_config) {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "AC-3 syncframes changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            expected = Some(decoder_config);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: frame_size,
            duration: 1536,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size_u64)
            .ok_or(MuxError::LayoutOverflow("AC-3 frame offset"))?;
    }

    let decoder_config = expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "AC-3 input contained no syncframes".to_string(),
    })?;
    Ok(ParsedAc3Track {
        decoder_config,
        sample_rate: decoder_config.sample_rate,
        sample_entry_box: build_ac3_sample_entry_box(&decoder_config, &samples)?,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_ac3_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedAc3Track, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<Ac3DecoderConfig>;
    while offset < total_size {
        if total_size - offset < 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated AC-3 syncframe header".to_string(),
            });
        }
        let mut header = [0_u8; 8];
        read_segmented_bytes_async(
            file,
            segments,
            total_size,
            offset,
            &mut header,
            spec,
            "truncated AC-3 syncframe header",
        )
        .await?;
        let (decoder_config, frame_size) = parse_ac3_frame_header(&header, offset, spec)?;
        let frame_size_u64 = u64::from(frame_size);
        if offset
            .checked_add(frame_size_u64)
            .is_none_or(|end| end > total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated AC-3 syncframe at logical byte offset {offset}"),
            });
        }
        if let Some(current) = &expected {
            if !same_ac3_config(current, &decoder_config) {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "AC-3 syncframes changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            expected = Some(decoder_config);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: frame_size,
            duration: 1536,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size_u64)
            .ok_or(MuxError::LayoutOverflow("AC-3 frame offset"))?;
    }

    let decoder_config = expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "AC-3 input contained no syncframes".to_string(),
    })?;
    Ok(ParsedAc3Track {
        decoder_config,
        sample_rate: decoder_config.sample_rate,
        sample_entry_box: build_ac3_sample_entry_box(&decoder_config, &samples)?,
        samples,
    })
}

fn same_ac3_config(left: &Ac3DecoderConfig, right: &Ac3DecoderConfig) -> bool {
    left.sample_rate == right.sample_rate
        && left.channel_count == right.channel_count
        && left.bsid == right.bsid
        && left.bsmod == right.bsmod
        && left.acmod == right.acmod
        && left.lfe_on == right.lfe_on
        && left.bit_rate_code == right.bit_rate_code
}

pub(in crate::mux) fn parse_ac3_frame_header(
    header: &[u8; 8],
    offset: u64,
    spec: &str,
) -> Result<(Ac3DecoderConfig, u32), MuxError> {
    if header[0] != 0x0B || header[1] != 0x77 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("missing AC-3 sync word at byte offset {offset}"),
        });
    }
    let fscod = header[4] >> 6;
    if fscod == 0x03 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "reserved AC-3 sample-rate code".to_string(),
        });
    }
    let frmsizecod = header[4] & 0x3F;
    let frame_size = ac3_frame_size_bytes(fscod, frmsizecod).ok_or_else(|| {
        MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported AC-3 frame-size code {frmsizecod}"),
        }
    })?;
    let bsid = (header[5] >> 3) & 0x1F;
    let bsmod = header[5] & 0x07;
    let mut reader = BitReader::new(Cursor::new(&header[6..8]));
    let acmod = read_bits_u8_labeled(&mut reader, 3, spec, "AC-3")?;
    if acmod & 0x01 != 0 && acmod != 0x01 {
        skip_bits_labeled(&mut reader, 2, spec, "AC-3")?;
    }
    if acmod & 0x04 != 0 {
        skip_bits_labeled(&mut reader, 2, spec, "AC-3")?;
    }
    if acmod == 0x02 {
        skip_bits_labeled(&mut reader, 2, spec, "AC-3")?;
    }
    let lfe_on = u8::from(read_bit_labeled(&mut reader, spec, "AC-3")?);
    let sample_rate = ac3_sample_rate(fscod).ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("unsupported AC-3 sample-rate code {fscod}"),
    })?;
    let channel_count = ac3_sample_entry_channel_count(acmod, lfe_on != 0).ok_or_else(|| {
        MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported AC-3 channel mode {acmod}"),
        }
    })?;
    Ok((
        Ac3DecoderConfig {
            sample_rate,
            channel_count,
            fscod: match sample_rate {
                48_000 => 0,
                44_100 => 1,
                32_000 => 2,
                _ => unreachable!(),
            },
            bsid,
            bsmod,
            acmod,
            lfe_on,
            bit_rate_code: frmsizecod >> 1,
        },
        frame_size,
    ))
}

pub(in crate::mux) fn build_ac3_sample_entry_box(
    parsed: &Ac3DecoderConfig,
    samples: &[StagedSample],
) -> Result<Vec<u8>, MuxError> {
    build_ac3_sample_entry_box_from_sample_iter(
        parsed,
        parsed.sample_rate,
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
    )
}

pub(in crate::mux) fn build_ac3_sample_entry_box_with_btrt<I>(
    parsed: &Ac3DecoderConfig,
    sample_rate: u32,
    samples: I,
) -> Result<Vec<u8>, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    build_ac3_sample_entry_box_from_sample_iter(parsed, sample_rate, samples)
}

fn build_ac3_sample_entry_box_from_sample_iter<I>(
    parsed: &Ac3DecoderConfig,
    sample_rate: u32,
    samples: I,
) -> Result<Vec<u8>, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    let mut sample_entry = AudioSampleEntry::default();
    sample_entry.set_box_type(FourCc::from_bytes(*b"ac-3"));
    sample_entry.sample_entry = SampleEntry {
        box_type: FourCc::from_bytes(*b"ac-3"),
        data_reference_index: 1,
    };
    sample_entry.channel_count = 2;
    sample_entry.sample_size = 16;
    sample_entry.sample_rate = parsed.sample_rate << 16;

    let dac3 = super::super::mp4::encode_typed_box(
        &Dac3 {
            fscod: parsed.fscod,
            bsid: parsed.bsid,
            bsmod: parsed.bsmod,
            acmod: parsed.acmod,
            lfe_on: parsed.lfe_on,
            bit_rate_code: parsed.bit_rate_code,
        },
        &[],
    )?;
    let btrt = super::super::mp4::encode_typed_box(&build_ac3_btrt(samples, sample_rate)?, &[])?;
    let mut children = dac3;
    children.extend_from_slice(&btrt);
    super::super::mp4::encode_typed_box(&sample_entry, &children)
}

fn build_ac3_btrt<I>(samples: I, sample_rate: u32) -> Result<Btrt, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    if sample_rate == 0 {
        return Ok(Btrt::default());
    }

    let mut buffer_size_db = 0_u32;
    let mut total_payload_bytes = 0_u64;
    let mut total_duration = 0_u64;
    let mut max_window_payload_bytes = 0_u64;
    let mut current_window_payload_bytes = 0_u64;
    let mut window_start_decode_time = 0_u64;
    let mut sample_decode_time = 0_u64;
    let mut saw_sample = false;

    for (data_size, duration) in samples {
        saw_sample = true;
        buffer_size_db = buffer_size_db.max(data_size);
        total_payload_bytes = total_payload_bytes
            .checked_add(u64::from(data_size))
            .ok_or(MuxError::LayoutOverflow("AC-3 total payload bytes"))?;
        total_duration = total_duration
            .checked_add(u64::from(duration))
            .ok_or(MuxError::LayoutOverflow("AC-3 total duration"))?;
        current_window_payload_bytes = current_window_payload_bytes
            .checked_add(u64::from(data_size))
            .ok_or(MuxError::LayoutOverflow("AC-3 bitrate window payload"))?;
        if sample_decode_time > window_start_decode_time.saturating_add(u64::from(sample_rate)) {
            max_window_payload_bytes = max_window_payload_bytes.max(current_window_payload_bytes);
            window_start_decode_time = sample_decode_time;
            current_window_payload_bytes = 0;
        }
        sample_decode_time = sample_decode_time
            .checked_add(u64::from(duration))
            .ok_or(MuxError::LayoutOverflow("AC-3 decode time"))?;
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
        .ok_or(MuxError::LayoutOverflow("AC-3 average bitrate"))?
        / total_duration;
    let avg_bitrate = avg_bitrate & !7;
    let max_bitrate = if max_window_payload_bytes == 0 {
        avg_bitrate
    } else {
        max_window_payload_bytes
            .checked_mul(8)
            .ok_or(MuxError::LayoutOverflow("AC-3 maximum bitrate"))?
    };

    Ok(Btrt {
        buffer_size_db,
        max_bitrate: u32::try_from(max_bitrate)
            .map_err(|_| MuxError::LayoutOverflow("AC-3 maximum bitrate"))?,
        avg_bitrate: u32::try_from(avg_bitrate)
            .map_err(|_| MuxError::LayoutOverflow("AC-3 average bitrate"))?,
    })
}

const fn ac3_sample_rate(fscod: u8) -> Option<u32> {
    match fscod {
        0 => Some(48_000),
        1 => Some(44_100),
        2 => Some(32_000),
        _ => None,
    }
}

fn ac3_frame_size_bytes(fscod: u8, frmsizecod: u8) -> Option<u32> {
    const AC3_FRAME_SIZE_WORDS: [[u16; 3]; 38] = [
        [96, 69, 64],
        [96, 70, 64],
        [120, 87, 80],
        [120, 88, 80],
        [144, 104, 96],
        [144, 105, 96],
        [168, 121, 112],
        [168, 122, 112],
        [192, 139, 128],
        [192, 140, 128],
        [240, 174, 160],
        [240, 175, 160],
        [288, 208, 192],
        [288, 209, 192],
        [336, 243, 224],
        [336, 244, 224],
        [384, 278, 256],
        [384, 279, 256],
        [480, 348, 320],
        [480, 349, 320],
        [576, 417, 384],
        [576, 418, 384],
        [672, 487, 448],
        [672, 488, 448],
        [768, 557, 512],
        [768, 558, 512],
        [960, 696, 640],
        [960, 697, 640],
        [1152, 835, 768],
        [1152, 836, 768],
        [1344, 975, 896],
        [1344, 976, 896],
        [1536, 1114, 1024],
        [1536, 1115, 1024],
        [1728, 1253, 1152],
        [1728, 1254, 1152],
        [1920, 1393, 1280],
        [1920, 1394, 1280],
    ];
    let frame_words = *AC3_FRAME_SIZE_WORDS.get(usize::from(frmsizecod))?;
    let sample_rate_index = match fscod {
        0 => 2,
        1 => 1,
        2 => 0,
        _ => return None,
    };
    Some(u32::from(frame_words[sample_rate_index]) * 2)
}

const fn ac3_sample_entry_channel_count(acmod: u8, _lfe_on: bool) -> Option<u16> {
    Some(match acmod {
        0 => 2,
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 3,
        5 => 4,
        6 => 4,
        7 => 5,
        _ => return None,
    })
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
    let _ = reader
        .read_bits(width)
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })?;
    Ok(())
}

fn read_bit_labeled<R>(reader: &mut BitReader<R>, spec: &str, label: &str) -> Result<bool, MuxError>
where
    R: std::io::Read,
{
    reader
        .read_bit()
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })
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
    let bits = reader
        .read_bits(width)
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })?;
    let mut value = 0_u16;
    for byte in bits {
        value = (value << 8) | u16::from(byte);
    }
    u8::try_from(value).map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("{label} bitfield does not fit in u8"),
    })
}
