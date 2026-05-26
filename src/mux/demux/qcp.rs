use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::threegpp::{Devc, Dqcp, Dsmv};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    StagedSample, build_generic_audio_sample_entry_box, read_exact_at_sync,
};

const RIFF_MAGIC: &[u8; 4] = b"RIFF";
const QLCM_MAGIC: &[u8; 4] = b"QLCM";
const FMT_CHUNK: &[u8; 4] = b"fmt ";
const VRAT_CHUNK: &[u8; 4] = b"vrat";
const DATA_CHUNK: &[u8; 4] = b"data";
const QCP_FMT_MIN_SIZE: usize = 150;
const QCP_VRAT_MIN_SIZE: usize = 8;
const QCP_RATE_TABLE_CAP: usize = 8;
const SAMPLE_ENTRY_SQCP: FourCc = FourCc::from_bytes(*b"sqcp");
const SAMPLE_ENTRY_SEVC: FourCc = FourCc::from_bytes(*b"sevc");
const SAMPLE_ENTRY_SSMV: FourCc = FourCc::from_bytes(*b"ssmv");
const QCP_QCELP_GUID_1: [u8; 16] = [
    0x41, 0x6D, 0x7F, 0x5E, 0x15, 0xB1, 0xD0, 0x11, 0xBA, 0x91, 0x00, 0x80, 0x5F, 0xB4, 0xB9, 0x7E,
];
const QCP_QCELP_GUID_2: [u8; 16] = [
    0x42, 0x6D, 0x7F, 0x5E, 0x15, 0xB1, 0xD0, 0x11, 0xBA, 0x91, 0x00, 0x80, 0x5F, 0xB4, 0xB9, 0x7E,
];
const QCP_EVRC_GUID: [u8; 16] = [
    0x8D, 0xD4, 0x89, 0xE6, 0x76, 0x90, 0xB5, 0x46, 0x91, 0xEF, 0x73, 0x6A, 0x51, 0x00, 0xCE, 0xB4,
];
const QCP_SMV_GUID: [u8; 16] = [
    0x75, 0x2B, 0x7C, 0x8D, 0x97, 0xA7, 0x46, 0xED, 0x98, 0x5E, 0xD5, 0x3C, 0x8C, 0xC7, 0x5F, 0x84,
];

pub(in crate::mux) struct ParsedQcpTrack {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
    pub(in crate::mux) handler_label: &'static str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct QcpRateEntry {
    packet_size: u8,
    rate_index: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QcpCodecKind {
    Qcelp,
    Evrc,
    Smv,
}

impl QcpCodecKind {
    const fn handler_label(self) -> &'static str {
        match self {
            Self::Qcelp => "qcelp",
            Self::Evrc => "evrc",
            Self::Smv => "smv",
        }
    }

    const fn sample_entry_type(self) -> FourCc {
        match self {
            Self::Qcelp => SAMPLE_ENTRY_SQCP,
            Self::Evrc => SAMPLE_ENTRY_SEVC,
            Self::Smv => SAMPLE_ENTRY_SSMV,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ParsedQcpFormat {
    codec_kind: QcpCodecKind,
    sample_rate: u32,
    block_size: u32,
    packet_size: u32,
    vrat_rate_flag: u32,
    rate_table_count: usize,
    rate_table: [QcpRateEntry; QCP_RATE_TABLE_CAP],
    data_offset: u64,
    data_size: u32,
}

pub(in crate::mux) fn scan_qcp_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedQcpTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let parsed = parse_qcp_container_sync(&mut file, file_size, spec)?;
    let samples = parse_qcp_samples_sync(&mut file, parsed, spec)?;
    Ok(ParsedQcpTrack {
        sample_rate: parsed.sample_rate,
        sample_entry_box: build_qcp_sample_entry_box(parsed)?,
        samples,
        handler_label: parsed.codec_kind.handler_label(),
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_qcp_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedQcpTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let parsed = parse_qcp_container_async(&mut file, file_size, spec).await?;
    let samples = parse_qcp_samples_async(&mut file, parsed, spec).await?;
    Ok(ParsedQcpTrack {
        sample_rate: parsed.sample_rate,
        sample_entry_box: build_qcp_sample_entry_box(parsed)?,
        samples,
        handler_label: parsed.codec_kind.handler_label(),
    })
}

fn parse_qcp_container_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<ParsedQcpFormat, MuxError> {
    validate_qcp_file_header_sync(file, file_size, spec)?;
    parse_qcp_chunks_sync(file, file_size, spec)
}

#[cfg(feature = "async")]
async fn parse_qcp_container_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<ParsedQcpFormat, MuxError> {
    validate_qcp_file_header_async(file, file_size, spec).await?;
    parse_qcp_chunks_async(file, file_size, spec).await
}

fn parse_qcp_chunks_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<ParsedQcpFormat, MuxError> {
    let mut offset = 12_u64;
    let mut format = None;
    let mut vrat_rate_flag = None;
    let mut data_chunk = None;
    while offset < file_size {
        let remaining = file_size - offset;
        if remaining < 8 {
            return qcp_error(
                spec,
                "QCP input is truncated before a complete chunk header",
            );
        }
        let mut chunk_header = [0_u8; 8];
        read_exact_at_sync(
            file,
            offset,
            &mut chunk_header,
            spec,
            "truncated QCP chunk header",
        )?;
        let chunk_type = &chunk_header[..4];
        let chunk_size = u32::from_le_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]);
        let chunk_data_offset = offset + 8;
        let padded_size = u64::from(chunk_size) + u64::from(chunk_size & 1);
        let chunk_end = chunk_data_offset
            .checked_add(padded_size)
            .ok_or(MuxError::LayoutOverflow("QCP chunk end"))?;
        if chunk_end > file_size {
            return qcp_error(spec, "QCP chunk payload extends past the end of the file");
        }

        match chunk_type {
            chunk if chunk == FMT_CHUNK => {
                if format.is_some() {
                    return qcp_error(spec, "QCP input carried more than one fmt chunk");
                }
                let mut payload = vec![
                    0_u8;
                    usize::try_from(chunk_size).map_err(|_| {
                        MuxError::LayoutOverflow("QCP fmt chunk size")
                    })?
                ];
                read_exact_at_sync(
                    file,
                    chunk_data_offset,
                    &mut payload,
                    spec,
                    "truncated QCP fmt chunk",
                )?;
                format = Some(parse_qcp_format_payload(&payload, spec)?);
            }
            chunk if chunk == VRAT_CHUNK => {
                if vrat_rate_flag.is_some() {
                    return qcp_error(spec, "QCP input carried more than one vrat chunk");
                }
                if chunk_size < u32::try_from(QCP_VRAT_MIN_SIZE).unwrap() {
                    return qcp_error(spec, "QCP vrat chunk was smaller than eight bytes");
                }
                let mut payload = [0_u8; 8];
                read_exact_at_sync(
                    file,
                    chunk_data_offset,
                    &mut payload,
                    spec,
                    "truncated QCP vrat chunk",
                )?;
                vrat_rate_flag = Some(u32::from_le_bytes([
                    payload[0], payload[1], payload[2], payload[3],
                ]));
            }
            chunk if chunk == DATA_CHUNK => {
                if data_chunk.is_some() {
                    return qcp_error(spec, "QCP input carried more than one data chunk");
                }
                data_chunk = Some((chunk_data_offset, chunk_size));
            }
            _ => {}
        }

        offset = chunk_end;
    }

    finalize_qcp_format(spec, format, vrat_rate_flag, data_chunk)
}

#[cfg(feature = "async")]
async fn parse_qcp_chunks_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<ParsedQcpFormat, MuxError> {
    let mut offset = 12_u64;
    let mut format = None;
    let mut vrat_rate_flag = None;
    let mut data_chunk = None;
    while offset < file_size {
        let remaining = file_size - offset;
        if remaining < 8 {
            return qcp_error(
                spec,
                "QCP input is truncated before a complete chunk header",
            );
        }
        let mut chunk_header = [0_u8; 8];
        read_exact_at_async(
            file,
            offset,
            &mut chunk_header,
            spec,
            "truncated QCP chunk header",
        )
        .await?;
        let chunk_type = &chunk_header[..4];
        let chunk_size = u32::from_le_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]);
        let chunk_data_offset = offset + 8;
        let padded_size = u64::from(chunk_size) + u64::from(chunk_size & 1);
        let chunk_end = chunk_data_offset
            .checked_add(padded_size)
            .ok_or(MuxError::LayoutOverflow("QCP chunk end"))?;
        if chunk_end > file_size {
            return qcp_error(spec, "QCP chunk payload extends past the end of the file");
        }

        match chunk_type {
            chunk if chunk == FMT_CHUNK => {
                if format.is_some() {
                    return qcp_error(spec, "QCP input carried more than one fmt chunk");
                }
                let mut payload = vec![
                    0_u8;
                    usize::try_from(chunk_size).map_err(|_| {
                        MuxError::LayoutOverflow("QCP fmt chunk size")
                    })?
                ];
                read_exact_at_async(
                    file,
                    chunk_data_offset,
                    &mut payload,
                    spec,
                    "truncated QCP fmt chunk",
                )
                .await?;
                format = Some(parse_qcp_format_payload(&payload, spec)?);
            }
            chunk if chunk == VRAT_CHUNK => {
                if vrat_rate_flag.is_some() {
                    return qcp_error(spec, "QCP input carried more than one vrat chunk");
                }
                if chunk_size < u32::try_from(QCP_VRAT_MIN_SIZE).unwrap() {
                    return qcp_error(spec, "QCP vrat chunk was smaller than eight bytes");
                }
                let mut payload = [0_u8; 8];
                read_exact_at_async(
                    file,
                    chunk_data_offset,
                    &mut payload,
                    spec,
                    "truncated QCP vrat chunk",
                )
                .await?;
                vrat_rate_flag = Some(u32::from_le_bytes([
                    payload[0], payload[1], payload[2], payload[3],
                ]));
            }
            chunk if chunk == DATA_CHUNK => {
                if data_chunk.is_some() {
                    return qcp_error(spec, "QCP input carried more than one data chunk");
                }
                data_chunk = Some((chunk_data_offset, chunk_size));
            }
            _ => {}
        }

        offset = chunk_end;
    }

    finalize_qcp_format(spec, format, vrat_rate_flag, data_chunk)
}

fn finalize_qcp_format(
    spec: &str,
    format: Option<ParsedQcpFormat>,
    vrat_rate_flag: Option<u32>,
    data_chunk: Option<(u64, u32)>,
) -> Result<ParsedQcpFormat, MuxError> {
    let mut format = format.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "QCP input did not carry a fmt chunk".to_string(),
    })?;
    let vrat_rate_flag = vrat_rate_flag.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "QCP input did not carry a vrat chunk".to_string(),
    })?;
    let (data_offset, data_size) = data_chunk.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "QCP input did not carry a data chunk".to_string(),
    })?;
    if data_size == 0 {
        return qcp_error(spec, "QCP data chunk did not contain any codec packets");
    }
    if vrat_rate_flag != 0 && format.rate_table_count == 0 {
        return qcp_error(
            spec,
            "QCP input marked variable-rate packets but did not carry any usable rate-table entries",
        );
    }
    if vrat_rate_flag == 0 && format.packet_size == 0 {
        return qcp_error(
            spec,
            "QCP input marked constant-rate packets but declared a zero packet size",
        );
    }
    format.vrat_rate_flag = vrat_rate_flag;
    format.data_offset = data_offset;
    format.data_size = data_size;
    Ok(format)
}

fn parse_qcp_format_payload(payload: &[u8], spec: &str) -> Result<ParsedQcpFormat, MuxError> {
    if payload.len() < QCP_FMT_MIN_SIZE {
        return qcp_error(
            spec,
            "QCP fmt chunk was smaller than the required 150 bytes",
        );
    }
    let guid =
        <[u8; 16]>::try_from(&payload[2..18]).map_err(|_| MuxError::LayoutOverflow("QCP GUID"))?;
    let codec_kind = parse_qcp_codec_kind(guid, spec)?;
    let packet_size = u32::from(u16::from_le_bytes([payload[102], payload[103]]));
    let block_size = u32::from(u16::from_le_bytes([payload[104], payload[105]]));
    let sample_rate = u32::from(u16::from_le_bytes([payload[106], payload[107]]));
    if block_size == 0 {
        return qcp_error(
            spec,
            "QCP fmt chunk declared a zero samples-per-packet block size",
        );
    }
    if sample_rate == 0 {
        return qcp_error(spec, "QCP fmt chunk declared a zero sample rate");
    }
    let rate_table_count = usize::try_from(u32::from_le_bytes([
        payload[110],
        payload[111],
        payload[112],
        payload[113],
    ]))
    .map_err(|_| MuxError::LayoutOverflow("QCP rate-table count"))?;
    if rate_table_count > QCP_RATE_TABLE_CAP {
        return qcp_error(
            spec,
            "QCP fmt chunk declared more than eight rate-table entries",
        );
    }
    let mut rate_table = [QcpRateEntry::default(); QCP_RATE_TABLE_CAP];
    for (index, entry) in rate_table.iter_mut().enumerate() {
        let offset = 114 + index * 2;
        *entry = QcpRateEntry {
            packet_size: payload[offset],
            rate_index: payload[offset + 1],
        };
    }
    Ok(ParsedQcpFormat {
        codec_kind,
        sample_rate,
        block_size,
        packet_size,
        vrat_rate_flag: 0,
        rate_table_count,
        rate_table,
        data_offset: 0,
        data_size: 0,
    })
}

fn parse_qcp_codec_kind(guid: [u8; 16], spec: &str) -> Result<QcpCodecKind, MuxError> {
    match guid {
        QCP_QCELP_GUID_1 | QCP_QCELP_GUID_2 => Ok(QcpCodecKind::Qcelp),
        QCP_EVRC_GUID => Ok(QcpCodecKind::Evrc),
        QCP_SMV_GUID => Ok(QcpCodecKind::Smv),
        _ => qcp_error(spec, "QCP input carried an unsupported codec GUID"),
    }
}

fn parse_qcp_samples_sync(
    file: &mut File,
    format: ParsedQcpFormat,
    spec: &str,
) -> Result<Vec<StagedSample>, MuxError> {
    let mut samples = Vec::new();
    let mut offset = format.data_offset;
    let mut remaining = u64::from(format.data_size);
    while remaining > 0 {
        let packet_size = if format.vrat_rate_flag != 0 {
            let mut rate_index = [0_u8; 1];
            read_exact_at_sync(
                file,
                offset,
                &mut rate_index,
                spec,
                "truncated QCP variable-rate packet header",
            )?;
            resolve_qcp_variable_packet_size(format, rate_index[0], spec)?
        } else {
            format.packet_size
        };
        if packet_size == 0 {
            return qcp_error(spec, "QCP packet parser produced a zero packet size");
        }
        if u64::from(packet_size) > remaining {
            return qcp_error(spec, "QCP data chunk ended in the middle of a codec packet");
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: packet_size,
            duration: format.block_size,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(u64::from(packet_size))
            .ok_or(MuxError::LayoutOverflow("QCP packet offset"))?;
        remaining -= u64::from(packet_size);
    }
    if samples.is_empty() {
        return qcp_error(spec, "QCP data chunk did not contain any codec packets");
    }
    Ok(samples)
}

#[cfg(feature = "async")]
async fn parse_qcp_samples_async(
    file: &mut TokioFile,
    format: ParsedQcpFormat,
    spec: &str,
) -> Result<Vec<StagedSample>, MuxError> {
    let mut samples = Vec::new();
    let mut offset = format.data_offset;
    let mut remaining = u64::from(format.data_size);
    while remaining > 0 {
        let packet_size = if format.vrat_rate_flag != 0 {
            let mut rate_index = [0_u8; 1];
            read_exact_at_async(
                file,
                offset,
                &mut rate_index,
                spec,
                "truncated QCP variable-rate packet header",
            )
            .await?;
            resolve_qcp_variable_packet_size(format, rate_index[0], spec)?
        } else {
            format.packet_size
        };
        if packet_size == 0 {
            return qcp_error(spec, "QCP packet parser produced a zero packet size");
        }
        if u64::from(packet_size) > remaining {
            return qcp_error(spec, "QCP data chunk ended in the middle of a codec packet");
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: packet_size,
            duration: format.block_size,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(u64::from(packet_size))
            .ok_or(MuxError::LayoutOverflow("QCP packet offset"))?;
        remaining -= u64::from(packet_size);
    }
    if samples.is_empty() {
        return qcp_error(spec, "QCP data chunk did not contain any codec packets");
    }
    Ok(samples)
}

fn resolve_qcp_variable_packet_size(
    format: ParsedQcpFormat,
    rate_index: u8,
    spec: &str,
) -> Result<u32, MuxError> {
    let payload_size = format.rate_table[..format.rate_table_count]
        .iter()
        .find(|entry| entry.rate_index == rate_index)
        .map(|entry| u32::from(entry.packet_size))
        .ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("QCP input used unknown variable-rate index {rate_index}"),
        })?;
    if payload_size == 0 {
        return qcp_error(
            spec,
            "QCP input used a variable-rate index whose table entry declared a zero payload size",
        );
    }
    payload_size
        .checked_add(1)
        .ok_or(MuxError::LayoutOverflow("QCP packet size"))
}

fn validate_qcp_file_header_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if file_size < 12 {
        return qcp_error(
            spec,
            "QCP input is truncated before the RIFF or QLCM header",
        );
    }
    let mut header = [0_u8; 12];
    read_exact_at_sync(file, 0, &mut header, spec, "truncated QCP file header")?;
    validate_qcp_file_header_bytes(&header, file_size, spec)
}

#[cfg(feature = "async")]
async fn validate_qcp_file_header_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if file_size < 12 {
        return qcp_error(
            spec,
            "QCP input is truncated before the RIFF or QLCM header",
        );
    }
    let mut header = [0_u8; 12];
    read_exact_at_async(file, 0, &mut header, spec, "truncated QCP file header").await?;
    validate_qcp_file_header_bytes(&header, file_size, spec)
}

fn validate_qcp_file_header_bytes(
    header: &[u8; 12],
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if &header[..4] != RIFF_MAGIC || &header[8..12] != QLCM_MAGIC {
        return qcp_error(spec, "QCP input did not start with a RIFF or QLCM header");
    }
    let declared_riff_size = u64::from(u32::from_le_bytes([
        header[4], header[5], header[6], header[7],
    ]));
    if declared_riff_size
        .checked_add(8)
        .is_none_or(|total| total > file_size)
    {
        return qcp_error(
            spec,
            "QCP input declared a RIFF payload size larger than the file itself",
        );
    }
    Ok(())
}

fn build_qcp_sample_entry_box(format: ParsedQcpFormat) -> Result<Vec<u8>, MuxError> {
    let config_box = match format.codec_kind {
        QcpCodecKind::Qcelp => super::super::mp4::encode_typed_box(
            &Dqcp {
                vendor: 0,
                decoder_version: 0,
                frames_per_sample: 1,
            },
            &[],
        )?,
        QcpCodecKind::Evrc => super::super::mp4::encode_typed_box(
            &Devc {
                vendor: 0,
                decoder_version: 0,
                frames_per_sample: 1,
            },
            &[],
        )?,
        QcpCodecKind::Smv => super::super::mp4::encode_typed_box(
            &Dsmv {
                vendor: 0,
                decoder_version: 0,
                frames_per_sample: 1,
            },
            &[],
        )?,
    };
    build_generic_audio_sample_entry_box(
        format.codec_kind.sample_entry_type(),
        format.sample_rate,
        1,
        16,
        &[config_box],
    )
}

fn qcp_error<T>(spec: &str, message: impl Into<String>) -> Result<T, MuxError> {
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.into(),
    })
}
