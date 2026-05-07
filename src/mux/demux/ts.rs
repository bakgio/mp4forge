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
    TrackCandidate, build_generic_media_sample_entry_box, direct_ingest_handler_name,
    direct_ingest_mux_policy, read_exact_at_sync,
};
#[cfg(feature = "async")]
use super::ac3::scan_ac3_segmented_async;
use super::ac3::scan_ac3_segmented_sync;
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::{append_file_range_segment, read_segmented_bytes_sync};
#[cfg(feature = "async")]
use super::eac3::scan_eac3_segmented_async;
use super::eac3::scan_eac3_segmented_sync;
#[cfg(feature = "async")]
use super::h264::stage_annex_b_h264_segmented_async;
use super::h264::stage_annex_b_h264_segmented_sync;
#[cfg(feature = "async")]
use super::h265::stage_annex_b_h265_segmented_async;
use super::h265::stage_annex_b_h265_segmented_sync;
use super::mp3::{build_mp3_sample_entry_box, parse_mp3_frame_header};
use super::mp4v::{scan_mp4v_segmented_async, scan_mp4v_segmented_sync};
#[cfg(feature = "async")]
use super::vvc::stage_annex_b_vvc_segmented_async;
use super::vvc::stage_annex_b_vvc_segmented_sync;
use crate::boxes::iso14496_12::DvsC;

const TS_PACKET_SIZE: usize = 188;
const PAT_PID: u16 = 0x0000;
const STREAM_TYPE_MPEG1_AUDIO: u8 = 0x03;
const STREAM_TYPE_MPEG2_AUDIO: u8 = 0x04;
const STREAM_TYPE_PRIVATE_DATA: u8 = 0x06;
const STREAM_TYPE_MPEG4_VIDEO: u8 = 0x10;
const STREAM_TYPE_H264_VIDEO: u8 = 0x1B;
const STREAM_TYPE_H265_VIDEO: u8 = 0x24;
const STREAM_TYPE_VVC_VIDEO: u8 = 0x33;
const STREAM_TYPE_VVC_VIDEO_TEMPORAL: u8 = 0x34;
const STREAM_TYPE_AC3_AUDIO: u8 = 0x81;
const STREAM_TYPE_EAC3_AUDIO: u8 = 0x84;
const STREAM_TYPE_AVS3_VIDEO: u8 = 0xD4;
const PMT_DESCRIPTOR_DVB_TELETEXT: u8 = 0x56;
const PMT_DESCRIPTOR_DVB_SUBTITLE: u8 = 0x59;
const PES_STREAM_ID_PRIVATE_STREAM_1: u8 = 0xBD;
const DIRECT_SUBTITLE_TIMESCALE: u32 = 1_000;
const DIRECT_SUBTITLE_SAMPLE_DURATION: u32 = 1_000;

#[derive(Clone, Copy)]
enum TransportTrackKind {
    Mp3,
    Ac3,
    Eac3,
    Mp4v,
    H264,
    H265,
    Vvc,
    DvbSubtitle,
    DvbTeletext,
}

#[derive(Clone, Copy)]
struct DvbSubtitleConfig {
    language: [u8; 3],
    composition_page_id: u16,
    ancillary_page_id: u16,
    subtitle_type: u8,
}

struct TransportTrackBuilder {
    pid: u16,
    kind: TransportTrackKind,
    segments: Vec<SegmentedMuxSourceSegment>,
    total_size: u64,
    sample_offsets: Vec<u64>,
    language: [u8; 3],
    dvb_subtitle: Option<DvbSubtitleConfig>,
}

fn new_transport_track_builder(pid: u16, kind: TransportTrackKind) -> TransportTrackBuilder {
    TransportTrackBuilder {
        pid,
        kind,
        segments: Vec::new(),
        total_size: 0,
        sample_offsets: Vec::new(),
        language: *b"und",
        dvb_subtitle: None,
    }
}

fn transport_track_uses_full_au(kind: TransportTrackKind) -> bool {
    matches!(
        kind,
        TransportTrackKind::DvbSubtitle | TransportTrackKind::DvbTeletext
    )
}

pub(in crate::mux) fn scan_transport_stream_sync(
    path: &Path,
    spec: &str,
) -> Result<Vec<CompositeTrackCandidate>, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    validate_transport_stream_sync(&mut file, file_size, spec)?;

    let mut pmt_pid = None::<u16>;
    let mut builders = BTreeMap::<u16, TransportTrackBuilder>::new();
    let mut offset = 0_u64;
    while offset + u64::try_from(TS_PACKET_SIZE).unwrap() <= file_size {
        let mut packet = [0_u8; TS_PACKET_SIZE];
        read_exact_at_sync(
            &mut file,
            offset,
            &mut packet,
            spec,
            "truncated MPEG transport stream packet",
        )?;
        parse_transport_packet_sync(spec, &packet, offset, &mut pmt_pid, &mut builders)?;
        offset += u64::try_from(TS_PACKET_SIZE).unwrap();
    }

    finalize_transport_tracks_sync(path, spec, &mut file, builders)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_transport_stream_async(
    path: &Path,
    spec: &str,
) -> Result<Vec<CompositeTrackCandidate>, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    validate_transport_stream_async(&mut file, file_size, spec).await?;

    let mut pmt_pid = None::<u16>;
    let mut builders = BTreeMap::<u16, TransportTrackBuilder>::new();
    let mut offset = 0_u64;
    while offset + u64::try_from(TS_PACKET_SIZE).unwrap() <= file_size {
        let mut packet = [0_u8; TS_PACKET_SIZE];
        read_exact_at_async(
            &mut file,
            offset,
            &mut packet,
            spec,
            "truncated MPEG transport stream packet",
        )
        .await?;
        parse_transport_packet_sync(spec, &packet, offset, &mut pmt_pid, &mut builders)?;
        offset += u64::try_from(TS_PACKET_SIZE).unwrap();
    }

    finalize_transport_tracks_async(path, spec, &mut file, builders).await
}

fn validate_transport_stream_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if file_size < u64::try_from(TS_PACKET_SIZE * 2).unwrap() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "transport stream input is too short to validate packet sync".to_string(),
        });
    }
    let mut prefix = [0_u8; TS_PACKET_SIZE * 2];
    read_exact_at_sync(
        file,
        0,
        &mut prefix,
        spec,
        "transport stream input is truncated before the first two packets",
    )?;
    if prefix[0] != 0x47 || prefix[TS_PACKET_SIZE] != 0x47 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "input does not carry MPEG transport stream packet sync bytes".to_string(),
        });
    }
    Ok(())
}

#[cfg(feature = "async")]
async fn validate_transport_stream_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if file_size < u64::try_from(TS_PACKET_SIZE * 2).unwrap() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "transport stream input is too short to validate packet sync".to_string(),
        });
    }
    let mut prefix = [0_u8; TS_PACKET_SIZE * 2];
    read_exact_at_async(
        file,
        0,
        &mut prefix,
        spec,
        "transport stream input is truncated before the first two packets",
    )
    .await?;
    if prefix[0] != 0x47 || prefix[TS_PACKET_SIZE] != 0x47 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "input does not carry MPEG transport stream packet sync bytes".to_string(),
        });
    }
    Ok(())
}

fn parse_transport_packet_sync(
    spec: &str,
    packet: &[u8; TS_PACKET_SIZE],
    packet_offset: u64,
    pmt_pid: &mut Option<u16>,
    builders: &mut BTreeMap<u16, TransportTrackBuilder>,
) -> Result<(), MuxError> {
    if packet[0] != 0x47 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("missing transport-stream sync byte at packet offset {packet_offset}"),
        });
    }
    if packet[1] & 0x80 != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "transport-stream packets with transport errors are not supported".to_string(),
        });
    }
    if packet[3] & 0xC0 != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "scrambled transport-stream packets are not supported".to_string(),
        });
    }
    let payload_unit_start = packet[1] & 0x40 != 0;
    let pid = (u16::from(packet[1] & 0x1F) << 8) | u16::from(packet[2]);
    let adaptation_control = (packet[3] >> 4) & 0x03;
    if adaptation_control == 0x00 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "transport-stream packets with reserved adaptation-control state are not supported"
                    .to_string(),
        });
    }
    if adaptation_control == 0x02 {
        return Ok(());
    }
    let mut payload_offset = 4usize;
    if adaptation_control == 0x03 {
        let adaptation_length = usize::from(packet[4]);
        payload_offset =
            payload_offset
                .checked_add(1 + adaptation_length)
                .ok_or(MuxError::LayoutOverflow(
                    "transport-stream adaptation field",
                ))?;
        if payload_offset > TS_PACKET_SIZE {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "transport-stream adaptation field overflowed the packet payload"
                    .to_string(),
            });
        }
    }
    if payload_offset >= TS_PACKET_SIZE {
        return Ok(());
    }
    let payload = &packet[payload_offset..];

    if pid == PAT_PID {
        if payload_unit_start && let Some(found_pmt_pid) = parse_pat_section(spec, payload)? {
            *pmt_pid = Some(found_pmt_pid);
        }
        return Ok(());
    }
    if Some(pid) == *pmt_pid {
        if payload_unit_start {
            parse_pmt_section(spec, payload, builders)?;
        }
        return Ok(());
    }
    let Some(builder) = builders.get_mut(&pid) else {
        return Ok(());
    };
    if payload_unit_start {
        let payload_body_offset = parse_ts_pes_payload_offset(spec, payload, builder.kind)?;
        if transport_track_uses_full_au(builder.kind) {
            builder.sample_offsets.push(builder.total_size);
        }
        let pes_payload = &payload[payload_body_offset..];
        if !pes_payload.is_empty() {
            append_file_range_segment(
                &mut builder.segments,
                &mut builder.total_size,
                packet_offset + u64::try_from(payload_offset + payload_body_offset).unwrap(),
                u32::try_from(pes_payload.len())
                    .map_err(|_| MuxError::LayoutOverflow("transport-stream PES payload"))?,
            );
        }
    } else if !payload.is_empty() {
        append_file_range_segment(
            &mut builder.segments,
            &mut builder.total_size,
            packet_offset + u64::try_from(payload_offset).unwrap(),
            u32::try_from(payload.len())
                .map_err(|_| MuxError::LayoutOverflow("transport-stream packet payload"))?,
        );
    }
    Ok(())
}

fn parse_pat_section(spec: &str, payload: &[u8]) -> Result<Option<u16>, MuxError> {
    if payload.is_empty() {
        return Ok(None);
    }
    let pointer_field = usize::from(payload[0]);
    let start = 1usize
        .checked_add(pointer_field)
        .ok_or(MuxError::LayoutOverflow("PAT pointer field"))?;
    if payload.len() < start + 8 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated PAT section".to_string(),
        });
    }
    if payload[start] != 0x00 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "unsupported PAT table id".to_string(),
        });
    }
    let section_length =
        usize::from(u16::from_be_bytes([payload[start + 1], payload[start + 2]]) & 0x0FFF);
    if payload.len() < start + 3 + section_length {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated PAT payload".to_string(),
        });
    }
    let mut entry_offset = start + 8;
    let section_end = start + 3 + section_length - 4;
    let mut found = None::<u16>;
    while entry_offset + 4 <= section_end {
        let program_number = u16::from_be_bytes([payload[entry_offset], payload[entry_offset + 1]]);
        let pid = (u16::from(payload[entry_offset + 2] & 0x1F) << 8)
            | u16::from(payload[entry_offset + 3]);
        if program_number != 0 && found.replace(pid).is_some() {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "multiple PAT program mappings are not supported on the native direct-ingest transport-stream path yet".to_string(),
            });
        }
        entry_offset += 4;
    }
    Ok(found)
}

fn parse_pmt_section(
    spec: &str,
    payload: &[u8],
    builders: &mut BTreeMap<u16, TransportTrackBuilder>,
) -> Result<(), MuxError> {
    if payload.is_empty() {
        return Ok(());
    }
    let pointer_field = usize::from(payload[0]);
    let start = 1usize
        .checked_add(pointer_field)
        .ok_or(MuxError::LayoutOverflow("PMT pointer field"))?;
    if payload.len() < start + 12 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated PMT section".to_string(),
        });
    }
    if payload[start] != 0x02 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "unsupported PMT table id".to_string(),
        });
    }
    let section_length =
        usize::from(u16::from_be_bytes([payload[start + 1], payload[start + 2]]) & 0x0FFF);
    if payload.len() < start + 3 + section_length {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated PMT payload".to_string(),
        });
    }
    let program_info_length =
        usize::from(u16::from_be_bytes([payload[start + 10], payload[start + 11]]) & 0x0FFF);
    let mut entry_offset = start + 12 + program_info_length;
    let section_end = start + 3 + section_length - 4;
    while entry_offset + 5 <= section_end {
        let stream_type = payload[entry_offset];
        let elementary_pid = (u16::from(payload[entry_offset + 1] & 0x1F) << 8)
            | u16::from(payload[entry_offset + 2]);
        let es_info_length = usize::from(
            u16::from_be_bytes([payload[entry_offset + 3], payload[entry_offset + 4]]) & 0x0FFF,
        );
        let es_info_start = entry_offset
            .checked_add(5)
            .ok_or(MuxError::LayoutOverflow("PMT elementary-stream info start"))?;
        let es_info_end = es_info_start
            .checked_add(es_info_length)
            .ok_or(MuxError::LayoutOverflow("PMT elementary-stream info end"))?;
        if es_info_end > section_end {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated PMT elementary-stream descriptor payload".to_string(),
            });
        }
        let es_info = &payload[es_info_start..es_info_end];
        match stream_type {
            STREAM_TYPE_MPEG1_AUDIO | STREAM_TYPE_MPEG2_AUDIO => {
                builders.entry(elementary_pid).or_insert_with(|| {
                    new_transport_track_builder(elementary_pid, TransportTrackKind::Mp3)
                });
            }
            STREAM_TYPE_AC3_AUDIO => {
                builders.entry(elementary_pid).or_insert_with(|| {
                    new_transport_track_builder(elementary_pid, TransportTrackKind::Ac3)
                });
            }
            STREAM_TYPE_EAC3_AUDIO => {
                builders.entry(elementary_pid).or_insert_with(|| {
                    new_transport_track_builder(elementary_pid, TransportTrackKind::Eac3)
                });
            }
            STREAM_TYPE_MPEG4_VIDEO => {
                builders.entry(elementary_pid).or_insert_with(|| {
                    new_transport_track_builder(elementary_pid, TransportTrackKind::Mp4v)
                });
            }
            STREAM_TYPE_H264_VIDEO => {
                builders.entry(elementary_pid).or_insert_with(|| {
                    new_transport_track_builder(elementary_pid, TransportTrackKind::H264)
                });
            }
            STREAM_TYPE_H265_VIDEO => {
                builders.entry(elementary_pid).or_insert_with(|| {
                    new_transport_track_builder(elementary_pid, TransportTrackKind::H265)
                });
            }
            STREAM_TYPE_VVC_VIDEO | STREAM_TYPE_VVC_VIDEO_TEMPORAL => {
                builders.entry(elementary_pid).or_insert_with(|| {
                    new_transport_track_builder(elementary_pid, TransportTrackKind::Vvc)
                });
            }
            STREAM_TYPE_PRIVATE_DATA => {
                if let Some(track) = parse_transport_private_data_track(spec, es_info)? {
                    builders.entry(elementary_pid).or_insert_with(|| {
                        let mut builder = new_transport_track_builder(elementary_pid, track.kind);
                        builder.language = track.language;
                        builder.dvb_subtitle = track.dvb_subtitle;
                        builder
                    });
                } else {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: "transport-stream private-data carriage is not supported on the native direct-ingest path yet".to_string(),
                    });
                }
            }
            STREAM_TYPE_AVS3_VIDEO => {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "transport-stream AVS3 video carriage is not supported on the native direct-ingest path yet"
                            .to_string(),
                });
            }
            0x02 => {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "transport-stream MPEG-2 video carriage is not supported on the native direct-ingest path yet"
                            .to_string(),
                });
            }
            other => {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!(
                        "transport-stream stream type 0x{other:02X} is not supported on the native direct-ingest path yet"
                    ),
                });
            }
        }
        entry_offset = es_info_end;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct TransportPrivateDataTrack {
    kind: TransportTrackKind,
    language: [u8; 3],
    dvb_subtitle: Option<DvbSubtitleConfig>,
}

fn parse_transport_private_data_track(
    spec: &str,
    es_info: &[u8],
) -> Result<Option<TransportPrivateDataTrack>, MuxError> {
    let mut descriptor_offset = 0usize;
    let mut found = None::<TransportPrivateDataTrack>;
    while descriptor_offset < es_info.len() {
        if es_info.len() - descriptor_offset < 2 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated PMT descriptor header".to_string(),
            });
        }
        let descriptor_tag = es_info[descriptor_offset];
        let descriptor_length = usize::from(es_info[descriptor_offset + 1]);
        let descriptor_end = descriptor_offset
            .checked_add(2 + descriptor_length)
            .ok_or(MuxError::LayoutOverflow("PMT descriptor length"))?;
        if descriptor_end > es_info.len() {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated PMT descriptor payload".to_string(),
            });
        }
        let descriptor_payload = &es_info[descriptor_offset + 2..descriptor_end];
        let parsed = match descriptor_tag {
            PMT_DESCRIPTOR_DVB_SUBTITLE => {
                Some(parse_dvb_subtitle_descriptor(spec, descriptor_payload)?)
            }
            PMT_DESCRIPTOR_DVB_TELETEXT => {
                Some(parse_dvb_teletext_descriptor(spec, descriptor_payload)?)
            }
            _ => None,
        };
        if let Some(parsed) = parsed
            && found.replace(parsed).is_some()
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message:
                    "multiple transport-stream private-data descriptor track declarations are not supported on the native direct-ingest path yet"
                        .to_string(),
            });
        }
        descriptor_offset = descriptor_end;
    }
    Ok(found)
}

fn parse_dvb_subtitle_descriptor(
    spec: &str,
    descriptor_payload: &[u8],
) -> Result<TransportPrivateDataTrack, MuxError> {
    if descriptor_payload.len() < 8 || !descriptor_payload.len().is_multiple_of(8) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "transport-stream DVB subtitle descriptors must contain whole 8-byte service entries".to_string(),
        });
    }
    if descriptor_payload.len() != 8 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "transport-stream DVB subtitle descriptors with multiple service entries are not supported on the native direct-ingest path yet"
                    .to_string(),
        });
    }
    Ok(TransportPrivateDataTrack {
        kind: TransportTrackKind::DvbSubtitle,
        language: [
            descriptor_payload[0],
            descriptor_payload[1],
            descriptor_payload[2],
        ],
        dvb_subtitle: Some(DvbSubtitleConfig {
            language: [
                descriptor_payload[0],
                descriptor_payload[1],
                descriptor_payload[2],
            ],
            subtitle_type: descriptor_payload[3],
            composition_page_id: u16::from_be_bytes([descriptor_payload[4], descriptor_payload[5]]),
            ancillary_page_id: u16::from_be_bytes([descriptor_payload[6], descriptor_payload[7]]),
        }),
    })
}

fn parse_dvb_teletext_descriptor(
    spec: &str,
    descriptor_payload: &[u8],
) -> Result<TransportPrivateDataTrack, MuxError> {
    if descriptor_payload.len() < 5 || !descriptor_payload.len().is_multiple_of(5) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "transport-stream DVB teletext descriptors must contain whole 5-byte service entries".to_string(),
        });
    }
    if descriptor_payload.len() != 5 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "transport-stream DVB teletext descriptors with multiple service entries are not supported on the native direct-ingest path yet"
                    .to_string(),
        });
    }
    Ok(TransportPrivateDataTrack {
        kind: TransportTrackKind::DvbTeletext,
        language: [
            descriptor_payload[0],
            descriptor_payload[1],
            descriptor_payload[2],
        ],
        dvb_subtitle: None,
    })
}

fn parse_ts_pes_payload_offset(
    spec: &str,
    payload: &[u8],
    kind: TransportTrackKind,
) -> Result<usize, MuxError> {
    if payload.len() < 9 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated transport-stream PES header".to_string(),
        });
    }
    if payload[..3] != [0x00, 0x00, 0x01] {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "transport-stream payload-unit start did not begin with a PES start code"
                .to_string(),
        });
    }
    match kind {
        TransportTrackKind::Mp3 if !(0xC0..=0xDF).contains(&payload[3]) => {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "transport-stream PES stream id is not a supported MPEG audio stream"
                    .to_string(),
            });
        }
        TransportTrackKind::Ac3 | TransportTrackKind::Eac3
            if payload[3] != PES_STREAM_ID_PRIVATE_STREAM_1 =>
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message:
                    "transport-stream PES stream id is not a supported AC-3 or E-AC-3 private audio stream"
                        .to_string(),
            });
        }
        TransportTrackKind::Mp4v if !(0xE0..=0xEF).contains(&payload[3]) => {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message:
                    "transport-stream PES stream id is not a supported MPEG-4 Part 2 video stream"
                        .to_string(),
            });
        }
        TransportTrackKind::H264 | TransportTrackKind::H265 | TransportTrackKind::Vvc
            if !(0xE0..=0xEF).contains(&payload[3]) =>
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "transport-stream PES stream id is not a supported video stream"
                    .to_string(),
            });
        }
        TransportTrackKind::DvbSubtitle | TransportTrackKind::DvbTeletext
            if payload[3] != PES_STREAM_ID_PRIVATE_STREAM_1 =>
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message:
                    "transport-stream PES stream id is not a supported private subtitle or teletext stream"
                        .to_string(),
            });
        }
        _ => {}
    }
    if payload[6] & 0xC0 != 0x80 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "unsupported transport-stream PES header flags".to_string(),
        });
    }
    let header_data_length = usize::from(payload[8]);
    let payload_offset = 9usize
        .checked_add(header_data_length)
        .ok_or(MuxError::LayoutOverflow("transport-stream PES header"))?;
    if payload_offset > payload.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated transport-stream PES optional header".to_string(),
        });
    }
    Ok(payload_offset)
}

fn finalize_transport_tracks_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    builders: BTreeMap<u16, TransportTrackBuilder>,
) -> Result<Vec<CompositeTrackCandidate>, MuxError> {
    if builders.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "transport stream input did not contain any supported native direct-ingest streams"
                    .to_string(),
        });
    }
    let mut tracks = Vec::new();
    for (track_index, builder) in builders.into_values().enumerate() {
        tracks.push(match builder.kind {
            TransportTrackKind::Mp3 => {
                finalize_transport_mp3_track_sync(path, spec, file, track_index, builder)?
            }
            TransportTrackKind::Ac3 => {
                finalize_transport_ac3_track_sync(path, spec, file, track_index, builder)?
            }
            TransportTrackKind::Eac3 => {
                finalize_transport_eac3_track_sync(path, spec, file, track_index, builder)?
            }
            TransportTrackKind::Mp4v => {
                finalize_transport_mp4v_track_sync(path, spec, file, track_index, builder)?
            }
            TransportTrackKind::H264 => {
                finalize_transport_h264_track_sync(path, spec, file, track_index, builder)?
            }
            TransportTrackKind::H265 => {
                finalize_transport_h265_track_sync(path, spec, file, track_index, builder)?
            }
            TransportTrackKind::Vvc => {
                finalize_transport_vvc_track_sync(path, spec, file, track_index, builder)?
            }
            TransportTrackKind::DvbSubtitle => {
                finalize_transport_dvb_subtitle_track_sync(path, spec, track_index, builder)?
            }
            TransportTrackKind::DvbTeletext => {
                finalize_transport_dvb_teletext_track_sync(path, spec, track_index, builder)?
            }
        });
    }
    Ok(tracks)
}

#[cfg(feature = "async")]
async fn finalize_transport_tracks_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    builders: BTreeMap<u16, TransportTrackBuilder>,
) -> Result<Vec<CompositeTrackCandidate>, MuxError> {
    if builders.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "transport stream input did not contain any supported native direct-ingest streams"
                    .to_string(),
        });
    }
    let mut tracks = Vec::new();
    for (track_index, builder) in builders.into_values().enumerate() {
        tracks.push(match builder.kind {
            TransportTrackKind::Mp3 => {
                finalize_transport_mp3_track_async(path, spec, file, track_index, builder).await?
            }
            TransportTrackKind::Ac3 => {
                finalize_transport_ac3_track_async(path, spec, file, track_index, builder).await?
            }
            TransportTrackKind::Eac3 => {
                finalize_transport_eac3_track_async(path, spec, file, track_index, builder).await?
            }
            TransportTrackKind::Mp4v => {
                finalize_transport_mp4v_track_async(path, spec, file, track_index, builder).await?
            }
            TransportTrackKind::H264 => {
                finalize_transport_h264_track_async(path, spec, file, track_index, builder).await?
            }
            TransportTrackKind::H265 => {
                finalize_transport_h265_track_async(path, spec, file, track_index, builder).await?
            }
            TransportTrackKind::Vvc => {
                finalize_transport_vvc_track_async(path, spec, file, track_index, builder).await?
            }
            TransportTrackKind::DvbSubtitle => {
                finalize_transport_dvb_subtitle_track_async(path, spec, track_index, builder)
                    .await?
            }
            TransportTrackKind::DvbTeletext => {
                finalize_transport_dvb_teletext_track_async(path, spec, track_index, builder)
                    .await?
            }
        });
    }
    Ok(tracks)
}

fn finalize_transport_mp3_track_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let mut offset = 0_u64;
    let mut expected = None::<(u32, u16, u32)>;
    let mut samples = Vec::new();
    while offset < builder.total_size {
        if builder.total_size - offset < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated MPEG audio frame header inside transport-stream payload"
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
            "truncated MPEG audio frame header inside transport-stream payload",
        )?;
        let parsed = parse_mp3_frame_header(&header, offset, spec)?;
        if offset
            .checked_add(u64::from(parsed.frame_length))
            .is_none_or(|end| end > builder.total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "truncated MPEG audio frame at logical transport-stream offset {offset}"
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
                        "transport-stream MPEG audio frames changed sample rate or channel layout mid-stream"
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
        offset =
            offset
                .checked_add(u64::from(parsed.frame_length))
                .ok_or(MuxError::LayoutOverflow(
                    "transport-stream MPEG audio offset",
                ))?;
    }
    let (sample_rate, channel_count, _) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "transport stream input did not contain any MPEG audio frames".to_string(),
        })?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
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

fn finalize_transport_mp4v_track_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed = scan_mp4v_segmented_sync(file, &builder.segments, builder.total_size, spec)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
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

fn finalize_transport_ac3_track_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed = scan_ac3_segmented_sync(file, &builder.segments, builder.total_size, spec)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
            kind: MuxTrackKind::Audio,
            timescale: parsed.sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("ac3"),
            mux_policy: direct_ingest_mux_policy("ac3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
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

fn finalize_transport_eac3_track_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed = scan_eac3_segmented_sync(file, &builder.segments, builder.total_size, spec)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
            kind: MuxTrackKind::Audio,
            timescale: parsed.sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("eac3"),
            mux_policy: direct_ingest_mux_policy("eac3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
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

fn finalize_transport_h264_track_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed =
        stage_annex_b_h264_segmented_sync(path, file, &builder.segments, builder.total_size, spec)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
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

fn finalize_transport_h265_track_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed =
        stage_annex_b_h265_segmented_sync(path, file, &builder.segments, builder.total_size, spec)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
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

fn finalize_transport_vvc_track_sync(
    path: &Path,
    spec: &str,
    file: &mut File,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed =
        stage_annex_b_vvc_segmented_sync(path, file, &builder.segments, builder.total_size, spec)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
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

#[cfg(feature = "async")]
async fn finalize_transport_mp3_track_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let mut offset = 0_u64;
    let mut expected = None::<(u32, u16, u32)>;
    let mut samples = Vec::new();
    while offset < builder.total_size {
        if builder.total_size - offset < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated MPEG audio frame header inside transport-stream payload"
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
            "truncated MPEG audio frame header inside transport-stream payload",
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
                    "truncated MPEG audio frame at logical transport-stream offset {offset}"
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
                        "transport-stream MPEG audio frames changed sample rate or channel layout mid-stream"
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
        offset =
            offset
                .checked_add(u64::from(parsed.frame_length))
                .ok_or(MuxError::LayoutOverflow(
                    "transport-stream MPEG audio offset",
                ))?;
    }
    let (sample_rate, channel_count, _) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "transport stream input did not contain any MPEG audio frames".to_string(),
        })?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
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
async fn finalize_transport_mp4v_track_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed =
        scan_mp4v_segmented_async(file, &builder.segments, builder.total_size, spec).await?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
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

#[cfg(feature = "async")]
async fn finalize_transport_ac3_track_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed =
        scan_ac3_segmented_async(file, &builder.segments, builder.total_size, spec).await?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
            kind: MuxTrackKind::Audio,
            timescale: parsed.sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("ac3"),
            mux_policy: direct_ingest_mux_policy("ac3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
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

#[cfg(feature = "async")]
async fn finalize_transport_eac3_track_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed =
        scan_eac3_segmented_async(file, &builder.segments, builder.total_size, spec).await?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
            kind: MuxTrackKind::Audio,
            timescale: parsed.sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("eac3"),
            mux_policy: direct_ingest_mux_policy("eac3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
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

#[cfg(feature = "async")]
async fn finalize_transport_h264_track_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed =
        stage_annex_b_h264_segmented_async(path, file, &builder.segments, builder.total_size, spec)
            .await?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
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

#[cfg(feature = "async")]
async fn finalize_transport_h265_track_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed =
        stage_annex_b_h265_segmented_async(path, file, &builder.segments, builder.total_size, spec)
            .await?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
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

#[cfg(feature = "async")]
async fn finalize_transport_vvc_track_async(
    path: &Path,
    spec: &str,
    file: &mut TokioFile,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let parsed =
        stage_annex_b_vvc_segmented_async(path, file, &builder.segments, builder.total_size, spec)
            .await?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
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

fn build_transport_full_au_samples(
    spec: &str,
    builder: &TransportTrackBuilder,
) -> Result<Vec<CandidateSample>, MuxError> {
    if builder.sample_offsets.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "transport stream input did not contain any subtitle or teletext PES payload units"
                    .to_string(),
        });
    }
    let mut samples = Vec::with_capacity(builder.sample_offsets.len());
    for (index, &sample_offset) in builder.sample_offsets.iter().enumerate() {
        let next_offset = builder
            .sample_offsets
            .get(index + 1)
            .copied()
            .unwrap_or(builder.total_size);
        if next_offset <= sample_offset {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message:
                    "transport stream carried subtitle or teletext samples must advance monotonically"
                        .to_string(),
            });
        }
        let data_size = u32::try_from(next_offset - sample_offset).map_err(|_| {
            MuxError::LayoutOverflow("transport-stream carried subtitle sample size")
        })?;
        samples.push(CandidateSample {
            source_index: usize::MAX,
            data_offset: sample_offset,
            data_size,
            duration: DIRECT_SUBTITLE_SAMPLE_DURATION,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    Ok(samples)
}

fn build_dvb_subtitle_sample_entry_box(
    spec: &str,
    builder: &TransportTrackBuilder,
) -> Result<Vec<u8>, MuxError> {
    let config = builder
        .dvb_subtitle
        .ok_or(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "transport-stream DVB subtitle builder is missing its descriptor configuration"
                    .to_string(),
        })?;
    let child_box = super::super::mp4::encode_typed_box(
        &DvsC {
            composition_page_id: config.composition_page_id,
            ancillary_page_id: config.ancillary_page_id,
            subtitle_type: config.subtitle_type,
        },
        &[],
    )?;
    build_generic_media_sample_entry_box(crate::FourCc::from_bytes(*b"dvbs"), &[child_box])
}

fn build_dvb_teletext_sample_entry_box() -> Result<Vec<u8>, MuxError> {
    build_generic_media_sample_entry_box(crate::FourCc::from_bytes(*b"dvbt"), &[])
}

fn finalize_transport_dvb_subtitle_track_sync(
    path: &Path,
    spec: &str,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let language = builder
        .dvb_subtitle
        .map(|config| config.language)
        .unwrap_or(builder.language);
    let sample_entry_box = build_dvb_subtitle_sample_entry_box(spec, &builder)?;
    let samples = build_transport_full_au_samples(spec, &builder)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
            kind: MuxTrackKind::Subtitle,
            timescale: DIRECT_SUBTITLE_TIMESCALE,
            language,
            handler_name: "SubtitleHandler".to_string(),
            mux_policy: direct_ingest_mux_policy("dvb-subtitle", MuxTrackKind::Subtitle),
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

fn finalize_transport_dvb_teletext_track_sync(
    path: &Path,
    spec: &str,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    let sample_entry_box = build_dvb_teletext_sample_entry_box()?;
    let samples = build_transport_full_au_samples(spec, &builder)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(builder.pid),
            kind: MuxTrackKind::Subtitle,
            timescale: DIRECT_SUBTITLE_TIMESCALE,
            language: builder.language,
            handler_name: "SubtitleHandler".to_string(),
            mux_policy: direct_ingest_mux_policy("dvb-teletext", MuxTrackKind::Subtitle),
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
async fn finalize_transport_dvb_subtitle_track_async(
    path: &Path,
    spec: &str,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    finalize_transport_dvb_subtitle_track_sync(path, spec, 0, builder)
}

#[cfg(feature = "async")]
async fn finalize_transport_dvb_teletext_track_async(
    path: &Path,
    spec: &str,
    _track_index: usize,
    builder: TransportTrackBuilder,
) -> Result<CompositeTrackCandidate, MuxError> {
    finalize_transport_dvb_teletext_track_sync(path, spec, 0, builder)
}
