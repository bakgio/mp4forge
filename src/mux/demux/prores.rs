use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs;

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
    let bytes = std::fs::read(path)?;
    parse_prores_bytes(path, spec, &bytes)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_prores_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedProresTrack, MuxError> {
    let bytes = fs::read(path).await?;
    parse_prores_bytes(path, spec, &bytes)
}

fn parse_prores_bytes(
    path: &Path,
    spec: &str,
    bytes: &[u8],
) -> Result<ParsedProresTrack, MuxError> {
    if bytes.len() < 28 {
        return Err(invalid_prores(
            spec,
            "ProRes input is truncated before the first frame header",
        ));
    }

    let mut offset = 0_usize;
    let mut samples = Vec::new();
    let mut config = None::<ProresTrackConfig>;
    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        if remaining < 28 {
            return Err(invalid_prores(
                spec,
                "ProRes input is truncated before one complete frame header",
            ));
        }
        let frame_size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let frame_size_usize = usize::try_from(frame_size)
            .map_err(|_| MuxError::LayoutOverflow("ProRes frame size"))?;
        if frame_size_usize < 28 {
            return Err(invalid_prores(
                spec,
                "ProRes frame declared a size smaller than the required header",
            ));
        }
        let frame_end = offset
            .checked_add(frame_size_usize)
            .ok_or(MuxError::LayoutOverflow("ProRes frame range"))?;
        if frame_end > bytes.len() {
            return Err(invalid_prores(
                spec,
                "ProRes frame overruns the input length",
            ));
        }
        if &bytes[offset + 4..offset + 8] != b"icpf" {
            return Err(invalid_prores(
                spec,
                "ProRes frame did not carry the required `icpf` identifier",
            ));
        }
        let parsed = parse_prores_frame_header(path, spec, &bytes[offset..frame_end])?;
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
            data_offset: u64::try_from(offset)
                .map_err(|_| MuxError::LayoutOverflow("ProRes frame offset"))?,
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

fn parse_prores_frame_header(
    path: &Path,
    spec: &str,
    frame: &[u8],
) -> Result<ProresTrackConfig, MuxError> {
    let frame_header_size = usize::from(u16::from_be_bytes(frame[8..10].try_into().unwrap()));
    if frame_header_size < 20 {
        return Err(invalid_prores(
            spec,
            "ProRes frame header declared a size smaller than the required 20-byte core layout",
        ));
    }
    if 8 + frame_header_size > frame.len() {
        return Err(invalid_prores(
            spec,
            "ProRes frame header overruns the declared frame size",
        ));
    }

    let width = u16::from_be_bytes(frame[16..18].try_into().unwrap());
    let height = u16::from_be_bytes(frame[18..20].try_into().unwrap());
    if width == 0 || height == 0 {
        return Err(invalid_prores(
            spec,
            "ProRes frame header declared zero width or zero height",
        ));
    }
    let chroma_format = frame[20] >> 6;
    let framerate_code = frame[21] & 0x0F;
    let (timescale, duration) = prores_frame_rate(framerate_code);
    let colour_primaries = normalize_prores_colour_component(frame[22]);
    let transfer_characteristics = normalize_prores_colour_component(frame[23]);
    let matrix_coefficients = normalize_prores_colour_component(frame[24]);
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
