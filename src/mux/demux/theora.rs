use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::iso14496_12::{Pasp, SampleEntry, VisualSampleEntry};
use crate::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    ES_DESCRIPTOR_TAG, EsDescriptor, Esds, SL_CONFIG_DESCRIPTOR_TAG,
};

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

const THEORA_ENTRY: FourCc = FourCc::from_bytes(*b"mp4v");

pub(in crate::mux) struct ParsedOggTheoraTrack {
    pub(in crate::mux) segmented_source: SegmentedMuxSourceSpec,
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) timescale: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

struct TheoraConfig {
    width: u16,
    height: u16,
    timescale: u32,
    frame_duration: u32,
    sar_num: u32,
    sar_den: u32,
}

pub(in crate::mux) fn scan_ogg_theora_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedOggTheoraTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut packet_builders = BTreeMap::<u32, OggPacketBuilder>::new();
    let mut target_serial = None::<u32>;
    let mut header_packets = Vec::new();
    let mut config = None;
    let mut comment_seen = false;
    let mut setup_seen = false;
    let mut logical_size = 0_u64;
    let mut transformed_segments = Vec::new();
    let mut samples = Vec::new();

    while offset < file_size {
        let page = read_ogg_page_header_sync(&mut file, offset, spec)?;
        offset = page
            .payload_offset
            .checked_add(page.payload_size)
            .ok_or(MuxError::LayoutOverflow("Ogg page range"))?;
        if target_serial.is_some_and(|serial_no| serial_no != page.serial_no) {
            continue;
        }
        let packet_builder = packet_builders.entry(page.serial_no).or_default();
        if packet_builder.is_empty() && page.header_type & 0x01 != 0 {
            if target_serial == Some(page.serial_no) {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "Ogg Theora input started in the middle of a continued packet"
                        .to_string(),
                });
            }
            continue;
        }
        let mut page_cursor = page.payload_offset;
        let mut completed_packets = Vec::new();
        for lacing in &page.lacing_values {
            packet_builder.push_span(page_cursor, u32::from(*lacing))?;
            page_cursor += u64::from(*lacing);
            if *lacing == 255 {
                continue;
            }
            let packet = packet_builder.finish();
            if packet.total_size == 0 {
                continue;
            }
            completed_packets.push(packet);
        }
        for packet in completed_packets {
            let packet_bytes = read_spans_sync(
                &mut file,
                &packet.spans,
                packet.total_size,
                spec,
                "Ogg Theora packet is truncated",
            )?;
            if target_serial.is_none() {
                let Some(parsed_config) =
                    parse_theora_identification_header(&packet_bytes, spec).ok()
                else {
                    continue;
                };
                config = Some(parsed_config);
                target_serial = Some(page.serial_no);
                packet_builders.retain(|serial_no, _| *serial_no == page.serial_no);
                header_packets.push(packet_bytes);
                continue;
            }
            if !comment_seen {
                validate_theora_header_packet(&packet_bytes, 0x81, spec, "comment")?;
                comment_seen = true;
                header_packets.push(packet_bytes);
                continue;
            }
            if !setup_seen {
                validate_theora_header_packet(&packet_bytes, 0x82, spec, "setup")?;
                setup_seen = true;
                header_packets.push(packet_bytes);
                continue;
            }
            if packet_bytes[0] & 0x80 != 0 {
                continue;
            }
            let is_sync_sample = packet_bytes[0] & 0x40 == 0;
            let data_offset = logical_size;
            for span in &packet.spans {
                transformed_segments.push(SegmentedMuxSourceSegment {
                    logical_offset: logical_size,
                    data: SegmentedMuxSourceSegmentData::FileRange {
                        source_offset: span.source_offset,
                        size: span.size,
                    },
                });
                logical_size = logical_size
                    .checked_add(u64::from(span.size))
                    .ok_or(MuxError::LayoutOverflow("Ogg Theora logical source size"))?;
            }
            samples.push(StagedSample {
                data_offset,
                data_size: packet.total_size,
                duration: config.as_ref().unwrap().frame_duration,
                composition_time_offset: 0,
                is_sync_sample,
            });
        }
    }

    let mut fallback_packet_builder = OggPacketBuilder::default();
    let packet_builder = target_serial
        .and_then(|serial_no| packet_builders.get_mut(&serial_no))
        .unwrap_or(&mut fallback_packet_builder);

    finalize_theora_track(
        path,
        spec,
        packet_builder,
        config,
        header_packets,
        logical_size,
        transformed_segments,
        samples,
        setup_seen,
    )
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_ogg_theora_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedOggTheoraTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut packet_builders = BTreeMap::<u32, OggPacketBuilder>::new();
    let mut target_serial = None::<u32>;
    let mut header_packets = Vec::new();
    let mut config = None;
    let mut comment_seen = false;
    let mut setup_seen = false;
    let mut logical_size = 0_u64;
    let mut transformed_segments = Vec::new();
    let mut samples = Vec::new();

    while offset < file_size {
        let page = read_ogg_page_header_async(&mut file, offset, spec).await?;
        offset = page
            .payload_offset
            .checked_add(page.payload_size)
            .ok_or(MuxError::LayoutOverflow("Ogg page range"))?;
        if target_serial.is_some_and(|serial_no| serial_no != page.serial_no) {
            continue;
        }
        let packet_builder = packet_builders.entry(page.serial_no).or_default();
        if packet_builder.is_empty() && page.header_type & 0x01 != 0 {
            if target_serial == Some(page.serial_no) {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "Ogg Theora input started in the middle of a continued packet"
                        .to_string(),
                });
            }
            continue;
        }
        let mut page_cursor = page.payload_offset;
        let mut completed_packets = Vec::new();
        for lacing in &page.lacing_values {
            packet_builder.push_span(page_cursor, u32::from(*lacing))?;
            page_cursor += u64::from(*lacing);
            if *lacing == 255 {
                continue;
            }
            let packet = packet_builder.finish();
            if packet.total_size == 0 {
                continue;
            }
            completed_packets.push(packet);
        }
        for packet in completed_packets {
            let packet_bytes: Vec<u8> = read_spans_async(
                &mut file,
                &packet.spans,
                packet.total_size,
                spec,
                "Ogg Theora packet is truncated",
            )
            .await?;
            if target_serial.is_none() {
                let Some(parsed_config) =
                    parse_theora_identification_header(&packet_bytes, spec).ok()
                else {
                    continue;
                };
                config = Some(parsed_config);
                target_serial = Some(page.serial_no);
                packet_builders.retain(|serial_no, _| *serial_no == page.serial_no);
                header_packets.push(packet_bytes);
                continue;
            }
            if !comment_seen {
                validate_theora_header_packet(&packet_bytes, 0x81, spec, "comment")?;
                comment_seen = true;
                header_packets.push(packet_bytes);
                continue;
            }
            if !setup_seen {
                validate_theora_header_packet(&packet_bytes, 0x82, spec, "setup")?;
                setup_seen = true;
                header_packets.push(packet_bytes);
                continue;
            }
            if packet_bytes[0] & 0x80 != 0 {
                continue;
            }
            let is_sync_sample = packet_bytes[0] & 0x40 == 0;
            let data_offset = logical_size;
            for span in &packet.spans {
                transformed_segments.push(SegmentedMuxSourceSegment {
                    logical_offset: logical_size,
                    data: SegmentedMuxSourceSegmentData::FileRange {
                        source_offset: span.source_offset,
                        size: span.size,
                    },
                });
                logical_size = logical_size
                    .checked_add(u64::from(span.size))
                    .ok_or(MuxError::LayoutOverflow("Ogg Theora logical source size"))?;
            }
            samples.push(StagedSample {
                data_offset,
                data_size: packet.total_size,
                duration: config.as_ref().unwrap().frame_duration,
                composition_time_offset: 0,
                is_sync_sample,
            });
        }
    }

    let mut fallback_packet_builder = OggPacketBuilder::default();
    let packet_builder = target_serial
        .and_then(|serial_no| packet_builders.get_mut(&serial_no))
        .unwrap_or(&mut fallback_packet_builder);

    finalize_theora_track(
        path,
        spec,
        packet_builder,
        config,
        header_packets,
        logical_size,
        transformed_segments,
        samples,
        setup_seen,
    )
}

#[allow(clippy::too_many_arguments)]
fn finalize_theora_track(
    path: &Path,
    spec: &str,
    packet_builder: &mut OggPacketBuilder,
    config: Option<TheoraConfig>,
    header_packets: Vec<Vec<u8>>,
    logical_size: u64,
    transformed_segments: Vec<SegmentedMuxSourceSegment>,
    samples: Vec<StagedSample>,
    setup_seen: bool,
) -> Result<ParsedOggTheoraTrack, MuxError> {
    if !packet_builder.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Theora input ended in the middle of a packet".to_string(),
        });
    }
    let config = config.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "Ogg Theora input did not contain an identification header".to_string(),
    })?;
    if !setup_seen || header_packets.len() != 3 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Theora input did not contain the full three-header setup".to_string(),
        });
    }
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Theora input did not contain any frame packets after headers".to_string(),
        });
    }

    let mut dsi = Vec::new();
    for packet in &header_packets {
        let packet_len = u16::try_from(packet.len())
            .map_err(|_| MuxError::LayoutOverflow("Theora header packet length"))?;
        dsi.extend_from_slice(&packet_len.to_be_bytes());
        dsi.extend_from_slice(packet);
    }

    Ok(ParsedOggTheoraTrack {
        segmented_source: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: transformed_segments,
            total_size: logical_size,
        },
        width: config.width,
        height: config.height,
        timescale: config.timescale,
        sample_entry_box: build_theora_sample_entry_box(
            &config,
            &dsi,
            build_btrt_from_sample_sizes(
                samples
                    .iter()
                    .map(|sample| (sample.data_size, sample.duration)),
                config.timescale,
            )?,
        )?,
        samples,
    })
}

fn parse_theora_identification_header(bytes: &[u8], spec: &str) -> Result<TheoraConfig, MuxError> {
    validate_theora_header_packet(bytes, 0x80, spec, "identification")?;
    if bytes.len() < 42 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Theora identification header is truncated".to_string(),
        });
    }
    let width = u16::from_be_bytes(bytes[10..12].try_into().unwrap()) << 4;
    let height = u16::from_be_bytes(bytes[12..14].try_into().unwrap()) << 4;
    let timescale = u32::from_be_bytes(bytes[22..26].try_into().unwrap());
    let frame_duration = u32::from_be_bytes(bytes[26..30].try_into().unwrap());
    let sar_num = (u32::from(bytes[30]) << 16) | (u32::from(bytes[31]) << 8) | u32::from(bytes[32]);
    let sar_den = (u32::from(bytes[33]) << 16) | (u32::from(bytes[34]) << 8) | u32::from(bytes[35]);
    if width == 0 || height == 0 || timescale == 0 || frame_duration == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Theora identification header carried zero-valued core fields".to_string(),
        });
    }
    Ok(TheoraConfig {
        width,
        height,
        timescale,
        frame_duration,
        sar_num,
        sar_den,
    })
}

fn validate_theora_header_packet(
    packet: &[u8],
    expected_type: u8,
    spec: &str,
    name: &str,
) -> Result<(), MuxError> {
    if packet.len() < 7 || packet[0] != expected_type || &packet[1..7] != b"theora" {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("Ogg Theora {name} header was missing the expected Theora signature"),
        });
    }
    Ok(())
}

fn build_theora_sample_entry_box(
    config: &TheoraConfig,
    decoder_specific_info: &[u8],
    decoder_bitrates: crate::boxes::iso14496_12::Btrt,
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
                object_type_indication: 0xDF,
                stream_type: 4,
                reserved: true,
                buffer_size_db: decoder_bitrates.buffer_size_db,
                max_bitrate: decoder_bitrates.max_bitrate,
                avg_bitrate: decoder_bitrates.avg_bitrate,
                ..DecoderConfigDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_SPECIFIC_INFO_TAG,
            size: u32::try_from(decoder_specific_info.len())
                .map_err(|_| MuxError::LayoutOverflow("Theora decoder config size"))?,
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
        .map_err(|_| MuxError::LayoutOverflow("Theora esds"))?;
    let mut child_boxes = vec![super::super::mp4::encode_typed_box(&esds, &[])?];
    if config.sar_num != 0 && config.sar_den != 0 {
        child_boxes.push(super::super::mp4::encode_typed_box(
            &Pasp {
                h_spacing: config.sar_num,
                v_spacing: config.sar_den,
            },
            &[],
        )?);
    }
    super::super::mp4::encode_typed_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: THEORA_ENTRY,
                data_reference_index: 1,
            },
            width: config.width,
            height: config.height,
            horizresolution: 72_u32 << 16,
            vertresolution: 72_u32 << 16,
            frame_count: 1,
            depth: 0x0018,
            pre_defined3: -1,
            ..VisualSampleEntry::default()
        },
        &child_boxes.concat(),
    )
}
