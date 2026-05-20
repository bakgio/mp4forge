use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

use crate::FourCc;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use super::super::MuxError;
use super::super::MuxTrackKind;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    CandidateSample, CompositeTrackCandidate, SegmentedMuxSourceSegment, SegmentedMuxSourceSpec,
    StagedSample, TrackCandidate, direct_ingest_handler_name, direct_ingest_mux_policy,
    read_exact_at_sync,
};
#[cfg(feature = "async")]
use super::ac3::scan_ac3_segmented_async;
use super::ac3::scan_ac3_segmented_sync;
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::{append_file_range_segment, read_segmented_bytes_sync};
use super::detect::{DetectedPathTrackKind, detect_path_track_kind_from_prefix};
#[cfg(feature = "async")]
use super::h264::stage_annex_b_h264_segmented_async;
use super::h264::stage_annex_b_h264_segmented_sync;
#[cfg(feature = "async")]
use super::h265::stage_annex_b_h265_segmented_async;
use super::h265::stage_annex_b_h265_segmented_sync;
use super::mp3::{build_mp3_sample_entry_box, parse_mp3_frame_header};
#[cfg(feature = "async")]
use super::mp4v::scan_mp4v_segmented_async;
use super::mp4v::scan_mp4v_segmented_sync;
#[cfg(feature = "async")]
use super::mpeg2v::scan_mpeg2v_segmented_async;
use super::mpeg2v::{
    ProgramStreamMpeg2vSampleEntryConfig, build_program_stream_mpeg2v_sample_entry_box,
    scan_mpeg2v_segmented_sync,
};
use super::pcm::build_pcm_sample_entry_box;
use super::vobsub::{
    VOBSUB_TIMESCALE, build_subpicture_sample_entry_box, effective_vobsub_duration,
    parse_vobsub_duration,
};
#[cfg(feature = "async")]
use super::vvc::stage_annex_b_vvc_segmented_async;
use super::vvc::stage_annex_b_vvc_segmented_sync;

const PACK_START_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xBA];
const SYSTEM_HEADER_START_CODE: u8 = 0xBB;
const PROGRAM_STREAM_MAP_START_CODE: u8 = 0xBC;
const PRIVATE_STREAM_1_START_CODE: u8 = 0xBD;
const PADDING_STREAM_START_CODE: u8 = 0xBE;
const PRIVATE_STREAM_2_START_CODE: u8 = 0xBF;
const PRIVATE_STREAM_1_AC3_MIN: u8 = 0x80;
const PRIVATE_STREAM_1_AC3_MAX: u8 = 0x8F;
const PRIVATE_STREAM_1_LPCM_MIN: u8 = 0xA0;
const PRIVATE_STREAM_1_LPCM_MAX: u8 = 0xAF;
const PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES: u32 = 4;
const PROGRAM_STREAM_MEDIA_TIMESCALE: u32 = 90_000;
const PROGRAM_STREAM_SCAN_CHUNK_BYTES: usize = 4096;
const PROGRAM_STREAM_LPCM_SAMPLE_ENTRY: FourCc = FourCc::from_bytes(*b"ipcm");

const fn program_stream_track_id(stream_id: u8) -> u32 {
    0x100 | stream_id as u32
}

struct ProgramStreamTrackBuilder {
    stream_id: u8,
    kind: ProgramStreamTrackKind,
    lpcm_format: Option<ProgramStreamLpcmFormat>,
    segments: Vec<SegmentedMuxSourceSegment>,
    total_size: u64,
    sample_offsets: Vec<u64>,
    sample_pts: Vec<u64>,
    sample_dts: Vec<u64>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ProgramStreamTrackKind {
    Mp3,
    Ac3,
    Lpcm,
    Video,
    Subpicture,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct ProgramStreamLpcmFormat {
    sample_rate: u32,
    channel_count: u16,
    bits_per_sample: u16,
    block_align: u16,
}

struct ParsedProgramStreamPesPacket {
    payload_offset: u64,
    payload_size: u32,
    packet_end: u64,
    presentation_time: Option<u64>,
    decode_time: Option<u64>,
}

struct ParsedPrivateStream1PesPacket {
    substream_id: u8,
    kind: ProgramStreamTrackKind,
    lpcm_format: Option<ProgramStreamLpcmFormat>,
    payload_offset: u64,
    payload_size: u32,
    packet_end: u64,
    presentation_time: Option<u64>,
}

pub(in crate::mux) fn scan_program_stream_sync(
    path: &Path,
    spec: &str,
) -> Result<Vec<CompositeTrackCandidate>, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    validate_program_stream_header_sync(&mut file, file_size, spec)?;

    let mut builders = BTreeMap::<u8, ProgramStreamTrackBuilder>::new();
    let mut offset = 0_u64;
    while offset < file_size {
        let start_code = read_program_stream_start_code_sync(&mut file, file_size, offset, spec)?;
        match start_code[3] {
            0xBA => {
                offset = parse_pack_header_sync(&mut file, file_size, offset, spec)?;
            }
            SYSTEM_HEADER_START_CODE
            | PROGRAM_STREAM_MAP_START_CODE
            | PADDING_STREAM_START_CODE
            | PRIVATE_STREAM_2_START_CODE => {
                offset = skip_length_delimited_ps_packet_sync(
                    &mut file,
                    file_size,
                    offset,
                    spec,
                    start_code[3],
                )?;
            }
            PRIVATE_STREAM_1_START_CODE => {
                let parsed = parse_private_stream_1_pes_packet_sync(
                    &mut file,
                    file_size,
                    offset,
                    spec,
                    start_code[3],
                )?;
                let builder = builders.entry(parsed.substream_id).or_insert_with(|| {
                    ProgramStreamTrackBuilder {
                        stream_id: parsed.substream_id,
                        kind: parsed.kind,
                        lpcm_format: parsed.lpcm_format,
                        segments: Vec::new(),
                        total_size: 0,
                        sample_offsets: Vec::new(),
                        sample_pts: Vec::new(),
                        sample_dts: Vec::new(),
                    }
                });
                if builder.kind != parsed.kind {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: format!(
                            "program stream private_stream_1 substream 0x{:02X} changed carried media kind mid-stream",
                            parsed.substream_id
                        ),
                    });
                }
                if let Some(parsed_format) = parsed.lpcm_format {
                    if let Some(expected_format) = builder.lpcm_format {
                        if expected_format != parsed_format {
                            return Err(MuxError::UnsupportedTrackImport {
                                spec: spec.to_string(),
                                message: format!(
                                    "program stream LPCM substream 0x{:02X} changed audio format mid-stream",
                                    parsed.substream_id
                                ),
                            });
                        }
                    } else {
                        builder.lpcm_format = Some(parsed_format);
                    }
                }
                if matches!(
                    builder.kind,
                    ProgramStreamTrackKind::Lpcm | ProgramStreamTrackKind::Subpicture
                ) {
                    builder.sample_offsets.push(builder.total_size);
                }
                if matches!(builder.kind, ProgramStreamTrackKind::Subpicture) {
                    builder.sample_pts.push(parsed.presentation_time.ok_or_else(|| {
                        MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "program stream subpicture PES packets must carry presentation timestamps"
                                    .to_string(),
                        }
                    })?);
                }
                append_file_range_segment(
                    &mut builder.segments,
                    &mut builder.total_size,
                    parsed.payload_offset,
                    parsed.payload_size,
                );
                offset = parsed.packet_end;
            }
            0xC0..=0xDF => {
                let parsed =
                    parse_pes_packet_sync(&mut file, file_size, offset, spec, start_code[3])?;
                let builder =
                    builders
                        .entry(start_code[3])
                        .or_insert_with(|| ProgramStreamTrackBuilder {
                            stream_id: start_code[3],
                            kind: ProgramStreamTrackKind::Mp3,
                            lpcm_format: None,
                            segments: Vec::new(),
                            total_size: 0,
                            sample_offsets: Vec::new(),
                            sample_pts: Vec::new(),
                            sample_dts: Vec::new(),
                        });
                append_file_range_segment(
                    &mut builder.segments,
                    &mut builder.total_size,
                    parsed.payload_offset,
                    parsed.payload_size,
                );
                offset = parsed.packet_end;
            }
            0xE0..=0xEF => {
                let parsed =
                    parse_pes_packet_sync(&mut file, file_size, offset, spec, start_code[3])?;
                let builder =
                    builders
                        .entry(start_code[3])
                        .or_insert_with(|| ProgramStreamTrackBuilder {
                            stream_id: start_code[3],
                            kind: ProgramStreamTrackKind::Video,
                            lpcm_format: None,
                            segments: Vec::new(),
                            total_size: 0,
                            sample_offsets: Vec::new(),
                            sample_pts: Vec::new(),
                            sample_dts: Vec::new(),
                        });
                if let Some(presentation_time) = parsed.presentation_time {
                    builder.sample_offsets.push(builder.total_size);
                    builder.sample_pts.push(presentation_time);
                    builder
                        .sample_dts
                        .push(parsed.decode_time.unwrap_or(presentation_time));
                }
                append_file_range_segment(
                    &mut builder.segments,
                    &mut builder.total_size,
                    parsed.payload_offset,
                    parsed.payload_size,
                );
                offset = parsed.packet_end;
            }
            0xB9 => break,
            other => {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!(
                        "unsupported MPEG program stream start code 0x{other:02X} on the native direct-ingest path"
                    ),
                });
            }
        }
    }

    finalize_program_stream_tracks_sync(path, spec, &mut file, builders)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_program_stream_async(
    path: &Path,
    spec: &str,
) -> Result<Vec<CompositeTrackCandidate>, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    validate_program_stream_header_async(&mut file, file_size, spec).await?;

    let mut builders = BTreeMap::<u8, ProgramStreamTrackBuilder>::new();
    let mut offset = 0_u64;
    while offset < file_size {
        let start_code =
            read_program_stream_start_code_async(&mut file, file_size, offset, spec).await?;
        match start_code[3] {
            0xBA => {
                offset = parse_pack_header_async(&mut file, file_size, offset, spec).await?;
            }
            SYSTEM_HEADER_START_CODE
            | PROGRAM_STREAM_MAP_START_CODE
            | PADDING_STREAM_START_CODE
            | PRIVATE_STREAM_2_START_CODE => {
                offset = skip_length_delimited_ps_packet_async(
                    &mut file,
                    file_size,
                    offset,
                    spec,
                    start_code[3],
                )
                .await?;
            }
            PRIVATE_STREAM_1_START_CODE => {
                let parsed = parse_private_stream_1_pes_packet_async(
                    &mut file,
                    file_size,
                    offset,
                    spec,
                    start_code[3],
                )
                .await?;
                let builder = builders.entry(parsed.substream_id).or_insert_with(|| {
                    ProgramStreamTrackBuilder {
                        stream_id: parsed.substream_id,
                        kind: parsed.kind,
                        lpcm_format: parsed.lpcm_format,
                        segments: Vec::new(),
                        total_size: 0,
                        sample_offsets: Vec::new(),
                        sample_pts: Vec::new(),
                        sample_dts: Vec::new(),
                    }
                });
                if builder.kind != parsed.kind {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: format!(
                            "program stream private_stream_1 substream 0x{:02X} changed carried media kind mid-stream",
                            parsed.substream_id
                        ),
                    });
                }
                if let Some(parsed_format) = parsed.lpcm_format {
                    if let Some(expected_format) = builder.lpcm_format {
                        if expected_format != parsed_format {
                            return Err(MuxError::UnsupportedTrackImport {
                                spec: spec.to_string(),
                                message: format!(
                                    "program stream LPCM substream 0x{:02X} changed audio format mid-stream",
                                    parsed.substream_id
                                ),
                            });
                        }
                    } else {
                        builder.lpcm_format = Some(parsed_format);
                    }
                }
                if matches!(
                    builder.kind,
                    ProgramStreamTrackKind::Lpcm | ProgramStreamTrackKind::Subpicture
                ) {
                    builder.sample_offsets.push(builder.total_size);
                }
                if matches!(builder.kind, ProgramStreamTrackKind::Subpicture) {
                    builder.sample_pts.push(parsed.presentation_time.ok_or_else(|| {
                        MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "program stream subpicture PES packets must carry presentation timestamps"
                                    .to_string(),
                        }
                    })?);
                }
                append_file_range_segment(
                    &mut builder.segments,
                    &mut builder.total_size,
                    parsed.payload_offset,
                    parsed.payload_size,
                );
                offset = parsed.packet_end;
            }
            0xC0..=0xDF => {
                let parsed =
                    parse_pes_packet_async(&mut file, file_size, offset, spec, start_code[3])
                        .await?;
                let builder =
                    builders
                        .entry(start_code[3])
                        .or_insert_with(|| ProgramStreamTrackBuilder {
                            stream_id: start_code[3],
                            kind: ProgramStreamTrackKind::Mp3,
                            lpcm_format: None,
                            segments: Vec::new(),
                            total_size: 0,
                            sample_offsets: Vec::new(),
                            sample_pts: Vec::new(),
                            sample_dts: Vec::new(),
                        });
                append_file_range_segment(
                    &mut builder.segments,
                    &mut builder.total_size,
                    parsed.payload_offset,
                    parsed.payload_size,
                );
                offset = parsed.packet_end;
            }
            0xE0..=0xEF => {
                let parsed =
                    parse_pes_packet_async(&mut file, file_size, offset, spec, start_code[3])
                        .await?;
                let builder =
                    builders
                        .entry(start_code[3])
                        .or_insert_with(|| ProgramStreamTrackBuilder {
                            stream_id: start_code[3],
                            kind: ProgramStreamTrackKind::Video,
                            lpcm_format: None,
                            segments: Vec::new(),
                            total_size: 0,
                            sample_offsets: Vec::new(),
                            sample_pts: Vec::new(),
                            sample_dts: Vec::new(),
                        });
                if let Some(presentation_time) = parsed.presentation_time {
                    builder.sample_offsets.push(builder.total_size);
                    builder.sample_pts.push(presentation_time);
                    builder
                        .sample_dts
                        .push(parsed.decode_time.unwrap_or(presentation_time));
                }
                append_file_range_segment(
                    &mut builder.segments,
                    &mut builder.total_size,
                    parsed.payload_offset,
                    parsed.payload_size,
                );
                offset = parsed.packet_end;
            }
            0xB9 => break,
            other => {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!(
                        "unsupported MPEG program stream start code 0x{other:02X} on the native direct-ingest path"
                    ),
                });
            }
        }
    }

    finalize_program_stream_tracks_async(path, spec, &mut file, builders).await
}

fn finalize_program_stream_tracks_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    builders: BTreeMap<u8, ProgramStreamTrackBuilder>,
) -> Result<Vec<CompositeTrackCandidate>, MuxError> {
    if builders.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "program stream input did not contain any supported MPEG audio, AC-3, LPCM, VobSub-style subpicture, or MPEG-2/MPEG-4 Part 2/H.264/H.265/VVC video payloads"
                    .to_string(),
        });
    }
    let mut tracks = Vec::new();
    for builder in builders.into_values() {
        tracks.push(match builder.kind {
            ProgramStreamTrackKind::Mp3 => {
                finalize_program_stream_mp3_track_sync(path, spec, file, builder)?
            }
            ProgramStreamTrackKind::Ac3 => {
                finalize_program_stream_ac3_track_sync(path, spec, file, builder)?
            }
            ProgramStreamTrackKind::Lpcm => {
                finalize_program_stream_lpcm_track_sync(path, spec, builder)?
            }
            ProgramStreamTrackKind::Subpicture => {
                finalize_program_stream_subpicture_track_sync(path, spec, file, builder)?
            }
            ProgramStreamTrackKind::Video => {
                finalize_program_stream_video_track_sync(path, spec, file, builder)?
            }
        });
    }
    Ok(tracks)
}

#[cfg(feature = "async")]
async fn finalize_program_stream_tracks_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    builders: BTreeMap<u8, ProgramStreamTrackBuilder>,
) -> Result<Vec<CompositeTrackCandidate>, MuxError> {
    if builders.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "program stream input did not contain any supported MPEG audio, AC-3, LPCM, VobSub-style subpicture, or MPEG-2/MPEG-4 Part 2/H.264/H.265/VVC video payloads"
                    .to_string(),
        });
    }
    let mut tracks = Vec::new();
    for builder in builders.into_values() {
        tracks.push(match builder.kind {
            ProgramStreamTrackKind::Mp3 => {
                finalize_program_stream_mp3_track_async(path, spec, file, builder).await?
            }
            ProgramStreamTrackKind::Ac3 => {
                finalize_program_stream_ac3_track_async(path, spec, file, builder).await?
            }
            ProgramStreamTrackKind::Lpcm => {
                finalize_program_stream_lpcm_track_async(path, spec, builder).await?
            }
            ProgramStreamTrackKind::Subpicture => {
                finalize_program_stream_subpicture_track_async(path, spec, file, builder).await?
            }
            ProgramStreamTrackKind::Video => {
                finalize_program_stream_video_track_async(path, spec, file, builder).await?
            }
        });
    }
    Ok(tracks)
}

fn finalize_program_stream_ac3_track_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    builder: ProgramStreamTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed = scan_ac3_segmented_sync(file, &builder.segments, builder.total_size, spec)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: program_stream_track_id(PRIVATE_STREAM_1_START_CODE),
            kind: MuxTrackKind::Audio,
            timescale: PROGRAM_STREAM_MEDIA_TIMESCALE,
            language: *b"und",
            handler_name: direct_ingest_handler_name("ac3"),
            mux_policy: direct_ingest_mux_policy("ac3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box: parsed.sample_entry_box,
            source_edit_media_time: None,
            samples: normalize_program_stream_ac3_samples(
                spec,
                parsed.sample_rate,
                parsed.samples,
            )?,
        },
        source_spec: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: builder.segments,
            total_size: builder.total_size,
        },
    })
}

fn finalize_program_stream_mp3_track_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    builder: ProgramStreamTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let mut offset = 0_u64;
    let mut expected = None::<(u32, u16, u32)>;
    let mut samples = Vec::new();
    while offset < builder.total_size {
        if builder.total_size - offset < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated MPEG audio frame header inside program stream payload"
                    .to_string(),
            });
        }
        let mut header = [0_u8; 4];
        read_segmented_bytes_sync(
            file,
            &builder.segments,
            builder.total_size,
            offset,
            &mut header,
            spec,
            "truncated MPEG audio frame header inside program stream payload",
        )?;
        let parsed = parse_mp3_frame_header(&header, offset, spec)?;
        if offset
            .checked_add(u64::from(parsed.frame_length))
            .is_none_or(|end| end > builder.total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "truncated MPEG audio frame at logical program-stream offset {offset}"
                ),
            });
        }
        let descriptor = (
            parsed.sample_rate,
            parsed.channel_count,
            parsed.sample_duration,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "program stream MPEG audio frames changed sample rate or channel layout mid-stream"
                            .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }
        samples.push(CandidateSample {
            source_index: usize::MAX,
            data_offset: offset,
            data_size: parsed.frame_length,
            duration: parsed.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(u64::from(parsed.frame_length))
            .ok_or(MuxError::LayoutOverflow("program stream MPEG audio offset"))?;
    }

    let (sample_rate, channel_count, _) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream input did not contain any MPEG audio frames".to_string(),
        })?;
    let sample_entry_box = build_mp3_sample_entry_box(
        sample_rate,
        channel_count,
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
    )?;
    let samples = normalize_program_stream_mp3_samples(spec, sample_rate, samples)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: program_stream_track_id(builder.stream_id),
            kind: MuxTrackKind::Audio,
            timescale: PROGRAM_STREAM_MEDIA_TIMESCALE,
            language: *b"und",
            handler_name: direct_ingest_handler_name("mp3"),
            mux_policy: direct_ingest_mux_policy("mp3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box,
            source_edit_media_time: None,
            samples,
        },
        source_spec: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: builder.segments,
            total_size: builder.total_size,
        },
    })
}

fn finalize_program_stream_video_track_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    builder: ProgramStreamTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let prefix = read_program_stream_video_prefix_sync(file, &builder, spec)?;
    match detect_path_track_kind_from_prefix(&prefix) {
        DetectedPathTrackKind::Raw(super::super::MuxRawCodec::Mpeg2v) => {
            let mut parsed =
                scan_mpeg2v_segmented_sync(file, &builder.segments, builder.total_size, spec)?;
            if parsed.eof_terminated_trailing_sample {
                parsed.samples.pop();
            }
            let (timescale, source_edit_media_time, samples) =
                normalize_program_stream_mpeg2v_samples(
                spec,
                parsed.timescale,
                parsed.samples,
                &builder.sample_offsets,
                &builder.sample_pts,
                &builder.sample_dts,
            )?;
            let sample_entry_box = build_program_stream_mpeg2v_sample_entry_box(
                ProgramStreamMpeg2vSampleEntryConfig {
                    width: parsed.width,
                    height: parsed.height,
                    decoder_specific_info: &parsed.decoder_specific_info,
                    object_type_indication: parsed.object_type_indication,
                    timescale,
                    leading_media_time: source_edit_media_time.unwrap_or(0),
                    pixel_aspect_ratio: parsed.pixel_aspect_ratio,
                },
                samples.iter().map(|sample| (sample.data_size, sample.duration)),
            )?;
            Ok(CompositeTrackCandidate {
                track: TrackCandidate {
                    track_id: program_stream_track_id(builder.stream_id),
                    kind: MuxTrackKind::Video,
                    timescale,
                    language: *b"und",
                    handler_name: direct_ingest_handler_name("mpeg2v"),
                    mux_policy: direct_ingest_mux_policy("mpeg2v", MuxTrackKind::Video),
                    width: parsed.width,
                    height: parsed.height,
                    sample_entry_box,
                    source_edit_media_time,
                    samples,
                },
                source_spec: SegmentedMuxSourceSpec {
                    path: path.to_path_buf(),
                    segments: builder.segments,
                    total_size: builder.total_size,
                },
            })
        }
        DetectedPathTrackKind::Raw(super::super::MuxRawCodec::Mp4v) => {
            let parsed = scan_mp4v_segmented_sync(file, &builder.segments, builder.total_size, spec)?;
            let (timescale, source_edit_media_time, samples) =
                normalize_program_stream_mp4v_samples(
                    spec,
                    parsed.timescale,
                    parsed.samples,
                    &builder.sample_offsets,
                    &builder.sample_pts,
                    &builder.sample_dts,
                )?;
            Ok(CompositeTrackCandidate {
                track: TrackCandidate {
                    track_id: program_stream_track_id(builder.stream_id),
                    kind: MuxTrackKind::Video,
                    timescale,
                    language: *b"und",
                    handler_name: direct_ingest_handler_name("mp4v"),
                    mux_policy: direct_ingest_mux_policy("mp4v", MuxTrackKind::Video),
                    width: parsed.width,
                    height: parsed.height,
                    sample_entry_box: super::super::mp4::strip_visual_sample_entry_immediate_children(
                        &parsed.sample_entry_box,
                        &[FourCc::from_bytes(*b"pasp")],
                    )?,
                    source_edit_media_time,
                    samples,
                },
                source_spec: SegmentedMuxSourceSpec {
                    path: path.to_path_buf(),
                    segments: builder.segments,
                    total_size: builder.total_size,
                },
            })
        }
        DetectedPathTrackKind::Raw(super::super::MuxRawCodec::H264) => {
            let parsed =
                stage_annex_b_h264_segmented_sync(path, file, &builder.segments, builder.total_size, spec)?;
            Ok(CompositeTrackCandidate {
                track: TrackCandidate {
                    track_id: program_stream_track_id(builder.stream_id),
                    kind: MuxTrackKind::Video,
                    timescale: parsed.timescale,
                    language: *b"und",
                    handler_name: direct_ingest_handler_name("h264"),
                    mux_policy: direct_ingest_mux_policy("h264", MuxTrackKind::Video),
                    width: parsed.track_width,
                    height: parsed.track_height,
                    sample_entry_box: parsed.sample_entry_box,
                    source_edit_media_time: parsed.source_edit_media_time,
                    samples: parsed
                        .samples
                        .into_iter()
                        .map(|sample| CandidateSample {
                            source_index: usize::MAX,
                            data_offset: sample.data_offset,
                            data_size: sample.data_size,
                            duration: sample.duration,
                            composition_time_offset: sample.composition_time_offset,
                            is_sync_sample: sample.is_sync_sample,
                        })
                        .collect(),
                },
                source_spec: parsed.segmented_source,
            })
        }
        DetectedPathTrackKind::Raw(super::super::MuxRawCodec::H265) => {
            let parsed =
                stage_annex_b_h265_segmented_sync(path, file, &builder.segments, builder.total_size, spec)?;
            Ok(CompositeTrackCandidate {
                track: TrackCandidate {
                    track_id: program_stream_track_id(builder.stream_id),
                    kind: MuxTrackKind::Video,
                    timescale: parsed.timescale,
                    language: *b"und",
                    handler_name: direct_ingest_handler_name("h265"),
                    mux_policy: direct_ingest_mux_policy("h265", MuxTrackKind::Video),
                    width: parsed.track_width,
                    height: parsed.track_height,
                    sample_entry_box: parsed.sample_entry_box,
                    source_edit_media_time: parsed.source_edit_media_time,
                    samples: parsed
                        .samples
                        .into_iter()
                        .map(|sample| CandidateSample {
                            source_index: usize::MAX,
                            data_offset: sample.data_offset,
                            data_size: sample.data_size,
                            duration: sample.duration,
                            composition_time_offset: sample.composition_time_offset,
                            is_sync_sample: sample.is_sync_sample,
                        })
                        .collect(),
                },
                source_spec: parsed.segmented_source,
            })
        }
        DetectedPathTrackKind::Raw(super::super::MuxRawCodec::Vvc) => {
            let parsed =
                stage_annex_b_vvc_segmented_sync(path, file, &builder.segments, builder.total_size, spec)?;
            Ok(CompositeTrackCandidate {
                track: TrackCandidate {
                    track_id: program_stream_track_id(builder.stream_id),
                    kind: MuxTrackKind::Video,
                    timescale: parsed.timescale,
                    language: *b"und",
                    handler_name: direct_ingest_handler_name("vvc"),
                    mux_policy: direct_ingest_mux_policy("vvc", MuxTrackKind::Video),
                    width: parsed.track_width,
                    height: parsed.track_height,
                    sample_entry_box: parsed.sample_entry_box,
                    source_edit_media_time: parsed.source_edit_media_time,
                    samples: parsed
                        .samples
                        .into_iter()
                        .map(|sample| CandidateSample {
                            source_index: usize::MAX,
                            data_offset: sample.data_offset,
                            data_size: sample.data_size,
                            duration: sample.duration,
                            composition_time_offset: sample.composition_time_offset,
                            is_sync_sample: sample.is_sync_sample,
                        })
                        .collect(),
                },
                source_spec: parsed.segmented_source,
            })
        }
        _ => Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "program stream video payload is not a supported MPEG-2, MPEG-4 Part 2, H.264, H.265, or VVC elementary stream"
                    .to_string(),
        }),
    }
}

fn finalize_program_stream_subpicture_track_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    builder: ProgramStreamTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let samples = build_program_stream_subpicture_samples_sync(file, spec, &builder)?;
    let sample_entry_box = build_subpicture_sample_entry_box(&[], &samples)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: program_stream_track_id(PRIVATE_STREAM_1_START_CODE),
            kind: MuxTrackKind::Subtitle,
            timescale: VOBSUB_TIMESCALE,
            language: *b"und",
            handler_name: direct_ingest_handler_name("vobsub"),
            mux_policy: direct_ingest_mux_policy("vobsub", MuxTrackKind::Subtitle),
            width: 0,
            height: 0,
            sample_entry_box,
            source_edit_media_time: None,
            samples,
        },
        source_spec: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: builder.segments,
            total_size: builder.total_size,
        },
    })
}

fn finalize_program_stream_lpcm_track_sync(
    path: &Path,
    spec: &str,
    builder: ProgramStreamTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let format = builder
        .lpcm_format
        .ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream LPCM track did not retain a parsed audio format".to_string(),
        })?;
    let sample_entry_box = build_pcm_sample_entry_box(
        PROGRAM_STREAM_LPCM_SAMPLE_ENTRY,
        format.sample_rate,
        format.channel_count,
        format.bits_per_sample,
        false,
    )?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: program_stream_track_id(PRIVATE_STREAM_1_START_CODE),
            kind: MuxTrackKind::Audio,
            timescale: format.sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("pcm"),
            mux_policy: direct_ingest_mux_policy("pcm", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box,
            source_edit_media_time: None,
            samples: build_program_stream_lpcm_samples(spec, &builder, format)?,
        },
        source_spec: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: builder.segments,
            total_size: builder.total_size,
        },
    })
}

#[cfg(feature = "async")]
async fn finalize_program_stream_mp3_track_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    builder: ProgramStreamTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let mut offset = 0_u64;
    let mut expected = None::<(u32, u16, u32)>;
    let mut samples = Vec::new();
    while offset < builder.total_size {
        if builder.total_size - offset < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated MPEG audio frame header inside program stream payload"
                    .to_string(),
            });
        }
        let mut header = [0_u8; 4];
        read_segmented_bytes_async(
            file,
            &builder.segments,
            builder.total_size,
            offset,
            &mut header,
            spec,
            "truncated MPEG audio frame header inside program stream payload",
        )
        .await?;
        let parsed = parse_mp3_frame_header(&header, offset, spec)?;
        if offset
            .checked_add(u64::from(parsed.frame_length))
            .is_none_or(|end| end > builder.total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "truncated MPEG audio frame at logical program-stream offset {offset}"
                ),
            });
        }
        let descriptor = (
            parsed.sample_rate,
            parsed.channel_count,
            parsed.sample_duration,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "program stream MPEG audio frames changed sample rate or channel layout mid-stream"
                            .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }
        samples.push(CandidateSample {
            source_index: usize::MAX,
            data_offset: offset,
            data_size: parsed.frame_length,
            duration: parsed.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(u64::from(parsed.frame_length))
            .ok_or(MuxError::LayoutOverflow("program stream MPEG audio offset"))?;
    }

    let (sample_rate, channel_count, _) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream input did not contain any MPEG audio frames".to_string(),
        })?;
    let sample_entry_box = build_mp3_sample_entry_box(
        sample_rate,
        channel_count,
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
    )?;
    let samples = normalize_program_stream_mp3_samples(spec, sample_rate, samples)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: program_stream_track_id(builder.stream_id),
            kind: MuxTrackKind::Audio,
            timescale: PROGRAM_STREAM_MEDIA_TIMESCALE,
            language: *b"und",
            handler_name: direct_ingest_handler_name("mp3"),
            mux_policy: direct_ingest_mux_policy("mp3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box,
            source_edit_media_time: None,
            samples,
        },
        source_spec: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: builder.segments,
            total_size: builder.total_size,
        },
    })
}

#[cfg(feature = "async")]
async fn finalize_program_stream_ac3_track_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    builder: ProgramStreamTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed =
        scan_ac3_segmented_async(file, &builder.segments, builder.total_size, spec).await?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: program_stream_track_id(PRIVATE_STREAM_1_START_CODE),
            kind: MuxTrackKind::Audio,
            timescale: PROGRAM_STREAM_MEDIA_TIMESCALE,
            language: *b"und",
            handler_name: direct_ingest_handler_name("ac3"),
            mux_policy: direct_ingest_mux_policy("ac3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box: parsed.sample_entry_box,
            source_edit_media_time: None,
            samples: normalize_program_stream_ac3_samples(
                spec,
                parsed.sample_rate,
                parsed.samples,
            )?,
        },
        source_spec: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: builder.segments,
            total_size: builder.total_size,
        },
    })
}

#[cfg(feature = "async")]
async fn finalize_program_stream_subpicture_track_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    builder: ProgramStreamTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let samples = build_program_stream_subpicture_samples_async(file, spec, &builder).await?;
    let sample_entry_box = build_subpicture_sample_entry_box(&[], &samples)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: program_stream_track_id(PRIVATE_STREAM_1_START_CODE),
            kind: MuxTrackKind::Subtitle,
            timescale: VOBSUB_TIMESCALE,
            language: *b"und",
            handler_name: direct_ingest_handler_name("vobsub"),
            mux_policy: direct_ingest_mux_policy("vobsub", MuxTrackKind::Subtitle),
            width: 0,
            height: 0,
            sample_entry_box,
            source_edit_media_time: None,
            samples,
        },
        source_spec: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: builder.segments,
            total_size: builder.total_size,
        },
    })
}

#[cfg(feature = "async")]
async fn finalize_program_stream_lpcm_track_async(
    path: &Path,
    spec: &str,
    builder: ProgramStreamTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let format = builder
        .lpcm_format
        .ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream LPCM track did not retain a parsed audio format".to_string(),
        })?;
    let sample_entry_box = build_pcm_sample_entry_box(
        PROGRAM_STREAM_LPCM_SAMPLE_ENTRY,
        format.sample_rate,
        format.channel_count,
        format.bits_per_sample,
        false,
    )?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: program_stream_track_id(PRIVATE_STREAM_1_START_CODE),
            kind: MuxTrackKind::Audio,
            timescale: format.sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("pcm"),
            mux_policy: direct_ingest_mux_policy("pcm", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box,
            source_edit_media_time: None,
            samples: build_program_stream_lpcm_samples(spec, &builder, format)?,
        },
        source_spec: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: builder.segments,
            total_size: builder.total_size,
        },
    })
}

fn build_program_stream_subpicture_samples_sync(
    file: &mut File,
    spec: &str,
    builder: &ProgramStreamTrackBuilder,
) -> Result<Vec<CandidateSample>, MuxError> {
    if builder.sample_offsets.len() != builder.sample_pts.len() || builder.sample_offsets.is_empty()
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "program stream subpicture input did not contain any complete VobSub-style PES payloads"
                    .to_string(),
        });
    }
    let mut samples = Vec::with_capacity(builder.sample_offsets.len());
    for (index, (&sample_offset, &sample_pts)) in builder
        .sample_offsets
        .iter()
        .zip(builder.sample_pts.iter())
        .enumerate()
    {
        let next_offset = builder
            .sample_offsets
            .get(index + 1)
            .copied()
            .unwrap_or(builder.total_size);
        if next_offset <= sample_offset {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "program stream subpicture samples must advance monotonically".to_string(),
            });
        }
        let data_size = u32::try_from(next_offset - sample_offset)
            .map_err(|_| MuxError::LayoutOverflow("program stream subpicture sample size"))?;
        let mut packet_bytes = vec![
            0_u8;
            usize::try_from(data_size).map_err(|_| {
                MuxError::LayoutOverflow("program stream subpicture sample size")
            })?
        ];
        read_segmented_bytes_sync(
            file,
            &builder.segments,
            builder.total_size,
            sample_offset,
            &mut packet_bytes,
            spec,
            "program stream subpicture payload is truncated",
        )?;
        let duration = subpicture_sample_duration(
            spec,
            &packet_bytes,
            sample_pts,
            builder.sample_pts.get(index + 1).copied(),
        )?;
        samples.push(CandidateSample {
            source_index: usize::MAX,
            data_offset: sample_offset,
            data_size,
            duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    Ok(samples)
}

#[cfg(feature = "async")]
async fn build_program_stream_subpicture_samples_async(
    file: &mut TokioFile,
    spec: &str,
    builder: &ProgramStreamTrackBuilder,
) -> Result<Vec<CandidateSample>, MuxError> {
    if builder.sample_offsets.len() != builder.sample_pts.len() || builder.sample_offsets.is_empty()
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "program stream subpicture input did not contain any complete VobSub-style PES payloads"
                    .to_string(),
        });
    }
    let mut samples = Vec::with_capacity(builder.sample_offsets.len());
    for (index, (&sample_offset, &sample_pts)) in builder
        .sample_offsets
        .iter()
        .zip(builder.sample_pts.iter())
        .enumerate()
    {
        let next_offset = builder
            .sample_offsets
            .get(index + 1)
            .copied()
            .unwrap_or(builder.total_size);
        if next_offset <= sample_offset {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "program stream subpicture samples must advance monotonically".to_string(),
            });
        }
        let data_size = u32::try_from(next_offset - sample_offset)
            .map_err(|_| MuxError::LayoutOverflow("program stream subpicture sample size"))?;
        let mut packet_bytes = vec![
            0_u8;
            usize::try_from(data_size).map_err(|_| {
                MuxError::LayoutOverflow("program stream subpicture sample size")
            })?
        ];
        read_segmented_bytes_async(
            file,
            &builder.segments,
            builder.total_size,
            sample_offset,
            &mut packet_bytes,
            spec,
            "program stream subpicture payload is truncated",
        )
        .await?;
        let duration = subpicture_sample_duration(
            spec,
            &packet_bytes,
            sample_pts,
            builder.sample_pts.get(index + 1).copied(),
        )?;
        samples.push(CandidateSample {
            source_index: usize::MAX,
            data_offset: sample_offset,
            data_size,
            duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    Ok(samples)
}

fn subpicture_sample_duration(
    spec: &str,
    packet_bytes: &[u8],
    start_pts: u64,
    next_start: Option<u64>,
) -> Result<u32, MuxError> {
    if packet_bytes.len() < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream subpicture payload".to_string(),
        });
    }
    let packet_size = u32::from(u16::from_be_bytes([packet_bytes[0], packet_bytes[1]]));
    let control_offset = u32::from(u16::from_be_bytes([packet_bytes[2], packet_bytes[3]]));
    let parsed_duration = parse_vobsub_duration(packet_bytes, packet_size, control_offset, spec)?;
    effective_vobsub_duration(parsed_duration, start_pts, next_start)
}

fn build_program_stream_lpcm_samples(
    spec: &str,
    builder: &ProgramStreamTrackBuilder,
    format: ProgramStreamLpcmFormat,
) -> Result<Vec<CandidateSample>, MuxError> {
    if builder.sample_offsets.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream input did not contain any complete LPCM PES payloads"
                .to_string(),
        });
    }
    builder
        .sample_offsets
        .iter()
        .enumerate()
        .map(|(index, &sample_offset)| {
            let next_offset = builder
                .sample_offsets
                .get(index + 1)
                .copied()
                .unwrap_or(builder.total_size);
            if next_offset <= sample_offset {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "program stream LPCM samples must advance monotonically"
                        .to_string(),
                });
            }
            let data_size = u32::try_from(next_offset - sample_offset)
                .map_err(|_| MuxError::LayoutOverflow("program stream LPCM sample size"))?;
            if data_size % u32::from(format.block_align) != 0 {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!(
                        "program stream LPCM sample size {data_size} is not aligned to the declared {}-byte frame size",
                        format.block_align
                    ),
                });
            }
            let duration = data_size / u32::from(format.block_align);
            if duration == 0 {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "program stream LPCM sample duration underflowed to zero"
                        .to_string(),
                });
            }
            Ok(CandidateSample {
                source_index: usize::MAX,
                data_offset: sample_offset,
                data_size,
                duration,
                composition_time_offset: 0,
                is_sync_sample: true,
            })
        })
        .collect()
}

fn normalize_program_stream_ac3_samples(
    spec: &str,
    sample_rate: u32,
    samples: Vec<StagedSample>,
) -> Result<Vec<CandidateSample>, MuxError> {
    if sample_rate == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream AC-3 input reported a zero sample rate".to_string(),
        });
    }

    let mut duration_remainder = 0_u64;
    samples
        .into_iter()
        .map(|sample| {
            let scaled_duration = u64::from(sample.duration)
                .checked_mul(u64::from(PROGRAM_STREAM_MEDIA_TIMESCALE))
                .ok_or(MuxError::LayoutOverflow("program stream AC-3 duration"))?
                .checked_add(duration_remainder)
                .ok_or(MuxError::LayoutOverflow("program stream AC-3 duration"))?;
            let duration = scaled_duration / u64::from(sample_rate);
            duration_remainder = scaled_duration % u64::from(sample_rate);
            let duration = u32::try_from(duration)
                .map_err(|_| MuxError::LayoutOverflow("program stream AC-3 duration"))?;
            if duration == 0 {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "program stream AC-3 frame duration underflowed after media-timescale normalization"
                            .to_string(),
                });
            }
            Ok(CandidateSample {
                source_index: usize::MAX,
                data_offset: sample.data_offset,
                data_size: sample.data_size,
                duration,
                composition_time_offset: sample.composition_time_offset,
                is_sync_sample: sample.is_sync_sample,
            })
        })
        .collect()
}

fn normalize_program_stream_mp3_samples(
    spec: &str,
    sample_rate: u32,
    samples: Vec<CandidateSample>,
) -> Result<Vec<CandidateSample>, MuxError> {
    if sample_rate == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream MPEG audio reported a zero sample rate".to_string(),
        });
    }

    let mut duration_remainder = 0_u64;
    samples
        .into_iter()
        .map(|sample| {
            let scaled_duration = u64::from(sample.duration)
                .checked_mul(u64::from(PROGRAM_STREAM_MEDIA_TIMESCALE))
                .ok_or(MuxError::LayoutOverflow(
                    "program stream MPEG audio duration",
                ))?
                .checked_add(duration_remainder)
                .ok_or(MuxError::LayoutOverflow(
                    "program stream MPEG audio duration",
                ))?;
            let duration = scaled_duration / u64::from(sample_rate);
            duration_remainder = scaled_duration % u64::from(sample_rate);
            Ok(CandidateSample {
                duration: u32::try_from(duration)
                    .map_err(|_| MuxError::LayoutOverflow("program stream MPEG audio duration"))?,
                ..sample
            })
        })
        .collect()
}

fn normalize_program_stream_mpeg2v_samples(
    spec: &str,
    elementary_timescale: u32,
    mut samples: Vec<StagedSample>,
    sample_offsets: &[u64],
    sample_pts: &[u64],
    sample_dts: &[u64],
) -> Result<(u32, Option<u64>, Vec<CandidateSample>), MuxError> {
    if sample_pts.is_empty() {
        return Ok((
            elementary_timescale,
            None,
            samples
                .into_iter()
                .map(|sample| CandidateSample {
                    source_index: usize::MAX,
                    data_offset: sample.data_offset,
                    data_size: sample.data_size,
                    duration: sample.duration,
                    composition_time_offset: sample.composition_time_offset,
                    is_sync_sample: sample.is_sync_sample,
                })
                .collect(),
        ));
    }

    if sample_pts.len() != sample_dts.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream MPEG-2 video timing anchors disagreed between presentation and decode timestamps"
                .to_string(),
        });
    }
    if sample_offsets.len() != sample_pts.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "program stream MPEG-2 video timing anchors disagreed between payload offsets and timestamps"
                    .to_string(),
        });
    }

    if sample_pts.len() + 1 == samples.len() {
        samples.pop();
    }

    if sample_pts.len() > samples.len() + 1 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "program stream MPEG-2 video PES timing anchors ({}) did not match parsed picture count ({})",
                sample_pts.len(),
                samples.len(),
            ),
        });
    }

    let anchor_to_sample =
        map_program_stream_mpeg2v_anchor_offsets_to_picture_samples(sample_offsets, &samples);
    let sample_to_anchor = build_program_stream_mpeg2v_sample_anchor_map(
        spec,
        sample_offsets,
        &anchor_to_sample,
        samples.len(),
    )?;

    let mut normalized = Vec::with_capacity(samples.len());
    let mut source_edit_media_time = None;
    let mut last_composition_time_offset = 0_i32;
    for (index, sample) in samples.into_iter().enumerate() {
        let scaled_sample_duration = scale_mpeg2v_duration_to_program_stream_clock(
            spec,
            elementary_timescale,
            sample.duration,
        )?;
        let (duration, composition_time_offset) = if let Some(anchor_index) =
            sample_to_anchor[index]
        {
            let current_pts = sample_pts[anchor_index];
            let current_dts = sample_dts[anchor_index];
            if current_pts < current_dts {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "program stream MPEG-2 video presentation timestamps must not precede decode timestamps"
                            .to_string(),
                });
            }
            let duration = if let Some((next_sample_index, next_anchor_index)) = sample_to_anchor
                [index + 1..]
                .iter()
                .enumerate()
                .find_map(|(delta, anchor)| {
                    anchor.map(|anchor_index| (index + 1 + delta, anchor_index))
                }) {
                let next_dts = sample_dts[next_anchor_index];
                if next_dts <= current_dts {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message:
                            "program stream MPEG-2 video decode timestamps must increase monotonically"
                                .to_string(),
                    });
                }
                if next_sample_index == index + 1 {
                    u32::try_from(next_dts - current_dts).map_err(|_| {
                        MuxError::LayoutOverflow("program stream MPEG-2 video duration")
                    })?
                } else {
                    scaled_sample_duration
                }
            } else {
                scaled_sample_duration
            };
            let composition_time_offset =
                i32::try_from(current_pts - current_dts).map_err(|_| {
                    MuxError::LayoutOverflow("program stream MPEG-2 video composition offset")
                })?;
            last_composition_time_offset = composition_time_offset;
            if index == 0 && composition_time_offset > 0 {
                source_edit_media_time =
                    Some(u64::try_from(composition_time_offset).map_err(|_| {
                        MuxError::LayoutOverflow("program stream MPEG-2 video edit")
                    })?);
            }
            (duration, composition_time_offset)
        } else {
            let duration = if sample_to_anchor[index + 1..].iter().any(Option::is_some) {
                scaled_sample_duration
            } else {
                sample.duration
            };
            (duration, last_composition_time_offset)
        };
        if duration == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message:
                    "program stream MPEG-2 video frame duration underflowed after media-timescale normalization"
                        .to_string(),
            });
        }
        normalized.push(CandidateSample {
            source_index: usize::MAX,
            data_offset: sample.data_offset,
            data_size: sample.data_size,
            duration,
            composition_time_offset,
            is_sync_sample: sample.is_sync_sample,
        });
    }

    Ok((
        PROGRAM_STREAM_MEDIA_TIMESCALE,
        source_edit_media_time,
        normalized,
    ))
}

fn normalize_program_stream_mp4v_samples(
    spec: &str,
    elementary_timescale: u32,
    mut samples: Vec<StagedSample>,
    sample_offsets: &[u64],
    sample_pts: &[u64],
    sample_dts: &[u64],
) -> Result<(u32, Option<u64>, Vec<CandidateSample>), MuxError> {
    if sample_pts.is_empty() {
        return Ok((
            elementary_timescale,
            None,
            samples
                .into_iter()
                .map(|sample| CandidateSample {
                    source_index: usize::MAX,
                    data_offset: sample.data_offset,
                    data_size: sample.data_size,
                    duration: sample.duration,
                    composition_time_offset: sample.composition_time_offset,
                    is_sync_sample: sample.is_sync_sample,
                })
                .collect(),
        ));
    }

    if sample_pts.len() != sample_dts.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream MPEG-4 Part 2 video timing anchors disagreed between presentation and decode timestamps"
                .to_string(),
        });
    }
    if sample_offsets.len() != sample_pts.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "program stream MPEG-4 Part 2 video timing anchors disagreed between payload offsets and timestamps"
                    .to_string(),
        });
    }

    if sample_pts.len() + 1 == samples.len() {
        samples.pop();
    }

    if sample_pts.len() > samples.len() + 1 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "program stream MPEG-4 Part 2 video PES timing anchors ({}) did not match parsed picture count ({})",
                sample_pts.len(),
                samples.len(),
            ),
        });
    }

    let anchor_to_sample =
        map_program_stream_mpeg2v_anchor_offsets_to_picture_samples(sample_offsets, &samples);
    let mut sample_to_anchor = build_program_stream_mpeg2v_sample_anchor_map(
        spec,
        sample_offsets,
        &anchor_to_sample,
        samples.len(),
    )?;
    if sample_to_anchor.len() > 1
        && sample_to_anchor.last().is_some_and(Option::is_some)
        && sample_pts.len() == samples.len()
        && sample_dts.len() == samples.len()
    {
        samples.pop();
        sample_to_anchor.pop();
    }

    let mut normalized = Vec::with_capacity(samples.len());
    let mut source_edit_media_time = None;
    let mut last_composition_time_offset = 0_i32;
    for (index, sample) in samples.into_iter().enumerate() {
        let scaled_sample_duration = scale_mp4v_duration_to_program_stream_clock(
            spec,
            elementary_timescale,
            sample.duration,
        )?;
        let (duration, composition_time_offset) = if let Some(anchor_index) =
            sample_to_anchor[index]
        {
            let current_pts = sample_pts[anchor_index];
            let current_dts = sample_dts[anchor_index];
            if current_pts < current_dts {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "program stream MPEG-4 Part 2 video presentation timestamps must not precede decode timestamps"
                            .to_string(),
                });
            }
            let duration = if let Some((next_sample_index, next_anchor_index)) = sample_to_anchor
                [index + 1..]
                .iter()
                .enumerate()
                .find_map(|(delta, anchor)| {
                    anchor.map(|anchor_index| (index + 1 + delta, anchor_index))
                }) {
                let next_dts = sample_dts[next_anchor_index];
                if next_dts <= current_dts {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message:
                            "program stream MPEG-4 Part 2 video decode timestamps must increase monotonically"
                                .to_string(),
                    });
                }
                if next_sample_index == index + 1 {
                    u32::try_from(next_dts - current_dts).map_err(|_| {
                        MuxError::LayoutOverflow("program stream MPEG-4 Part 2 video duration")
                    })?
                } else {
                    scaled_sample_duration
                }
            } else {
                scaled_sample_duration
            };
            let composition_time_offset =
                i32::try_from(current_pts - current_dts).map_err(|_| {
                    MuxError::LayoutOverflow(
                        "program stream MPEG-4 Part 2 video composition offset",
                    )
                })?;
            last_composition_time_offset = composition_time_offset;
            if index == 0 && composition_time_offset > 0 {
                source_edit_media_time =
                    Some(u64::try_from(composition_time_offset).map_err(|_| {
                        MuxError::LayoutOverflow("program stream MPEG-4 Part 2 video edit")
                    })?);
            }
            (duration, composition_time_offset)
        } else {
            (scaled_sample_duration, last_composition_time_offset)
        };
        if duration == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message:
                    "program stream MPEG-4 Part 2 video frame duration underflowed after media-timescale normalization"
                        .to_string(),
            });
        }
        normalized.push(CandidateSample {
            source_index: usize::MAX,
            data_offset: sample.data_offset,
            data_size: sample.data_size,
            duration,
            composition_time_offset,
            is_sync_sample: sample.is_sync_sample,
        });
    }

    Ok((
        PROGRAM_STREAM_MEDIA_TIMESCALE,
        source_edit_media_time,
        normalized,
    ))
}

fn map_program_stream_mpeg2v_anchor_offsets_to_picture_samples(
    sample_offsets: &[u64],
    samples: &[StagedSample],
) -> Vec<Option<usize>> {
    sample_offsets
        .iter()
        .map(|&sample_offset| {
            if sample_offset == 0 {
                return Some(0);
            }
            samples
                .iter()
                .position(|sample| sample.data_offset >= sample_offset)
        })
        .collect()
}

fn build_program_stream_mpeg2v_sample_anchor_map(
    spec: &str,
    sample_offsets: &[u64],
    anchor_to_sample: &[Option<usize>],
    sample_count: usize,
) -> Result<Vec<Option<usize>>, MuxError> {
    let mut sample_to_anchor = vec![None; sample_count];
    for (anchor_index, sample_index) in anchor_to_sample.iter().copied().enumerate() {
        let Some(sample_index) = sample_index else {
            continue;
        };
        if sample_index >= sample_count {
            continue;
        }
        if sample_to_anchor[sample_index].is_some() {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "program stream MPEG-2 video carried multiple timing anchors for one parsed picture sample near byte offset {}",
                    sample_offsets[anchor_index]
                ),
            });
        }
        sample_to_anchor[sample_index] = Some(anchor_index);
    }
    Ok(sample_to_anchor)
}

fn scale_mpeg2v_duration_to_program_stream_clock(
    spec: &str,
    elementary_timescale: u32,
    duration: u32,
) -> Result<u32, MuxError> {
    if elementary_timescale == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream MPEG-2 video reported a zero media timescale".to_string(),
        });
    }
    let scaled = u64::from(duration)
        .checked_mul(u64::from(PROGRAM_STREAM_MEDIA_TIMESCALE))
        .ok_or(MuxError::LayoutOverflow(
            "program stream MPEG-2 video duration",
        ))?;
    if scaled % u64::from(elementary_timescale) != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "program stream MPEG-2 video cadence does not rescale cleanly onto the 90_000 media clock"
                    .to_string(),
        });
    }
    u32::try_from(scaled / u64::from(elementary_timescale))
        .map_err(|_| MuxError::LayoutOverflow("program stream MPEG-2 video duration"))
}

fn scale_mp4v_duration_to_program_stream_clock(
    spec: &str,
    elementary_timescale: u32,
    duration: u32,
) -> Result<u32, MuxError> {
    if elementary_timescale == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream MPEG-4 Part 2 video reported a zero media timescale"
                .to_string(),
        });
    }
    let scaled = u64::from(duration)
        .checked_mul(u64::from(PROGRAM_STREAM_MEDIA_TIMESCALE))
        .ok_or(MuxError::LayoutOverflow(
            "program stream MPEG-4 Part 2 video duration",
        ))?;
    if scaled % u64::from(elementary_timescale) != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "program stream MPEG-4 Part 2 video cadence does not rescale cleanly onto the 90_000 media clock"
                    .to_string(),
        });
    }
    u32::try_from(scaled / u64::from(elementary_timescale))
        .map_err(|_| MuxError::LayoutOverflow("program stream MPEG-4 Part 2 video duration"))
}

#[cfg(feature = "async")]
async fn finalize_program_stream_video_track_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    builder: ProgramStreamTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let prefix = read_program_stream_video_prefix_async(file, &builder, spec).await?;
    match detect_path_track_kind_from_prefix(&prefix) {
        DetectedPathTrackKind::Raw(super::super::MuxRawCodec::Mpeg2v) => {
            let mut parsed =
                scan_mpeg2v_segmented_async(file, &builder.segments, builder.total_size, spec)
                    .await?;
            if parsed.eof_terminated_trailing_sample {
                parsed.samples.pop();
            }
            let (timescale, source_edit_media_time, samples) =
                normalize_program_stream_mpeg2v_samples(
                spec,
                parsed.timescale,
                parsed.samples,
                &builder.sample_offsets,
                &builder.sample_pts,
                &builder.sample_dts,
            )?;
            let sample_entry_box = build_program_stream_mpeg2v_sample_entry_box(
                ProgramStreamMpeg2vSampleEntryConfig {
                    width: parsed.width,
                    height: parsed.height,
                    decoder_specific_info: &parsed.decoder_specific_info,
                    object_type_indication: parsed.object_type_indication,
                    timescale,
                    leading_media_time: source_edit_media_time.unwrap_or(0),
                    pixel_aspect_ratio: parsed.pixel_aspect_ratio,
                },
                samples.iter().map(|sample| (sample.data_size, sample.duration)),
            )?;
            Ok(CompositeTrackCandidate {
                track: TrackCandidate {
                    track_id: program_stream_track_id(builder.stream_id),
                    kind: MuxTrackKind::Video,
                    timescale,
                    language: *b"und",
                    handler_name: direct_ingest_handler_name("mpeg2v"),
                    mux_policy: direct_ingest_mux_policy("mpeg2v", MuxTrackKind::Video),
                    width: parsed.width,
                    height: parsed.height,
                    sample_entry_box,
                    source_edit_media_time,
                    samples,
                },
                source_spec: SegmentedMuxSourceSpec {
                    path: path.to_path_buf(),
                    segments: builder.segments,
                    total_size: builder.total_size,
                },
            })
        }
        DetectedPathTrackKind::Raw(super::super::MuxRawCodec::Mp4v) => {
            let parsed =
                scan_mp4v_segmented_async(file, &builder.segments, builder.total_size, spec).await?;
            let (timescale, source_edit_media_time, samples) =
                normalize_program_stream_mp4v_samples(
                    spec,
                    parsed.timescale,
                    parsed.samples,
                    &builder.sample_offsets,
                    &builder.sample_pts,
                    &builder.sample_dts,
                )?;
            Ok(CompositeTrackCandidate {
                track: TrackCandidate {
                    track_id: program_stream_track_id(builder.stream_id),
                    kind: MuxTrackKind::Video,
                    timescale,
                    language: *b"und",
                    handler_name: direct_ingest_handler_name("mp4v"),
                    mux_policy: direct_ingest_mux_policy("mp4v", MuxTrackKind::Video),
                    width: parsed.width,
                    height: parsed.height,
                    sample_entry_box: super::super::mp4::strip_visual_sample_entry_immediate_children(
                        &parsed.sample_entry_box,
                        &[FourCc::from_bytes(*b"pasp")],
                    )?,
                    source_edit_media_time,
                    samples,
                },
                source_spec: SegmentedMuxSourceSpec {
                    path: path.to_path_buf(),
                    segments: builder.segments,
                    total_size: builder.total_size,
                },
            })
        }
        DetectedPathTrackKind::Raw(super::super::MuxRawCodec::H264) => {
            let parsed = stage_annex_b_h264_segmented_async(
                path,
                file,
                &builder.segments,
                builder.total_size,
                spec,
            )
            .await?;
            Ok(CompositeTrackCandidate {
                track: TrackCandidate {
                    track_id: program_stream_track_id(builder.stream_id),
                    kind: MuxTrackKind::Video,
                    timescale: parsed.timescale,
                    language: *b"und",
                    handler_name: direct_ingest_handler_name("h264"),
                    mux_policy: direct_ingest_mux_policy("h264", MuxTrackKind::Video),
                    width: parsed.track_width,
                    height: parsed.track_height,
                    sample_entry_box: parsed.sample_entry_box,
                    source_edit_media_time: parsed.source_edit_media_time,
                    samples: parsed
                        .samples
                        .into_iter()
                        .map(|sample| CandidateSample {
                            source_index: usize::MAX,
                            data_offset: sample.data_offset,
                            data_size: sample.data_size,
                            duration: sample.duration,
                            composition_time_offset: sample.composition_time_offset,
                            is_sync_sample: sample.is_sync_sample,
                        })
                        .collect(),
                },
                source_spec: parsed.segmented_source,
            })
        }
        DetectedPathTrackKind::Raw(super::super::MuxRawCodec::H265) => {
            let parsed = stage_annex_b_h265_segmented_async(
                path,
                file,
                &builder.segments,
                builder.total_size,
                spec,
            )
            .await?;
            Ok(CompositeTrackCandidate {
                track: TrackCandidate {
                    track_id: program_stream_track_id(builder.stream_id),
                    kind: MuxTrackKind::Video,
                    timescale: parsed.timescale,
                    language: *b"und",
                    handler_name: direct_ingest_handler_name("h265"),
                    mux_policy: direct_ingest_mux_policy("h265", MuxTrackKind::Video),
                    width: parsed.track_width,
                    height: parsed.track_height,
                    sample_entry_box: parsed.sample_entry_box,
                    source_edit_media_time: parsed.source_edit_media_time,
                    samples: parsed
                        .samples
                        .into_iter()
                        .map(|sample| CandidateSample {
                            source_index: usize::MAX,
                            data_offset: sample.data_offset,
                            data_size: sample.data_size,
                            duration: sample.duration,
                            composition_time_offset: sample.composition_time_offset,
                            is_sync_sample: sample.is_sync_sample,
                        })
                        .collect(),
                },
                source_spec: parsed.segmented_source,
            })
        }
        DetectedPathTrackKind::Raw(super::super::MuxRawCodec::Vvc) => {
            let parsed = stage_annex_b_vvc_segmented_async(
                path,
                file,
                &builder.segments,
                builder.total_size,
                spec,
            )
            .await?;
            Ok(CompositeTrackCandidate {
                track: TrackCandidate {
                    track_id: program_stream_track_id(builder.stream_id),
                    kind: MuxTrackKind::Video,
                    timescale: parsed.timescale,
                    language: *b"und",
                    handler_name: direct_ingest_handler_name("vvc"),
                    mux_policy: direct_ingest_mux_policy("vvc", MuxTrackKind::Video),
                    width: parsed.track_width,
                    height: parsed.track_height,
                    sample_entry_box: parsed.sample_entry_box,
                    source_edit_media_time: parsed.source_edit_media_time,
                    samples: parsed
                        .samples
                        .into_iter()
                        .map(|sample| CandidateSample {
                            source_index: usize::MAX,
                            data_offset: sample.data_offset,
                            data_size: sample.data_size,
                            duration: sample.duration,
                            composition_time_offset: sample.composition_time_offset,
                            is_sync_sample: sample.is_sync_sample,
                        })
                        .collect(),
                },
                source_spec: parsed.segmented_source,
            })
        }
        _ => Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "program stream video payload is not a supported MPEG-2, MPEG-4 Part 2, H.264, H.265, or VVC elementary stream"
                    .to_string(),
        }),
    }
}

fn parse_private_stream_1_pes_packet_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
    stream_id: u8,
) -> Result<ParsedPrivateStream1PesPacket, MuxError> {
    let parsed = parse_pes_packet_sync(file, file_size, offset, spec, stream_id)?;
    if parsed.payload_size < PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "program stream private_stream_1 payload is truncated before the 4-byte private header"
                    .to_string(),
        });
    }
    let mut private_header = [0_u8; PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES as usize];
    read_exact_at_sync(
        file,
        parsed.payload_offset,
        &mut private_header,
        spec,
        "program stream private_stream_1 payload is truncated before the 4-byte private header",
    )?;
    finalize_private_stream_1_pes_packet(
        spec,
        private_header,
        parsed.presentation_time,
        parsed.payload_offset,
        parsed.payload_size,
        parsed.packet_end,
    )
}

#[cfg(feature = "async")]
async fn parse_private_stream_1_pes_packet_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
    stream_id: u8,
) -> Result<ParsedPrivateStream1PesPacket, MuxError> {
    let parsed = parse_pes_packet_async(file, file_size, offset, spec, stream_id).await?;
    if parsed.payload_size < PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "program stream private_stream_1 payload is truncated before the 4-byte private header"
                    .to_string(),
        });
    }
    let mut private_header = [0_u8; PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES as usize];
    read_exact_at_async(
        file,
        parsed.payload_offset,
        &mut private_header,
        spec,
        "program stream private_stream_1 payload is truncated before the 4-byte private header",
    )
    .await?;
    finalize_private_stream_1_pes_packet(
        spec,
        private_header,
        parsed.presentation_time,
        parsed.payload_offset,
        parsed.payload_size,
        parsed.packet_end,
    )
}

fn finalize_private_stream_1_pes_packet(
    spec: &str,
    private_header: [u8; PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES as usize],
    presentation_time: Option<u64>,
    payload_offset: u64,
    payload_size: u32,
    packet_end: u64,
) -> Result<ParsedPrivateStream1PesPacket, MuxError> {
    let substream_id = private_header[0];
    let (kind, lpcm_format) = if (PRIVATE_STREAM_1_AC3_MIN..=PRIVATE_STREAM_1_AC3_MAX)
        .contains(&substream_id)
    {
        (ProgramStreamTrackKind::Ac3, None)
    } else if (PRIVATE_STREAM_1_LPCM_MIN..=PRIVATE_STREAM_1_LPCM_MAX).contains(&substream_id) {
        let lpcm_format = parse_program_stream_lpcm_format(
            spec,
            substream_id,
            [private_header[1], private_header[2], private_header[3]],
        )?;
        (ProgramStreamTrackKind::Lpcm, Some(lpcm_format))
    } else if (0x20..=0x3F).contains(&substream_id) {
        (ProgramStreamTrackKind::Subpicture, None)
    } else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "program stream private_stream_1 substream 0x{substream_id:02X} is not supported on the native direct-ingest path yet"
            ),
        });
    };
    Ok(ParsedPrivateStream1PesPacket {
        substream_id,
        kind,
        lpcm_format,
        presentation_time,
        payload_offset: payload_offset + u64::from(PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES),
        payload_size: payload_size - PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES,
        packet_end,
    })
}

fn parse_program_stream_lpcm_format(
    spec: &str,
    substream_id: u8,
    private_header_bytes: [u8; 3],
) -> Result<ProgramStreamLpcmFormat, MuxError> {
    let format_byte = private_header_bytes[2];
    let bits_per_sample = match format_byte >> 6 {
        0 => 16,
        1 => 20,
        2 => 24,
        other => {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "program stream LPCM substream 0x{substream_id:02X} used unsupported sample-size code {other}"
                ),
            });
        }
    };
    if bits_per_sample % 8 != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "program stream LPCM substream 0x{substream_id:02X} used unsupported non-byte-aligned {bits_per_sample}-bit samples"
            ),
        });
    }
    let sample_rate = match (format_byte >> 4) & 0x03 {
        0 => 48_000,
        1 => 96_000,
        2 => 44_100,
        3 => 32_000,
        _ => unreachable!(),
    };
    let channel_count = u16::from(format_byte & 0x07) + 1;
    let block_align =
        channel_count
            .checked_mul(bits_per_sample / 8)
            .ok_or(MuxError::LayoutOverflow(
                "program stream LPCM block alignment",
            ))?;
    if block_align == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "program stream LPCM substream 0x{substream_id:02X} declared an invalid zero-byte frame size"
            ),
        });
    }
    Ok(ProgramStreamLpcmFormat {
        sample_rate,
        channel_count,
        bits_per_sample,
        block_align,
    })
}

fn read_program_stream_video_prefix_sync(
    file: &mut File,
    builder: &ProgramStreamTrackBuilder,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    let prefix_len = usize::try_from(builder.total_size.min(4 * 1024))
        .map_err(|_| MuxError::LayoutOverflow("program stream video prefix length"))?;
    let mut prefix = vec![0_u8; prefix_len];
    read_segmented_bytes_sync(
        file,
        &builder.segments,
        builder.total_size,
        0,
        &mut prefix,
        spec,
        "program stream video prefix is truncated",
    )?;
    Ok(prefix)
}

#[cfg(feature = "async")]
async fn read_program_stream_video_prefix_async(
    file: &mut TokioFile,
    builder: &ProgramStreamTrackBuilder,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    let prefix_len = usize::try_from(builder.total_size.min(4 * 1024))
        .map_err(|_| MuxError::LayoutOverflow("program stream video prefix length"))?;
    let mut prefix = vec![0_u8; prefix_len];
    read_segmented_bytes_async(
        file,
        &builder.segments,
        builder.total_size,
        0,
        &mut prefix,
        spec,
        "program stream video prefix is truncated",
    )
    .await?;
    Ok(prefix)
}

fn validate_program_stream_header_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if file_size < 14 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream input is truncated before the pack header".to_string(),
        });
    }
    let mut header = [0_u8; 4];
    read_exact_at_sync(
        file,
        0,
        &mut header,
        spec,
        "program stream input is truncated before the pack header",
    )?;
    if header != PACK_START_CODE {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "input is not an MPEG program stream pack header".to_string(),
        });
    }
    Ok(())
}

#[cfg(feature = "async")]
async fn validate_program_stream_header_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if file_size < 14 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream input is truncated before the pack header".to_string(),
        });
    }
    let mut header = [0_u8; 4];
    read_exact_at_async(
        file,
        0,
        &mut header,
        spec,
        "program stream input is truncated before the pack header",
    )
    .await?;
    if header != PACK_START_CODE {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "input is not an MPEG program stream pack header".to_string(),
        });
    }
    Ok(())
}

fn read_program_stream_start_code_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<[u8; 4], MuxError> {
    if file_size - offset < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated MPEG program stream start code".to_string(),
        });
    }
    let mut start_code = [0_u8; 4];
    read_exact_at_sync(
        file,
        offset,
        &mut start_code,
        spec,
        "truncated MPEG program stream start code",
    )?;
    if start_code[..3] != [0x00, 0x00, 0x01] {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("invalid MPEG program stream start code at byte offset {offset}"),
        });
    }
    Ok(start_code)
}

#[cfg(feature = "async")]
async fn read_program_stream_start_code_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<[u8; 4], MuxError> {
    if file_size - offset < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated MPEG program stream start code".to_string(),
        });
    }
    let mut start_code = [0_u8; 4];
    read_exact_at_async(
        file,
        offset,
        &mut start_code,
        spec,
        "truncated MPEG program stream start code",
    )
    .await?;
    if start_code[..3] != [0x00, 0x00, 0x01] {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("invalid MPEG program stream start code at byte offset {offset}"),
        });
    }
    Ok(start_code)
}

fn parse_pack_header_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<u64, MuxError> {
    if file_size - offset < 12 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream pack header".to_string(),
        });
    }
    let mut first_byte = [0_u8; 1];
    read_exact_at_sync(
        file,
        offset + 4,
        &mut first_byte,
        spec,
        "truncated program stream pack header",
    )?;
    if first_byte[0] & 0xF0 == 0x20 {
        return Ok(offset + 12);
    }
    if file_size - offset < 14 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream pack header".to_string(),
        });
    }
    let mut header = [0_u8; 10];
    header[0] = first_byte[0];
    read_exact_at_sync(
        file,
        offset + 5,
        &mut header[1..],
        spec,
        "truncated program stream pack header",
    )?;
    if header[0] & 0xC0 != 0x40 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "unsupported program stream pack-header layout".to_string(),
        });
    }
    let packet_size = 14_u64 + u64::from(header[9] & 0x07);
    if offset
        .checked_add(packet_size)
        .is_none_or(|end| end > file_size)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream pack stuffing bytes".to_string(),
        });
    }
    Ok(offset + packet_size)
}

#[cfg(feature = "async")]
async fn parse_pack_header_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<u64, MuxError> {
    if file_size - offset < 12 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream pack header".to_string(),
        });
    }
    let mut first_byte = [0_u8; 1];
    read_exact_at_async(
        file,
        offset + 4,
        &mut first_byte,
        spec,
        "truncated program stream pack header",
    )
    .await?;
    if first_byte[0] & 0xF0 == 0x20 {
        return Ok(offset + 12);
    }
    if file_size - offset < 14 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream pack header".to_string(),
        });
    }
    let mut header = [0_u8; 10];
    header[0] = first_byte[0];
    read_exact_at_async(
        file,
        offset + 5,
        &mut header[1..],
        spec,
        "truncated program stream pack header",
    )
    .await?;
    if header[0] & 0xC0 != 0x40 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "unsupported program stream pack-header layout".to_string(),
        });
    }
    let packet_size = 14_u64 + u64::from(header[9] & 0x07);
    if offset
        .checked_add(packet_size)
        .is_none_or(|end| end > file_size)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream pack stuffing bytes".to_string(),
        });
    }
    Ok(offset + packet_size)
}

fn skip_length_delimited_ps_packet_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
    packet_id: u8,
) -> Result<u64, MuxError> {
    if file_size - offset < 6 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "truncated program stream packet header for start code 0x{packet_id:02X}"
            ),
        });
    }
    let mut length_bytes = [0_u8; 2];
    read_exact_at_sync(
        file,
        offset + 4,
        &mut length_bytes,
        spec,
        "truncated program stream packet length",
    )?;
    let packet_size = 6_u64 + u64::from(u16::from_be_bytes(length_bytes));
    if offset
        .checked_add(packet_size)
        .is_none_or(|end| end > file_size)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "truncated program stream packet body for start code 0x{packet_id:02X}"
            ),
        });
    }
    Ok(offset + packet_size)
}

fn is_supported_program_stream_packet_id(packet_id: u8) -> bool {
    matches!(
        packet_id,
        0xB9
            | 0xBA
            | SYSTEM_HEADER_START_CODE
            | PROGRAM_STREAM_MAP_START_CODE
            | PRIVATE_STREAM_1_START_CODE
            | PADDING_STREAM_START_CODE
            | PRIVATE_STREAM_2_START_CODE
            | 0xC0..=0xDF
            | 0xE0..=0xEF
    )
}

fn find_program_stream_packet_start_in_bytes(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| {
        window[..3] == [0x00, 0x00, 0x01] && is_supported_program_stream_packet_id(window[3])
    })
}

// Open-ended PES packets still need a deterministic end boundary. We scan for the next
// recognized program-stream packet start code rather than treating any `00 00 01 xx` sequence as
// a boundary, which avoids colliding with carried Annex B or MPEG-4 Part 2 start-code families.
fn find_next_program_stream_packet_start_sync(
    file: &mut File,
    file_size: u64,
    search_offset: u64,
    spec: &str,
) -> Result<Option<u64>, MuxError> {
    if search_offset >= file_size {
        return Ok(None);
    }

    let mut scan_offset = search_offset;
    let mut carry = Vec::new();
    while scan_offset < file_size {
        let remaining = usize::try_from(file_size - scan_offset).unwrap_or(usize::MAX);
        let chunk_len = remaining.min(PROGRAM_STREAM_SCAN_CHUNK_BYTES);
        let mut chunk = vec![0_u8; chunk_len];
        read_exact_at_sync(
            file,
            scan_offset,
            &mut chunk,
            spec,
            "truncated program stream open-ended PES scan chunk",
        )?;

        let mut scan_bytes = carry;
        let base_offset = scan_offset - u64::try_from(scan_bytes.len()).unwrap();
        scan_bytes.extend_from_slice(&chunk);
        if let Some(found) = find_program_stream_packet_start_in_bytes(&scan_bytes) {
            return Ok(Some(base_offset + u64::try_from(found).unwrap()));
        }

        let keep = scan_bytes.len().min(3);
        carry = scan_bytes[scan_bytes.len() - keep..].to_vec();
        scan_offset += u64::try_from(chunk_len).unwrap();
    }

    Ok(None)
}

#[cfg(feature = "async")]
async fn skip_length_delimited_ps_packet_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
    packet_id: u8,
) -> Result<u64, MuxError> {
    if file_size - offset < 6 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "truncated program stream packet header for start code 0x{packet_id:02X}"
            ),
        });
    }
    let mut length_bytes = [0_u8; 2];
    read_exact_at_async(
        file,
        offset + 4,
        &mut length_bytes,
        spec,
        "truncated program stream packet length",
    )
    .await?;
    let packet_size = 6_u64 + u64::from(u16::from_be_bytes(length_bytes));
    if offset
        .checked_add(packet_size)
        .is_none_or(|end| end > file_size)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "truncated program stream packet body for start code 0x{packet_id:02X}"
            ),
        });
    }
    Ok(offset + packet_size)
}

#[cfg(feature = "async")]
async fn find_next_program_stream_packet_start_async(
    file: &mut TokioFile,
    file_size: u64,
    search_offset: u64,
    spec: &str,
) -> Result<Option<u64>, MuxError> {
    if search_offset >= file_size {
        return Ok(None);
    }

    let mut scan_offset = search_offset;
    let mut carry = Vec::new();
    while scan_offset < file_size {
        let remaining = usize::try_from(file_size - scan_offset).unwrap_or(usize::MAX);
        let chunk_len = remaining.min(PROGRAM_STREAM_SCAN_CHUNK_BYTES);
        let mut chunk = vec![0_u8; chunk_len];
        read_exact_at_async(
            file,
            scan_offset,
            &mut chunk,
            spec,
            "truncated program stream open-ended PES scan chunk",
        )
        .await?;

        let mut scan_bytes = carry;
        let base_offset = scan_offset - u64::try_from(scan_bytes.len()).unwrap();
        scan_bytes.extend_from_slice(&chunk);
        if let Some(found) = find_program_stream_packet_start_in_bytes(&scan_bytes) {
            return Ok(Some(base_offset + u64::try_from(found).unwrap()));
        }

        let keep = scan_bytes.len().min(3);
        carry = scan_bytes[scan_bytes.len() - keep..].to_vec();
        scan_offset += u64::try_from(chunk_len).unwrap();
    }

    Ok(None)
}

fn parse_pes_packet_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
    stream_id: u8,
) -> Result<ParsedProgramStreamPesPacket, MuxError> {
    if file_size - offset < 9 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("truncated PES header for program stream id 0x{stream_id:02X}"),
        });
    }
    let mut header = [0_u8; 5];
    read_exact_at_sync(
        file,
        offset + 4,
        &mut header,
        spec,
        "truncated program stream PES header",
    )?;
    let pes_packet_length = u16::from_be_bytes([header[0], header[1]]);
    let (payload_offset, presentation_time, decode_time) = if header[2] & 0xC0 == 0x80 {
        let header_data_length = u64::from(header[4]);
        let payload_offset = offset + 9 + header_data_length;
        if payload_offset > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated program stream PES payload".to_string(),
            });
        }
        let presentation_time = if header[3] & 0x80 != 0 {
            Some(parse_program_stream_pes_timestamp_sync(
                file,
                offset + 9,
                file_size,
                spec,
            )?)
        } else {
            None
        };
        let decode_time = if header[3] & 0x40 != 0 {
            Some(parse_program_stream_pes_timestamp_sync(
                file,
                offset + 14,
                file_size,
                spec,
            )?)
        } else {
            presentation_time
        };
        (payload_offset, presentation_time, decode_time)
    } else {
        parse_mpeg1_pes_header_sync(file, file_size, offset, spec)?
    };
    let packet_end = if pes_packet_length == 0 {
        if !matches!(stream_id, 0xE0..=0xEF) {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "open-ended PES packets are only supported for program-stream video carriage on the native direct-ingest path"
                    .to_string(),
            });
        }
        find_next_program_stream_packet_start_sync(file, file_size, payload_offset, spec)?
            .unwrap_or(file_size)
    } else {
        offset + 6 + u64::from(pes_packet_length)
    };
    if payload_offset > packet_end || packet_end > file_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream PES payload".to_string(),
        });
    }
    let payload_size = u32::try_from(packet_end - payload_offset)
        .map_err(|_| MuxError::LayoutOverflow("program stream PES payload"))?;
    Ok(ParsedProgramStreamPesPacket {
        payload_offset,
        payload_size,
        packet_end,
        presentation_time,
        decode_time,
    })
}

#[cfg(feature = "async")]
async fn parse_pes_packet_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
    stream_id: u8,
) -> Result<ParsedProgramStreamPesPacket, MuxError> {
    if file_size - offset < 9 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("truncated PES header for program stream id 0x{stream_id:02X}"),
        });
    }
    let mut header = [0_u8; 5];
    read_exact_at_async(
        file,
        offset + 4,
        &mut header,
        spec,
        "truncated program stream PES header",
    )
    .await?;
    let pes_packet_length = u16::from_be_bytes([header[0], header[1]]);
    let (payload_offset, presentation_time, decode_time) = if header[2] & 0xC0 == 0x80 {
        let header_data_length = u64::from(header[4]);
        let payload_offset = offset + 9 + header_data_length;
        if payload_offset > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated program stream PES payload".to_string(),
            });
        }
        let presentation_time = if header[3] & 0x80 != 0 {
            Some(parse_program_stream_pes_timestamp_async(file, offset + 9, file_size, spec).await?)
        } else {
            None
        };
        let decode_time = if header[3] & 0x40 != 0 {
            Some(
                parse_program_stream_pes_timestamp_async(file, offset + 14, file_size, spec)
                    .await?,
            )
        } else {
            presentation_time
        };
        (payload_offset, presentation_time, decode_time)
    } else {
        parse_mpeg1_pes_header_async(file, file_size, offset, spec).await?
    };
    let packet_end = if pes_packet_length == 0 {
        if !matches!(stream_id, 0xE0..=0xEF) {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "open-ended PES packets are only supported for program-stream video carriage on the native direct-ingest path"
                    .to_string(),
            });
        }
        find_next_program_stream_packet_start_async(file, file_size, payload_offset, spec)
            .await?
            .unwrap_or(file_size)
    } else {
        offset + 6 + u64::from(pes_packet_length)
    };
    if payload_offset > packet_end || packet_end > file_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream PES payload".to_string(),
        });
    }
    let payload_size = u32::try_from(packet_end - payload_offset)
        .map_err(|_| MuxError::LayoutOverflow("program stream PES payload"))?;
    Ok(ParsedProgramStreamPesPacket {
        payload_offset,
        payload_size,
        packet_end,
        presentation_time,
        decode_time,
    })
}

fn parse_mpeg1_pes_header_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<(u64, Option<u64>, Option<u64>), MuxError> {
    let mut cursor = offset + 6;
    let mut next = read_program_stream_byte_sync(file, file_size, cursor, spec)?;
    while next == 0xFF {
        cursor += 1;
        next = read_program_stream_byte_sync(file, file_size, cursor, spec)?;
    }
    if next & 0xC0 == 0x40 {
        cursor += 2;
        next = read_program_stream_byte_sync(file, file_size, cursor, spec)?;
    }
    if next & 0xF0 == 0x20 {
        let presentation_time =
            parse_program_stream_pes_timestamp_sync(file, cursor, file_size, spec)?;
        return Ok((cursor + 5, Some(presentation_time), Some(presentation_time)));
    }
    if next & 0xF0 == 0x30 {
        let presentation_time =
            parse_program_stream_pes_timestamp_sync(file, cursor, file_size, spec)?;
        let decode_time =
            parse_program_stream_pes_timestamp_sync(file, cursor + 5, file_size, spec)?;
        return Ok((cursor + 10, Some(presentation_time), Some(decode_time)));
    }
    if next == 0x0F {
        return Ok((cursor + 1, None, None));
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "unsupported PES header flags on the native direct-ingest program-stream path"
            .to_string(),
    })
}

#[cfg(feature = "async")]
async fn parse_mpeg1_pes_header_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<(u64, Option<u64>, Option<u64>), MuxError> {
    let mut cursor = offset + 6;
    let mut next = read_program_stream_byte_async(file, file_size, cursor, spec).await?;
    while next == 0xFF {
        cursor += 1;
        next = read_program_stream_byte_async(file, file_size, cursor, spec).await?;
    }
    if next & 0xC0 == 0x40 {
        cursor += 2;
        next = read_program_stream_byte_async(file, file_size, cursor, spec).await?;
    }
    if next & 0xF0 == 0x20 {
        let presentation_time =
            parse_program_stream_pes_timestamp_async(file, cursor, file_size, spec).await?;
        return Ok((cursor + 5, Some(presentation_time), Some(presentation_time)));
    }
    if next & 0xF0 == 0x30 {
        let presentation_time =
            parse_program_stream_pes_timestamp_async(file, cursor, file_size, spec).await?;
        let decode_time =
            parse_program_stream_pes_timestamp_async(file, cursor + 5, file_size, spec).await?;
        return Ok((cursor + 10, Some(presentation_time), Some(decode_time)));
    }
    if next == 0x0F {
        return Ok((cursor + 1, None, None));
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "unsupported PES header flags on the native direct-ingest program-stream path"
            .to_string(),
    })
}

fn read_program_stream_byte_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<u8, MuxError> {
    if offset >= file_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream PES header".to_string(),
        });
    }
    let mut byte = [0_u8; 1];
    read_exact_at_sync(
        file,
        offset,
        &mut byte,
        spec,
        "truncated program stream PES header",
    )?;
    Ok(byte[0])
}

#[cfg(feature = "async")]
async fn read_program_stream_byte_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<u8, MuxError> {
    if offset >= file_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream PES header".to_string(),
        });
    }
    let mut byte = [0_u8; 1];
    read_exact_at_async(
        file,
        offset,
        &mut byte,
        spec,
        "truncated program stream PES header",
    )
    .await?;
    Ok(byte[0])
}

fn parse_program_stream_pes_timestamp_sync(
    file: &mut File,
    timestamp_offset: u64,
    file_size: u64,
    spec: &str,
) -> Result<u64, MuxError> {
    if file_size.saturating_sub(timestamp_offset) < 5 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream PES timestamp".to_string(),
        });
    }
    let mut pts = [0_u8; 5];
    read_exact_at_sync(
        file,
        timestamp_offset,
        &mut pts,
        spec,
        "truncated program stream PES timestamp",
    )?;
    parse_program_stream_pes_timestamp_bytes(&pts, spec)
}

#[cfg(feature = "async")]
async fn parse_program_stream_pes_timestamp_async(
    file: &mut TokioFile,
    timestamp_offset: u64,
    file_size: u64,
    spec: &str,
) -> Result<u64, MuxError> {
    if file_size.saturating_sub(timestamp_offset) < 5 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream PES timestamp".to_string(),
        });
    }
    let mut pts = [0_u8; 5];
    read_exact_at_async(
        file,
        timestamp_offset,
        &mut pts,
        spec,
        "truncated program stream PES timestamp",
    )
    .await?;
    parse_program_stream_pes_timestamp_bytes(&pts, spec)
}

fn parse_program_stream_pes_timestamp_bytes(pts: &[u8; 5], spec: &str) -> Result<u64, MuxError> {
    let prefix = pts[0] & 0xF1;
    if !matches!(prefix, 0x11 | 0x21 | 0x31) || pts[2] & 0x01 != 0x01 || pts[4] & 0x01 != 0x01 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "program stream PES timestamp markers are malformed".to_string(),
        });
    }
    Ok((u64::from((pts[0] >> 1) & 0x07) << 30)
        | (u64::from(pts[1]) << 22)
        | (u64::from((pts[2] >> 1) & 0x7F) << 15)
        | (u64::from(pts[3]) << 7)
        | u64::from((pts[4] >> 1) & 0x7F))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::path::PathBuf;

    use super::{
        PADDING_STREAM_START_CODE, PRIVATE_STREAM_1_START_CODE, PRIVATE_STREAM_2_START_CODE,
        PROGRAM_STREAM_MAP_START_CODE, PROGRAM_STREAM_MEDIA_TIMESCALE, ProgramStreamTrackBuilder,
        ProgramStreamTrackKind, SYSTEM_HEADER_START_CODE, append_file_range_segment,
        normalize_program_stream_mpeg2v_samples, parse_pes_packet_sync,
        read_program_stream_start_code_sync, scan_mpeg2v_segmented_sync, scan_program_stream_sync,
        skip_length_delimited_ps_packet_sync, validate_program_stream_header_sync,
    };
    use crate::mux::MuxTrackKind;
    use crate::mux::import::SegmentedMuxSourceSegment;
    use crate::mux::import::StagedSample;

    #[test]
    fn normalize_program_stream_mpeg2v_samples_maps_timed_pes_offsets_to_picture_samples() {
        let samples = vec![
            StagedSample {
                data_offset: 0,
                data_size: 7032,
                duration: 1000,
                composition_time_offset: 0,
                is_sync_sample: true,
            },
            StagedSample {
                data_offset: 7032,
                data_size: 3242,
                duration: 1000,
                composition_time_offset: 0,
                is_sync_sample: false,
            },
            StagedSample {
                data_offset: 10274,
                data_size: 1283,
                duration: 1000,
                composition_time_offset: 0,
                is_sync_sample: false,
            },
            StagedSample {
                data_offset: 11557,
                data_size: 1259,
                duration: 1000,
                composition_time_offset: 0,
                is_sync_sample: false,
            },
            StagedSample {
                data_offset: 12816,
                data_size: 1261,
                duration: 1000,
                composition_time_offset: 0,
                is_sync_sample: false,
            },
        ];

        let (timescale, source_edit_media_time, normalized) =
            normalize_program_stream_mpeg2v_samples(
                "test",
                25_000,
                samples,
                &[0, 6059, 10097, 12111],
                &[48_600, 52_200, 55_800, 63_000],
                &[45_000, 48_600, 52_200, 59_400],
            )
            .unwrap();

        assert_eq!(timescale, PROGRAM_STREAM_MEDIA_TIMESCALE);
        assert_eq!(source_edit_media_time, Some(3600));
        assert_eq!(
            normalized
                .iter()
                .map(|sample| (sample.data_offset, sample.data_size, sample.duration))
                .collect::<Vec<_>>(),
            vec![
                (0, 7032, 3600),
                (7032, 3242, 3600),
                (10274, 1283, 3600),
                (11557, 1259, 1000),
            ]
        );
        assert_eq!(
            normalized
                .iter()
                .map(|sample| sample.composition_time_offset)
                .collect::<Vec<_>>(),
            vec![3600, 3600, 3600, 3600]
        );
        assert!(normalized[0].is_sync_sample);
    }

    #[test]
    fn program_stream_mpeg2v_fixture_maps_expected_flat_sampleization() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("mux")
            .join("program_stream_video.mpeg");
        let spec = format!("{}#video", path.display());
        let mut file = File::open(&path).unwrap();
        let file_size = file.metadata().unwrap().len();
        validate_program_stream_header_sync(&mut file, file_size, &spec).unwrap();
        let mut builders = BTreeMap::<u8, ProgramStreamTrackBuilder>::new();
        let mut offset = 0_u64;
        while offset < file_size {
            let start_code =
                read_program_stream_start_code_sync(&mut file, file_size, offset, &spec).unwrap();
            match start_code[3] {
                0xBA => {
                    offset =
                        super::parse_pack_header_sync(&mut file, file_size, offset, &spec).unwrap();
                }
                SYSTEM_HEADER_START_CODE
                | PROGRAM_STREAM_MAP_START_CODE
                | PADDING_STREAM_START_CODE
                | PRIVATE_STREAM_2_START_CODE => {
                    offset = skip_length_delimited_ps_packet_sync(
                        &mut file,
                        file_size,
                        offset,
                        &spec,
                        start_code[3],
                    )
                    .unwrap();
                }
                PRIVATE_STREAM_1_START_CODE => {
                    offset = super::parse_private_stream_1_pes_packet_sync(
                        &mut file,
                        file_size,
                        offset,
                        &spec,
                        start_code[3],
                    )
                    .unwrap()
                    .packet_end;
                }
                0xC0..=0xDF => {
                    let parsed =
                        parse_pes_packet_sync(&mut file, file_size, offset, &spec, start_code[3])
                            .unwrap();
                    let builder = builders.entry(start_code[3]).or_insert_with(|| {
                        ProgramStreamTrackBuilder {
                            stream_id: start_code[3],
                            kind: ProgramStreamTrackKind::Mp3,
                            lpcm_format: None,
                            segments: Vec::<SegmentedMuxSourceSegment>::new(),
                            total_size: 0,
                            sample_offsets: Vec::new(),
                            sample_pts: Vec::new(),
                            sample_dts: Vec::new(),
                        }
                    });
                    append_file_range_segment(
                        &mut builder.segments,
                        &mut builder.total_size,
                        parsed.payload_offset,
                        parsed.payload_size,
                    );
                    offset = parsed.packet_end;
                }
                0xE0..=0xEF => {
                    let parsed =
                        parse_pes_packet_sync(&mut file, file_size, offset, &spec, start_code[3])
                            .unwrap();
                    let builder = builders.entry(start_code[3]).or_insert_with(|| {
                        ProgramStreamTrackBuilder {
                            stream_id: start_code[3],
                            kind: ProgramStreamTrackKind::Video,
                            lpcm_format: None,
                            segments: Vec::<SegmentedMuxSourceSegment>::new(),
                            total_size: 0,
                            sample_offsets: Vec::new(),
                            sample_pts: Vec::new(),
                            sample_dts: Vec::new(),
                        }
                    });
                    if let Some(presentation_time) = parsed.presentation_time {
                        builder.sample_offsets.push(builder.total_size);
                        builder.sample_pts.push(presentation_time);
                        builder
                            .sample_dts
                            .push(parsed.decode_time.unwrap_or(presentation_time));
                    }
                    append_file_range_segment(
                        &mut builder.segments,
                        &mut builder.total_size,
                        parsed.payload_offset,
                        parsed.payload_size,
                    );
                    offset = parsed.packet_end;
                }
                0xB9 => break,
                other => panic!("unexpected start code 0x{other:02X}"),
            }
        }
        let builder = builders.remove(&0xE0).unwrap();
        let _parsed =
            scan_mpeg2v_segmented_sync(&mut file, &builder.segments, builder.total_size, &spec)
                .unwrap();
        let tracks = scan_program_stream_sync(&path, &spec).unwrap();
        let video = tracks
            .into_iter()
            .find(|candidate| candidate.track.kind == MuxTrackKind::Video)
            .unwrap();
        let samples = video.track.samples;
        assert_eq!(video.track.timescale, PROGRAM_STREAM_MEDIA_TIMESCALE);
        assert_eq!(video.track.source_edit_media_time, Some(3003));
        assert_eq!(samples.len(), 29);
        assert_eq!(
            samples
                .iter()
                .filter(|sample| sample.is_sync_sample)
                .count(),
            3
        );
        assert_eq!(
            samples
                .iter()
                .map(|sample| sample.duration)
                .collect::<Vec<_>>(),
            {
                let mut durations = vec![3003; 28];
                durations.push(1001);
                durations
            }
        );
    }
}
