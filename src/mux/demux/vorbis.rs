use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    ES_DESCRIPTOR_TAG, EsDescriptor, Esds, SL_CONFIG_DESCRIPTOR_TAG,
};

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

const VORBIS_ENTRY: FourCc = FourCc::from_bytes(*b"mp4a");

pub(in crate::mux) struct ParsedOggVorbisTrack {
    pub(in crate::mux) segmented_source: SegmentedMuxSourceSpec,
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

struct VorbisParser {
    channels: u16,
    sample_rate: u32,
    min_block: u32,
    max_block: u32,
    mode_bits: u8,
    mode_flags: [bool; 64],
    saw_identification: bool,
}

impl Default for VorbisParser {
    fn default() -> Self {
        Self {
            channels: 0,
            sample_rate: 0,
            min_block: 0,
            max_block: 0,
            mode_bits: 0,
            mode_flags: [false; 64],
            saw_identification: false,
        }
    }
}

struct CompletedPacketPageState {
    packets: Vec<super::ogg_common::CompletedOggPacket>,
    granule_position: u64,
    eos: bool,
}

pub(in crate::mux) fn scan_ogg_vorbis_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedOggVorbisTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut packet_builder = OggPacketBuilder::default();
    let mut parser = VorbisParser::default();
    let mut setup_seen = false;
    let mut comment_seen = false;
    let mut header_packets = Vec::new();
    let mut decoded_samples = 0_u64;
    let mut logical_size = 0_u64;
    let mut transformed_segments = Vec::new();
    let mut samples = Vec::new();

    while offset < file_size {
        let page = read_ogg_page_header_sync(&mut file, offset, spec)?;
        if packet_builder.is_empty() && page.header_type & 0x01 != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Vorbis input started in the middle of a continued packet".to_string(),
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
        process_vorbis_completed_page_sync(
            &mut file,
            spec,
            &mut parser,
            &mut header_packets,
            &mut comment_seen,
            &mut setup_seen,
            &mut decoded_samples,
            &mut logical_size,
            &mut transformed_segments,
            &mut samples,
            CompletedPacketPageState {
                packets: completed,
                granule_position: page.granule_position,
                eos: page.header_type & 0x04 != 0,
            },
        )?;
    }

    finalize_vorbis_track(
        path,
        spec,
        &parser,
        &mut packet_builder,
        header_packets,
        logical_size,
        transformed_segments,
        samples,
        setup_seen,
    )
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_ogg_vorbis_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedOggVorbisTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut packet_builder = OggPacketBuilder::default();
    let mut parser = VorbisParser::default();
    let mut setup_seen = false;
    let mut comment_seen = false;
    let mut header_packets = Vec::new();
    let mut decoded_samples = 0_u64;
    let mut logical_size = 0_u64;
    let mut transformed_segments = Vec::new();
    let mut samples = Vec::new();

    while offset < file_size {
        let page = read_ogg_page_header_async(&mut file, offset, spec).await?;
        if packet_builder.is_empty() && page.header_type & 0x01 != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Vorbis input started in the middle of a continued packet".to_string(),
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
        process_vorbis_completed_page_async(
            &mut file,
            spec,
            &mut parser,
            &mut header_packets,
            &mut comment_seen,
            &mut setup_seen,
            &mut decoded_samples,
            &mut logical_size,
            &mut transformed_segments,
            &mut samples,
            CompletedPacketPageState {
                packets: completed,
                granule_position: page.granule_position,
                eos: page.header_type & 0x04 != 0,
            },
        )
        .await?;
    }

    finalize_vorbis_track(
        path,
        spec,
        &parser,
        &mut packet_builder,
        header_packets,
        logical_size,
        transformed_segments,
        samples,
        setup_seen,
    )
}

#[allow(clippy::too_many_arguments)]
fn finalize_vorbis_track(
    path: &Path,
    spec: &str,
    parser: &VorbisParser,
    packet_builder: &mut OggPacketBuilder,
    header_packets: Vec<Vec<u8>>,
    logical_size: u64,
    transformed_segments: Vec<SegmentedMuxSourceSegment>,
    samples: Vec<StagedSample>,
    setup_seen: bool,
) -> Result<ParsedOggVorbisTrack, MuxError> {
    if !packet_builder.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Vorbis input ended in the middle of a packet".to_string(),
        });
    }
    if !parser.saw_identification || !setup_seen || header_packets.len() != 3 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Vorbis input did not contain the full three-header setup".to_string(),
        });
    }
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg Vorbis input did not contain any audio packets after headers".to_string(),
        });
    }

    let mut dsi = Vec::new();
    for packet in &header_packets {
        let packet_len = u16::try_from(packet.len())
            .map_err(|_| MuxError::LayoutOverflow("Vorbis header packet length"))?;
        dsi.extend_from_slice(&packet_len.to_be_bytes());
        dsi.extend_from_slice(packet);
    }

    Ok(ParsedOggVorbisTrack {
        segmented_source: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: transformed_segments,
            total_size: logical_size,
        },
        sample_rate: parser.sample_rate,
        sample_entry_box: build_vorbis_sample_entry_box(
            parser.sample_rate,
            parser.channels,
            &dsi,
            build_btrt_from_sample_sizes(
                samples
                    .iter()
                    .map(|sample| (sample.data_size, sample.duration)),
                parser.sample_rate,
            )?,
        )?,
        samples,
    })
}

#[allow(clippy::too_many_arguments)]
fn process_vorbis_completed_page_sync(
    file: &mut File,
    spec: &str,
    parser: &mut VorbisParser,
    header_packets: &mut Vec<Vec<u8>>,
    comment_seen: &mut bool,
    setup_seen: &mut bool,
    decoded_samples: &mut u64,
    logical_size: &mut u64,
    transformed_segments: &mut Vec<SegmentedMuxSourceSegment>,
    samples: &mut Vec<StagedSample>,
    page: CompletedPacketPageState,
) -> Result<(), MuxError> {
    let mut audio_packets = Vec::new();
    for packet in page.packets {
        let packet_bytes = read_spans_sync(
            file,
            &packet.spans,
            packet.total_size,
            spec,
            "Ogg Vorbis packet is truncated",
        )?;
        if !parser.saw_identification {
            parser.parse_identification_header(&packet_bytes, spec)?;
            header_packets.push(packet_bytes);
            continue;
        }
        if !*comment_seen {
            validate_vorbis_header_packet(&packet_bytes, 0x03, spec, "comment")?;
            *comment_seen = true;
            header_packets.push(packet_bytes);
            continue;
        }
        if !*setup_seen {
            parser.parse_setup_header(&packet_bytes, spec)?;
            *setup_seen = true;
            header_packets.push(packet_bytes);
            continue;
        }
        audio_packets.push((packet, packet_bytes));
    }
    append_vorbis_audio_packets(
        spec,
        parser,
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
async fn process_vorbis_completed_page_async(
    file: &mut TokioFile,
    spec: &str,
    parser: &mut VorbisParser,
    header_packets: &mut Vec<Vec<u8>>,
    comment_seen: &mut bool,
    setup_seen: &mut bool,
    decoded_samples: &mut u64,
    logical_size: &mut u64,
    transformed_segments: &mut Vec<SegmentedMuxSourceSegment>,
    samples: &mut Vec<StagedSample>,
    page: CompletedPacketPageState,
) -> Result<(), MuxError> {
    let mut audio_packets = Vec::new();
    for packet in page.packets {
        let packet_bytes: Vec<u8> = read_spans_async(
            file,
            &packet.spans,
            packet.total_size,
            spec,
            "Ogg Vorbis packet is truncated",
        )
        .await?;
        if !parser.saw_identification {
            parser.parse_identification_header(&packet_bytes, spec)?;
            header_packets.push(packet_bytes);
            continue;
        }
        if !*comment_seen {
            validate_vorbis_header_packet(&packet_bytes, 0x03, spec, "comment")?;
            *comment_seen = true;
            header_packets.push(packet_bytes);
            continue;
        }
        if !*setup_seen {
            parser.parse_setup_header(&packet_bytes, spec)?;
            *setup_seen = true;
            header_packets.push(packet_bytes);
            continue;
        }
        audio_packets.push((packet, packet_bytes));
    }
    append_vorbis_audio_packets(
        spec,
        parser,
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
fn append_vorbis_audio_packets(
    spec: &str,
    parser: &VorbisParser,
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
        nominal_durations.push(u64::from(parser.packet_duration(packet_bytes, spec)?));
    }
    let mut prior_page_duration = 0_u64;
    let last_index = audio_packets.len().saturating_sub(1);
    for (index, (packet, _packet_bytes)) in audio_packets.into_iter().enumerate() {
        let mut duration = nominal_durations[index];
        if eos && index == last_index && granule_position != u64::MAX {
            let remaining = granule_position
                .saturating_sub(*decoded_samples)
                .saturating_sub(prior_page_duration);
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
                .ok_or(MuxError::LayoutOverflow("Ogg Vorbis logical source size"))?;
        }
        samples.push(StagedSample {
            data_offset,
            data_size: packet.total_size,
            duration: u32::try_from(duration)
                .map_err(|_| MuxError::LayoutOverflow("Ogg Vorbis packet duration"))?,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        *decoded_samples = decoded_samples
            .checked_add(duration)
            .ok_or(MuxError::LayoutOverflow("Ogg Vorbis decoded sample count"))?;
        prior_page_duration = prior_page_duration
            .checked_add(nominal_durations[index])
            .ok_or(MuxError::LayoutOverflow("Ogg Vorbis page duration"))?;
    }
    Ok(())
}

fn build_vorbis_sample_entry_box(
    sample_rate: u32,
    channel_count: u16,
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
                object_type_indication: 0xDD,
                stream_type: 5,
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
                .map_err(|_| MuxError::LayoutOverflow("Vorbis decoder config size"))?,
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
        .map_err(|_| MuxError::LayoutOverflow("Vorbis esds"))?;
    let esds_box = super::super::mp4::encode_typed_box(&esds, &[])?;
    build_generic_audio_sample_entry_box(VORBIS_ENTRY, sample_rate, channel_count, 16, &[esds_box])
}

fn validate_vorbis_header_packet(
    packet: &[u8],
    expected_type: u8,
    spec: &str,
    name: &str,
) -> Result<(), MuxError> {
    if packet.len() < 7 || packet[0] != expected_type || &packet[1..7] != b"vorbis" {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("Ogg Vorbis {name} header was missing the expected Vorbis signature"),
        });
    }
    Ok(())
}

impl VorbisParser {
    fn parse_identification_header(&mut self, packet: &[u8], spec: &str) -> Result<(), MuxError> {
        validate_vorbis_header_packet(packet, 0x01, spec, "identification")?;
        if packet.len() < 30 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Vorbis identification header is truncated".to_string(),
            });
        }
        let version = u32::from_le_bytes(packet[7..11].try_into().unwrap());
        if version != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "Ogg Vorbis identification header used unsupported version {version}"
                ),
            });
        }
        let channels = packet[11];
        let sample_rate = u32::from_le_bytes(packet[12..16].try_into().unwrap());
        let min_block_exp = packet[28] & 0x0F;
        let max_block_exp = packet[28] >> 4;
        let framing_flag = packet[29];
        let min_block = 1_u32 << u32::from(min_block_exp);
        let max_block = 1_u32 << u32::from(max_block_exp);
        if channels == 0
            || sample_rate == 0
            || min_block < 8
            || max_block < min_block
            || framing_flag != 1
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Vorbis identification header carried invalid core fields".to_string(),
            });
        }
        self.channels = u16::from(channels);
        self.sample_rate = sample_rate;
        self.min_block = min_block;
        self.max_block = max_block;
        self.saw_identification = true;
        Ok(())
    }

    fn parse_setup_header(&mut self, packet: &[u8], spec: &str) -> Result<(), MuxError> {
        validate_vorbis_header_packet(packet, 0x05, spec, "setup")?;
        if !self.saw_identification {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Vorbis setup header appeared before identification".to_string(),
            });
        }
        let mut reader = LsbBitReader::new(packet);
        let packet_type = reader
            .read(8)
            .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
        let _ = packet_type;
        for _ in 0..6 {
            reader
                .read(8)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
        }
        let codebook_count = reader
            .read(8)
            .ok_or_else(|| truncated_vorbis_setup_error(spec))?
            + 1;
        for _ in 0..codebook_count {
            reader
                .read(24)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            let dimensions = reader
                .read(16)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            let entries = reader
                .read(24)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            if reader
                .read(1)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                == 0
            {
                if reader
                    .read(1)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                    != 0
                {
                    for _ in 0..entries {
                        if reader
                            .read(1)
                            .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                            != 0
                        {
                            reader
                                .read(5)
                                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                        }
                    }
                } else {
                    for _ in 0..entries {
                        reader
                            .read(5)
                            .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                    }
                }
            } else {
                reader
                    .read(5)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                let mut index = 0_u32;
                while index < entries {
                    let bits = ilog(entries.saturating_sub(index), false);
                    let count = reader
                        .read(bits)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                    index = index.saturating_add(count);
                }
            }

            let map_type = reader
                .read(4)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            match map_type {
                0 => {}
                1 | 2 => {
                    reader
                        .read(32)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                    reader
                        .read(32)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                    let quant_bits = reader
                        .read(4)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                        + 1;
                    reader
                        .read(1)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                    let quant_value_count = match map_type {
                        1 => vorbis_book_maptype1_quantvals(entries, dimensions),
                        2 => entries.saturating_mul(dimensions),
                        _ => 0,
                    };
                    for _ in 0..quant_value_count {
                        reader
                            .read(quant_bits)
                            .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                    }
                }
                _ => {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: "Ogg Vorbis setup header used an unsupported codebook map type"
                            .to_string(),
                    });
                }
            }
        }

        let time_count = reader
            .read(6)
            .ok_or_else(|| truncated_vorbis_setup_error(spec))?
            + 1;
        for _ in 0..time_count {
            reader
                .read(16)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
        }

        let floor_count = reader
            .read(6)
            .ok_or_else(|| truncated_vorbis_setup_error(spec))?
            + 1;
        for _ in 0..floor_count {
            let floor_type = reader
                .read(16)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            if floor_type != 0 {
                let partition_count = reader
                    .read(5)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                let mut partitions = Vec::with_capacity(partition_count as usize);
                let mut max_class = 0_u32;
                for _ in 0..partition_count {
                    let partition = reader
                        .read(4)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                    max_class = max_class.max(partition);
                    partitions.push(partition);
                }
                let mut class_dimensions = vec![0_u32; usize::try_from(max_class + 1).unwrap()];
                for class in &mut class_dimensions {
                    *class = reader
                        .read(3)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                        + 1;
                    let subclass_bits = reader
                        .read(2)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                    if subclass_bits != 0 {
                        reader
                            .read(8)
                            .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                    }
                    for _ in 0..(1_u32 << subclass_bits) {
                        reader
                            .read(8)
                            .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                    }
                }
                reader
                    .read(2)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                let range_bits = reader
                    .read(4)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                let mut count = 0_u32;
                let mut value_index = 0_u32;
                for partition in partitions {
                    count =
                        count.saturating_add(class_dimensions[usize::try_from(partition).unwrap()]);
                    while value_index < count {
                        reader
                            .read(range_bits)
                            .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                        value_index += 1;
                    }
                }
            } else {
                reader
                    .read(8)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                reader
                    .read(16)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                reader
                    .read(16)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                reader
                    .read(6)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                reader
                    .read(8)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                let book_count = reader
                    .read(4)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                    + 1;
                for _ in 0..book_count {
                    reader
                        .read(8)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                }
            }
        }

        let residue_count = reader
            .read(6)
            .ok_or_else(|| truncated_vorbis_setup_error(spec))?
            + 1;
        for _ in 0..residue_count {
            reader
                .read(16)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            reader
                .read(24)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            reader
                .read(24)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            reader
                .read(24)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            let partition_count = reader
                .read(6)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                + 1;
            reader
                .read(8)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            let mut cascade_count = 0_u32;
            for _ in 0..partition_count {
                let mut cascade = reader
                    .read(3)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                if reader
                    .read(1)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                    != 0
                {
                    cascade |= reader
                        .read(5)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                        << 3;
                }
                cascade_count = cascade_count.saturating_add(icount(cascade));
            }
            for _ in 0..cascade_count {
                reader
                    .read(8)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            }
        }

        let mapping_count = reader
            .read(6)
            .ok_or_else(|| truncated_vorbis_setup_error(spec))?
            + 1;
        for _ in 0..mapping_count {
            let mut sub_maps = 1_u32;
            reader
                .read(16)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            if reader
                .read(1)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                != 0
            {
                sub_maps = reader
                    .read(4)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                    + 1;
            }
            if reader
                .read(1)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                != 0
            {
                let coupling_steps = reader
                    .read(8)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                    + 1;
                let channel_bits = ilog(u32::from(self.channels), true);
                for _ in 0..coupling_steps {
                    reader
                        .read(channel_bits)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                    reader
                        .read(channel_bits)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                }
            }
            reader
                .read(2)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            if sub_maps > 1 {
                for _ in 0..self.channels {
                    reader
                        .read(4)
                        .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                }
            }
            for _ in 0..sub_maps {
                reader
                    .read(8)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                reader
                    .read(8)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
                reader
                    .read(8)
                    .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            }
        }

        let mode_count = reader
            .read(6)
            .ok_or_else(|| truncated_vorbis_setup_error(spec))?
            + 1;
        for mode_index in 0..mode_count {
            self.mode_flags[usize::try_from(mode_index).unwrap()] = reader
                .read(1)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?
                != 0;
            reader
                .read(16)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            reader
                .read(16)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
            reader
                .read(8)
                .ok_or_else(|| truncated_vorbis_setup_error(spec))?;
        }
        self.mode_bits = 0;
        let mut remaining_modes = mode_count;
        while remaining_modes > 1 {
            self.mode_bits = self.mode_bits.saturating_add(1);
            remaining_modes >>= 1;
        }
        Ok(())
    }

    fn packet_duration(&self, packet: &[u8], spec: &str) -> Result<u32, MuxError> {
        if self.mode_bits == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Vorbis setup did not expose any audio modes".to_string(),
            });
        }
        let mut reader = LsbBitReader::new(packet);
        let packet_type = reader
            .read(1)
            .ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Vorbis audio packet is truncated".to_string(),
            })?;
        if packet_type != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Vorbis packet was not an audio packet".to_string(),
            });
        }
        let mode = reader.read(u32::from(self.mode_bits)).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg Vorbis audio packet is truncated before the mode index".to_string(),
            }
        })?;
        let mode_index = usize::try_from(mode).unwrap_or(usize::MAX);
        if mode_index >= self.mode_flags.len() {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("Ogg Vorbis audio packet used invalid mode index {mode}"),
            });
        }
        let block_size = if self.mode_flags[mode_index] {
            self.max_block
        } else {
            self.min_block
        };
        Ok(block_size / 2)
    }
}

fn truncated_vorbis_setup_error(spec: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "Ogg Vorbis setup header is truncated".to_string(),
    }
}

fn vorbis_book_maptype1_quantvals(entries: u32, dimensions: u32) -> u32 {
    if entries == 0 || dimensions == 0 {
        return 0;
    }
    let mut values = (entries as f64).powf(1.0 / f64::from(dimensions)) as u32;
    while values.saturating_pow(dimensions) > entries {
        values = values.saturating_sub(1);
    }
    while values.saturating_add(1).saturating_pow(dimensions) <= entries {
        values = values.saturating_add(1);
    }
    values
}

fn icount(mut value: u32) -> u32 {
    let mut count = 0_u32;
    while value != 0 {
        count += value & 1;
        value >>= 1;
    }
    count
}

fn ilog(mut value: u32, allow_zero: bool) -> u32 {
    if value == 0 {
        return u32::from(!allow_zero);
    }
    let mut bits = 0_u32;
    while value != 0 {
        bits += 1;
        value >>= 1;
    }
    bits
}

struct LsbBitReader<'a> {
    data: &'a [u8],
    bit_offset: usize,
}

impl<'a> LsbBitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_offset: 0,
        }
    }

    fn read(&mut self, bits: u32) -> Option<u32> {
        if bits == 0 {
            return Some(0);
        }
        let mut value = 0_u32;
        for bit_index in 0..bits {
            let byte_index = self.bit_offset / 8;
            if byte_index >= self.data.len() {
                return None;
            }
            let bit_index_in_byte = self.bit_offset % 8;
            let bit = (self.data[byte_index] >> bit_index_in_byte) & 1;
            value |= u32::from(bit) << bit_index;
            self.bit_offset += 1;
        }
        Some(value)
    }
}
