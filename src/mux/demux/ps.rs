use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

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
use super::mp4v::{scan_mp4v_segmented_async, scan_mp4v_segmented_sync};
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
const PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES: u32 = 4;
const PROGRAM_STREAM_MEDIA_TIMESCALE: u32 = 90_000;

struct ProgramStreamTrackBuilder {
    stream_id: u8,
    kind: ProgramStreamTrackKind,
    segments: Vec<SegmentedMuxSourceSegment>,
    total_size: u64,
    sample_offsets: Vec<u64>,
    sample_pts: Vec<u64>,
}

#[derive(Clone, Copy)]
enum ProgramStreamTrackKind {
    Mp3,
    Ac3,
    Video,
    Subpicture,
}

struct ParsedProgramStreamPesPacket {
    payload_offset: u64,
    payload_size: u32,
    packet_end: u64,
    presentation_time: Option<u64>,
}

struct ParsedPrivateStream1PesPacket {
    substream_id: u8,
    kind: ProgramStreamTrackKind,
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
                        segments: Vec::new(),
                        total_size: 0,
                        sample_offsets: Vec::new(),
                        sample_pts: Vec::new(),
                    }
                });
                if matches!(builder.kind, ProgramStreamTrackKind::Subpicture) {
                    builder.sample_offsets.push(builder.total_size);
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
                            segments: Vec::new(),
                            total_size: 0,
                            sample_offsets: Vec::new(),
                            sample_pts: Vec::new(),
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
                            segments: Vec::new(),
                            total_size: 0,
                            sample_offsets: Vec::new(),
                            sample_pts: Vec::new(),
                        });
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
                        segments: Vec::new(),
                        total_size: 0,
                        sample_offsets: Vec::new(),
                        sample_pts: Vec::new(),
                    }
                });
                if matches!(builder.kind, ProgramStreamTrackKind::Subpicture) {
                    builder.sample_offsets.push(builder.total_size);
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
                            segments: Vec::new(),
                            total_size: 0,
                            sample_offsets: Vec::new(),
                            sample_pts: Vec::new(),
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
                            segments: Vec::new(),
                            total_size: 0,
                            sample_offsets: Vec::new(),
                            sample_pts: Vec::new(),
                        });
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
                "program stream input did not contain any supported MPEG audio, AC-3, VobSub-style subpicture, or MPEG-4 Part 2/H.264/H.265/VVC video payloads"
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
                "program stream input did not contain any supported MPEG audio, AC-3, VobSub-style subpicture, or MPEG-4 Part 2/H.264/H.265/VVC video payloads"
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
            track_id: u32::from(builder.stream_id),
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
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.stream_id),
            kind: MuxTrackKind::Audio,
            timescale: sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("mp3"),
            mux_policy: direct_ingest_mux_policy("mp3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box: build_mp3_sample_entry_box(
                sample_rate,
                channel_count,
                samples
                    .iter()
                    .map(|sample| (sample.data_size, sample.duration)),
            )?,
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
        DetectedPathTrackKind::Raw(super::super::MuxRawCodec::Mp4v) => {
            let parsed = scan_mp4v_segmented_sync(file, &builder.segments, builder.total_size, spec)?;
            Ok(CompositeTrackCandidate {
                track: TrackCandidate {
                    track_id: u32::from(builder.stream_id),
                    kind: MuxTrackKind::Video,
                    timescale: parsed.timescale,
                    language: *b"und",
                    handler_name: direct_ingest_handler_name("mp4v"),
                    mux_policy: direct_ingest_mux_policy("mp4v", MuxTrackKind::Video),
                    width: parsed.width,
                    height: parsed.height,
                    sample_entry_box: parsed.sample_entry_box,
                    source_edit_media_time: None,
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
                    track_id: u32::from(builder.stream_id),
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
                    track_id: u32::from(builder.stream_id),
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
                    track_id: u32::from(builder.stream_id),
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
                "program stream video payload is not a supported MPEG-4 Part 2, H.264, H.265, or VVC elementary stream"
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
            track_id: u32::from(builder.stream_id),
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
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.stream_id),
            kind: MuxTrackKind::Audio,
            timescale: sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("mp3"),
            mux_policy: direct_ingest_mux_policy("mp3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box: build_mp3_sample_entry_box(
                sample_rate,
                channel_count,
                samples
                    .iter()
                    .map(|sample| (sample.data_size, sample.duration)),
            )?,
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
            track_id: u32::from(builder.stream_id),
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
            track_id: u32::from(builder.stream_id),
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

#[cfg(feature = "async")]
async fn finalize_program_stream_video_track_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    builder: ProgramStreamTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let prefix = read_program_stream_video_prefix_async(file, &builder, spec).await?;
    match detect_path_track_kind_from_prefix(&prefix) {
        DetectedPathTrackKind::Raw(super::super::MuxRawCodec::Mp4v) => {
            let parsed =
                scan_mp4v_segmented_async(file, &builder.segments, builder.total_size, spec).await?;
            Ok(CompositeTrackCandidate {
                track: TrackCandidate {
                    track_id: u32::from(builder.stream_id),
                    kind: MuxTrackKind::Video,
                    timescale: parsed.timescale,
                    language: *b"und",
                    handler_name: direct_ingest_handler_name("mp4v"),
                    mux_policy: direct_ingest_mux_policy("mp4v", MuxTrackKind::Video),
                    width: parsed.width,
                    height: parsed.height,
                    sample_entry_box: parsed.sample_entry_box,
                    source_edit_media_time: None,
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
                    track_id: u32::from(builder.stream_id),
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
                    track_id: u32::from(builder.stream_id),
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
                    track_id: u32::from(builder.stream_id),
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
                "program stream video payload is not a supported MPEG-4 Part 2, H.264, H.265, or VVC elementary stream"
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
        private_header[0],
        parsed.presentation_time,
        parsed.payload_offset + u64::from(PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES),
        parsed.payload_size - PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES,
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
        private_header[0],
        parsed.presentation_time,
        parsed.payload_offset + u64::from(PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES),
        parsed.payload_size - PRIVATE_STREAM_1_PRIVATE_HEADER_BYTES,
        parsed.packet_end,
    )
}

fn finalize_private_stream_1_pes_packet(
    spec: &str,
    substream_id: u8,
    presentation_time: Option<u64>,
    payload_offset: u64,
    payload_size: u32,
    packet_end: u64,
) -> Result<ParsedPrivateStream1PesPacket, MuxError> {
    let kind = if (PRIVATE_STREAM_1_AC3_MIN..=PRIVATE_STREAM_1_AC3_MAX).contains(&substream_id) {
        ProgramStreamTrackKind::Ac3
    } else if (0x20..=0x3F).contains(&substream_id) {
        ProgramStreamTrackKind::Subpicture
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
        presentation_time,
        payload_offset,
        payload_size,
        packet_end,
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
    if file_size - offset < 14 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream pack header".to_string(),
        });
    }
    let mut header = [0_u8; 10];
    read_exact_at_sync(
        file,
        offset + 4,
        &mut header,
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
    if file_size - offset < 14 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated program stream pack header".to_string(),
        });
    }
    let mut header = [0_u8; 10];
    read_exact_at_async(
        file,
        offset + 4,
        &mut header,
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
    if pes_packet_length == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "open-ended PES packets are not supported on the native direct-ingest program-stream path yet".to_string(),
        });
    }
    if header[2] & 0xC0 != 0x80 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "unsupported PES header flags on the native direct-ingest program-stream path"
                .to_string(),
        });
    }
    let header_data_length = u64::from(header[4]);
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
    let packet_end = offset + 6 + u64::from(pes_packet_length);
    let payload_offset = offset + 9 + header_data_length;
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
    if pes_packet_length == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "open-ended PES packets are not supported on the native direct-ingest program-stream path yet".to_string(),
        });
    }
    if header[2] & 0xC0 != 0x80 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "unsupported PES header flags on the native direct-ingest program-stream path"
                .to_string(),
        });
    }
    let header_data_length = u64::from(header[4]);
    let presentation_time = if header[3] & 0x80 != 0 {
        Some(parse_program_stream_pes_timestamp_async(file, offset + 9, file_size, spec).await?)
    } else {
        None
    };
    let packet_end = offset + 6 + u64::from(pes_packet_length);
    let payload_offset = offset + 9 + header_data_length;
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
    })
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
    if pts[0] & 0x11 != 0x01 || pts[2] & 0x01 != 0x01 || pts[4] & 0x01 != 0x01 {
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
