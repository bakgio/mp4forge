//! SAF direct-ingest parsing on the current path-first mux lane.
//!
//! The current importer supports local SAF files that carry one declared audio, visual, or
//! scene-description stream whose payload AUs can be mapped truthfully onto authored MP4 sample
//! entries. Remote SAF URL declarations and custom MIME-style declarations stay outside the
//! current path-only contract.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::FourCc;
use crate::bitio::BitReader;
use crate::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    ES_DESCRIPTOR_TAG, EsDescriptor, Esds, SL_CONFIG_DESCRIPTOR_TAG,
};

#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    CandidateSample, TrackCandidate, build_generic_audio_sample_entry_box,
    build_generic_media_sample_entry_box, direct_ingest_handler_name, direct_ingest_mux_policy,
    read_exact_at_sync,
};
use super::super::{MuxError, MuxTrackKind};
use super::jpeg::parse_jpeg_bytes;
use super::mp4v::{build_direct_mp4v_sample_entry_box, parse_mp4v_decoder_specific_info};
use super::png::parse_png_bytes;

const SAF_STREAM_TYPE_SCENE: u8 = 0x03;
const SAF_STREAM_TYPE_VISUAL: u8 = 0x04;
const SAF_STREAM_TYPE_AUDIO: u8 = 0x05;

const SAF_OBJECT_TYPE_AAC: u8 = 0x40;
const SAF_OBJECT_TYPE_MP4V: u8 = 0x20;
const SAF_OBJECT_TYPE_JPEG: u8 = 0x6C;
const SAF_OBJECT_TYPE_PNG: u8 = 0x6D;
const SAF_OBJECT_TYPE_CUSTOM: u8 = 0xFF;
const SCENE_OBJECT_TYPES: [u8; 5] = [0x01, 0x02, 0x04, 0x09, 0x0A];

#[derive(Clone, Copy)]
struct PendingSafSample {
    data_offset: u64,
    data_size: u32,
    cts: u32,
    is_sync_sample: bool,
}

enum SafTrackTemplate {
    Aac {
        sample_rate: u32,
        channel_configuration: u16,
        audio_specific_config: Vec<u8>,
    },
    Mp4v {
        width: u16,
        height: u16,
        decoder_specific_info: Vec<u8>,
    },
    Jpeg,
    Png,
    Scene {
        object_type_indication: u8,
        decoder_specific_info: Vec<u8>,
    },
}

struct DeclaredSafTrack {
    stream_id: u16,
    kind: MuxTrackKind,
    handler_name: String,
    codec_label: &'static str,
    timescale: u32,
    template: SafTrackTemplate,
    samples: Vec<PendingSafSample>,
}

/// Scans one local SAF source synchronously and returns direct-ingest track candidates whose
/// samples keep pointing at the original file.
pub(in crate::mux) fn scan_saf_source_sync(
    path: &Path,
    spec: &str,
    source_index: usize,
) -> Result<Vec<TrackCandidate>, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut declared = scan_declared_saf_tracks_sync(&mut file, file_size, spec)?;
    finalize_declared_saf_tracks_sync(&mut file, spec, source_index, &mut declared)
}

/// Async companion to [`scan_saf_source_sync`] on the additive Tokio surface.
#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_saf_source_async(
    path: &Path,
    spec: &str,
    source_index: usize,
) -> Result<Vec<TrackCandidate>, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut declared = scan_declared_saf_tracks_async(&mut file, file_size, spec).await?;
    finalize_declared_saf_tracks_async(&mut file, spec, source_index, &mut declared).await
}

fn scan_declared_saf_tracks_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<Vec<DeclaredSafTrack>, MuxError> {
    let mut declared = Vec::<DeclaredSafTrack>::new();
    let mut offset = 0_u64;
    while offset < file_size {
        if file_size - offset < 10 {
            return Err(invalid_saf(
                spec,
                "SAF input is truncated before one complete AU header",
            ));
        }
        let header = read_saf_au_header_sync(file, &mut offset, spec)?;
        match header.au_type {
            1 | 2 | 7 => {
                if declared
                    .iter()
                    .any(|track| track.stream_id == header.stream_id)
                {
                    skip_sync(file, &mut offset, u64::from(header.payload_size))?;
                    continue;
                }
                declared.push(read_saf_declaration_sync(file, &mut offset, spec, header)?);
            }
            4 => {
                let payload_offset = offset;
                let Some(track) = declared
                    .iter_mut()
                    .find(|track| track.stream_id == header.stream_id)
                else {
                    return Err(invalid_saf(
                        spec,
                        &format!(
                            "SAF stream {} carried media data before any supported declaration AU",
                            header.stream_id
                        ),
                    ));
                };
                track.samples.push(PendingSafSample {
                    data_offset: payload_offset,
                    data_size: header.payload_size,
                    cts: header.cts,
                    is_sync_sample: header.is_rap,
                });
                skip_sync(file, &mut offset, u64::from(header.payload_size))?;
            }
            _ => {
                skip_sync(file, &mut offset, u64::from(header.payload_size))?;
            }
        }
    }
    Ok(declared)
}

#[cfg(feature = "async")]
async fn scan_declared_saf_tracks_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<Vec<DeclaredSafTrack>, MuxError> {
    let mut declared = Vec::<DeclaredSafTrack>::new();
    let mut offset = 0_u64;
    while offset < file_size {
        if file_size - offset < 10 {
            return Err(invalid_saf(
                spec,
                "SAF input is truncated before one complete AU header",
            ));
        }
        let header = read_saf_au_header_async(file, &mut offset, spec).await?;
        match header.au_type {
            1 | 2 | 7 => {
                if declared
                    .iter()
                    .any(|track| track.stream_id == header.stream_id)
                {
                    skip_async(file, &mut offset, u64::from(header.payload_size)).await?;
                    continue;
                }
                declared.push(read_saf_declaration_async(file, &mut offset, spec, header).await?);
            }
            4 => {
                let payload_offset = offset;
                let Some(track) = declared
                    .iter_mut()
                    .find(|track| track.stream_id == header.stream_id)
                else {
                    return Err(invalid_saf(
                        spec,
                        &format!(
                            "SAF stream {} carried media data before any supported declaration AU",
                            header.stream_id
                        ),
                    ));
                };
                track.samples.push(PendingSafSample {
                    data_offset: payload_offset,
                    data_size: header.payload_size,
                    cts: header.cts,
                    is_sync_sample: header.is_rap,
                });
                skip_async(file, &mut offset, u64::from(header.payload_size)).await?;
            }
            _ => {
                skip_async(file, &mut offset, u64::from(header.payload_size)).await?;
            }
        }
    }
    Ok(declared)
}

fn finalize_declared_saf_tracks_sync(
    file: &mut File,
    spec: &str,
    source_index: usize,
    declared: &mut [DeclaredSafTrack],
) -> Result<Vec<TrackCandidate>, MuxError> {
    let mut tracks = Vec::new();
    for track in declared
        .iter_mut()
        .filter(|track| !track.samples.is_empty())
    {
        tracks.push(finalize_declared_saf_track_sync(
            file,
            spec,
            source_index,
            track,
        )?);
    }
    if tracks.is_empty() {
        return Err(invalid_saf(
            spec,
            "SAF input did not expose any declared stream carrying media AUs",
        ));
    }
    Ok(tracks)
}

#[cfg(feature = "async")]
async fn finalize_declared_saf_tracks_async(
    file: &mut TokioFile,
    spec: &str,
    source_index: usize,
    declared: &mut [DeclaredSafTrack],
) -> Result<Vec<TrackCandidate>, MuxError> {
    let mut tracks = Vec::new();
    for track in declared
        .iter_mut()
        .filter(|track| !track.samples.is_empty())
    {
        tracks.push(finalize_declared_saf_track_async(file, spec, source_index, track).await?);
    }
    if tracks.is_empty() {
        return Err(invalid_saf(
            spec,
            "SAF input did not expose any declared stream carrying media AUs",
        ));
    }
    Ok(tracks)
}

fn finalize_declared_saf_track_sync(
    file: &mut File,
    spec: &str,
    source_index: usize,
    track: &DeclaredSafTrack,
) -> Result<TrackCandidate, MuxError> {
    let samples: Vec<CandidateSample> = candidate_samples_from_pending(spec, &track.samples)?
        .into_iter()
        .map(|mut sample| {
            sample.source_index = source_index;
            sample
        })
        .collect();
    let (width, height, sample_entry_box) = match &track.template {
        SafTrackTemplate::Aac {
            sample_rate,
            channel_configuration,
            audio_specific_config,
        } => (
            0,
            0,
            build_saf_aac_sample_entry_box(
                audio_specific_config,
                *sample_rate,
                *channel_configuration,
            )?,
        ),
        SafTrackTemplate::Mp4v {
            width,
            height,
            decoder_specific_info,
        } => (
            *width,
            *height,
            build_direct_mp4v_sample_entry_box(
                *width,
                *height,
                decoder_specific_info,
                track.timescale,
                samples
                    .iter()
                    .map(|sample| (sample.data_size, sample.duration)),
            )?,
        ),
        SafTrackTemplate::Jpeg => {
            let payload = read_sample_payload_sync(file, spec, track.samples[0])?;
            let parsed = parse_jpeg_bytes(spec, &payload)?;
            (parsed.width, parsed.height, parsed.sample_entry_box)
        }
        SafTrackTemplate::Png => {
            let payload = read_sample_payload_sync(file, spec, track.samples[0])?;
            let parsed = parse_png_bytes(spec, &payload)?;
            (parsed.width, parsed.height, parsed.sample_entry_box)
        }
        SafTrackTemplate::Scene {
            object_type_indication,
            decoder_specific_info,
        } => (
            0,
            0,
            build_saf_generic_media_entry_box(
                FourCc::from_bytes(*b"mp4s"),
                *object_type_indication,
                SAF_STREAM_TYPE_SCENE,
                decoder_specific_info,
            )?,
        ),
    };

    Ok(TrackCandidate {
        track_id: u32::from(track.stream_id),
        kind: track.kind,
        timescale: track.timescale,
        language: *b"und",
        handler_name: track.handler_name.clone(),
        mux_policy: direct_ingest_mux_policy(track.codec_label, track.kind),
        width,
        height,
        sample_entry_box,
        source_edit_media_time: None,
        samples,
    })
}

#[cfg(feature = "async")]
async fn finalize_declared_saf_track_async(
    file: &mut TokioFile,
    spec: &str,
    source_index: usize,
    track: &DeclaredSafTrack,
) -> Result<TrackCandidate, MuxError> {
    let samples = candidate_samples_from_pending(spec, &track.samples)?;
    let (width, height, sample_entry_box) = match &track.template {
        SafTrackTemplate::Aac {
            sample_rate,
            channel_configuration,
            audio_specific_config,
        } => (
            0,
            0,
            build_saf_aac_sample_entry_box(
                audio_specific_config,
                *sample_rate,
                *channel_configuration,
            )?,
        ),
        SafTrackTemplate::Mp4v {
            width,
            height,
            decoder_specific_info,
        } => (
            *width,
            *height,
            build_direct_mp4v_sample_entry_box(
                *width,
                *height,
                decoder_specific_info,
                track.timescale,
                samples
                    .iter()
                    .map(|sample| (sample.data_size, sample.duration)),
            )?,
        ),
        SafTrackTemplate::Jpeg => {
            let payload = read_sample_payload_async(file, spec, track.samples[0]).await?;
            let parsed = parse_jpeg_bytes(spec, &payload)?;
            (parsed.width, parsed.height, parsed.sample_entry_box)
        }
        SafTrackTemplate::Png => {
            let payload = read_sample_payload_async(file, spec, track.samples[0]).await?;
            let parsed = parse_png_bytes(spec, &payload)?;
            (parsed.width, parsed.height, parsed.sample_entry_box)
        }
        SafTrackTemplate::Scene {
            object_type_indication,
            decoder_specific_info,
        } => (
            0,
            0,
            build_saf_generic_media_entry_box(
                FourCc::from_bytes(*b"mp4s"),
                *object_type_indication,
                SAF_STREAM_TYPE_SCENE,
                decoder_specific_info,
            )?,
        ),
    };

    Ok(TrackCandidate {
        track_id: u32::from(track.stream_id),
        kind: track.kind,
        timescale: track.timescale,
        language: *b"und",
        handler_name: track.handler_name.clone(),
        mux_policy: direct_ingest_mux_policy(track.codec_label, track.kind),
        width,
        height,
        sample_entry_box,
        source_edit_media_time: None,
        samples: samples
            .into_iter()
            .map(|mut sample| {
                sample.source_index = source_index;
                sample
            })
            .collect(),
    })
}

fn candidate_samples_from_pending(
    spec: &str,
    samples: &[PendingSafSample],
) -> Result<Vec<CandidateSample>, MuxError> {
    let mut result = Vec::with_capacity(samples.len());
    let mut last_delta = None::<u32>;
    for (index, sample) in samples.iter().enumerate() {
        let duration = if let Some(next) = samples.get(index + 1) {
            if next.cts < sample.cts {
                return Err(invalid_saf(
                    spec,
                    "SAF stream carried a decode-time regression between successive AUs",
                ));
            }
            let delta = next.cts - sample.cts;
            last_delta = Some(delta);
            delta
        } else {
            last_delta.unwrap_or(1)
        };
        result.push(CandidateSample {
            source_index: 0,
            data_offset: sample.data_offset,
            data_size: sample.data_size,
            duration,
            composition_time_offset: 0,
            is_sync_sample: sample.is_sync_sample,
        });
    }
    Ok(result)
}

fn read_sample_payload_sync(
    file: &mut File,
    spec: &str,
    sample: PendingSafSample,
) -> Result<Vec<u8>, MuxError> {
    let mut payload = vec![
        0_u8;
        usize::try_from(sample.data_size)
            .map_err(|_| MuxError::LayoutOverflow("SAF sample payload size"))?
    ];
    read_exact_at_sync(
        file,
        sample.data_offset,
        &mut payload,
        spec,
        "SAF sample payload is truncated",
    )?;
    Ok(payload)
}

#[cfg(feature = "async")]
async fn read_sample_payload_async(
    file: &mut TokioFile,
    spec: &str,
    sample: PendingSafSample,
) -> Result<Vec<u8>, MuxError> {
    let mut payload = vec![
        0_u8;
        usize::try_from(sample.data_size)
            .map_err(|_| MuxError::LayoutOverflow("SAF sample payload size"))?
    ];
    read_exact_at_async(
        file,
        sample.data_offset,
        &mut payload,
        spec,
        "SAF sample payload is truncated",
    )
    .await?;
    Ok(payload)
}

#[derive(Clone, Copy)]
struct SafAuHeader {
    is_rap: bool,
    cts: u32,
    payload_size: u32,
    au_type: u8,
    stream_id: u16,
}

fn read_saf_au_header_sync(
    file: &mut File,
    offset: &mut u64,
    spec: &str,
) -> Result<SafAuHeader, MuxError> {
    let mut outer = [0_u8; 8];
    file.read_exact(&mut outer)
        .map_err(|error| saf_io_error(spec, "read SAF AU header", error))?;
    *offset += 8;
    let outer = u64::from_be_bytes(outer);
    let payload_size = u32::from((outer & 0xFFFF) as u16);
    if payload_size < 2 {
        return Err(invalid_saf(
            spec,
            "SAF AU payload is shorter than the mandatory stream header",
        ));
    }
    let mut inner = [0_u8; 2];
    file.read_exact(&mut inner)
        .map_err(|error| saf_io_error(spec, "read SAF stream header", error))?;
    *offset += 2;
    let inner = u16::from_be_bytes(inner);
    Ok(SafAuHeader {
        is_rap: ((outer >> 63) & 1) != 0,
        cts: ((outer >> 16) & 0x3FFF_FFFF) as u32,
        payload_size: payload_size - 2,
        au_type: u8::try_from((inner >> 12) & 0x0F).unwrap(),
        stream_id: inner & 0x0FFF,
    })
}

#[cfg(feature = "async")]
async fn read_saf_au_header_async(
    file: &mut TokioFile,
    offset: &mut u64,
    spec: &str,
) -> Result<SafAuHeader, MuxError> {
    let mut outer = [0_u8; 8];
    file.read_exact(&mut outer)
        .await
        .map_err(|error| saf_io_error(spec, "read SAF AU header", error))?;
    *offset += 8;
    let outer = u64::from_be_bytes(outer);
    let payload_size = u32::from((outer & 0xFFFF) as u16);
    if payload_size < 2 {
        return Err(invalid_saf(
            spec,
            "SAF AU payload is shorter than the mandatory stream header",
        ));
    }
    let mut inner = [0_u8; 2];
    file.read_exact(&mut inner)
        .await
        .map_err(|error| saf_io_error(spec, "read SAF stream header", error))?;
    *offset += 2;
    let inner = u16::from_be_bytes(inner);
    Ok(SafAuHeader {
        is_rap: ((outer >> 63) & 1) != 0,
        cts: ((outer >> 16) & 0x3FFF_FFFF) as u32,
        payload_size: payload_size - 2,
        au_type: u8::try_from((inner >> 12) & 0x0F).unwrap(),
        stream_id: inner & 0x0FFF,
    })
}

fn read_saf_declaration_sync(
    file: &mut File,
    offset: &mut u64,
    spec: &str,
    header: SafAuHeader,
) -> Result<DeclaredSafTrack, MuxError> {
    if header.payload_size < 7 {
        return Err(invalid_saf(
            spec,
            "SAF stream declaration is shorter than its fixed descriptor header",
        ));
    }
    let mut fixed = [0_u8; 7];
    file.read_exact(&mut fixed)
        .map_err(|error| saf_io_error(spec, "read SAF stream declaration", error))?;
    *offset += 7;
    let remaining = header.payload_size - 7;
    let object_type_indication = fixed[0];
    let stream_type = fixed[1];
    let timescale = u32::from_be_bytes([0, fixed[2], fixed[3], fixed[4]]);
    if timescale == 0 {
        return Err(invalid_saf(
            spec,
            &format!("SAF stream {} declared a zero timescale", header.stream_id),
        ));
    }

    if object_type_indication == SAF_OBJECT_TYPE_CUSTOM && stream_type == SAF_OBJECT_TYPE_CUSTOM {
        return Err(invalid_saf(
            spec,
            "SAF custom MIME declarations are outside the current authored import surface",
        ));
    }

    if header.au_type == 7 {
        return Err(invalid_saf(
            spec,
            "SAF remote URL declarations are outside the current path-only import contract",
        ));
    }

    let mut decoder_specific_info = vec![
        0_u8;
        usize::try_from(remaining).map_err(|_| {
            MuxError::LayoutOverflow("SAF decoder config size")
        })?
    ];
    if remaining != 0 {
        file.read_exact(&mut decoder_specific_info)
            .map_err(|error| saf_io_error(spec, "read SAF decoder config", error))?;
        *offset += u64::from(remaining);
    }

    build_declared_saf_track(
        spec,
        header.stream_id,
        object_type_indication,
        stream_type,
        timescale,
        decoder_specific_info,
    )
}

#[cfg(feature = "async")]
async fn read_saf_declaration_async(
    file: &mut TokioFile,
    offset: &mut u64,
    spec: &str,
    header: SafAuHeader,
) -> Result<DeclaredSafTrack, MuxError> {
    if header.payload_size < 7 {
        return Err(invalid_saf(
            spec,
            "SAF stream declaration is shorter than its fixed descriptor header",
        ));
    }
    let mut fixed = [0_u8; 7];
    file.read_exact(&mut fixed)
        .await
        .map_err(|error| saf_io_error(spec, "read SAF stream declaration", error))?;
    *offset += 7;
    let remaining = header.payload_size - 7;
    let object_type_indication = fixed[0];
    let stream_type = fixed[1];
    let timescale = u32::from_be_bytes([0, fixed[2], fixed[3], fixed[4]]);
    if timescale == 0 {
        return Err(invalid_saf(
            spec,
            &format!("SAF stream {} declared a zero timescale", header.stream_id),
        ));
    }

    if object_type_indication == SAF_OBJECT_TYPE_CUSTOM && stream_type == SAF_OBJECT_TYPE_CUSTOM {
        return Err(invalid_saf(
            spec,
            "SAF custom MIME declarations are outside the current authored import surface",
        ));
    }

    if header.au_type == 7 {
        return Err(invalid_saf(
            spec,
            "SAF remote URL declarations are outside the current path-only import contract",
        ));
    }

    let mut decoder_specific_info = vec![
        0_u8;
        usize::try_from(remaining).map_err(|_| {
            MuxError::LayoutOverflow("SAF decoder config size")
        })?
    ];
    if remaining != 0 {
        file.read_exact(&mut decoder_specific_info)
            .await
            .map_err(|error| saf_io_error(spec, "read SAF decoder config", error))?;
        *offset += u64::from(remaining);
    }

    build_declared_saf_track(
        spec,
        header.stream_id,
        object_type_indication,
        stream_type,
        timescale,
        decoder_specific_info,
    )
}

fn build_declared_saf_track(
    spec: &str,
    stream_id: u16,
    object_type_indication: u8,
    stream_type: u8,
    timescale: u32,
    decoder_specific_info: Vec<u8>,
) -> Result<DeclaredSafTrack, MuxError> {
    let (kind, handler_name, codec_label, template) = match stream_type {
        SAF_STREAM_TYPE_AUDIO => {
            if object_type_indication != SAF_OBJECT_TYPE_AAC {
                return Err(invalid_saf(
                    spec,
                    &format!(
                        "SAF stream {stream_id} declared unsupported authored audio object type 0x{object_type_indication:02x}"
                    ),
                ));
            }
            let parsed = parse_aac_audio_specific_config(spec, &decoder_specific_info)?;
            (
                MuxTrackKind::Audio,
                direct_ingest_handler_name("aac"),
                "aac",
                SafTrackTemplate::Aac {
                    sample_rate: parsed.sample_rate,
                    channel_configuration: parsed.channel_configuration,
                    audio_specific_config: decoder_specific_info,
                },
            )
        }
        SAF_STREAM_TYPE_VISUAL => match object_type_indication {
            SAF_OBJECT_TYPE_MP4V => {
                let (width, height) =
                    parse_mp4v_decoder_specific_info(&decoder_specific_info, spec)?;
                (
                    MuxTrackKind::Video,
                    direct_ingest_handler_name("mp4v"),
                    "mp4v",
                    SafTrackTemplate::Mp4v {
                        width,
                        height,
                        decoder_specific_info,
                    },
                )
            }
            SAF_OBJECT_TYPE_JPEG => (
                MuxTrackKind::Video,
                direct_ingest_handler_name("jpeg"),
                "jpeg",
                SafTrackTemplate::Jpeg,
            ),
            SAF_OBJECT_TYPE_PNG => (
                MuxTrackKind::Video,
                direct_ingest_handler_name("png"),
                "png",
                SafTrackTemplate::Png,
            ),
            _ => {
                return Err(invalid_saf(
                    spec,
                    &format!(
                        "SAF stream {stream_id} declared unsupported authored visual object type 0x{object_type_indication:02x}"
                    ),
                ));
            }
        },
        SAF_STREAM_TYPE_SCENE => {
            if !SCENE_OBJECT_TYPES.contains(&object_type_indication) {
                return Err(invalid_saf(
                    spec,
                    &format!(
                        "SAF stream {stream_id} declared unsupported scene object type 0x{object_type_indication:02x}"
                    ),
                ));
            }
            (
                MuxTrackKind::Video,
                "SceneHandler".to_string(),
                "saf-scene",
                SafTrackTemplate::Scene {
                    object_type_indication,
                    decoder_specific_info,
                },
            )
        }
        _ => {
            return Err(invalid_saf(
                spec,
                &format!(
                    "SAF stream {stream_id} declared unsupported stream type 0x{stream_type:02x}"
                ),
            ));
        }
    };

    Ok(DeclaredSafTrack {
        stream_id,
        kind,
        handler_name,
        codec_label,
        timescale,
        template,
        samples: Vec::new(),
    })
}

struct ParsedAacAudioSpecificConfig {
    sample_rate: u32,
    channel_configuration: u16,
}

fn parse_aac_audio_specific_config(
    spec: &str,
    audio_specific_config: &[u8],
) -> Result<ParsedAacAudioSpecificConfig, MuxError> {
    let mut reader = BitReader::new(std::io::Cursor::new(audio_specific_config));
    let audio_object_type = read_audio_object_type(&mut reader, spec)?;
    if audio_object_type == 0 {
        return Err(invalid_saf(
            spec,
            "SAF AAC declaration carried an invalid audio object type of zero",
        ));
    }
    let sampling_frequency_index = read_bits_u8(&mut reader, 4, spec, "SAF AAC sample rate")?;
    let sample_rate = if sampling_frequency_index == 0x0F {
        read_bits_u32(&mut reader, 24, spec, "SAF AAC explicit sample rate")?
    } else {
        aac_sample_rate(sampling_frequency_index).ok_or_else(|| {
            invalid_saf(
                spec,
                &format!(
                    "SAF AAC declaration carried unsupported sample-rate index {sampling_frequency_index}"
                ),
            )
        })?
    };
    let channel_configuration = u16::from(read_bits_u8(
        &mut reader,
        4,
        spec,
        "SAF AAC channel configuration",
    )?);
    if channel_configuration == 0 {
        return Err(invalid_saf(
            spec,
            "SAF AAC declarations that rely on program-config channel signaling are not supported on the current authored lane",
        ));
    }
    Ok(ParsedAacAudioSpecificConfig {
        sample_rate,
        channel_configuration,
    })
}

fn build_saf_aac_sample_entry_box(
    audio_specific_config: &[u8],
    sample_rate: u32,
    channel_configuration: u16,
) -> Result<Vec<u8>, MuxError> {
    let mut esds = Esds::default();
    esds.descriptors = vec![
        Descriptor {
            tag: ES_DESCRIPTOR_TAG,
            es_descriptor: Some(EsDescriptor::default()),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_CONFIG_DESCRIPTOR_TAG,
            decoder_config_descriptor: Some(DecoderConfigDescriptor {
                object_type_indication: SAF_OBJECT_TYPE_AAC,
                stream_type: 5,
                reserved: true,
                ..DecoderConfigDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_SPECIFIC_INFO_TAG,
            size: u32::try_from(audio_specific_config.len())
                .map_err(|_| MuxError::LayoutOverflow("SAF AAC decoder config size"))?,
            data: audio_specific_config.to_vec(),
            ..Descriptor::default()
        },
        Descriptor {
            tag: SL_CONFIG_DESCRIPTOR_TAG,
            size: 1,
            data: vec![0x02],
            ..Descriptor::default()
        },
    ];
    esds.normalize_descriptor_sizes_for_mux()
        .map_err(|_| MuxError::LayoutOverflow("SAF AAC esds"))?;
    let esds_box = super::super::mp4::encode_typed_box(&esds, &[])?;
    build_generic_audio_sample_entry_box(
        FourCc::from_bytes(*b"mp4a"),
        sample_rate,
        channel_configuration,
        16,
        &[esds_box],
    )
}

fn build_saf_generic_media_entry_box(
    sample_entry_type: FourCc,
    object_type_indication: u8,
    stream_type: u8,
    decoder_specific_info: &[u8],
) -> Result<Vec<u8>, MuxError> {
    let mut esds = Esds::default();
    esds.descriptors = vec![
        Descriptor {
            tag: ES_DESCRIPTOR_TAG,
            es_descriptor: Some(EsDescriptor::default()),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_CONFIG_DESCRIPTOR_TAG,
            decoder_config_descriptor: Some(DecoderConfigDescriptor {
                object_type_indication,
                stream_type,
                reserved: true,
                ..DecoderConfigDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_SPECIFIC_INFO_TAG,
            size: u32::try_from(decoder_specific_info.len())
                .map_err(|_| MuxError::LayoutOverflow("SAF generic decoder config size"))?,
            data: decoder_specific_info.to_vec(),
            ..Descriptor::default()
        },
        Descriptor {
            tag: SL_CONFIG_DESCRIPTOR_TAG,
            size: 1,
            data: vec![0x02],
            ..Descriptor::default()
        },
    ];
    esds.normalize_descriptor_sizes_for_mux()
        .map_err(|_| MuxError::LayoutOverflow("SAF generic esds"))?;
    let esds_box = super::super::mp4::encode_typed_box(&esds, &[])?;
    build_generic_media_sample_entry_box(sample_entry_type, &[esds_box])
}

fn read_audio_object_type<R>(reader: &mut BitReader<R>, spec: &str) -> Result<u8, MuxError>
where
    R: Read,
{
    let audio_object_type = read_bits_u8(reader, 5, spec, "SAF AAC audio object type")?;
    if audio_object_type == 31 {
        Ok(32 + read_bits_u8(reader, 6, spec, "SAF AAC extended audio object type")?)
    } else {
        Ok(audio_object_type)
    }
}

fn read_bits_u8<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u8, MuxError>
where
    R: Read,
{
    let bits = reader.read_bits(width).map_err(|_| {
        invalid_saf(
            spec,
            &format!("{label} is truncated before it exposes {width} bits"),
        )
    })?;
    let value = bits
        .iter()
        .fold(0_u16, |value, byte| (value << 8) | u16::from(*byte));
    u8::try_from(value).map_err(|_| invalid_saf(spec, &format!("{label} exceeds one byte")))
}

fn read_bits_u32<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u32, MuxError>
where
    R: Read,
{
    let bits = reader.read_bits(width).map_err(|_| {
        invalid_saf(
            spec,
            &format!("{label} is truncated before it exposes {width} bits"),
        )
    })?;
    Ok(bits
        .iter()
        .fold(0_u32, |value, byte| (value << 8) | u32::from(*byte)))
}

const fn aac_sample_rate(index: u8) -> Option<u32> {
    match index {
        0 => Some(96_000),
        1 => Some(88_200),
        2 => Some(64_000),
        3 => Some(48_000),
        4 => Some(44_100),
        5 => Some(32_000),
        6 => Some(24_000),
        7 => Some(22_050),
        8 => Some(16_000),
        9 => Some(12_000),
        10 => Some(11_025),
        11 => Some(8_000),
        12 => Some(7_350),
        _ => None,
    }
}

fn skip_sync(file: &mut File, offset: &mut u64, size: u64) -> Result<(), MuxError> {
    file.seek(SeekFrom::Current(
        i64::try_from(size).map_err(|_| MuxError::LayoutOverflow("SAF skip size"))?,
    ))
    .map_err(|error| saf_io_error("SAF", "skip SAF payload", error))?;
    *offset += size;
    Ok(())
}

#[cfg(feature = "async")]
async fn skip_async(file: &mut TokioFile, offset: &mut u64, size: u64) -> Result<(), MuxError> {
    file.seek(SeekFrom::Current(
        i64::try_from(size).map_err(|_| MuxError::LayoutOverflow("SAF skip size"))?,
    ))
    .await
    .map_err(|error| saf_io_error("SAF", "skip SAF payload", error))?;
    *offset += size;
    Ok(())
}

fn invalid_saf(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}

fn saf_io_error(spec: &str, operation: &str, error: std::io::Error) -> MuxError {
    invalid_saf(spec, &format!("failed to {operation}: {error}"))
}
