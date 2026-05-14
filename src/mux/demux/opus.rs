use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::iso14496_12::Btrt;
use crate::boxes::opus::DOps;

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_spans_async;
use super::super::import::{
    SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData, SegmentedMuxSourceSpec, StagedSample,
    build_btrt_from_sample_sizes, build_generic_audio_sample_entry_box, read_spans_sync,
};
#[cfg(feature = "async")]
use super::ogg_common::read_ogg_page_header_async;
use super::ogg_common::{OggPacketBuilder, read_ogg_page_header_sync};

const OPUS_ENTRY: FourCc = FourCc::from_bytes(*b"Opus");
const OPUS_FRAME_DURATION_TABLE_48K: [u32; 32] = [
    480, 960, 1920, 2880, 480, 960, 1920, 2880, 480, 960, 1920, 2880, 480, 960, 480, 960, 120, 240,
    480, 960, 120, 240, 480, 960, 120, 240, 480, 960, 120, 240, 480, 960,
];

pub(in crate::mux) struct ParsedOggOpusTrack {
    pub(in crate::mux) segmented_source: SegmentedMuxSourceSpec,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) edit_media_time: Option<u64>,
    pub(in crate::mux) sample_roll_distance: Option<i16>,
    pub(in crate::mux) flat_source_encoding_metadata: Option<String>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

struct CompletedOpusPageState {
    packets: Vec<super::ogg_common::CompletedOggPacket>,
    granule_position: u64,
    eos: bool,
}

pub(in crate::mux) fn scan_ogg_opus_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedOggOpusTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut packet_builder = OggPacketBuilder::default();
    let mut config = None;
    let mut saw_tags_packet = false;
    let mut flat_source_encoding_metadata = None;
    let mut logical_size = 0_u64;
    let mut transformed_segments = Vec::new();
    let mut samples = Vec::new();
    let mut decoded_samples = 0_u64;
    while offset < file_size {
        let page = read_ogg_page_header_sync(&mut file, offset, spec)?;
        if packet_builder.is_empty() && page.header_type & 0x01 != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Opus input started in the middle of a continued packet".to_string(),
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
        process_opus_completed_page_sync(
            &mut file,
            spec,
            &mut config,
            &mut saw_tags_packet,
            &mut flat_source_encoding_metadata,
            &mut logical_size,
            &mut transformed_segments,
            &mut samples,
            &mut decoded_samples,
            CompletedOpusPageState {
                packets: completed,
                granule_position: page.granule_position,
                eos: page.header_type & 0x04 != 0,
            },
        )?;
    }
    if !packet_builder.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Opus input ended in the middle of a packet".to_string(),
        });
    }
    let config = config.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "Ogg Opus input did not contain an Opus identification packet".to_string(),
    })?;
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Opus input did not contain any audio packets after headers".to_string(),
        });
    }
    let btrt = build_btrt_from_sample_sizes(
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        48_000,
    )?;
    Ok(ParsedOggOpusTrack {
        segmented_source: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: transformed_segments,
            total_size: logical_size,
        },
        sample_entry_box: build_opus_sample_entry_box(&config, btrt)?,
        edit_media_time: (config.pre_skip != 0).then_some(u64::from(config.pre_skip)),
        sample_roll_distance: Some(3_840),
        flat_source_encoding_metadata,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_ogg_opus_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedOggOpusTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut packet_builder = OggPacketBuilder::default();
    let mut config = None;
    let mut saw_tags_packet = false;
    let mut flat_source_encoding_metadata = None;
    let mut logical_size = 0_u64;
    let mut transformed_segments = Vec::new();
    let mut samples = Vec::new();
    let mut decoded_samples = 0_u64;
    while offset < file_size {
        let page = read_ogg_page_header_async(&mut file, offset, spec).await?;
        if packet_builder.is_empty() && page.header_type & 0x01 != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Opus input started in the middle of a continued packet".to_string(),
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
        process_opus_completed_page_async(
            &mut file,
            spec,
            &mut config,
            &mut saw_tags_packet,
            &mut flat_source_encoding_metadata,
            &mut logical_size,
            &mut transformed_segments,
            &mut samples,
            &mut decoded_samples,
            CompletedOpusPageState {
                packets: completed,
                granule_position: page.granule_position,
                eos: page.header_type & 0x04 != 0,
            },
        )
        .await?;
    }
    if !packet_builder.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Opus input ended in the middle of a packet".to_string(),
        });
    }
    let config = config.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "Ogg Opus input did not contain an Opus identification packet".to_string(),
    })?;
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Opus input did not contain any audio packets after headers".to_string(),
        });
    }
    let btrt = build_btrt_from_sample_sizes(
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        48_000,
    )?;
    Ok(ParsedOggOpusTrack {
        segmented_source: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: transformed_segments,
            total_size: logical_size,
        },
        sample_entry_box: build_opus_sample_entry_box(&config, btrt)?,
        edit_media_time: (config.pre_skip != 0).then_some(u64::from(config.pre_skip)),
        sample_roll_distance: Some(3_840),
        flat_source_encoding_metadata,
        samples,
    })
}

#[allow(clippy::too_many_arguments)]
fn process_opus_completed_page_sync(
    file: &mut File,
    spec: &str,
    config: &mut Option<DOps>,
    saw_tags_packet: &mut bool,
    flat_source_encoding_metadata: &mut Option<String>,
    logical_size: &mut u64,
    transformed_segments: &mut Vec<SegmentedMuxSourceSegment>,
    samples: &mut Vec<StagedSample>,
    decoded_samples: &mut u64,
    page: CompletedOpusPageState,
) -> Result<(), MuxError> {
    let mut audio_packets = Vec::new();
    for packet in page.packets {
        let packet_bytes = read_spans_sync(
            file,
            &packet.spans,
            packet.total_size,
            spec,
            "Ogg Opus packet is truncated",
        )?;
        if config.is_none() {
            *config = Some(parse_opus_head_packet(&packet_bytes, spec)?);
            continue;
        }
        if !*saw_tags_packet && packet_bytes.starts_with(b"OpusTags") {
            *saw_tags_packet = true;
            if flat_source_encoding_metadata.is_none() {
                *flat_source_encoding_metadata = parse_opus_tags_encoding_metadata(&packet_bytes);
            }
            continue;
        }
        *saw_tags_packet = true;
        audio_packets.push((packet, packet_bytes));
    }
    append_opus_audio_packets(
        spec,
        decoded_samples,
        logical_size,
        transformed_segments,
        samples,
        audio_packets,
        page.granule_position,
        page.eos,
    )
}

#[cfg(feature = "async")]
#[allow(clippy::too_many_arguments)]
async fn process_opus_completed_page_async(
    file: &mut TokioFile,
    spec: &str,
    config: &mut Option<DOps>,
    saw_tags_packet: &mut bool,
    flat_source_encoding_metadata: &mut Option<String>,
    logical_size: &mut u64,
    transformed_segments: &mut Vec<SegmentedMuxSourceSegment>,
    samples: &mut Vec<StagedSample>,
    decoded_samples: &mut u64,
    page: CompletedOpusPageState,
) -> Result<(), MuxError> {
    let mut audio_packets = Vec::new();
    for packet in page.packets {
        let packet_bytes: Vec<u8> = read_spans_async(
            file,
            &packet.spans,
            packet.total_size,
            spec,
            "Ogg Opus packet is truncated",
        )
        .await?;
        if config.is_none() {
            *config = Some(parse_opus_head_packet(&packet_bytes, spec)?);
            continue;
        }
        if !*saw_tags_packet && packet_bytes.starts_with(b"OpusTags") {
            *saw_tags_packet = true;
            if flat_source_encoding_metadata.is_none() {
                *flat_source_encoding_metadata = parse_opus_tags_encoding_metadata(&packet_bytes);
            }
            continue;
        }
        *saw_tags_packet = true;
        audio_packets.push((packet, packet_bytes));
    }
    append_opus_audio_packets(
        spec,
        decoded_samples,
        logical_size,
        transformed_segments,
        samples,
        audio_packets,
        page.granule_position,
        page.eos,
    )
}

#[allow(clippy::too_many_arguments)]
fn append_opus_audio_packets(
    spec: &str,
    decoded_samples: &mut u64,
    logical_size: &mut u64,
    transformed_segments: &mut Vec<SegmentedMuxSourceSegment>,
    samples: &mut Vec<StagedSample>,
    audio_packets: Vec<(super::ogg_common::CompletedOggPacket, Vec<u8>)>,
    granule_position: u64,
    eos: bool,
) -> Result<(), MuxError> {
    let mut nominal_durations = Vec::with_capacity(audio_packets.len());
    for (_, packet_bytes) in &audio_packets {
        nominal_durations.push(u64::from(opus_packet_duration_from_bytes(
            packet_bytes,
            spec,
        )?));
    }
    let last_index = audio_packets.len().saturating_sub(1);
    for (index, (packet, packet_bytes)) in audio_packets.into_iter().enumerate() {
        let mut duration = nominal_durations[index];
        if eos && index == last_index && granule_position != u64::MAX {
            let remaining = granule_position.saturating_sub(*decoded_samples);
            if remaining < duration {
                duration = remaining;
            }
        }
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
                .ok_or(MuxError::LayoutOverflow("Ogg Opus logical source size"))?;
        }
        samples.push(StagedSample {
            data_offset,
            data_size: packet.total_size,
            duration: u32::try_from(duration)
                .map_err(|_| MuxError::LayoutOverflow("Ogg Opus packet duration"))?,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        *decoded_samples = decoded_samples
            .checked_add(duration)
            .ok_or(MuxError::LayoutOverflow("Ogg Opus decoded sample count"))?;
        let _ = packet_bytes;
    }
    Ok(())
}

fn build_opus_sample_entry_box(config: &DOps, btrt: Btrt) -> Result<Vec<u8>, MuxError> {
    let dops_bytes = super::super::mp4::encode_typed_box(config, &[])?;
    let btrt_bytes = super::super::mp4::encode_typed_box(&btrt, &[])?;
    build_generic_audio_sample_entry_box(
        OPUS_ENTRY,
        48_000,
        u16::from(config.output_channel_count),
        16,
        &[dops_bytes, btrt_bytes],
    )
}

fn parse_opus_tags_encoding_metadata(packet_bytes: &[u8]) -> Option<String> {
    if !packet_bytes.starts_with(b"OpusTags") || packet_bytes.len() < 12 {
        return None;
    }

    let vendor_len =
        usize::try_from(u32::from_le_bytes(packet_bytes[8..12].try_into().ok()?)).ok()?;
    let vendor_start = 12usize;
    let vendor_end = vendor_start.checked_add(vendor_len)?;
    let vendor = packet_bytes.get(vendor_start..vendor_end)?;
    let vendor = String::from_utf8_lossy(vendor).into_owned();

    let Some(comment_count_bytes) = packet_bytes.get(vendor_end..vendor_end.checked_add(4)?) else {
        return Some(vendor);
    };
    let comment_count =
        usize::try_from(u32::from_le_bytes(comment_count_bytes.try_into().ok()?)).ok()?;
    let mut cursor = vendor_end.checked_add(4)?;
    for _ in 0..comment_count {
        let Some(comment_len_bytes) = packet_bytes.get(cursor..cursor.checked_add(4)?) else {
            return Some(vendor);
        };
        let comment_len =
            usize::try_from(u32::from_le_bytes(comment_len_bytes.try_into().ok()?)).ok()?;
        cursor = cursor.checked_add(4)?;
        let comment_end = cursor.checked_add(comment_len)?;
        let Some(comment_bytes) = packet_bytes.get(cursor..comment_end) else {
            return Some(vendor);
        };
        let comment = String::from_utf8_lossy(comment_bytes);
        if let Some((key, value)) = comment.split_once('=')
            && key.eq_ignore_ascii_case("encoder")
            && !value.is_empty()
        {
            return Some(value.to_string());
        }
        cursor = comment_end;
    }

    Some(vendor)
}

fn parse_opus_head_packet(packet: &[u8], spec: &str) -> Result<DOps, MuxError> {
    if packet.len() < 19 || !packet.starts_with(b"OpusHead") {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Opus identification packet was missing the `OpusHead` signature"
                .to_string(),
        });
    }
    let version = packet[8];
    if version != 1 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported Opus identification packet version {version}"),
        });
    }
    let output_channel_count = packet[9];
    let pre_skip = u16::from_le_bytes([packet[10], packet[11]]);
    let input_sample_rate = u32::from_le_bytes([packet[12], packet[13], packet[14], packet[15]]);
    let output_gain = i16::from_le_bytes([packet[16], packet[17]]);
    let channel_mapping_family = packet[18];
    let (stream_count, coupled_count, channel_mapping) = if channel_mapping_family == 0 {
        (0, 0, Vec::new())
    } else {
        let required = 21usize
            .checked_add(usize::from(output_channel_count))
            .ok_or(MuxError::LayoutOverflow("Opus mapping header"))?;
        if packet.len() < required {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Opus identification packet is truncated before channel mapping data"
                    .to_string(),
            });
        }
        (packet[19], packet[20], packet[21..required].to_vec())
    };
    Ok(DOps {
        version: 0,
        output_channel_count,
        pre_skip,
        input_sample_rate,
        output_gain,
        channel_mapping_family,
        stream_count,
        coupled_count,
        channel_mapping,
    })
}

fn opus_packet_duration_from_bytes(packet: &[u8], spec: &str) -> Result<u32, MuxError> {
    let Some(&toc) = packet.first() else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Opus packet was empty".to_string(),
        });
    };
    let config = toc >> 3;
    let frame_duration = OPUS_FRAME_DURATION_TABLE_48K[usize::from(config)];
    let frame_count_code = toc & 0x03;
    let duration = match frame_count_code {
        0 => frame_duration,
        1 | 2 => frame_duration * 2,
        3 => {
            let Some(&frame_count_byte) = packet.get(1) else {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "Ogg Opus packet used code 3 framing without a frame-count byte"
                        .to_string(),
                });
            };
            frame_duration
                .checked_mul(u32::from(frame_count_byte & 0x3F))
                .ok_or(MuxError::LayoutOverflow("Opus duration"))?
        }
        _ => unreachable!(),
    };
    Ok(duration)
}

#[cfg(test)]
mod tests {
    use super::parse_opus_tags_encoding_metadata;

    #[test]
    fn parse_opus_tags_encoding_metadata_prefers_encoder_comment() {
        let comment = b"encoder=Lavc61.2.100 libopus";
        let mut packet = Vec::from(&b"OpusTags"[..]);
        packet.extend_from_slice(&12_u32.to_le_bytes());
        packet.extend_from_slice(b"Lavf61.0.100");
        packet.extend_from_slice(&1_u32.to_le_bytes());
        packet.extend_from_slice(&(comment.len() as u32).to_le_bytes());
        packet.extend_from_slice(comment);

        assert_eq!(
            parse_opus_tags_encoding_metadata(&packet).as_deref(),
            Some("Lavc61.2.100 libopus")
        );
    }

    #[test]
    fn parse_opus_tags_encoding_metadata_falls_back_to_vendor() {
        let mut packet = Vec::from(&b"OpusTags"[..]);
        packet.extend_from_slice(&5_u32.to_le_bytes());
        packet.extend_from_slice(b"Lavf1");
        packet.extend_from_slice(&0_u32.to_le_bytes());

        assert_eq!(
            parse_opus_tags_encoding_metadata(&packet).as_deref(),
            Some("Lavf1")
        );
    }

    #[test]
    fn parse_opus_tags_encoding_metadata_rejects_truncated_packet() {
        let mut packet = Vec::from(&b"OpusTags"[..]);
        packet.extend_from_slice(&10_u32.to_le_bytes());
        packet.extend_from_slice(b"short");

        assert_eq!(parse_opus_tags_encoding_metadata(&packet), None);
    }
}
