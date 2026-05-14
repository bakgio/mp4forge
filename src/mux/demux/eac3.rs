use std::fs::File;
use std::io::Cursor;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::bitio::BitReader;
use crate::boxes::AnyTypeBox;
use crate::boxes::etsi_ts_102_366::{Dec3, Ec3Substream};
use crate::boxes::iso14496_12::{AudioSampleEntry, Btrt, SampleEntry};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{SegmentedMuxSourceSegment, StagedSample, read_exact_at_sync};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;

pub(in crate::mux) struct ParsedEac3Track {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) decoder_config: Eac3DecoderConfig,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mux) struct Eac3DecoderConfig {
    sample_rate: u32,
    channel_count: u16,
    fscod: u8,
    bsid: u8,
    bsmod: u8,
    acmod: u8,
    lfe_on: u8,
}

pub(in crate::mux) fn scan_eac3_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedEac3Track, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<Eac3DecoderConfig>;
    while offset < file_size {
        if file_size - offset < 6 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated E-AC-3 syncframe header".to_string(),
            });
        }
        let mut header = [0_u8; 6];
        read_exact_at_sync(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated E-AC-3 syncframe header",
        )?;
        let (decoder_config, frame_size, sample_duration) =
            parse_eac3_frame_header(&header, offset, spec)?;
        if offset
            .checked_add(frame_size)
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated E-AC-3 syncframe at byte offset {offset}"),
            });
        }
        if let Some(current) = &expected {
            if !same_eac3_config(current, &decoder_config) {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "E-AC-3 syncframes changed decoder configuration mid-stream"
                        .to_string(),
                });
            }
        } else {
            expected = Some(decoder_config);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_size)
                .map_err(|_| MuxError::LayoutOverflow("E-AC-3 frame size"))?,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size)
            .ok_or(MuxError::LayoutOverflow("E-AC-3 frame offset"))?;
    }

    let decoder_config = expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "E-AC-3 input contained no syncframes".to_string(),
    })?;
    Ok(ParsedEac3Track {
        sample_rate: decoder_config.sample_rate,
        decoder_config,
        sample_entry_box: build_eac3_sample_entry_box(&decoder_config, &samples)?,
        samples,
    })
}

pub(in crate::mux) fn scan_eac3_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedEac3Track, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<Eac3DecoderConfig>;
    while offset < total_size {
        if total_size - offset < 6 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated E-AC-3 syncframe header".to_string(),
            });
        }
        let mut header = [0_u8; 6];
        read_segmented_bytes_sync(
            file,
            segments,
            total_size,
            offset,
            &mut header,
            spec,
            "truncated E-AC-3 syncframe header",
        )?;
        let (decoder_config, frame_size, sample_duration) =
            parse_eac3_frame_header(&header, offset, spec)?;
        if offset
            .checked_add(frame_size)
            .is_none_or(|end| end > total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated E-AC-3 syncframe at logical byte offset {offset}"),
            });
        }
        if let Some(current) = &expected {
            if !same_eac3_config(current, &decoder_config) {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "E-AC-3 syncframes changed decoder configuration mid-stream"
                        .to_string(),
                });
            }
        } else {
            expected = Some(decoder_config);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_size)
                .map_err(|_| MuxError::LayoutOverflow("E-AC-3 frame size"))?,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size)
            .ok_or(MuxError::LayoutOverflow("E-AC-3 frame offset"))?;
    }

    let decoder_config = expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "E-AC-3 input contained no syncframes".to_string(),
    })?;
    Ok(ParsedEac3Track {
        sample_rate: decoder_config.sample_rate,
        decoder_config,
        sample_entry_box: build_eac3_sample_entry_box(&decoder_config, &samples)?,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_eac3_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedEac3Track, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<Eac3DecoderConfig>;
    while offset < file_size {
        if file_size - offset < 6 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated E-AC-3 syncframe header".to_string(),
            });
        }
        let mut header = [0_u8; 6];
        read_exact_at_async(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated E-AC-3 syncframe header",
        )
        .await?;
        let (decoder_config, frame_size, sample_duration) =
            parse_eac3_frame_header(&header, offset, spec)?;
        if offset
            .checked_add(frame_size)
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated E-AC-3 syncframe at byte offset {offset}"),
            });
        }
        if let Some(current) = &expected {
            if !same_eac3_config(current, &decoder_config) {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "E-AC-3 syncframes changed decoder configuration mid-stream"
                        .to_string(),
                });
            }
        } else {
            expected = Some(decoder_config);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_size)
                .map_err(|_| MuxError::LayoutOverflow("E-AC-3 frame size"))?,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size)
            .ok_or(MuxError::LayoutOverflow("E-AC-3 frame offset"))?;
    }

    let decoder_config = expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "E-AC-3 input contained no syncframes".to_string(),
    })?;
    Ok(ParsedEac3Track {
        sample_rate: decoder_config.sample_rate,
        decoder_config,
        sample_entry_box: build_eac3_sample_entry_box(&decoder_config, &samples)?,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_eac3_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedEac3Track, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<Eac3DecoderConfig>;
    while offset < total_size {
        if total_size - offset < 6 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated E-AC-3 syncframe header".to_string(),
            });
        }
        let mut header = [0_u8; 6];
        read_segmented_bytes_async(
            file,
            segments,
            total_size,
            offset,
            &mut header,
            spec,
            "truncated E-AC-3 syncframe header",
        )
        .await?;
        let (decoder_config, frame_size, sample_duration) =
            parse_eac3_frame_header(&header, offset, spec)?;
        if offset
            .checked_add(frame_size)
            .is_none_or(|end| end > total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated E-AC-3 syncframe at logical byte offset {offset}"),
            });
        }
        if let Some(current) = &expected {
            if !same_eac3_config(current, &decoder_config) {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "E-AC-3 syncframes changed decoder configuration mid-stream"
                        .to_string(),
                });
            }
        } else {
            expected = Some(decoder_config);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_size)
                .map_err(|_| MuxError::LayoutOverflow("E-AC-3 frame size"))?,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size)
            .ok_or(MuxError::LayoutOverflow("E-AC-3 frame offset"))?;
    }

    let decoder_config = expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "E-AC-3 input contained no syncframes".to_string(),
    })?;
    Ok(ParsedEac3Track {
        sample_rate: decoder_config.sample_rate,
        decoder_config,
        sample_entry_box: build_eac3_sample_entry_box(&decoder_config, &samples)?,
        samples,
    })
}

fn same_eac3_config(left: &Eac3DecoderConfig, right: &Eac3DecoderConfig) -> bool {
    left.sample_rate == right.sample_rate
        && left.channel_count == right.channel_count
        && left.fscod == right.fscod
        && left.bsid == right.bsid
        && left.bsmod == right.bsmod
        && left.acmod == right.acmod
        && left.lfe_on == right.lfe_on
}

fn parse_eac3_frame_header(
    header: &[u8; 6],
    offset: u64,
    spec: &str,
) -> Result<(Eac3DecoderConfig, u64, u32), MuxError> {
    if header[0] != 0x0B || header[1] != 0x77 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("missing E-AC-3 sync word at byte offset {offset}"),
        });
    }
    let mut reader = BitReader::new(Cursor::new(&header[2..]));
    let stream_type = read_bits_u8_labeled(&mut reader, 2, spec, "E-AC-3")?;
    if stream_type != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "the current raw E-AC-3 importer only supports independent substreams"
                .to_string(),
        });
    }
    let _substream_id = read_bits_u8_labeled(&mut reader, 3, spec, "E-AC-3")?;
    let frame_size_words_minus_one = read_bits_u16_labeled(&mut reader, 11, spec, "E-AC-3")?;
    let frame_size = u64::from(frame_size_words_minus_one.saturating_add(1))
        .checked_mul(2)
        .ok_or(MuxError::LayoutOverflow("E-AC-3 frame size"))?;
    let fscod = read_bits_u8_labeled(&mut reader, 2, spec, "E-AC-3")?;
    let (sample_rate, sample_duration) = if fscod == 0x03 {
        let fscod2 = read_bits_u8_labeled(&mut reader, 2, spec, "E-AC-3")?;
        let sample_rate = match fscod2 {
            0 => 24_000,
            1 => 22_050,
            2 => 16_000,
            _ => {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!("unsupported E-AC-3 half-rate code {fscod2}"),
                });
            }
        };
        (sample_rate, 1536)
    } else {
        let numblkscod = read_bits_u8_labeled(&mut reader, 2, spec, "E-AC-3")?;
        let sample_rate =
            ac3_sample_rate(fscod).ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported E-AC-3 sample-rate code {fscod}"),
            })?;
        let sample_duration = match numblkscod {
            0 => 256,
            1 => 512,
            2 => 768,
            3 => 1536,
            _ => unreachable!(),
        };
        (sample_rate, sample_duration)
    };
    let acmod = read_bits_u8_labeled(&mut reader, 3, spec, "E-AC-3")?;
    let lfe_on = u8::from(read_bit_labeled(&mut reader, spec, "E-AC-3")?);
    let bsid = read_bits_u8_labeled(&mut reader, 5, spec, "E-AC-3")?;
    let channel_count =
        ac3_channel_count(acmod, lfe_on != 0).ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported E-AC-3 channel mode {acmod}"),
        })?;
    Ok((
        Eac3DecoderConfig {
            sample_rate,
            channel_count,
            fscod: match sample_rate {
                48_000 => 0,
                44_100 => 1,
                32_000 => 2,
                _ => 3,
            },
            bsid,
            bsmod: 0,
            acmod,
            lfe_on,
        },
        frame_size,
        sample_duration,
    ))
}

pub(in crate::mux) fn build_eac3_sample_entry_box(
    parsed: &Eac3DecoderConfig,
    samples: &[StagedSample],
) -> Result<Vec<u8>, MuxError> {
    build_eac3_sample_entry_box_with_timescale(parsed, samples, parsed.sample_rate)
}

pub(in crate::mux) fn build_eac3_sample_entry_box_with_timescale(
    parsed: &Eac3DecoderConfig,
    samples: &[StagedSample],
    timescale: u32,
) -> Result<Vec<u8>, MuxError> {
    build_eac3_sample_entry_box_with_btrt(parsed, build_eac3_btrt(samples, timescale)?)
}

pub(in crate::mux) fn build_eac3_sample_entry_box_with_btrt(
    parsed: &Eac3DecoderConfig,
    btrt: Btrt,
) -> Result<Vec<u8>, MuxError> {
    let mut sample_entry = AudioSampleEntry::default();
    sample_entry.set_box_type(FourCc::from_bytes(*b"ec-3"));
    sample_entry.sample_entry = SampleEntry {
        box_type: FourCc::from_bytes(*b"ec-3"),
        data_reference_index: 1,
    };
    sample_entry.channel_count = eac3_sample_entry_channel_count(parsed);
    sample_entry.sample_size = 16;
    sample_entry.sample_rate = parsed.sample_rate << 16;

    let dec3 = super::super::mp4::encode_typed_box(
        &Dec3 {
            data_rate: u16::try_from(btrt.avg_bitrate / 1_000)
                .map_err(|_| MuxError::LayoutOverflow("E-AC-3 data_rate"))?,
            num_ind_sub: 0,
            ec3_substreams: vec![Ec3Substream {
                fscod: parsed.fscod,
                bsid: parsed.bsid,
                asvc: 0,
                bsmod: parsed.bsmod,
                acmod: parsed.acmod,
                lfe_on: parsed.lfe_on,
                num_dep_sub: 0,
                chan_loc: 0,
            }],
            reserved: Vec::new(),
        },
        &[],
    )?;
    let btrt = super::super::mp4::encode_typed_box(&btrt, &[])?;
    let mut children = dec3;
    children.extend_from_slice(&btrt);
    super::super::mp4::encode_typed_box(&sample_entry, &children)
}

const fn eac3_sample_entry_channel_count(parsed: &Eac3DecoderConfig) -> u16 {
    // Keep the authored `AudioSampleEntry.channel_count` aligned with the EC-3
    // sample-entry convention used by the retained flat-parity overlap, which
    // keeps the base dependent-layout count separate from the `dec3` LFE flag.
    match parsed.acmod {
        0 => 2,
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 3,
        5 => 4,
        6 => 4,
        7 => 5,
        _ => parsed.channel_count,
    }
}

fn build_eac3_btrt(samples: &[StagedSample], sample_rate: u32) -> Result<Btrt, MuxError> {
    if samples.is_empty() || sample_rate == 0 {
        return Ok(Btrt::default());
    }

    let mut buffer_size_db = 0_u32;
    let mut total_payload_bytes = 0_u64;
    let mut total_duration = 0_u64;
    let mut max_window_payload_bytes = 0_u64;
    let mut current_window_payload_bytes = 0_u64;
    let mut window_start_decode_time = 0_u64;
    let mut sample_decode_time = 0_u64;

    for sample in samples {
        buffer_size_db = buffer_size_db.max(sample.data_size);
        total_payload_bytes = total_payload_bytes
            .checked_add(u64::from(sample.data_size))
            .ok_or(MuxError::LayoutOverflow("E-AC-3 total payload bytes"))?;
        total_duration = total_duration
            .checked_add(u64::from(sample.duration))
            .ok_or(MuxError::LayoutOverflow("E-AC-3 total duration"))?;
        current_window_payload_bytes = current_window_payload_bytes
            .checked_add(u64::from(sample.data_size))
            .ok_or(MuxError::LayoutOverflow("E-AC-3 bitrate window payload"))?;
        if sample_decode_time > window_start_decode_time.saturating_add(u64::from(sample_rate)) {
            max_window_payload_bytes = max_window_payload_bytes.max(current_window_payload_bytes);
            window_start_decode_time = sample_decode_time;
            current_window_payload_bytes = 0;
        }
        sample_decode_time = sample_decode_time
            .checked_add(u64::from(sample.duration))
            .ok_or(MuxError::LayoutOverflow("E-AC-3 decode time"))?;
    }

    if total_duration == 0 {
        return Ok(Btrt::default());
    }

    let avg_bitrate = total_payload_bytes
        .checked_mul(8)
        .and_then(|bits| bits.checked_mul(u64::from(sample_rate)))
        .ok_or(MuxError::LayoutOverflow("E-AC-3 average bitrate"))?
        / total_duration;
    let avg_bitrate = avg_bitrate & !7;
    let max_bitrate = if max_window_payload_bytes == 0 {
        avg_bitrate
    } else {
        max_window_payload_bytes
            .checked_mul(8)
            .ok_or(MuxError::LayoutOverflow("E-AC-3 maximum bitrate"))?
    };

    Ok(Btrt {
        buffer_size_db,
        max_bitrate: u32::try_from(max_bitrate)
            .map_err(|_| MuxError::LayoutOverflow("E-AC-3 maximum bitrate"))?,
        avg_bitrate: u32::try_from(avg_bitrate)
            .map_err(|_| MuxError::LayoutOverflow("E-AC-3 average bitrate"))?,
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

const fn ac3_channel_count(acmod: u8, lfe_on: bool) -> Option<u16> {
    let base = match acmod {
        0 => 2,
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 3,
        5 => 4,
        6 => 4,
        7 => 5,
        _ => return None,
    };
    Some(base + if lfe_on { 1 } else { 0 })
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

fn read_bits_u16_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u16, MuxError>
where
    R: std::io::Read,
{
    let bits = reader
        .read_bits(width)
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })?;
    let mut value = 0_u32;
    for byte in bits {
        value = (value << 8) | u32::from(byte);
    }
    u16::try_from(value).map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("{label} bitfield does not fit in u16"),
    })
}
