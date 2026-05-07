use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::iamf::Iacb;

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    StagedSample, build_generic_audio_sample_entry_box, read_exact_at_sync,
};

const IAMF_ENTRY: FourCc = FourCc::from_bytes(*b"iamf");
const IAMF_SIGNATURE: [u8; 4] = *b"iamf";
const OBU_IAMF_CODEC_CONFIG: u8 = 0;
const OBU_IAMF_AUDIO_ELEMENT: u8 = 1;
const OBU_IAMF_PARAMETER_BLOCK: u8 = 3;
const OBU_IAMF_TEMPORAL_DELIMITER: u8 = 4;
const OBU_IAMF_AUDIO_FRAME: u8 = 5;
const OBU_IAMF_SEQUENCE_HEADER: u8 = 31;

const AAC_SAMPLE_RATE_TABLE: [u32; 16] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350, 0, 0, 0,
];

pub(in crate::mux) struct ParsedIamfTrack {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

pub(in crate::mux) fn looks_like_iamf_prefix(prefix: &[u8]) -> bool {
    let Some(&first) = prefix.first() else {
        return false;
    };
    let obu_type = first >> 3;
    if obu_type != OBU_IAMF_SEQUENCE_HEADER {
        return false;
    }
    if first & 0x06 != 0 {
        return false;
    }
    let Ok((obu_size_field, size_len)) =
        read_leb128_from_slice(&prefix[1..], "__detect__", "IAMF OBU size", 0)
    else {
        return false;
    };
    let payload_offset = 1usize.saturating_add(size_len);
    let total_size = 1usize
        .checked_add(size_len)
        .and_then(|value| value.checked_add(usize::try_from(obu_size_field).ok()?));
    let Some(total_size) = total_size else {
        return false;
    };
    if prefix.len() < total_size || prefix.len() < payload_offset + 6 {
        return false;
    }
    let payload = &prefix[payload_offset..];
    if payload[..4] != IAMF_SIGNATURE {
        return false;
    }
    let primary = payload[4];
    let additional = payload[5];
    matches!(primary, 0..=2) || matches!(additional, 0..=2)
}

#[derive(Clone, Copy)]
struct IamfObuHeader {
    obu_type: u8,
    total_size: u64,
    header_size: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ParsedCodecConfig {
    codec_id: FourCc,
    num_samples_per_frame: u32,
    sample_rate: u32,
    sample_size: u16,
    channel_count_hint: Option<u16>,
}

pub(in crate::mux) fn scan_iamf_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedIamfTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_iamf_stream_sync(&mut file, file_size, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_iamf_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedIamfTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_iamf_stream_async(&mut file, file_size, spec).await
}

fn parse_iamf_stream_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<ParsedIamfTrack, MuxError> {
    let mut offset = 0u64;
    let mut descriptor_obus = Vec::new();
    let mut codec_config = None::<ParsedCodecConfig>;
    let mut total_substreams = 0u16;
    let mut current_sample_start = None::<u64>;
    let mut audio_frames_in_current_sample = 0u16;
    let mut saw_temporal_units = false;
    let mut saw_delimiter_mode = None::<bool>;
    let mut samples = Vec::new();

    while offset < file_size {
        let header = read_iamf_obu_header_sync(file, offset, file_size, spec)?;
        let obu_end = offset
            .checked_add(header.total_size)
            .ok_or(MuxError::LayoutOverflow("IAMF OBU range"))?;
        if obu_end > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("IAMF OBU at byte offset {offset} overruns the input length"),
            });
        }
        match header.obu_type {
            OBU_IAMF_SEQUENCE_HEADER => {
                ensure_iamf_sequence_header_sync(file, offset, &header, spec)?;
                if saw_temporal_units {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: "IAMF sequence headers must not appear after temporal-unit data"
                            .to_string(),
                    });
                }
                descriptor_obus.extend_from_slice(&read_obu_bytes_sync(
                    file,
                    offset,
                    header.total_size,
                    spec,
                    "IAMF sequence header is truncated",
                )?);
            }
            OBU_IAMF_CODEC_CONFIG => {
                if saw_temporal_units {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message:
                            "IAMF codec configuration OBUs must not appear after temporal-unit data"
                                .to_string(),
                    });
                }
                let payload = read_iamf_payload_sync(file, offset, &header, spec)?;
                let parsed = parse_iamf_codec_config_payload(&payload, spec)?;
                if let Some(current) = codec_config {
                    if current != parsed {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "IAMF codec configuration changed sample rate, sample size, frame length, or codec id mid-stream"
                                    .to_string(),
                        });
                    }
                } else {
                    codec_config = Some(parsed);
                }
                descriptor_obus.extend_from_slice(&read_obu_bytes_sync(
                    file,
                    offset,
                    header.total_size,
                    spec,
                    "IAMF codec configuration OBU is truncated",
                )?);
            }
            OBU_IAMF_AUDIO_ELEMENT => {
                if saw_temporal_units {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: "IAMF audio-element OBUs must not appear after temporal-unit data"
                            .to_string(),
                    });
                }
                let payload = read_iamf_payload_sync(file, offset, &header, spec)?;
                let substreams = parse_iamf_audio_element_payload(&payload, spec)?;
                total_substreams = total_substreams
                    .checked_add(substreams)
                    .ok_or(MuxError::LayoutOverflow("IAMF total substreams"))?;
                descriptor_obus.extend_from_slice(&read_obu_bytes_sync(
                    file,
                    offset,
                    header.total_size,
                    spec,
                    "IAMF audio-element OBU is truncated",
                )?);
            }
            OBU_IAMF_PARAMETER_BLOCK | OBU_IAMF_TEMPORAL_DELIMITER | OBU_IAMF_AUDIO_FRAME => {
                let config = codec_config.ok_or_else(|| MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "IAMF temporal-unit data appeared before any codec configuration OBU"
                        .to_string(),
                })?;
                if total_substreams == 0 {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: "IAMF temporal-unit data appeared before any audio-element OBU"
                            .to_string(),
                    });
                }
                saw_temporal_units = true;
                if header.obu_type == OBU_IAMF_TEMPORAL_DELIMITER {
                    if audio_frames_in_current_sample != 0 {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "IAMF temporal delimiters must not appear in the middle of a temporal unit"
                                    .to_string(),
                        });
                    }
                    saw_delimiter_mode.get_or_insert(true);
                    if current_sample_start.is_some() {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "IAMF temporal unit carried more than one delimiter before its audio frames"
                                    .to_string(),
                        });
                    }
                    current_sample_start = Some(offset);
                } else {
                    if saw_delimiter_mode == Some(true) && current_sample_start.is_none() {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "IAMF temporal-unit data stopped using temporal delimiters after they had already started"
                                    .to_string(),
                        });
                    }
                    saw_delimiter_mode.get_or_insert(false);
                    if current_sample_start.is_none() {
                        current_sample_start = Some(offset);
                    }
                    if header.obu_type == OBU_IAMF_AUDIO_FRAME {
                        audio_frames_in_current_sample = audio_frames_in_current_sample
                            .checked_add(1)
                            .ok_or(MuxError::LayoutOverflow("IAMF audio-frame count"))?;
                    }
                    if audio_frames_in_current_sample == total_substreams {
                        let sample_start = current_sample_start.take().unwrap();
                        let data_size = u32::try_from(obu_end - sample_start)
                            .map_err(|_| MuxError::LayoutOverflow("IAMF temporal-unit size"))?;
                        samples.push(StagedSample {
                            data_offset: sample_start,
                            data_size,
                            duration: config.num_samples_per_frame,
                            composition_time_offset: 0,
                            is_sync_sample: true,
                        });
                        audio_frames_in_current_sample = 0;
                    } else if audio_frames_in_current_sample > total_substreams {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "IAMF temporal unit carried more audio frames than the declared substream count"
                                    .to_string(),
                        });
                    }
                }
            }
            _ => {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!(
                        "IAMF OBU at byte offset {offset} used unsupported OBU type {}",
                        header.obu_type
                    ),
                });
            }
        }
        offset = obu_end;
    }

    finalize_iamf_track(
        spec,
        descriptor_obus,
        codec_config,
        total_substreams,
        current_sample_start,
        audio_frames_in_current_sample,
        samples,
    )
}

#[cfg(feature = "async")]
async fn parse_iamf_stream_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<ParsedIamfTrack, MuxError> {
    let mut offset = 0u64;
    let mut descriptor_obus = Vec::new();
    let mut codec_config = None::<ParsedCodecConfig>;
    let mut total_substreams = 0u16;
    let mut current_sample_start = None::<u64>;
    let mut audio_frames_in_current_sample = 0u16;
    let mut saw_temporal_units = false;
    let mut saw_delimiter_mode = None::<bool>;
    let mut samples = Vec::new();

    while offset < file_size {
        let header = read_iamf_obu_header_async(file, offset, file_size, spec).await?;
        let obu_end = offset
            .checked_add(header.total_size)
            .ok_or(MuxError::LayoutOverflow("IAMF OBU range"))?;
        if obu_end > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("IAMF OBU at byte offset {offset} overruns the input length"),
            });
        }
        match header.obu_type {
            OBU_IAMF_SEQUENCE_HEADER => {
                ensure_iamf_sequence_header_async(file, offset, &header, spec).await?;
                if saw_temporal_units {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: "IAMF sequence headers must not appear after temporal-unit data"
                            .to_string(),
                    });
                }
                descriptor_obus.extend_from_slice(
                    &read_obu_bytes_async(
                        file,
                        offset,
                        header.total_size,
                        spec,
                        "IAMF sequence header is truncated",
                    )
                    .await?,
                );
            }
            OBU_IAMF_CODEC_CONFIG => {
                if saw_temporal_units {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message:
                            "IAMF codec configuration OBUs must not appear after temporal-unit data"
                                .to_string(),
                    });
                }
                let payload = read_iamf_payload_async(file, offset, &header, spec).await?;
                let parsed = parse_iamf_codec_config_payload(&payload, spec)?;
                if let Some(current) = codec_config {
                    if current != parsed {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "IAMF codec configuration changed sample rate, sample size, frame length, or codec id mid-stream"
                                    .to_string(),
                        });
                    }
                } else {
                    codec_config = Some(parsed);
                }
                descriptor_obus.extend_from_slice(
                    &read_obu_bytes_async(
                        file,
                        offset,
                        header.total_size,
                        spec,
                        "IAMF codec configuration OBU is truncated",
                    )
                    .await?,
                );
            }
            OBU_IAMF_AUDIO_ELEMENT => {
                if saw_temporal_units {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: "IAMF audio-element OBUs must not appear after temporal-unit data"
                            .to_string(),
                    });
                }
                let payload = read_iamf_payload_async(file, offset, &header, spec).await?;
                let substreams = parse_iamf_audio_element_payload(&payload, spec)?;
                total_substreams = total_substreams
                    .checked_add(substreams)
                    .ok_or(MuxError::LayoutOverflow("IAMF total substreams"))?;
                descriptor_obus.extend_from_slice(
                    &read_obu_bytes_async(
                        file,
                        offset,
                        header.total_size,
                        spec,
                        "IAMF audio-element OBU is truncated",
                    )
                    .await?,
                );
            }
            OBU_IAMF_PARAMETER_BLOCK | OBU_IAMF_TEMPORAL_DELIMITER | OBU_IAMF_AUDIO_FRAME => {
                let config = codec_config.ok_or_else(|| MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "IAMF temporal-unit data appeared before any codec configuration OBU"
                        .to_string(),
                })?;
                if total_substreams == 0 {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: "IAMF temporal-unit data appeared before any audio-element OBU"
                            .to_string(),
                    });
                }
                saw_temporal_units = true;
                if header.obu_type == OBU_IAMF_TEMPORAL_DELIMITER {
                    if audio_frames_in_current_sample != 0 {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "IAMF temporal delimiters must not appear in the middle of a temporal unit"
                                    .to_string(),
                        });
                    }
                    saw_delimiter_mode.get_or_insert(true);
                    if current_sample_start.is_some() {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "IAMF temporal unit carried more than one delimiter before its audio frames"
                                    .to_string(),
                        });
                    }
                    current_sample_start = Some(offset);
                } else {
                    if saw_delimiter_mode == Some(true) && current_sample_start.is_none() {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "IAMF temporal-unit data stopped using temporal delimiters after they had already started"
                                    .to_string(),
                        });
                    }
                    saw_delimiter_mode.get_or_insert(false);
                    if current_sample_start.is_none() {
                        current_sample_start = Some(offset);
                    }
                    if header.obu_type == OBU_IAMF_AUDIO_FRAME {
                        audio_frames_in_current_sample = audio_frames_in_current_sample
                            .checked_add(1)
                            .ok_or(MuxError::LayoutOverflow("IAMF audio-frame count"))?;
                    }
                    if audio_frames_in_current_sample == total_substreams {
                        let sample_start = current_sample_start.take().unwrap();
                        let data_size = u32::try_from(obu_end - sample_start)
                            .map_err(|_| MuxError::LayoutOverflow("IAMF temporal-unit size"))?;
                        samples.push(StagedSample {
                            data_offset: sample_start,
                            data_size,
                            duration: config.num_samples_per_frame,
                            composition_time_offset: 0,
                            is_sync_sample: true,
                        });
                        audio_frames_in_current_sample = 0;
                    } else if audio_frames_in_current_sample > total_substreams {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "IAMF temporal unit carried more audio frames than the declared substream count"
                                    .to_string(),
                        });
                    }
                }
            }
            _ => {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!(
                        "IAMF OBU at byte offset {offset} used unsupported OBU type {}",
                        header.obu_type
                    ),
                });
            }
        }
        offset = obu_end;
    }

    finalize_iamf_track(
        spec,
        descriptor_obus,
        codec_config,
        total_substreams,
        current_sample_start,
        audio_frames_in_current_sample,
        samples,
    )
}

fn finalize_iamf_track(
    spec: &str,
    descriptor_obus: Vec<u8>,
    codec_config: Option<ParsedCodecConfig>,
    total_substreams: u16,
    current_sample_start: Option<u64>,
    audio_frames_in_current_sample: u16,
    samples: Vec<StagedSample>,
) -> Result<ParsedIamfTrack, MuxError> {
    if current_sample_start.is_some() || audio_frames_in_current_sample != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF input ended in the middle of a temporal unit".to_string(),
        });
    }
    if descriptor_obus.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF input did not contain any configuration descriptor OBUs".to_string(),
        });
    }
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF input did not contain any temporal-unit audio frames".to_string(),
        });
    }
    let codec_config = codec_config.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "IAMF input did not contain any codec configuration OBU".to_string(),
    })?;
    if total_substreams == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF input did not contain any audio-element OBU".to_string(),
        });
    }
    let iacb_bytes = super::super::mp4::encode_typed_box(
        &Iacb {
            configuration_version: 1,
            config_obus: descriptor_obus,
        },
        &[],
    )?;
    let sample_entry_box =
        build_generic_audio_sample_entry_box(IAMF_ENTRY, 0, 0, 0, &[iacb_bytes])?;
    Ok(ParsedIamfTrack {
        sample_rate: codec_config.sample_rate,
        sample_entry_box,
        samples,
    })
}

fn read_iamf_obu_header_sync(
    file: &mut File,
    offset: u64,
    file_size: u64,
    spec: &str,
) -> Result<IamfObuHeader, MuxError> {
    let remaining = file_size.saturating_sub(offset);
    let header_probe_len = usize::try_from(remaining.min(32)).unwrap();
    let mut probe = vec![0u8; header_probe_len];
    read_exact_at_sync(
        file,
        offset,
        &mut probe,
        spec,
        "IAMF OBU header is truncated",
    )?;
    parse_iamf_obu_header(&probe, spec, offset)
}

#[cfg(feature = "async")]
async fn read_iamf_obu_header_async(
    file: &mut TokioFile,
    offset: u64,
    file_size: u64,
    spec: &str,
) -> Result<IamfObuHeader, MuxError> {
    let remaining = file_size.saturating_sub(offset);
    let header_probe_len = usize::try_from(remaining.min(32)).unwrap();
    let mut probe = vec![0u8; header_probe_len];
    read_exact_at_async(
        file,
        offset,
        &mut probe,
        spec,
        "IAMF OBU header is truncated",
    )
    .await?;
    parse_iamf_obu_header(&probe, spec, offset)
}

fn parse_iamf_obu_header(bytes: &[u8], spec: &str, offset: u64) -> Result<IamfObuHeader, MuxError> {
    let Some(&first) = bytes.first() else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF OBU header is truncated".to_string(),
        });
    };
    let obu_type = first >> 3;
    let redundant_copy = first & 0x04 != 0;
    let trimming_status_flag = first & 0x02 != 0;
    let extension_flag = first & 0x01 != 0;
    if redundant_copy && is_iamf_temporal_unit_obu(obu_type) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "IAMF temporal-unit OBU at byte offset {offset} used an unsupported redundant-copy flag"
            ),
        });
    }
    if trimming_status_flag && obu_type != OBU_IAMF_AUDIO_FRAME {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "IAMF non-audio-frame OBU at byte offset {offset} used an unsupported trimming flag"
            ),
        });
    }
    let (obu_size_field, size_len) =
        read_leb128_from_slice(&bytes[1..], spec, "IAMF OBU size", offset)?;
    let mut header_size = 1u64
        .checked_add(u64::try_from(size_len).unwrap())
        .ok_or(MuxError::LayoutOverflow("IAMF OBU header size"))?;
    let mut cursor = 1usize + size_len;
    if trimming_status_flag {
        let (_, trim_end_len) =
            read_leb128_from_slice(&bytes[cursor..], spec, "IAMF trim-end", offset)?;
        cursor += trim_end_len;
        let (_, trim_start_len) =
            read_leb128_from_slice(&bytes[cursor..], spec, "IAMF trim-start", offset)?;
        cursor += trim_start_len;
        header_size = u64::try_from(cursor).unwrap();
    }
    if extension_flag {
        let (extension_size, extension_len) =
            read_leb128_from_slice(&bytes[cursor..], spec, "IAMF extension header size", offset)?;
        let extension_size = usize::try_from(extension_size)
            .map_err(|_| MuxError::LayoutOverflow("IAMF extension header size"))?;
        cursor = cursor
            .checked_add(extension_len)
            .and_then(|value| value.checked_add(extension_size))
            .ok_or(MuxError::LayoutOverflow("IAMF extension header size"))?;
        if bytes.len() < cursor {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "IAMF OBU at byte offset {offset} truncated inside its extension header"
                ),
            });
        }
        header_size = u64::try_from(cursor).unwrap();
    }
    let total_size = 1u64
        .checked_add(obu_size_field)
        .and_then(|value| value.checked_add(u64::try_from(size_len).unwrap()))
        .ok_or(MuxError::LayoutOverflow("IAMF OBU size"))?;
    if total_size < header_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "IAMF OBU at byte offset {offset} declared a size shorter than its parsed header"
            ),
        });
    }
    Ok(IamfObuHeader {
        obu_type,
        total_size,
        header_size,
    })
}

fn ensure_iamf_sequence_header_sync(
    file: &mut File,
    offset: u64,
    header: &IamfObuHeader,
    spec: &str,
) -> Result<(), MuxError> {
    let payload = read_iamf_payload_sync(file, offset, header, spec)?;
    ensure_iamf_sequence_header_payload(&payload, spec, offset)
}

#[cfg(feature = "async")]
async fn ensure_iamf_sequence_header_async(
    file: &mut TokioFile,
    offset: u64,
    header: &IamfObuHeader,
    spec: &str,
) -> Result<(), MuxError> {
    let payload = read_iamf_payload_async(file, offset, header, spec).await?;
    ensure_iamf_sequence_header_payload(&payload, spec, offset)
}

fn ensure_iamf_sequence_header_payload(
    payload: &[u8],
    spec: &str,
    offset: u64,
) -> Result<(), MuxError> {
    if payload.len() < 6 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("IAMF sequence header at byte offset {offset} is truncated"),
        });
    }
    if payload[..4] != IAMF_SIGNATURE {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "IAMF sequence header at byte offset {offset} did not start with the `iamf` signature"
            ),
        });
    }
    let primary = payload[4];
    let additional = payload[5];
    if !matches!(primary, 0..=2) && !matches!(additional, 0..=2) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "IAMF sequence header at byte offset {offset} used unsupported profiles {primary} and {additional}"
            ),
        });
    }
    Ok(())
}

fn parse_iamf_codec_config_payload(
    payload: &[u8],
    spec: &str,
) -> Result<ParsedCodecConfig, MuxError> {
    let (codec_config_id, id_len) =
        read_leb128_from_slice(payload, spec, "IAMF codec_config_id", 0)?;
    let _ = codec_config_id;
    if payload.len() < id_len + 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF codec configuration OBU is truncated".to_string(),
        });
    }
    let codec_id = FourCc::from_bytes(payload[id_len..id_len + 4].try_into().unwrap());
    let (num_samples_per_frame, frame_len_len) = read_leb128_from_slice(
        &payload[id_len + 4..],
        spec,
        "IAMF num_samples_per_frame",
        0,
    )?;
    let cursor = id_len + 4 + frame_len_len;
    if payload.len() < cursor + 2 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF codec configuration OBU is truncated before audio roll distance"
                .to_string(),
        });
    }
    let codec_payload = &payload[cursor + 2..];
    let (sample_rate, sample_size, channel_count_hint) = match codec_id {
        fourcc if fourcc == FourCc::from_bytes(*b"Opus") => (48_000, 16, None),
        fourcc if fourcc == FourCc::from_bytes(*b"fLaC") => {
            parse_iamf_flac_config(codec_payload, spec)?
        }
        fourcc if fourcc == FourCc::from_bytes(*b"ipcm") => {
            parse_iamf_lpcm_config(codec_payload, spec)?
        }
        fourcc if fourcc == FourCc::from_bytes(*b"mp4a") => {
            parse_iamf_aac_config(codec_payload, spec)?
        }
        _ => {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "IAMF codec configuration used unsupported codec id `{}`",
                    codec_id
                ),
            });
        }
    };
    let num_samples_per_frame = u32::try_from(num_samples_per_frame)
        .map_err(|_| MuxError::LayoutOverflow("IAMF samples per frame"))?;
    if num_samples_per_frame == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF codec configuration declared a zero samples-per-frame value".to_string(),
        });
    }
    Ok(ParsedCodecConfig {
        codec_id,
        num_samples_per_frame,
        sample_rate,
        sample_size,
        channel_count_hint,
    })
}

fn parse_iamf_audio_element_payload(payload: &[u8], spec: &str) -> Result<u16, MuxError> {
    let (_, id_len) = read_leb128_from_slice(payload, spec, "IAMF audio_element_id", 0)?;
    if payload.len() < id_len + 2 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF audio-element OBU is truncated".to_string(),
        });
    }
    let cursor = id_len + 1;
    let (_, codec_config_len) = read_leb128_from_slice(
        &payload[cursor..],
        spec,
        "IAMF audio-element codec_config_id",
        0,
    )?;
    let next = cursor + codec_config_len;
    let (num_substreams, _) = read_leb128_from_slice(
        &payload[next..],
        spec,
        "IAMF audio-element num_substreams",
        0,
    )?;
    let substreams = u16::try_from(num_substreams)
        .map_err(|_| MuxError::LayoutOverflow("IAMF audio substreams"))?;
    if substreams == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF audio-element OBU declared zero substreams".to_string(),
        });
    }
    Ok(substreams)
}

fn parse_iamf_flac_config(payload: &[u8], spec: &str) -> Result<(u32, u16, Option<u16>), MuxError> {
    if payload.len() < 18 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF FLAC codec configuration is truncated".to_string(),
        });
    }
    let sample_rate = (u32::from(payload[10]) << 12)
        | (u32::from(payload[11]) << 4)
        | u32::from(payload[12] >> 4);
    let channel_count = u16::from(((payload[12] >> 1) & 0x07) + 1);
    let sample_size = u16::from((((payload[12] & 0x01) << 4) | (payload[13] >> 4)) + 1);
    if sample_rate == 0 || sample_size == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF FLAC codec configuration declared a zero-valued audio parameter"
                .to_string(),
        });
    }
    Ok((sample_rate, sample_size, Some(channel_count)))
}

fn parse_iamf_lpcm_config(payload: &[u8], spec: &str) -> Result<(u32, u16, Option<u16>), MuxError> {
    if payload.len() < 6 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF LPCM codec configuration is truncated".to_string(),
        });
    }
    let sample_size = u16::from(payload[1]);
    let sample_rate = u32::from_be_bytes(payload[2..6].try_into().unwrap());
    if sample_size == 0 || sample_rate == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "IAMF LPCM codec configuration declared a zero-valued audio parameter"
                .to_string(),
        });
    }
    Ok((sample_rate, sample_size, None))
}

fn parse_iamf_aac_config(payload: &[u8], spec: &str) -> Result<(u32, u16, Option<u16>), MuxError> {
    let mut cursor = BitCursor::new(payload);
    let audio_object_type = cursor.read_bits(5, spec, "IAMF AAC audio object type")?;
    let audio_object_type = if audio_object_type == 31 {
        32 + cursor.read_bits(6, spec, "IAMF AAC extended audio object type")?
    } else {
        audio_object_type
    };
    let sample_rate_index = cursor.read_bits(4, spec, "IAMF AAC sample rate index")?;
    let sample_rate = if sample_rate_index == 0x0F {
        cursor.read_bits(24, spec, "IAMF AAC explicit sample rate")?
    } else {
        AAC_SAMPLE_RATE_TABLE
            .get(usize::try_from(sample_rate_index).unwrap())
            .copied()
            .unwrap_or(0)
    };
    let channel_configuration = cursor.read_bits(4, spec, "IAMF AAC channel configuration")?;
    if audio_object_type == 0 || sample_rate == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "IAMF AAC codec configuration declared an unsupported object type or sample rate"
                    .to_string(),
        });
    }
    Ok((sample_rate, 16, u16::try_from(channel_configuration).ok()))
}

fn read_iamf_payload_sync(
    file: &mut File,
    offset: u64,
    header: &IamfObuHeader,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    let payload_size = usize::try_from(header.total_size - header.header_size)
        .map_err(|_| MuxError::LayoutOverflow("IAMF payload size"))?;
    let mut payload = vec![0u8; payload_size];
    read_exact_at_sync(
        file,
        offset + header.header_size,
        &mut payload,
        spec,
        "IAMF OBU payload is truncated",
    )?;
    Ok(payload)
}

#[cfg(feature = "async")]
async fn read_iamf_payload_async(
    file: &mut TokioFile,
    offset: u64,
    header: &IamfObuHeader,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    let payload_size = usize::try_from(header.total_size - header.header_size)
        .map_err(|_| MuxError::LayoutOverflow("IAMF payload size"))?;
    let mut payload = vec![0u8; payload_size];
    read_exact_at_async(
        file,
        offset + header.header_size,
        &mut payload,
        spec,
        "IAMF OBU payload is truncated",
    )
    .await?;
    Ok(payload)
}

fn read_obu_bytes_sync(
    file: &mut File,
    offset: u64,
    size: u64,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let size = usize::try_from(size).map_err(|_| MuxError::LayoutOverflow("IAMF OBU size"))?;
    let mut bytes = vec![0u8; size];
    read_exact_at_sync(file, offset, &mut bytes, spec, truncated_message)?;
    Ok(bytes)
}

#[cfg(feature = "async")]
async fn read_obu_bytes_async(
    file: &mut TokioFile,
    offset: u64,
    size: u64,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let size = usize::try_from(size).map_err(|_| MuxError::LayoutOverflow("IAMF OBU size"))?;
    let mut bytes = vec![0u8; size];
    read_exact_at_async(file, offset, &mut bytes, spec, truncated_message).await?;
    Ok(bytes)
}

fn is_iamf_temporal_unit_obu(obu_type: u8) -> bool {
    matches!(
        obu_type,
        OBU_IAMF_PARAMETER_BLOCK | OBU_IAMF_TEMPORAL_DELIMITER | OBU_IAMF_AUDIO_FRAME
    )
}

fn read_leb128_from_slice(
    bytes: &[u8],
    spec: &str,
    field_name: &str,
    offset: u64,
) -> Result<(u64, usize), MuxError> {
    let mut value = 0u64;
    let mut shift = 0u32;
    for (index, byte) in bytes.iter().copied().enumerate() {
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
        shift += 7;
        if shift >= 63 {
            break;
        }
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!(
            "{field_name} at byte offset {offset} used an unterminated or unsupported leb128 value"
        ),
    })
}

struct BitCursor<'a> {
    data: &'a [u8],
    bit_offset: usize,
}

impl<'a> BitCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_offset: 0,
        }
    }

    fn read_bits(&mut self, width: usize, spec: &str, label: &str) -> Result<u32, MuxError> {
        let end = self
            .bit_offset
            .checked_add(width)
            .ok_or(MuxError::LayoutOverflow("IAMF bit reader position"))?;
        if end > self.data.len() * 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated IAMF payload while reading {label}"),
            });
        }
        let mut value = 0u32;
        for _ in 0..width {
            let byte = self.data[self.bit_offset / 8];
            let shift = 7 - (self.bit_offset % 8);
            value = (value << 1) | u32::from((byte >> shift) & 0x01);
            self.bit_offset += 1;
        }
        Ok(value)
    }
}
