use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::iso14496_12::Chnl;
use crate::boxes::iso23001_5::PcmC;

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData, SegmentedMuxSourceSpec,
    build_generic_audio_sample_entry_box, read_exact_at_sync,
};

const FORM: &[u8; 4] = b"FORM";
const AIFF: &[u8; 4] = b"AIFF";
const AIFC: &[u8; 4] = b"AIFC";
const COMM: &[u8; 4] = b"COMM";
const SSND: &[u8; 4] = b"SSND";
const RIFF: &[u8; 4] = b"RIFF";
const WAVE: &[u8; 4] = b"WAVE";
const FMT: &[u8; 4] = b"fmt ";
const DATA: &[u8; 4] = b"data";
const WAVE_FORMAT_PCM: u16 = 0x0001;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const AIFC_COMPRESSION_NONE: FourCc = FourCc::from_bytes(*b"NONE");
const AIFC_COMPRESSION_TWOS: FourCc = FourCc::from_bytes(*b"twos");
const AIFC_COMPRESSION_ALAW: FourCc = FourCc::from_bytes(*b"ALAW");
const AIFC_COMPRESSION_ULAW: FourCc = FourCc::from_bytes(*b"ULAW");
const AIFC_COMPRESSION_FL32: FourCc = FourCc::from_bytes(*b"fl32");
const AIFC_COMPRESSION_FL64: FourCc = FourCc::from_bytes(*b"fl64");
const SAMPLE_ENTRY_IPCM: FourCc = FourCc::from_bytes(*b"ipcm");
const SAMPLE_ENTRY_FPCM: FourCc = FourCc::from_bytes(*b"fpcm");
const AIFC_FLOAT_VENDOR_CODE: [u8; 4] = *b"GPAC";
const KSDATAFORMAT_SUBTYPE_PCM: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: [u8; 16] = [
    0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

pub(in crate::mux) struct ParsedPcmTrack {
    pub(in crate::mux) container_kind: PcmContainerKind,
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) data_offset: u64,
    pub(in crate::mux) frame_size: u32,
    pub(in crate::mux) frame_count: u32,
    pub(in crate::mux) transformed_source: Option<SegmentedMuxSourceSpec>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mux) enum PcmContainerKind {
    Wave,
    Aiff,
    Aifc,
}

#[derive(Clone, Copy)]
enum CompandedPcmKind {
    Alaw,
    Ulaw,
}

#[derive(Clone, Copy)]
struct ParsedPcmFormat {
    sample_entry_type: FourCc,
    sample_rate: u32,
    channel_count: u16,
    bits_per_sample: u16,
    block_align: u16,
    is_little_endian: bool,
    companded_kind: Option<CompandedPcmKind>,
}

#[derive(Clone, Copy)]
struct ParsedAiffCommonChunk {
    format: ParsedPcmFormat,
    declared_sample_frames: u32,
}

pub(in crate::mux) fn scan_pcm_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedPcmTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_pcm_stream_sync(path, &mut file, file_size, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_pcm_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedPcmTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_pcm_stream_async(path, &mut file, file_size, spec).await
}

fn parse_pcm_stream_sync(
    path: &Path,
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<ParsedPcmTrack, MuxError> {
    if file_size < 12 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "PCM input is truncated before the 12-byte container header".to_string(),
        });
    }
    let mut header = [0_u8; 12];
    read_exact_at_sync(
        file,
        0,
        &mut header,
        spec,
        "PCM input is truncated before the 12-byte container header",
    )?;
    if &header[..4] == RIFF && &header[8..12] == WAVE {
        validate_riff_wave_header(&header, file_size, spec)?;
        let (format, data_offset, data_size) = parse_wave_chunks_sync(file, file_size, spec)?;
        return finalize_pcm_track_sync(
            path,
            file,
            PcmContainerKind::Wave,
            format,
            data_offset,
            data_size,
            None,
            spec,
        );
    }
    if &header[..4] == FORM && (&header[8..12] == AIFF || &header[8..12] == AIFC) {
        validate_aiff_form_header(&header, file_size, spec)?;
        let is_aifc = &header[8..12] == AIFC;
        let (common, data_offset, data_size) =
            parse_aiff_chunks_sync(file, file_size, is_aifc, spec)?;
        return finalize_pcm_track_sync(
            path,
            file,
            if is_aifc {
                PcmContainerKind::Aifc
            } else {
                PcmContainerKind::Aiff
            },
            common.format,
            data_offset,
            data_size,
            Some(common.declared_sample_frames),
            spec,
        );
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "PCM direct ingest currently supports WAVE, AIFF, and AIFC inputs".to_string(),
    })
}

#[cfg(feature = "async")]
async fn parse_pcm_stream_async(
    path: &Path,
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<ParsedPcmTrack, MuxError> {
    if file_size < 12 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "PCM input is truncated before the 12-byte container header".to_string(),
        });
    }
    let mut header = [0_u8; 12];
    read_exact_at_async(
        file,
        0,
        &mut header,
        spec,
        "PCM input is truncated before the 12-byte container header",
    )
    .await?;
    if &header[..4] == RIFF && &header[8..12] == WAVE {
        validate_riff_wave_header(&header, file_size, spec)?;
        let (format, data_offset, data_size) =
            parse_wave_chunks_async(file, file_size, spec).await?;
        return finalize_pcm_track_async(
            path,
            file,
            PcmContainerKind::Wave,
            format,
            data_offset,
            data_size,
            None,
            spec,
        )
        .await;
    }
    if &header[..4] == FORM && (&header[8..12] == AIFF || &header[8..12] == AIFC) {
        validate_aiff_form_header(&header, file_size, spec)?;
        let is_aifc = &header[8..12] == AIFC;
        let (common, data_offset, data_size) =
            parse_aiff_chunks_async(file, file_size, is_aifc, spec).await?;
        return finalize_pcm_track_async(
            path,
            file,
            if is_aifc {
                PcmContainerKind::Aifc
            } else {
                PcmContainerKind::Aiff
            },
            common.format,
            data_offset,
            data_size,
            Some(common.declared_sample_frames),
            spec,
        )
        .await;
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "PCM direct ingest currently supports WAVE, AIFF, and AIFC inputs".to_string(),
    })
}

fn validate_riff_wave_header(
    riff_header: &[u8; 12],
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if &riff_header[..4] != RIFF {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "WAVE input did not start with the `RIFF` signature".to_string(),
        });
    }
    if &riff_header[8..12] != WAVE {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "WAVE input did not carry the `WAVE` RIFF form type".to_string(),
        });
    }
    let declared_size = u64::from(u32::from_le_bytes(riff_header[4..8].try_into().unwrap())) + 8;
    if declared_size > file_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "WAVE RIFF size field declared {declared_size} bytes but the file only contains {file_size}"
            ),
        });
    }
    Ok(())
}

fn parse_wave_chunks_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<(ParsedPcmFormat, u64, u32), MuxError> {
    let mut chunk_offset = 12_u64;
    let mut format = None::<ParsedPcmFormat>;
    let mut data = None::<(u64, u32)>;
    while chunk_offset < file_size {
        if file_size - chunk_offset < 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "WAVE chunk header is truncated".to_string(),
            });
        }
        let mut chunk_header = [0_u8; 8];
        read_exact_at_sync(
            file,
            chunk_offset,
            &mut chunk_header,
            spec,
            "WAVE chunk header is truncated",
        )?;
        let chunk_type = &chunk_header[..4];
        let chunk_size = u64::from(u32::from_le_bytes(chunk_header[4..8].try_into().unwrap()));
        let chunk_payload_offset = chunk_offset + 8;
        let chunk_end = chunk_payload_offset
            .checked_add(chunk_size)
            .ok_or(MuxError::LayoutOverflow("WAVE chunk range"))?;
        if chunk_end > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "WAVE chunk `{}` overruns the input length",
                    String::from_utf8_lossy(chunk_type)
                ),
            });
        }
        if chunk_type == FMT {
            if format.is_some() {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "WAVE input carried more than one `fmt ` chunk".to_string(),
                });
            }
            format = Some(parse_wave_format_chunk_sync(
                file,
                chunk_payload_offset,
                chunk_size,
                spec,
            )?);
        } else if chunk_type == DATA {
            if data.is_some() {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "WAVE input carried more than one `data` chunk".to_string(),
                });
            }
            data = Some((
                chunk_payload_offset,
                u32::try_from(chunk_size)
                    .map_err(|_| MuxError::LayoutOverflow("WAVE data size"))?,
            ));
        }
        chunk_offset = chunk_end + (chunk_size & 1);
    }
    let format = format.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "WAVE input did not contain a `fmt ` chunk".to_string(),
    })?;
    let (data_offset, data_size) = data.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "WAVE input did not contain a `data` chunk".to_string(),
    })?;
    Ok((format, data_offset, data_size))
}

#[cfg(feature = "async")]
async fn parse_wave_chunks_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<(ParsedPcmFormat, u64, u32), MuxError> {
    let mut chunk_offset = 12_u64;
    let mut format = None::<ParsedPcmFormat>;
    let mut data = None::<(u64, u32)>;
    while chunk_offset < file_size {
        if file_size - chunk_offset < 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "WAVE chunk header is truncated".to_string(),
            });
        }
        let mut chunk_header = [0_u8; 8];
        read_exact_at_async(
            file,
            chunk_offset,
            &mut chunk_header,
            spec,
            "WAVE chunk header is truncated",
        )
        .await?;
        let chunk_type = &chunk_header[..4];
        let chunk_size = u64::from(u32::from_le_bytes(chunk_header[4..8].try_into().unwrap()));
        let chunk_payload_offset = chunk_offset + 8;
        let chunk_end = chunk_payload_offset
            .checked_add(chunk_size)
            .ok_or(MuxError::LayoutOverflow("WAVE chunk range"))?;
        if chunk_end > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "WAVE chunk `{}` overruns the input length",
                    String::from_utf8_lossy(chunk_type)
                ),
            });
        }
        if chunk_type == FMT {
            if format.is_some() {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "WAVE input carried more than one `fmt ` chunk".to_string(),
                });
            }
            format = Some(
                parse_wave_format_chunk_async(file, chunk_payload_offset, chunk_size, spec).await?,
            );
        } else if chunk_type == DATA {
            if data.is_some() {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "WAVE input carried more than one `data` chunk".to_string(),
                });
            }
            data = Some((
                chunk_payload_offset,
                u32::try_from(chunk_size)
                    .map_err(|_| MuxError::LayoutOverflow("WAVE data size"))?,
            ));
        }
        chunk_offset = chunk_end + (chunk_size & 1);
    }
    let format = format.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "WAVE input did not contain a `fmt ` chunk".to_string(),
    })?;
    let (data_offset, data_size) = data.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "WAVE input did not contain a `data` chunk".to_string(),
    })?;
    Ok((format, data_offset, data_size))
}

fn parse_wave_format_chunk_sync(
    file: &mut File,
    offset: u64,
    chunk_size: u64,
    spec: &str,
) -> Result<ParsedPcmFormat, MuxError> {
    let mut bytes = vec![
        0_u8;
        usize::try_from(chunk_size)
            .map_err(|_| MuxError::LayoutOverflow("WAVE fmt chunk size"))?
    ];
    read_exact_at_sync(
        file,
        offset,
        &mut bytes,
        spec,
        "WAVE `fmt ` chunk is truncated",
    )?;
    parse_wave_format_chunk_bytes(&bytes, spec)
}

#[cfg(feature = "async")]
async fn parse_wave_format_chunk_async(
    file: &mut TokioFile,
    offset: u64,
    chunk_size: u64,
    spec: &str,
) -> Result<ParsedPcmFormat, MuxError> {
    let mut bytes = vec![
        0_u8;
        usize::try_from(chunk_size)
            .map_err(|_| MuxError::LayoutOverflow("WAVE fmt chunk size"))?
    ];
    read_exact_at_async(
        file,
        offset,
        &mut bytes,
        spec,
        "WAVE `fmt ` chunk is truncated",
    )
    .await?;
    parse_wave_format_chunk_bytes(&bytes, spec)
}

fn parse_wave_format_chunk_bytes(bytes: &[u8], spec: &str) -> Result<ParsedPcmFormat, MuxError> {
    if bytes.len() < 16 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "WAVE `fmt ` chunk is truncated before 16 bytes".to_string(),
        });
    }
    let audio_format = u16::from_le_bytes(bytes[0..2].try_into().unwrap());
    let channel_count = u16::from_le_bytes(bytes[2..4].try_into().unwrap());
    let sample_rate = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let byte_rate = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let block_align = u16::from_le_bytes(bytes[12..14].try_into().unwrap());
    let bits_per_sample = u16::from_le_bytes(bytes[14..16].try_into().unwrap());
    if channel_count == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "WAVE `fmt ` chunk used zero channels".to_string(),
        });
    }
    if sample_rate == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "WAVE `fmt ` chunk used a zero sample rate".to_string(),
        });
    }
    if bits_per_sample == 0 || bits_per_sample % 8 != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "unsupported WAVE bits-per-sample value {bits_per_sample}; only byte-aligned PCM or float samples are supported"
            ),
        });
    }

    let parsed = match audio_format {
        WAVE_FORMAT_PCM => parse_pcm_format(
            bits_per_sample,
            channel_count,
            sample_rate,
            block_align,
            byte_rate,
            spec,
        )?,
        WAVE_FORMAT_IEEE_FLOAT => parse_float_format(
            bits_per_sample,
            channel_count,
            sample_rate,
            block_align,
            byte_rate,
            spec,
        )?,
        WAVE_FORMAT_EXTENSIBLE => parse_extensible_format(
            bytes,
            bits_per_sample,
            channel_count,
            sample_rate,
            block_align,
            byte_rate,
            spec,
        )?,
        other => {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported WAVE format tag {other:#06x}"),
            });
        }
    };
    Ok(parsed)
}

fn parse_pcm_format(
    bits_per_sample: u16,
    channel_count: u16,
    sample_rate: u32,
    block_align: u16,
    byte_rate: u32,
    spec: &str,
) -> Result<ParsedPcmFormat, MuxError> {
    if !matches!(bits_per_sample, 8 | 16 | 24 | 32 | 64) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported integer PCM sample size {bits_per_sample}"),
        });
    }
    validate_wave_stride(
        bits_per_sample,
        channel_count,
        sample_rate,
        block_align,
        byte_rate,
        spec,
    )?;
    Ok(ParsedPcmFormat {
        sample_entry_type: SAMPLE_ENTRY_IPCM,
        sample_rate,
        channel_count,
        bits_per_sample,
        block_align,
        is_little_endian: true,
        companded_kind: None,
    })
}

fn parse_float_format(
    bits_per_sample: u16,
    channel_count: u16,
    sample_rate: u32,
    block_align: u16,
    byte_rate: u32,
    spec: &str,
) -> Result<ParsedPcmFormat, MuxError> {
    if !matches!(bits_per_sample, 32 | 64) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported floating-point PCM sample size {bits_per_sample}"),
        });
    }
    validate_wave_stride(
        bits_per_sample,
        channel_count,
        sample_rate,
        block_align,
        byte_rate,
        spec,
    )?;
    Ok(ParsedPcmFormat {
        sample_entry_type: SAMPLE_ENTRY_FPCM,
        sample_rate,
        channel_count,
        bits_per_sample,
        block_align,
        is_little_endian: true,
        companded_kind: None,
    })
}

fn parse_extensible_format(
    bytes: &[u8],
    bits_per_sample: u16,
    channel_count: u16,
    sample_rate: u32,
    block_align: u16,
    byte_rate: u32,
    spec: &str,
) -> Result<ParsedPcmFormat, MuxError> {
    if bytes.len() < 40 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "WAVE extensible `fmt ` chunk is truncated before the subtype GUID"
                .to_string(),
        });
    }
    let cb_size = u16::from_le_bytes(bytes[16..18].try_into().unwrap());
    if cb_size < 22 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "WAVE extensible `fmt ` chunk declared an unsupported extension size of {cb_size}"
            ),
        });
    }
    let subformat = &bytes[24..40];
    if subformat == KSDATAFORMAT_SUBTYPE_PCM {
        parse_pcm_format(
            bits_per_sample,
            channel_count,
            sample_rate,
            block_align,
            byte_rate,
            spec,
        )
    } else if subformat == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
        parse_float_format(
            bits_per_sample,
            channel_count,
            sample_rate,
            block_align,
            byte_rate,
            spec,
        )
    } else {
        Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "unsupported WAVE extensible subtype GUID".to_string(),
        })
    }
}

fn validate_wave_stride(
    bits_per_sample: u16,
    channel_count: u16,
    sample_rate: u32,
    block_align: u16,
    byte_rate: u32,
    spec: &str,
) -> Result<(), MuxError> {
    let expected_block_align = u32::from(channel_count)
        .checked_mul(u32::from(bits_per_sample / 8))
        .ok_or(MuxError::LayoutOverflow("WAVE block alignment"))?;
    if u32::from(block_align) != expected_block_align {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "WAVE `fmt ` chunk declared block alignment {block_align}, but {expected_block_align} is required for {channel_count} channels at {bits_per_sample} bits"
            ),
        });
    }
    let expected_byte_rate = sample_rate
        .checked_mul(expected_block_align)
        .ok_or(MuxError::LayoutOverflow("WAVE byte rate"))?;
    if byte_rate != expected_byte_rate {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "WAVE `fmt ` chunk declared byte rate {byte_rate}, but {expected_byte_rate} is required for the declared sample rate and block alignment"
            ),
        });
    }
    Ok(())
}

fn validate_aiff_form_header(
    form_header: &[u8; 12],
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if &form_header[..4] != FORM {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AIFF input did not start with the `FORM` signature".to_string(),
        });
    }
    if &form_header[8..12] != AIFF && &form_header[8..12] != AIFC {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AIFF input did not carry the `AIFF` or `AIFC` form type".to_string(),
        });
    }
    let declared_size = u64::from(u32::from_be_bytes(form_header[4..8].try_into().unwrap())) + 8;
    if declared_size > file_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "AIFF FORM size field declared {declared_size} bytes but the file only contains {file_size}"
            ),
        });
    }
    Ok(())
}

fn parse_aiff_chunks_sync(
    file: &mut File,
    file_size: u64,
    is_aifc: bool,
    spec: &str,
) -> Result<(ParsedAiffCommonChunk, u64, u32), MuxError> {
    let mut chunk_offset = 12_u64;
    let mut common = None::<ParsedAiffCommonChunk>;
    let mut data = None::<(u64, u32)>;
    while chunk_offset < file_size {
        if file_size - chunk_offset < 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "AIFF chunk header is truncated".to_string(),
            });
        }
        let mut chunk_header = [0_u8; 8];
        read_exact_at_sync(
            file,
            chunk_offset,
            &mut chunk_header,
            spec,
            "AIFF chunk header is truncated",
        )?;
        let chunk_type = &chunk_header[..4];
        let chunk_size = u64::from(u32::from_be_bytes(chunk_header[4..8].try_into().unwrap()));
        let chunk_payload_offset = chunk_offset + 8;
        let chunk_end = chunk_payload_offset
            .checked_add(chunk_size)
            .ok_or(MuxError::LayoutOverflow("AIFF chunk range"))?;
        if chunk_end > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "AIFF chunk `{}` overruns the input length",
                    String::from_utf8_lossy(chunk_type)
                ),
            });
        }
        if chunk_type == COMM {
            if common.is_some() {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "AIFF input carried more than one `COMM` chunk".to_string(),
                });
            }
            common = Some(parse_aiff_common_chunk_sync(
                file,
                chunk_payload_offset,
                chunk_size,
                is_aifc,
                spec,
            )?);
        } else if chunk_type == SSND {
            if data.is_some() {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "AIFF input carried more than one `SSND` chunk".to_string(),
                });
            }
            data = Some(parse_aiff_sound_data_chunk_sync(
                file,
                chunk_payload_offset,
                chunk_size,
                spec,
            )?);
        }
        chunk_offset = chunk_end + (chunk_size & 1);
    }
    let common = common.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "AIFF input did not contain a `COMM` chunk".to_string(),
    })?;
    let (data_offset, data_size) = data.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "AIFF input did not contain an `SSND` chunk".to_string(),
    })?;
    Ok((common, data_offset, data_size))
}

#[cfg(feature = "async")]
async fn parse_aiff_chunks_async(
    file: &mut TokioFile,
    file_size: u64,
    is_aifc: bool,
    spec: &str,
) -> Result<(ParsedAiffCommonChunk, u64, u32), MuxError> {
    let mut chunk_offset = 12_u64;
    let mut common = None::<ParsedAiffCommonChunk>;
    let mut data = None::<(u64, u32)>;
    while chunk_offset < file_size {
        if file_size - chunk_offset < 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "AIFF chunk header is truncated".to_string(),
            });
        }
        let mut chunk_header = [0_u8; 8];
        read_exact_at_async(
            file,
            chunk_offset,
            &mut chunk_header,
            spec,
            "AIFF chunk header is truncated",
        )
        .await?;
        let chunk_type = &chunk_header[..4];
        let chunk_size = u64::from(u32::from_be_bytes(chunk_header[4..8].try_into().unwrap()));
        let chunk_payload_offset = chunk_offset + 8;
        let chunk_end = chunk_payload_offset
            .checked_add(chunk_size)
            .ok_or(MuxError::LayoutOverflow("AIFF chunk range"))?;
        if chunk_end > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "AIFF chunk `{}` overruns the input length",
                    String::from_utf8_lossy(chunk_type)
                ),
            });
        }
        if chunk_type == COMM {
            if common.is_some() {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "AIFF input carried more than one `COMM` chunk".to_string(),
                });
            }
            common = Some(
                parse_aiff_common_chunk_async(
                    file,
                    chunk_payload_offset,
                    chunk_size,
                    is_aifc,
                    spec,
                )
                .await?,
            );
        } else if chunk_type == SSND {
            if data.is_some() {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "AIFF input carried more than one `SSND` chunk".to_string(),
                });
            }
            data = Some(
                parse_aiff_sound_data_chunk_async(file, chunk_payload_offset, chunk_size, spec)
                    .await?,
            );
        }
        chunk_offset = chunk_end + (chunk_size & 1);
    }
    let common = common.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "AIFF input did not contain a `COMM` chunk".to_string(),
    })?;
    let (data_offset, data_size) = data.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "AIFF input did not contain an `SSND` chunk".to_string(),
    })?;
    Ok((common, data_offset, data_size))
}

fn parse_aiff_common_chunk_sync(
    file: &mut File,
    offset: u64,
    chunk_size: u64,
    is_aifc: bool,
    spec: &str,
) -> Result<ParsedAiffCommonChunk, MuxError> {
    let mut bytes = vec![
        0_u8;
        usize::try_from(chunk_size)
            .map_err(|_| MuxError::LayoutOverflow("AIFF COMM chunk size"))?
    ];
    read_exact_at_sync(
        file,
        offset,
        &mut bytes,
        spec,
        "AIFF `COMM` chunk is truncated",
    )?;
    parse_aiff_common_chunk_bytes(&bytes, is_aifc, spec)
}

#[cfg(feature = "async")]
async fn parse_aiff_common_chunk_async(
    file: &mut TokioFile,
    offset: u64,
    chunk_size: u64,
    is_aifc: bool,
    spec: &str,
) -> Result<ParsedAiffCommonChunk, MuxError> {
    let mut bytes = vec![
        0_u8;
        usize::try_from(chunk_size)
            .map_err(|_| MuxError::LayoutOverflow("AIFF COMM chunk size"))?
    ];
    read_exact_at_async(
        file,
        offset,
        &mut bytes,
        spec,
        "AIFF `COMM` chunk is truncated",
    )
    .await?;
    parse_aiff_common_chunk_bytes(&bytes, is_aifc, spec)
}

fn parse_aiff_common_chunk_bytes(
    bytes: &[u8],
    is_aifc: bool,
    spec: &str,
) -> Result<ParsedAiffCommonChunk, MuxError> {
    let minimum = if is_aifc { 22 } else { 18 };
    if bytes.len() < minimum {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("AIFF `COMM` chunk is truncated before {minimum} bytes"),
        });
    }
    let channel_count = u16::from_be_bytes(bytes[0..2].try_into().unwrap());
    let declared_sample_frames = u32::from_be_bytes(bytes[2..6].try_into().unwrap());
    let bits_per_sample = u16::from_be_bytes(bytes[6..8].try_into().unwrap());
    let sample_rate = decode_aiff_extended_sample_rate(&bytes[8..18], spec)?;
    if channel_count == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AIFF `COMM` chunk used zero channels".to_string(),
        });
    }
    if bits_per_sample == 0 || bits_per_sample % 8 != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "unsupported AIFF bits-per-sample value {bits_per_sample}; only byte-aligned PCM or float samples are supported"
            ),
        });
    }
    let bytes_per_sample = bits_per_sample / 8;
    let block_align = channel_count
        .checked_mul(bytes_per_sample)
        .ok_or(MuxError::LayoutOverflow("AIFF block alignment"))?;
    if block_align == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AIFF `COMM` chunk used a zero block alignment".to_string(),
        });
    }

    let format = if is_aifc {
        match FourCc::from_bytes(bytes[18..22].try_into().unwrap()) {
            AIFC_COMPRESSION_NONE | AIFC_COMPRESSION_TWOS => parse_pcm_format_without_stride(
                bits_per_sample,
                channel_count,
                sample_rate,
                block_align,
                spec,
            )?,
            AIFC_COMPRESSION_FL32 | AIFC_COMPRESSION_FL64 => parse_float_format_without_stride(
                bits_per_sample,
                channel_count,
                sample_rate,
                block_align,
                spec,
            )?,
            AIFC_COMPRESSION_ALAW => parse_companded_aifc_format(
                channel_count,
                sample_rate,
                bits_per_sample,
                CompandedPcmKind::Alaw,
                spec,
            )?,
            AIFC_COMPRESSION_ULAW => parse_companded_aifc_format(
                channel_count,
                sample_rate,
                bits_per_sample,
                CompandedPcmKind::Ulaw,
                spec,
            )?,
            compression => {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!(
                        "unsupported AIFC compression type `{compression}`; only `NONE`, `twos`, `fl32`, `fl64`, `ALAW`, and `ULAW` are supported"
                    ),
                });
            }
        }
    } else {
        parse_pcm_format_without_stride(
            bits_per_sample,
            channel_count,
            sample_rate,
            block_align,
            spec,
        )?
    };

    Ok(ParsedAiffCommonChunk {
        format,
        declared_sample_frames,
    })
}

fn parse_aiff_sound_data_chunk_sync(
    file: &mut File,
    offset: u64,
    chunk_size: u64,
    spec: &str,
) -> Result<(u64, u32), MuxError> {
    let mut header = [0_u8; 8];
    read_exact_at_sync(
        file,
        offset,
        &mut header,
        spec,
        "AIFF `SSND` chunk is truncated",
    )?;
    parse_aiff_sound_data_chunk_header(&header, offset, chunk_size, spec)
}

#[cfg(feature = "async")]
async fn parse_aiff_sound_data_chunk_async(
    file: &mut TokioFile,
    offset: u64,
    chunk_size: u64,
    spec: &str,
) -> Result<(u64, u32), MuxError> {
    let mut header = [0_u8; 8];
    read_exact_at_async(
        file,
        offset,
        &mut header,
        spec,
        "AIFF `SSND` chunk is truncated",
    )
    .await?;
    parse_aiff_sound_data_chunk_header(&header, offset, chunk_size, spec)
}

fn parse_aiff_sound_data_chunk_header(
    header: &[u8; 8],
    payload_offset: u64,
    chunk_size: u64,
    spec: &str,
) -> Result<(u64, u32), MuxError> {
    if chunk_size < 8 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AIFF `SSND` chunk is shorter than its required 8-byte header".to_string(),
        });
    }
    let offset_bytes = u64::from(u32::from_be_bytes(header[0..4].try_into().unwrap()));
    let block_size = u32::from_be_bytes(header[4..8].try_into().unwrap());
    if block_size != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AIFF `SSND` chunks with non-zero block size are not supported".to_string(),
        });
    }
    let sound_payload_size = chunk_size - 8;
    if offset_bytes > sound_payload_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AIFF `SSND` chunk offset exceeds the carried sound payload".to_string(),
        });
    }
    let data_offset = payload_offset
        .checked_add(8)
        .and_then(|value| value.checked_add(offset_bytes))
        .ok_or(MuxError::LayoutOverflow("AIFF SSND payload offset"))?;
    let data_size = u32::try_from(sound_payload_size - offset_bytes)
        .map_err(|_| MuxError::LayoutOverflow("AIFF SSND payload size"))?;
    Ok((data_offset, data_size))
}

fn decode_aiff_extended_sample_rate(bytes: &[u8], spec: &str) -> Result<u32, MuxError> {
    let bytes: &[u8; 10] = bytes
        .try_into()
        .map_err(|_| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AIFF `COMM` sample rate field is truncated".to_string(),
        })?;
    let exponent_and_sign = u16::from_be_bytes(bytes[0..2].try_into().unwrap());
    let sign = exponent_and_sign >> 15;
    if sign != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AIFF `COMM` sample rate used a negative extended-float value".to_string(),
        });
    }
    let exponent = exponent_and_sign & 0x7FFF;
    let mantissa = u64::from_be_bytes(bytes[2..10].try_into().unwrap());
    if exponent == 0 && mantissa == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AIFF `COMM` sample rate used a zero extended-float value".to_string(),
        });
    }
    if exponent == 0x7FFF {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AIFF `COMM` sample rate used an unsupported non-finite extended-float value"
                .to_string(),
        });
    }
    let sample_rate = (mantissa as f64) * 2_f64.powi(i32::from(exponent) - 16383 - 63);
    let rounded = sample_rate.round();
    if !rounded.is_finite()
        || rounded <= 0.0
        || rounded > f64::from(u32::MAX)
        || (sample_rate - rounded).abs() > 0.000_1
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AIFF `COMM` sample rate did not decode to a supported integer rate"
                .to_string(),
        });
    }
    Ok(rounded as u32)
}

fn parse_pcm_format_without_stride(
    bits_per_sample: u16,
    channel_count: u16,
    sample_rate: u32,
    block_align: u16,
    spec: &str,
) -> Result<ParsedPcmFormat, MuxError> {
    if !matches!(bits_per_sample, 8 | 16 | 24 | 32 | 64) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported integer PCM sample size {bits_per_sample}"),
        });
    }
    Ok(ParsedPcmFormat {
        sample_entry_type: SAMPLE_ENTRY_IPCM,
        sample_rate,
        channel_count,
        bits_per_sample,
        block_align,
        is_little_endian: false,
        companded_kind: None,
    })
}

fn parse_float_format_without_stride(
    bits_per_sample: u16,
    channel_count: u16,
    sample_rate: u32,
    block_align: u16,
    spec: &str,
) -> Result<ParsedPcmFormat, MuxError> {
    if !matches!(bits_per_sample, 32 | 64) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported floating-point PCM sample size {bits_per_sample}"),
        });
    }
    Ok(ParsedPcmFormat {
        sample_entry_type: SAMPLE_ENTRY_FPCM,
        sample_rate,
        channel_count,
        bits_per_sample,
        block_align,
        is_little_endian: true,
        companded_kind: None,
    })
}

fn parse_companded_aifc_format(
    channel_count: u16,
    sample_rate: u32,
    bits_per_sample: u16,
    companded_kind: CompandedPcmKind,
    spec: &str,
) -> Result<ParsedPcmFormat, MuxError> {
    if !matches!(bits_per_sample, 8 | 16) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "unsupported AIFC companded sample size {bits_per_sample}; only 8-bit and 16-bit declared sample sizes are supported on the companded PCM path"
            ),
        });
    }
    let block_align = match bits_per_sample {
        8 => channel_count,
        16 => channel_count
            .checked_mul(2)
            .ok_or(MuxError::LayoutOverflow("AIFC companded block alignment"))?,
        _ => unreachable!(),
    };
    Ok(ParsedPcmFormat {
        sample_entry_type: SAMPLE_ENTRY_IPCM,
        sample_rate,
        channel_count,
        bits_per_sample: 16,
        block_align,
        is_little_endian: true,
        companded_kind: Some(companded_kind),
    })
}

#[allow(clippy::too_many_arguments)]
fn finalize_pcm_track_sync(
    path: &Path,
    file: &mut File,
    container_kind: PcmContainerKind,
    format: ParsedPcmFormat,
    data_offset: u64,
    data_size: u32,
    declared_sample_frames: Option<u32>,
    spec: &str,
) -> Result<ParsedPcmTrack, MuxError> {
    if data_size == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "PCM input did not contain any audio payload in its media-data chunk"
                .to_string(),
        });
    }
    if !data_size.is_multiple_of(u32::from(format.block_align)) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "PCM media-data chunk size is not a whole number of PCM frames".to_string(),
        });
    }
    let frame_size = u32::from(format.block_align);
    let frame_count = data_size / frame_size;
    if frame_count == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "PCM input did not contain a complete PCM frame".to_string(),
        });
    }
    if let Some(declared_sample_frames) = declared_sample_frames
        && !declared_pcm_sample_frames_match(&format, declared_sample_frames, frame_count)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "PCM container declared {declared_sample_frames} sample frames but the media-data chunk encoded {frame_count}"
            ),
        });
    }
    let transformed_source = build_companded_aifc_transformed_source_sync(
        file,
        path,
        data_offset,
        data_size,
        format,
        spec,
    )?;
    let (data_offset, frame_size) = if transformed_source.is_some() {
        let frame_size = u32::from(format.channel_count)
            .checked_mul(2)
            .ok_or(MuxError::LayoutOverflow("companded PCM output frame size"))?;
        (0, frame_size)
    } else {
        (data_offset, frame_size)
    };
    let sample_entry_box = build_pcm_container_sample_entry_box(container_kind, &format)?;
    Ok(ParsedPcmTrack {
        container_kind,
        sample_rate: format.sample_rate,
        sample_entry_box,
        data_offset,
        frame_size,
        frame_count,
        transformed_source,
    })
}

#[cfg(feature = "async")]
#[allow(clippy::too_many_arguments)]
async fn finalize_pcm_track_async(
    path: &Path,
    file: &mut TokioFile,
    container_kind: PcmContainerKind,
    format: ParsedPcmFormat,
    data_offset: u64,
    data_size: u32,
    declared_sample_frames: Option<u32>,
    spec: &str,
) -> Result<ParsedPcmTrack, MuxError> {
    if data_size == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "PCM input did not contain any audio payload in its media-data chunk"
                .to_string(),
        });
    }
    if !data_size.is_multiple_of(u32::from(format.block_align)) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "PCM media-data chunk size is not a whole number of PCM frames".to_string(),
        });
    }
    let frame_size = u32::from(format.block_align);
    let frame_count = data_size / frame_size;
    if frame_count == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "PCM input did not contain a complete PCM frame".to_string(),
        });
    }
    if let Some(declared_sample_frames) = declared_sample_frames
        && !declared_pcm_sample_frames_match(&format, declared_sample_frames, frame_count)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "PCM container declared {declared_sample_frames} sample frames but the media-data chunk encoded {frame_count}"
            ),
        });
    }
    let transformed_source = build_companded_aifc_transformed_source_async(
        file,
        path,
        data_offset,
        data_size,
        format,
        spec,
    )
    .await?;
    let (data_offset, frame_size) = if transformed_source.is_some() {
        let frame_size = u32::from(format.channel_count)
            .checked_mul(2)
            .ok_or(MuxError::LayoutOverflow("companded PCM output frame size"))?;
        (0, frame_size)
    } else {
        (data_offset, frame_size)
    };
    let sample_entry_box = build_pcm_container_sample_entry_box(container_kind, &format)?;
    Ok(ParsedPcmTrack {
        container_kind,
        sample_rate: format.sample_rate,
        sample_entry_box,
        data_offset,
        frame_size,
        frame_count,
        transformed_source,
    })
}

fn declared_pcm_sample_frames_match(
    format: &ParsedPcmFormat,
    declared_sample_frames: u32,
    frame_count: u32,
) -> bool {
    if declared_sample_frames == frame_count {
        return true;
    }
    let Some(_) = format.companded_kind else {
        return false;
    };
    if format.channel_count == 0 {
        return false;
    }
    let bytes_per_channel = u32::from(format.block_align) / u32::from(format.channel_count);
    bytes_per_channel > 1
        && frame_count
            .checked_mul(bytes_per_channel)
            .is_some_and(|packed_frame_count| packed_frame_count == declared_sample_frames)
}

fn build_companded_aifc_transformed_source_sync(
    file: &mut File,
    path: &Path,
    data_offset: u64,
    data_size: u32,
    format: ParsedPcmFormat,
    spec: &str,
) -> Result<Option<SegmentedMuxSourceSpec>, MuxError> {
    let Some(companded_kind) = format.companded_kind else {
        return Ok(None);
    };
    if format.block_align != format.channel_count {
        return Ok(None);
    }
    let mut encoded = vec![
        0_u8;
        usize::try_from(data_size)
            .map_err(|_| MuxError::LayoutOverflow("companded PCM input size"))?
    ];
    read_exact_at_sync(
        file,
        data_offset,
        &mut encoded,
        spec,
        "PCM input is truncated while reading companded AIFC payload",
    )?;
    Ok(Some(build_inline_companded_pcm_source(
        path,
        &encoded,
        companded_kind,
    )?))
}

#[cfg(feature = "async")]
async fn build_companded_aifc_transformed_source_async(
    file: &mut TokioFile,
    path: &Path,
    data_offset: u64,
    data_size: u32,
    format: ParsedPcmFormat,
    spec: &str,
) -> Result<Option<SegmentedMuxSourceSpec>, MuxError> {
    let Some(companded_kind) = format.companded_kind else {
        return Ok(None);
    };
    if format.block_align != format.channel_count {
        return Ok(None);
    }
    let mut encoded = vec![
        0_u8;
        usize::try_from(data_size)
            .map_err(|_| MuxError::LayoutOverflow("companded PCM input size"))?
    ];
    read_exact_at_async(
        file,
        data_offset,
        &mut encoded,
        spec,
        "PCM input is truncated while reading companded AIFC payload",
    )
    .await?;
    Ok(Some(build_inline_companded_pcm_source(
        path,
        &encoded,
        companded_kind,
    )?))
}

fn build_inline_companded_pcm_source(
    path: &Path,
    encoded: &[u8],
    companded_kind: CompandedPcmKind,
) -> Result<SegmentedMuxSourceSpec, MuxError> {
    let decoded = decode_companded_pcm_payload(encoded, companded_kind);
    let total_size = u64::try_from(decoded.len())
        .map_err(|_| MuxError::LayoutOverflow("companded PCM output size"))?;
    Ok(SegmentedMuxSourceSpec {
        path: path.to_path_buf(),
        segments: vec![SegmentedMuxSourceSegment {
            logical_offset: 0,
            data: SegmentedMuxSourceSegmentData::Bytes(decoded),
        }],
        total_size,
    })
}

fn decode_companded_pcm_payload(encoded: &[u8], companded_kind: CompandedPcmKind) -> Vec<u8> {
    let mut decoded = Vec::with_capacity(encoded.len().saturating_mul(2));
    for &value in encoded {
        let sample = match companded_kind {
            CompandedPcmKind::Alaw => decode_alaw_pcm_sample(value),
            CompandedPcmKind::Ulaw => decode_ulaw_pcm_sample(value),
        };
        decoded.extend_from_slice(&sample.to_le_bytes());
    }
    decoded
}

fn decode_alaw_pcm_sample(value: u8) -> i16 {
    let value = value ^ 0x55;
    let mut sample = i16::from(value & 0x0F) << 4;
    let segment = i16::from((value & 0x70) >> 4);
    sample += 8;
    if segment != 0 {
        sample += 0x100;
    }
    if segment > 1 {
        sample <<= u32::try_from(segment - 1).unwrap();
    }
    if value & 0x80 == 0 { -sample } else { sample }
}

fn decode_ulaw_pcm_sample(value: u8) -> i16 {
    let value = !value;
    let mut sample = (i16::from(value & 0x0F) << 3) + 0x84;
    sample <<= u32::from((value & 0x70) >> 4);
    if value & 0x80 != 0 {
        0x84 - sample
    } else {
        sample - 0x84
    }
}

fn build_wave_sample_entry_box(format: &ParsedPcmFormat) -> Result<Vec<u8>, MuxError> {
    build_pcm_sample_entry_box(
        format.sample_entry_type,
        format.sample_rate,
        format.channel_count,
        format.bits_per_sample,
        format.is_little_endian,
    )
}

fn build_pcm_container_sample_entry_box(
    container_kind: PcmContainerKind,
    format: &ParsedPcmFormat,
) -> Result<Vec<u8>, MuxError> {
    let sample_entry_box = build_wave_sample_entry_box(format)?;
    if container_kind == PcmContainerKind::Aifc && format.sample_entry_type == SAMPLE_ENTRY_FPCM {
        return super::super::mp4::replace_audio_sample_entry_vendor_code(
            &sample_entry_box,
            AIFC_FLOAT_VENDOR_CODE,
        );
    }
    Ok(sample_entry_box)
}

pub(in crate::mux) fn build_pcm_sample_entry_box(
    sample_entry_type: FourCc,
    sample_rate: u32,
    channel_count: u16,
    bits_per_sample: u16,
    is_little_endian: bool,
) -> Result<Vec<u8>, MuxError> {
    let mut pcmc = PcmC::default();
    pcmc.format_flags = if is_little_endian { 1 } else { 0 };
    pcmc.pcm_sample_size =
        u8::try_from(bits_per_sample).map_err(|_| MuxError::LayoutOverflow("PCM sample size"))?;
    let pcmc_bytes = super::super::mp4::encode_typed_box(&pcmc, &[])?;
    let mut child_boxes = vec![pcmc_bytes];
    if let Some(chnl_bytes) = build_pcm_channel_layout_box(channel_count)? {
        child_boxes.push(chnl_bytes);
    }
    build_generic_audio_sample_entry_box(
        sample_entry_type,
        sample_rate,
        channel_count,
        bits_per_sample,
        &child_boxes,
    )
}

fn build_pcm_channel_layout_box(channel_count: u16) -> Result<Option<Vec<u8>>, MuxError> {
    let payload = match channel_count {
        1 => {
            let mut payload = vec![0_u8; 14];
            payload[4] = 1;
            payload[5] = 1;
            payload
        }
        2 => {
            let mut payload = vec![0_u8; 14];
            payload[4] = 1;
            payload[5] = 2;
            payload
        }
        4 => {
            let mut payload = vec![0_u8; 10];
            payload[4] = 1;
            payload
        }
        _ => return Ok(None),
    };
    Ok(Some(super::super::mp4::encode_typed_box(
        &Chnl { data: payload },
        &[],
    )?))
}
