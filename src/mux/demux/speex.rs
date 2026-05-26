use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_spans_async;
use super::super::import::{
    SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData, SegmentedMuxSourceSpec, StagedSample,
    build_btrt_from_sample_sizes, read_spans_sync,
};
#[cfg(feature = "async")]
use super::ogg_common::read_ogg_page_header_async;
use super::ogg_common::{OggPacketBuilder, read_ogg_page_header_sync};
use crate::FourCc;

const SPEEX_ENTRY: FourCc = FourCc::from_bytes(*b"spex");
const SPEEX_VENDOR: [u8; 4] = *b"mp4f";

pub(in crate::mux) struct ParsedOggSpeexTrack {
    pub(in crate::mux) segmented_source: SegmentedMuxSourceSpec,
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

struct CompletedSpeexPageState {
    packets: Vec<super::ogg_common::CompletedOggPacket>,
    eos: bool,
}

struct SpeexConfig {
    sample_rate: u32,
}

pub(in crate::mux) fn scan_ogg_speex_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedOggSpeexTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut packet_builder = OggPacketBuilder::default();
    let mut config = None;
    let mut saw_tags_packet = false;
    let mut logical_size = 0_u64;
    let mut transformed_segments = Vec::new();
    let mut samples = Vec::new();
    let mut decoded_samples = 0_u64;

    while offset < file_size {
        let page = read_ogg_page_header_sync(&mut file, offset, spec)?;
        if packet_builder.is_empty() && page.header_type & 0x01 != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Speex input started in the middle of a continued packet".to_string(),
            });
        }
        offset = page
            .payload_offset
            .checked_add(page.payload_size)
            .ok_or(MuxError::LayoutOverflow("Ogg page range"))?;
        let mut page_cursor = page.payload_offset;
        let mut completed = Vec::new();
        for lacing in &page.lacing_values {
            packet_builder.push_span(page_cursor, u32::from(*lacing))?;
            page_cursor += u64::from(*lacing);
            if *lacing < 255 {
                let packet = packet_builder.finish();
                if packet.total_size != 0 {
                    completed.push(packet);
                }
            }
        }
        if completed.is_empty() {
            continue;
        }
        process_speex_completed_page_sync(
            &mut file,
            spec,
            &mut config,
            &mut saw_tags_packet,
            &mut logical_size,
            &mut transformed_segments,
            &mut samples,
            &mut decoded_samples,
            CompletedSpeexPageState {
                packets: completed,
                eos: page.header_type & 0x04 != 0,
            },
        )?;
    }

    finalize_speex_track(
        path,
        spec,
        &mut packet_builder,
        config,
        logical_size,
        transformed_segments,
        samples,
    )
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_ogg_speex_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedOggSpeexTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut packet_builder = OggPacketBuilder::default();
    let mut config = None;
    let mut saw_tags_packet = false;
    let mut logical_size = 0_u64;
    let mut transformed_segments = Vec::new();
    let mut samples = Vec::new();
    let mut decoded_samples = 0_u64;

    while offset < file_size {
        let page = read_ogg_page_header_async(&mut file, offset, spec).await?;
        if packet_builder.is_empty() && page.header_type & 0x01 != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Speex input started in the middle of a continued packet".to_string(),
            });
        }
        offset = page
            .payload_offset
            .checked_add(page.payload_size)
            .ok_or(MuxError::LayoutOverflow("Ogg page range"))?;
        let mut page_cursor = page.payload_offset;
        let mut completed = Vec::new();
        for lacing in &page.lacing_values {
            packet_builder.push_span(page_cursor, u32::from(*lacing))?;
            page_cursor += u64::from(*lacing);
            if *lacing < 255 {
                let packet = packet_builder.finish();
                if packet.total_size != 0 {
                    completed.push(packet);
                }
            }
        }
        if completed.is_empty() {
            continue;
        }
        process_speex_completed_page_async(
            &mut file,
            spec,
            &mut config,
            &mut saw_tags_packet,
            &mut logical_size,
            &mut transformed_segments,
            &mut samples,
            &mut decoded_samples,
            CompletedSpeexPageState {
                packets: completed,
                eos: page.header_type & 0x04 != 0,
            },
        )
        .await?;
    }

    finalize_speex_track(
        path,
        spec,
        &mut packet_builder,
        config,
        logical_size,
        transformed_segments,
        samples,
    )
}

#[allow(clippy::too_many_arguments)]
fn process_speex_completed_page_sync(
    file: &mut File,
    spec: &str,
    config: &mut Option<SpeexConfig>,
    saw_tags_packet: &mut bool,
    logical_size: &mut u64,
    transformed_segments: &mut Vec<SegmentedMuxSourceSegment>,
    samples: &mut Vec<StagedSample>,
    decoded_samples: &mut u64,
    page: CompletedSpeexPageState,
) -> Result<(), MuxError> {
    let mut audio_packets = Vec::new();
    for packet in page.packets {
        let packet_bytes = read_spans_sync(
            file,
            &packet.spans,
            packet.total_size,
            spec,
            "Ogg Speex packet is truncated",
        )?;
        if config.is_none() {
            *config = Some(parse_speex_header(&packet_bytes, spec)?);
            continue;
        }
        if !*saw_tags_packet && packet_bytes.starts_with(b"SpeexTags") {
            *saw_tags_packet = true;
            continue;
        }
        *saw_tags_packet = true;
        audio_packets.push(packet);
    }
    if audio_packets.is_empty() {
        return Ok(());
    }
    append_speex_audio_packets(
        decoded_samples,
        logical_size,
        transformed_segments,
        samples,
        audio_packets,
        page.eos,
    )
}

#[cfg(feature = "async")]
#[allow(clippy::too_many_arguments)]
async fn process_speex_completed_page_async(
    file: &mut TokioFile,
    spec: &str,
    config: &mut Option<SpeexConfig>,
    saw_tags_packet: &mut bool,
    logical_size: &mut u64,
    transformed_segments: &mut Vec<SegmentedMuxSourceSegment>,
    samples: &mut Vec<StagedSample>,
    decoded_samples: &mut u64,
    page: CompletedSpeexPageState,
) -> Result<(), MuxError> {
    let mut audio_packets = Vec::new();
    for packet in page.packets {
        let packet_bytes: Vec<u8> = read_spans_async(
            file,
            &packet.spans,
            packet.total_size,
            spec,
            "Ogg Speex packet is truncated",
        )
        .await?;
        if config.is_none() {
            *config = Some(parse_speex_header(&packet_bytes, spec)?);
            continue;
        }
        if !*saw_tags_packet && packet_bytes.starts_with(b"SpeexTags") {
            *saw_tags_packet = true;
            continue;
        }
        *saw_tags_packet = true;
        audio_packets.push(packet);
    }
    if audio_packets.is_empty() {
        return Ok(());
    }
    append_speex_audio_packets(
        decoded_samples,
        logical_size,
        transformed_segments,
        samples,
        audio_packets,
        page.eos,
    )
}

#[allow(clippy::too_many_arguments)]
fn append_speex_audio_packets(
    decoded_samples: &mut u64,
    logical_size: &mut u64,
    transformed_segments: &mut Vec<SegmentedMuxSourceSegment>,
    samples: &mut Vec<StagedSample>,
    audio_packets: Vec<super::ogg_common::CompletedOggPacket>,
    eos: bool,
) -> Result<(), MuxError> {
    let last_index = audio_packets.len().saturating_sub(1);
    for (index, packet) in audio_packets.into_iter().enumerate() {
        let duration = if eos && index == last_index {
            0_u64
        } else {
            1_u64
        };
        let data_offset = *logical_size;
        for span in &packet.spans {
            transformed_segments.push(SegmentedMuxSourceSegment {
                logical_offset: *logical_size,
                data: SegmentedMuxSourceSegmentData::FileRange {
                    source_offset: span.source_offset,
                    size: span.size,
                },
            });
            *logical_size = logical_size
                .checked_add(u64::from(span.size))
                .ok_or(MuxError::LayoutOverflow("Ogg Speex logical source size"))?;
        }
        samples.push(StagedSample {
            data_offset,
            data_size: packet.total_size,
            duration: u32::try_from(duration)
                .map_err(|_| MuxError::LayoutOverflow("Ogg Speex packet duration"))?,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        *decoded_samples = decoded_samples
            .checked_add(duration)
            .ok_or(MuxError::LayoutOverflow("Ogg Speex decoded sample count"))?;
    }
    Ok(())
}

fn finalize_speex_track(
    path: &Path,
    spec: &str,
    packet_builder: &mut OggPacketBuilder,
    config: Option<SpeexConfig>,
    logical_size: u64,
    transformed_segments: Vec<SegmentedMuxSourceSegment>,
    samples: Vec<StagedSample>,
) -> Result<ParsedOggSpeexTrack, MuxError> {
    if !packet_builder.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Speex input ended in the middle of a packet".to_string(),
        });
    }
    let config = config.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "Ogg Speex input did not contain a Speex header packet".to_string(),
    })?;
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Speex input did not contain any audio packets after headers".to_string(),
        });
    }
    Ok(ParsedOggSpeexTrack {
        segmented_source: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: transformed_segments,
            total_size: logical_size,
        },
        sample_rate: config.sample_rate,
        sample_entry_box: build_speex_sample_entry_box(&config, &samples)?,
        samples,
    })
}

fn build_speex_sample_entry_box(
    config: &SpeexConfig,
    samples: &[StagedSample],
) -> Result<Vec<u8>, MuxError> {
    let btrt = build_btrt_from_sample_sizes(
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        config.sample_rate,
    )?;
    let btrt_bytes = super::super::mp4::encode_typed_box(&btrt, &[])?;
    build_speex_audio_sample_entry_box(config.sample_rate, &[btrt_bytes])
}

fn build_speex_audio_sample_entry_box(
    sample_rate: u32,
    child_boxes: &[Vec<u8>],
) -> Result<Vec<u8>, MuxError> {
    let mut payload = Vec::with_capacity(28 + child_boxes.iter().map(Vec::len).sum::<usize>());
    payload.extend_from_slice(&[0; 6]);
    payload.extend_from_slice(&1_u16.to_be_bytes());
    payload.extend_from_slice(&0_u16.to_be_bytes());
    payload.extend_from_slice(&0_u16.to_be_bytes());
    // Speex sample entries keep an authored vendor code in the version-0 audio entry header.
    payload.extend_from_slice(SPEEX_VENDOR.as_slice());
    payload.extend_from_slice(&0_u16.to_be_bytes());
    payload.extend_from_slice(&16_u16.to_be_bytes());
    payload.extend_from_slice(&0_u16.to_be_bytes());
    payload.extend_from_slice(&0_u16.to_be_bytes());
    payload.extend_from_slice(&(sample_rate << 16).to_be_bytes());
    for child in child_boxes {
        payload.extend_from_slice(child);
    }
    super::super::mp4::encode_raw_box(SPEEX_ENTRY, &payload)
}

fn parse_speex_header(bytes: &[u8], spec: &str) -> Result<SpeexConfig, MuxError> {
    if bytes.len() < 80 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Speex header is truncated before the fixed 80-byte header".to_string(),
        });
    }
    if &bytes[..5] != b"Speex" {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Speex header did not start with the `Speex` signature".to_string(),
        });
    }
    // Speex stores `version_id` before `header_size`; real files often use `version_id = 1`,
    // so reading the wrong word here incorrectly rejects otherwise valid headers.
    let header_size = u32::from_le_bytes(bytes[32..36].try_into().unwrap());
    if header_size < 80 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("Ogg Speex header declared an unsupported header size {header_size}"),
        });
    }
    let sample_rate = u32::from_le_bytes(bytes[36..40].try_into().unwrap());
    let channel_count = u32::from_le_bytes(bytes[48..52].try_into().unwrap());
    let frame_size = u32::from_le_bytes(bytes[56..60].try_into().unwrap());
    let frames_per_packet = u32::from_le_bytes(bytes[64..68].try_into().unwrap());
    if sample_rate == 0 || channel_count == 0 || frame_size == 0 || frames_per_packet == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Speex header carried zero-valued core audio fields".to_string(),
        });
    }
    Ok(SpeexConfig { sample_rate })
}
