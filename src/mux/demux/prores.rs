use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::FourCc;

use super::super::MuxError;
use super::super::import::StagedSample;
use super::raw_visual::build_prores_sample_entry_box;

const APCO: FourCc = FourCc::from_bytes(*b"apco");
const APCN: FourCc = FourCc::from_bytes(*b"apcn");
const APCH: FourCc = FourCc::from_bytes(*b"apch");
const APCS: FourCc = FourCc::from_bytes(*b"apcs");
const AP4X: FourCc = FourCc::from_bytes(*b"ap4x");
const AP4H: FourCc = FourCc::from_bytes(*b"ap4h");

pub(in crate::mux) struct ParsedProresTrack {
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) media_timescale: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProresTrackConfig {
    sample_entry_type: FourCc,
    width: u16,
    height: u16,
    timescale: u32,
    duration: u32,
    colour_primaries: u16,
    transfer_characteristics: u16,
    matrix_coefficients: u16,
}

pub(in crate::mux) fn scan_prores_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedProresTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_prores_file_sync(path, spec, &mut file, file_size)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_prores_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedProresTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_prores_file_async(path, spec, &mut file, file_size).await
}

fn parse_prores_file_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    file_size: u64,
) -> Result<ParsedProresTrack, MuxError> {
    if file_size < 28 {
        return Err(invalid_prores(
            spec,
            "ProRes input is truncated before the first frame header",
        ));
    }

    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut config = None::<ProresTrackConfig>;
    let mut header = [0_u8; 28];
    while offset < file_size {
        let remaining = file_size - offset;
        if remaining < 28 {
            return Err(invalid_prores(
                spec,
                "ProRes input is truncated before one complete frame header",
            ));
        }
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut header)?;
        let frame_size = u32::from_be_bytes(header[0..4].try_into().unwrap());
        if frame_size < 28 {
            return Err(invalid_prores(
                spec,
                "ProRes frame declared a size smaller than the required header",
            ));
        }
        let frame_end = offset
            .checked_add(u64::from(frame_size))
            .ok_or(MuxError::LayoutOverflow("ProRes frame range"))?;
        if frame_end > file_size {
            return Err(invalid_prores(
                spec,
                "ProRes frame overruns the input length",
            ));
        }
        if &header[4..8] != b"icpf" {
            return Err(invalid_prores(
                spec,
                "ProRes frame did not carry the required `icpf` identifier",
            ));
        }
        let parsed = parse_prores_frame_header(path, spec, &header, frame_size)?;
        if let Some(previous) = config {
            if previous != parsed {
                return Err(invalid_prores(
                    spec,
                    "ProRes input changed its frame configuration mid-stream",
                ));
            }
        } else {
            config = Some(parsed);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: frame_size,
            duration: parsed.duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = frame_end;
    }

    let config =
        config.ok_or_else(|| invalid_prores(spec, "ProRes input did not carry any frames"))?;
    // The retained reference raw-ProRes lane leaves the trailing sample duration unresolved.
    // Keeping the final sample open preserves that one-frame `stts` behavior without
    // changing the earlier frame-spacing we still need on longer inputs.
    if let Some(last_sample) = samples.last_mut() {
        last_sample.duration = 0;
    }
    let sample_entry_box = build_prores_sample_entry_box(
        config.sample_entry_type,
        config.width,
        config.height,
        prores_compressor_name(config.sample_entry_type),
        config.colour_primaries,
        config.transfer_characteristics,
        config.matrix_coefficients,
    )?;
    Ok(ParsedProresTrack {
        width: config.width,
        height: config.height,
        media_timescale: config.timescale,
        sample_entry_box,
        samples,
    })
}

#[cfg(feature = "async")]
async fn parse_prores_file_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    file_size: u64,
) -> Result<ParsedProresTrack, MuxError> {
    if file_size < 28 {
        return Err(invalid_prores(
            spec,
            "ProRes input is truncated before the first frame header",
        ));
    }

    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut config = None::<ProresTrackConfig>;
    let mut header = [0_u8; 28];
    while offset < file_size {
        let remaining = file_size - offset;
        if remaining < 28 {
            return Err(invalid_prores(
                spec,
                "ProRes input is truncated before one complete frame header",
            ));
        }
        file.seek(SeekFrom::Start(offset)).await?;
        file.read_exact(&mut header).await?;
        let frame_size = u32::from_be_bytes(header[0..4].try_into().unwrap());
        if frame_size < 28 {
            return Err(invalid_prores(
                spec,
                "ProRes frame declared a size smaller than the required header",
            ));
        }
        let frame_end = offset
            .checked_add(u64::from(frame_size))
            .ok_or(MuxError::LayoutOverflow("ProRes frame range"))?;
        if frame_end > file_size {
            return Err(invalid_prores(
                spec,
                "ProRes frame overruns the input length",
            ));
        }
        if &header[4..8] != b"icpf" {
            return Err(invalid_prores(
                spec,
                "ProRes frame did not carry the required `icpf` identifier",
            ));
        }
        let parsed = parse_prores_frame_header(path, spec, &header, frame_size)?;
        if let Some(previous) = config {
            if previous != parsed {
                return Err(invalid_prores(
                    spec,
                    "ProRes input changed its frame configuration mid-stream",
                ));
            }
        } else {
            config = Some(parsed);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: frame_size,
            duration: parsed.duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = frame_end;
    }

    let config =
        config.ok_or_else(|| invalid_prores(spec, "ProRes input did not carry any frames"))?;
    if let Some(last_sample) = samples.last_mut() {
        last_sample.duration = 0;
    }
    let sample_entry_box = build_prores_sample_entry_box(
        config.sample_entry_type,
        config.width,
        config.height,
        prores_compressor_name(config.sample_entry_type),
        config.colour_primaries,
        config.transfer_characteristics,
        config.matrix_coefficients,
    )?;
    Ok(ParsedProresTrack {
        width: config.width,
        height: config.height,
        media_timescale: config.timescale,
        sample_entry_box,
        samples,
    })
}

fn parse_prores_frame_header(
    path: &Path,
    spec: &str,
    header: &[u8; 28],
    frame_size: u32,
) -> Result<ProresTrackConfig, MuxError> {
    let frame_header_size = usize::from(u16::from_be_bytes(header[8..10].try_into().unwrap()));
    if frame_header_size < 20 {
        return Err(invalid_prores(
            spec,
            "ProRes frame header declared a size smaller than the required 20-byte core layout",
        ));
    }
    if 8 + frame_header_size > usize::try_from(frame_size).unwrap() {
        return Err(invalid_prores(
            spec,
            "ProRes frame header overruns the declared frame size",
        ));
    }

    let width = u16::from_be_bytes(header[16..18].try_into().unwrap());
    let height = u16::from_be_bytes(header[18..20].try_into().unwrap());
    if width == 0 || height == 0 {
        return Err(invalid_prores(
            spec,
            "ProRes frame header declared zero width or zero height",
        ));
    }
    let chroma_format = header[20] >> 6;
    let framerate_code = header[21] & 0x0F;
    let (timescale, duration) = prores_frame_rate(framerate_code);
    let colour_primaries = normalize_prores_colour_component(header[22]);
    let transfer_characteristics = normalize_prores_colour_component(header[23]);
    let matrix_coefficients = normalize_prores_colour_component(header[24]);
    let sample_entry_type = prores_sample_entry_type(path, chroma_format);
    Ok(ProresTrackConfig {
        sample_entry_type,
        width,
        height,
        timescale,
        duration,
        colour_primaries,
        transfer_characteristics,
        matrix_coefficients,
    })
}

fn prores_frame_rate(code: u8) -> (u32, u32) {
    match code {
        1 => (24_000, 1_001),
        2 | 3 => (2_400, 100),
        4 => (30_000, 1_001),
        5 => (3_000, 100),
        6 => (5_000, 100),
        7 => (60_000, 1_001),
        8 => (6_000, 100),
        9 => (10_000, 100),
        10 => (120_000, 1_001),
        11 => (12_000, 100),
        _ => (2_500, 100),
    }
}

fn prores_sample_entry_type(path: &Path, chroma_format: u8) -> FourCc {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return default_prores_sample_entry_type(chroma_format);
    };
    match extension.to_ascii_lowercase().as_str() {
        "apco" => APCO,
        "apcn" => APCN,
        "apch" => APCH,
        "apcs" => APCS,
        "ap4x" => AP4X,
        "ap4h" => AP4H,
        _ => default_prores_sample_entry_type(chroma_format),
    }
}

fn default_prores_sample_entry_type(chroma_format: u8) -> FourCc {
    if chroma_format == 3 { AP4H } else { APCH }
}

fn prores_compressor_name(sample_entry_type: FourCc) -> &'static [u8] {
    match sample_entry_type {
        APCO => b"ProRes Video 422 Proxy",
        APCN => b"ProRes Video 422",
        APCH => b"ProRes Video 422 HQ",
        APCS => b"ProRes Video 422 LT",
        AP4X => b"ProRes Video 4444 XQ",
        AP4H => b"ProRes Video 4444",
        _ => b"ProRes Video 422 HQ",
    }
}

fn normalize_prores_colour_component(value: u8) -> u16 {
    match value {
        0 => 1,
        other => u16::from(other),
    }
}

fn invalid_prores(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}
