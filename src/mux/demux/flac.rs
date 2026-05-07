use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::flac::{DfLa, FlacMetadataBlock};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
#[cfg(feature = "async")]
use super::super::import::read_spans_async;
use super::super::import::{
    SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData, SegmentedMuxSourceSpec, StagedSample,
    build_btrt_from_sample_sizes, build_generic_audio_sample_entry_box, read_exact_at_sync,
    read_spans_sync,
};
#[cfg(feature = "async")]
use super::ogg_common::read_ogg_page_header_async;
use super::ogg_common::{OggPacketBuilder, read_ogg_page_header_sync};

const FLAC_ENTRY: FourCc = FourCc::from_bytes(*b"fLaC");
const FLAC_SCAN_CHUNK_SIZE: usize = 64 * 1024;
const FLAC_BLOCK_SIZE_TABLE: [u32; 16] = [
    0, 192, 576, 1_152, 2_304, 4_608, 0, 0, 256, 512, 1_024, 2_048, 4_096, 8_192, 16_384, 32_768,
];
const FLAC_SAMPLE_RATE_TABLE: [u32; 12] = [
    0, 88_200, 176_400, 192_000, 8_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 96_000,
];

pub(in crate::mux) struct ParsedFlacTrack {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

pub(in crate::mux) struct ParsedOggFlacTrack {
    pub(in crate::mux) segmented_source: SegmentedMuxSourceSpec,
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

#[derive(Clone)]
struct ParsedFlacMetadataBlock {
    block_type: u8,
    length: u32,
    block_data: Vec<u8>,
}

struct ParsedFlacStreamInfo {
    sample_rate: u32,
    channel_count: u16,
    bits_per_sample: u16,
    total_samples: u64,
}

struct ParsedFlacFrameHeader {
    block_size: u32,
}

struct OggFlacHeaderState {
    header_bytes: Vec<u8>,
    extra_header_packets_remaining: u16,
}

struct ParsedOggFlacHeaderPacket<'a> {
    native_header_bytes: &'a [u8],
    extra_header_packets_remaining: u16,
}

pub(in crate::mux) fn scan_flac_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedFlacTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    if file_size < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "FLAC input is truncated before the 4-byte stream marker".to_string(),
        });
    }

    let mut signature = [0_u8; 4];
    read_exact_at_sync(
        &mut file,
        0,
        &mut signature,
        spec,
        "FLAC input is truncated before the 4-byte stream marker",
    )?;
    if &signature != b"fLaC" {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "FLAC input did not start with the `fLaC` stream marker".to_string(),
        });
    }

    let mut offset = 4_u64;
    let mut metadata_blocks = Vec::new();
    let mut stream_info = None::<ParsedFlacStreamInfo>;
    loop {
        if file_size - offset < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "FLAC metadata block header is truncated".to_string(),
            });
        }
        let mut header = [0_u8; 4];
        read_exact_at_sync(
            &mut file,
            offset,
            &mut header,
            spec,
            "FLAC metadata block header is truncated",
        )?;
        let last_metadata_block_flag = header[0] & 0x80 != 0;
        let block_type = header[0] & 0x7F;
        let length =
            (u32::from(header[1]) << 16) | (u32::from(header[2]) << 8) | u32::from(header[3]);
        offset = offset
            .checked_add(4)
            .ok_or(MuxError::LayoutOverflow("FLAC metadata header offset"))?;
        if offset
            .checked_add(u64::from(length))
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("FLAC metadata block type {block_type} overruns the input length"),
            });
        }
        let mut block_data = vec![0_u8; usize::try_from(length).unwrap()];
        read_exact_at_sync(
            &mut file,
            offset,
            &mut block_data,
            spec,
            "FLAC metadata block payload is truncated",
        )?;
        if block_type == 0 {
            stream_info = Some(parse_flac_stream_info(&block_data, spec)?);
        }
        metadata_blocks.push(ParsedFlacMetadataBlock {
            block_type,
            length,
            block_data,
        });
        offset = offset
            .checked_add(u64::from(length))
            .ok_or(MuxError::LayoutOverflow("FLAC metadata offset"))?;
        if last_metadata_block_flag {
            break;
        }
    }

    let stream_info = stream_info.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "FLAC input did not contain a STREAMINFO metadata block".to_string(),
    })?;
    if file_size == offset {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "FLAC input did not contain any frame payload after metadata".to_string(),
        });
    }
    let samples = scan_native_flac_frames_sync(&mut file, file_size, offset, spec, &stream_info)?;
    let sample_entry_box = build_flac_sample_entry_box(
        stream_info.sample_rate,
        stream_info.channel_count,
        stream_info.bits_per_sample,
        &metadata_blocks,
        Some(build_btrt_from_sample_sizes(
            samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
            stream_info.sample_rate,
        )?),
    )?;
    Ok(ParsedFlacTrack {
        sample_rate: stream_info.sample_rate,
        sample_entry_box,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_flac_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedFlacTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    if file_size < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "FLAC input is truncated before the 4-byte stream marker".to_string(),
        });
    }

    let mut signature = [0_u8; 4];
    read_exact_at_async(
        &mut file,
        0,
        &mut signature,
        spec,
        "FLAC input is truncated before the 4-byte stream marker",
    )
    .await?;
    if &signature != b"fLaC" {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "FLAC input did not start with the `fLaC` stream marker".to_string(),
        });
    }

    let mut offset = 4_u64;
    let mut metadata_blocks = Vec::new();
    let mut stream_info = None::<ParsedFlacStreamInfo>;
    loop {
        if file_size - offset < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "FLAC metadata block header is truncated".to_string(),
            });
        }
        let mut header = [0_u8; 4];
        read_exact_at_async(
            &mut file,
            offset,
            &mut header,
            spec,
            "FLAC metadata block header is truncated",
        )
        .await?;
        let last_metadata_block_flag = header[0] & 0x80 != 0;
        let block_type = header[0] & 0x7F;
        let length =
            (u32::from(header[1]) << 16) | (u32::from(header[2]) << 8) | u32::from(header[3]);
        offset = offset
            .checked_add(4)
            .ok_or(MuxError::LayoutOverflow("FLAC metadata header offset"))?;
        if offset
            .checked_add(u64::from(length))
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("FLAC metadata block type {block_type} overruns the input length"),
            });
        }
        let mut block_data = vec![0_u8; usize::try_from(length).unwrap()];
        read_exact_at_async(
            &mut file,
            offset,
            &mut block_data,
            spec,
            "FLAC metadata block payload is truncated",
        )
        .await?;
        if block_type == 0 {
            stream_info = Some(parse_flac_stream_info(&block_data, spec)?);
        }
        metadata_blocks.push(ParsedFlacMetadataBlock {
            block_type,
            length,
            block_data,
        });
        offset = offset
            .checked_add(u64::from(length))
            .ok_or(MuxError::LayoutOverflow("FLAC metadata offset"))?;
        if last_metadata_block_flag {
            break;
        }
    }

    let stream_info = stream_info.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "FLAC input did not contain a STREAMINFO metadata block".to_string(),
    })?;
    if file_size == offset {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "FLAC input did not contain any frame payload after metadata".to_string(),
        });
    }
    let samples =
        scan_native_flac_frames_async(&mut file, file_size, offset, spec, &stream_info).await?;
    let sample_entry_box = build_flac_sample_entry_box(
        stream_info.sample_rate,
        stream_info.channel_count,
        stream_info.bits_per_sample,
        &metadata_blocks,
        Some(build_btrt_from_sample_sizes(
            samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
            stream_info.sample_rate,
        )?),
    )?;
    Ok(ParsedFlacTrack {
        sample_rate: stream_info.sample_rate,
        sample_entry_box,
        samples,
    })
}

pub(in crate::mux) fn scan_ogg_flac_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedOggFlacTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut packet_builder = OggPacketBuilder::default();
    let mut stream_info = None::<ParsedFlacStreamInfo>;
    let mut sample_entry_box = None::<Vec<u8>>;
    let mut header_state = None::<OggFlacHeaderState>;
    let mut logical_size = 0_u64;
    let mut transformed_segments = Vec::new();
    let mut samples = Vec::new();
    while offset < file_size {
        let page = read_ogg_page_header_sync(&mut file, offset, spec)?;
        if packet_builder.is_empty() && page.header_type & 0x01 != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg FLAC input started in the middle of a continued packet".to_string(),
            });
        }
        offset = page
            .payload_offset
            .checked_add(page.payload_size)
            .ok_or(MuxError::LayoutOverflow("Ogg page range"))?;
        let mut page_cursor = page.payload_offset;
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
            if sample_entry_box.is_none() {
                let packet_bytes = read_spans_sync(
                    &mut file,
                    &packet.spans,
                    packet.total_size,
                    spec,
                    "Ogg FLAC identification packet is truncated",
                )?;
                if let Some(state) = &mut header_state {
                    state.append_extra_packet(&packet_bytes);
                } else {
                    header_state = Some(parse_ogg_flac_header_start(&packet_bytes, spec)?);
                }
                if header_state
                    .as_ref()
                    .is_some_and(|state| state.extra_header_packets_remaining == 0)
                {
                    let state = header_state.take().unwrap();
                    let (metadata_blocks, parsed_stream_info) =
                        parse_ogg_flac_header_packet(&state.header_bytes, spec)?;
                    sample_entry_box = Some(build_flac_sample_entry_box(
                        parsed_stream_info.sample_rate,
                        parsed_stream_info.channel_count,
                        parsed_stream_info.bits_per_sample,
                        &metadata_blocks,
                        None,
                    )?);
                    stream_info = Some(parsed_stream_info);
                }
                continue;
            }
            let packet_bytes = read_spans_sync(
                &mut file,
                &packet.spans,
                packet.total_size,
                spec,
                "Ogg FLAC frame packet is truncated",
            )?;
            let parsed_header = parse_flac_frame_packet(
                &packet_bytes,
                spec,
                packet.spans.first().map_or(0, |span| span.source_offset),
                stream_info.as_ref().unwrap(),
            )?;
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
                    .ok_or(MuxError::LayoutOverflow("Ogg FLAC logical source size"))?;
            }
            samples.push(StagedSample {
                data_offset,
                data_size: packet.total_size,
                duration: parsed_header.block_size,
                composition_time_offset: 0,
                is_sync_sample: true,
            });
        }
    }
    if !packet_builder.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg FLAC input ended in the middle of a packet".to_string(),
        });
    }
    if header_state
        .as_ref()
        .is_some_and(|state| state.extra_header_packets_remaining != 0)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg FLAC input ended before all mapping-header metadata packets were present"
                .to_string(),
        });
    }
    let stream_info = stream_info.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "Ogg FLAC input did not contain an identification packet".to_string(),
    })?;
    let sample_entry_box = sample_entry_box.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "Ogg FLAC input did not yield any FLAC metadata blocks".to_string(),
    })?;
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg FLAC input did not contain any audio packets after headers".to_string(),
        });
    }
    Ok(ParsedOggFlacTrack {
        segmented_source: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: transformed_segments,
            total_size: logical_size,
        },
        sample_rate: stream_info.sample_rate,
        sample_entry_box,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_ogg_flac_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedOggFlacTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut packet_builder = OggPacketBuilder::default();
    let mut stream_info = None::<ParsedFlacStreamInfo>;
    let mut sample_entry_box = None::<Vec<u8>>;
    let mut header_state = None::<OggFlacHeaderState>;
    let mut logical_size = 0_u64;
    let mut transformed_segments = Vec::new();
    let mut samples = Vec::new();
    while offset < file_size {
        let page = read_ogg_page_header_async(&mut file, offset, spec).await?;
        if packet_builder.is_empty() && page.header_type & 0x01 != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg FLAC input started in the middle of a continued packet".to_string(),
            });
        }
        offset = page
            .payload_offset
            .checked_add(page.payload_size)
            .ok_or(MuxError::LayoutOverflow("Ogg page range"))?;
        let mut page_cursor = page.payload_offset;
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
            if sample_entry_box.is_none() {
                let packet_bytes = read_spans_async(
                    &mut file,
                    &packet.spans,
                    packet.total_size,
                    spec,
                    "Ogg FLAC identification packet is truncated",
                )
                .await?;
                if let Some(state) = &mut header_state {
                    state.append_extra_packet(&packet_bytes);
                } else {
                    header_state = Some(parse_ogg_flac_header_start(&packet_bytes, spec)?);
                }
                if header_state
                    .as_ref()
                    .is_some_and(|state| state.extra_header_packets_remaining == 0)
                {
                    let state = header_state.take().unwrap();
                    let (metadata_blocks, parsed_stream_info) =
                        parse_ogg_flac_header_packet(&state.header_bytes, spec)?;
                    sample_entry_box = Some(build_flac_sample_entry_box(
                        parsed_stream_info.sample_rate,
                        parsed_stream_info.channel_count,
                        parsed_stream_info.bits_per_sample,
                        &metadata_blocks,
                        None,
                    )?);
                    stream_info = Some(parsed_stream_info);
                }
                continue;
            }
            let packet_bytes = read_spans_async(
                &mut file,
                &packet.spans,
                packet.total_size,
                spec,
                "Ogg FLAC frame packet is truncated",
            )
            .await?;
            let parsed_header = parse_flac_frame_packet(
                &packet_bytes,
                spec,
                packet.spans.first().map_or(0, |span| span.source_offset),
                stream_info.as_ref().unwrap(),
            )?;
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
                    .ok_or(MuxError::LayoutOverflow("Ogg FLAC logical source size"))?;
            }
            samples.push(StagedSample {
                data_offset,
                data_size: packet.total_size,
                duration: parsed_header.block_size,
                composition_time_offset: 0,
                is_sync_sample: true,
            });
        }
    }
    if !packet_builder.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg FLAC input ended in the middle of a packet".to_string(),
        });
    }
    if header_state
        .as_ref()
        .is_some_and(|state| state.extra_header_packets_remaining != 0)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg FLAC input ended before all mapping-header metadata packets were present"
                .to_string(),
        });
    }
    let stream_info = stream_info.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "Ogg FLAC input did not contain an identification packet".to_string(),
    })?;
    let sample_entry_box = sample_entry_box.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "Ogg FLAC input did not yield any FLAC metadata blocks".to_string(),
    })?;
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg FLAC input did not contain any audio packets after headers".to_string(),
        });
    }
    Ok(ParsedOggFlacTrack {
        segmented_source: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: transformed_segments,
            total_size: logical_size,
        },
        sample_rate: stream_info.sample_rate,
        sample_entry_box,
        samples,
    })
}

fn build_flac_sample_entry_box(
    sample_rate: u32,
    channel_count: u16,
    sample_size: u16,
    metadata_blocks: &[ParsedFlacMetadataBlock],
    btrt: Option<crate::boxes::iso14496_12::Btrt>,
) -> Result<Vec<u8>, MuxError> {
    let mut dfla = DfLa::default();
    dfla.metadata_blocks = minimal_flac_sample_entry_metadata_blocks(metadata_blocks, sample_rate)?;
    let mut dfla_box = super::super::mp4::encode_typed_box(&dfla, &[])?;
    // The typed `dfLa` model stays strict about the final-block bit, but the flat authored sample
    // entry preserves the retained one-block payload shape that the comparison target writes.
    if let Some(first_metadata_block) = dfla_box.get_mut(12) {
        *first_metadata_block &= 0x7F;
    } else {
        return Err(MuxError::LayoutOverflow("dfLa metadata header"));
    }
    let mut child_boxes = vec![dfla_box];
    if let Some(btrt) = btrt {
        child_boxes.push(super::super::mp4::encode_typed_box(&btrt, &[])?);
    }
    build_generic_audio_sample_entry_box(
        FLAC_ENTRY,
        sample_rate,
        channel_count,
        sample_size,
        &child_boxes,
    )
}

fn scan_native_flac_frames_sync(
    file: &mut File,
    file_size: u64,
    frame_data_offset: u64,
    spec: &str,
    stream_info: &ParsedFlacStreamInfo,
) -> Result<Vec<StagedSample>, MuxError> {
    let mut scan_offset = frame_data_offset;
    let mut frame_offset = frame_data_offset;
    let mut frame_buffer = Vec::new();
    let mut samples = Vec::new();
    let mut decoded_samples = 0_u64;

    loop {
        if let Some((frame_size, block_size)) =
            split_scanned_flac_frame(&frame_buffer, spec, frame_offset, stream_info)?
        {
            push_flac_frame_sample(
                &mut samples,
                &mut decoded_samples,
                frame_offset,
                frame_size,
                block_size,
                stream_info,
                spec,
            )?;
            frame_buffer.drain(..frame_size);
            frame_offset = frame_offset
                .checked_add(u64::try_from(frame_size).unwrap())
                .ok_or(MuxError::LayoutOverflow("FLAC frame offset"))?;
            continue;
        }

        if scan_offset >= file_size {
            break;
        }
        let chunk_size =
            usize::try_from((file_size - scan_offset).min(FLAC_SCAN_CHUNK_SIZE as u64)).unwrap();
        let buffer_len = frame_buffer.len();
        frame_buffer.resize(buffer_len + chunk_size, 0);
        read_exact_at_sync(
            file,
            scan_offset,
            &mut frame_buffer[buffer_len..],
            spec,
            "FLAC frame payload is truncated while scanning native frame boundaries",
        )?;
        scan_offset = scan_offset
            .checked_add(u64::try_from(chunk_size).unwrap())
            .ok_or(MuxError::LayoutOverflow("FLAC scan offset"))?;
    }

    finalize_native_flac_frame_scan(
        frame_buffer,
        frame_offset,
        &mut samples,
        &mut decoded_samples,
        stream_info,
        spec,
    )?;
    Ok(samples)
}

#[cfg(feature = "async")]
async fn scan_native_flac_frames_async(
    file: &mut TokioFile,
    file_size: u64,
    frame_data_offset: u64,
    spec: &str,
    stream_info: &ParsedFlacStreamInfo,
) -> Result<Vec<StagedSample>, MuxError> {
    let mut scan_offset = frame_data_offset;
    let mut frame_offset = frame_data_offset;
    let mut frame_buffer = Vec::new();
    let mut samples = Vec::new();
    let mut decoded_samples = 0_u64;

    loop {
        if let Some((frame_size, block_size)) =
            split_scanned_flac_frame(&frame_buffer, spec, frame_offset, stream_info)?
        {
            push_flac_frame_sample(
                &mut samples,
                &mut decoded_samples,
                frame_offset,
                frame_size,
                block_size,
                stream_info,
                spec,
            )?;
            frame_buffer.drain(..frame_size);
            frame_offset = frame_offset
                .checked_add(u64::try_from(frame_size).unwrap())
                .ok_or(MuxError::LayoutOverflow("FLAC frame offset"))?;
            continue;
        }

        if scan_offset >= file_size {
            break;
        }
        let chunk_size =
            usize::try_from((file_size - scan_offset).min(FLAC_SCAN_CHUNK_SIZE as u64)).unwrap();
        let buffer_len = frame_buffer.len();
        frame_buffer.resize(buffer_len + chunk_size, 0);
        read_exact_at_async(
            file,
            scan_offset,
            &mut frame_buffer[buffer_len..],
            spec,
            "FLAC frame payload is truncated while scanning native frame boundaries",
        )
        .await?;
        scan_offset = scan_offset
            .checked_add(u64::try_from(chunk_size).unwrap())
            .ok_or(MuxError::LayoutOverflow("FLAC scan offset"))?;
    }

    finalize_native_flac_frame_scan(
        frame_buffer,
        frame_offset,
        &mut samples,
        &mut decoded_samples,
        stream_info,
        spec,
    )?;
    Ok(samples)
}

fn finalize_native_flac_frame_scan(
    frame_buffer: Vec<u8>,
    frame_offset: u64,
    samples: &mut Vec<StagedSample>,
    decoded_samples: &mut u64,
    stream_info: &ParsedFlacStreamInfo,
    spec: &str,
) -> Result<(), MuxError> {
    if frame_buffer.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "FLAC input did not contain any native audio frames after metadata"
                .to_string(),
        });
    }
    let header = parse_flac_frame_packet(&frame_buffer, spec, frame_offset, stream_info)?;
    push_flac_frame_sample(
        samples,
        decoded_samples,
        frame_offset,
        frame_buffer.len(),
        header.block_size,
        stream_info,
        spec,
    )?;
    if stream_info.total_samples != 0 && *decoded_samples != stream_info.total_samples {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "FLAC frame durations summed to {} samples, but STREAMINFO declared {}",
                *decoded_samples, stream_info.total_samples
            ),
        });
    }
    Ok(())
}

fn split_scanned_flac_frame(
    frame_buffer: &[u8],
    spec: &str,
    frame_offset: u64,
    stream_info: &ParsedFlacStreamInfo,
) -> Result<Option<(usize, u32)>, MuxError> {
    if frame_buffer.len() < 2 {
        return Ok(None);
    }
    if !looks_like_flac_frame_start(frame_buffer) {
        return Err(invalid_flac_frame(
            spec,
            frame_offset,
            "FLAC frame payload did not start with the expected sync code",
        ));
    }
    let mut candidate_index = 2_usize;
    while let Some(next_index) = find_next_flac_frame_start(frame_buffer, candidate_index) {
        if let Ok(header) =
            parse_flac_frame_packet(&frame_buffer[..next_index], spec, frame_offset, stream_info)
        {
            return Ok(Some((next_index, header.block_size)));
        }
        candidate_index = next_index + 1;
    }
    Ok(None)
}

fn push_flac_frame_sample(
    samples: &mut Vec<StagedSample>,
    decoded_samples: &mut u64,
    frame_offset: u64,
    frame_size: usize,
    block_size: u32,
    stream_info: &ParsedFlacStreamInfo,
    spec: &str,
) -> Result<(), MuxError> {
    if frame_size == 0 {
        return Err(MuxError::LayoutOverflow("FLAC frame size"));
    }
    let remaining_samples = if stream_info.total_samples == 0 {
        u64::from(block_size)
    } else {
        let remaining = stream_info.total_samples.saturating_sub(*decoded_samples);
        if remaining == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "FLAC input carried more frame data than STREAMINFO declared".to_string(),
            });
        }
        remaining.min(u64::from(block_size))
    };
    let duration = u32::try_from(remaining_samples)
        .map_err(|_| MuxError::LayoutOverflow("FLAC frame duration"))?;
    samples.push(StagedSample {
        data_offset: frame_offset,
        data_size: u32::try_from(frame_size)
            .map_err(|_| MuxError::LayoutOverflow("FLAC frame size"))?,
        duration,
        composition_time_offset: 0,
        is_sync_sample: true,
    });
    *decoded_samples = decoded_samples
        .checked_add(u64::from(duration))
        .ok_or(MuxError::LayoutOverflow("FLAC decoded sample count"))?;
    Ok(())
}

fn looks_like_flac_frame_start(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xFE) == 0xF8
}

fn find_next_flac_frame_start(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.len() < 2 || start >= bytes.len().saturating_sub(1) {
        return None;
    }
    (start..bytes.len() - 1).find(|&index| looks_like_flac_frame_start(&bytes[index..]))
}

fn minimal_flac_sample_entry_metadata_blocks(
    metadata_blocks: &[ParsedFlacMetadataBlock],
    sample_rate: u32,
) -> Result<Vec<FlacMetadataBlock>, MuxError> {
    let stream_info = metadata_blocks
        .iter()
        .find(|block| block.block_type == 0)
        .ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: "FLAC sample entry".to_string(),
            message: format!(
                "missing required STREAMINFO metadata block for {sample_rate} Hz FLAC sample entry"
            ),
        })?;
    Ok(vec![FlacMetadataBlock {
        last_metadata_block_flag: true,
        block_type: stream_info.block_type,
        length: stream_info.length,
        block_data: stream_info.block_data.clone(),
    }])
}

impl OggFlacHeaderState {
    fn append_extra_packet(&mut self, packet: &[u8]) {
        self.header_bytes.extend_from_slice(packet);
        self.extra_header_packets_remaining -= 1;
    }
}

fn parse_ogg_flac_header_start(packet: &[u8], spec: &str) -> Result<OggFlacHeaderState, MuxError> {
    let parsed = normalize_ogg_flac_header_packet(packet, spec)?;
    Ok(OggFlacHeaderState {
        header_bytes: parsed.native_header_bytes.to_vec(),
        extra_header_packets_remaining: parsed.extra_header_packets_remaining,
    })
}

fn parse_ogg_flac_header_packet(
    packet: &[u8],
    spec: &str,
) -> Result<(Vec<ParsedFlacMetadataBlock>, ParsedFlacStreamInfo), MuxError> {
    if !packet.starts_with(b"fLaC") {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "Ogg FLAC header payload did not start with the native `fLaC` stream marker"
                .to_string(),
        });
    }
    let mut offset = 4usize;
    let mut metadata_blocks = Vec::new();
    let mut stream_info = None::<ParsedFlacStreamInfo>;
    loop {
        if packet.len().saturating_sub(offset) < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg FLAC metadata block header is truncated".to_string(),
            });
        }
        let header = &packet[offset..offset + 4];
        let last_metadata_block_flag = header[0] & 0x80 != 0;
        let block_type = header[0] & 0x7F;
        let length =
            (u32::from(header[1]) << 16) | (u32::from(header[2]) << 8) | u32::from(header[3]);
        offset = offset
            .checked_add(4)
            .ok_or(MuxError::LayoutOverflow("Ogg FLAC metadata offset"))?;
        let end = offset
            .checked_add(usize::try_from(length).unwrap())
            .ok_or(MuxError::LayoutOverflow("Ogg FLAC metadata size"))?;
        if end > packet.len() {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "Ogg FLAC metadata block type {block_type} overruns the identification packet"
                ),
            });
        }
        let block_data = packet[offset..end].to_vec();
        if block_type == 0 {
            stream_info = Some(parse_flac_stream_info(&block_data, spec)?);
        }
        metadata_blocks.push(ParsedFlacMetadataBlock {
            block_type,
            length,
            block_data,
        });
        offset = end;
        if last_metadata_block_flag {
            break;
        }
    }
    if offset != packet.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "Ogg FLAC identification packet carried unexpected bytes after the metadata blocks"
                    .to_string(),
        });
    }
    let stream_info = stream_info.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "Ogg FLAC identification packet did not contain a STREAMINFO metadata block"
            .to_string(),
    })?;
    Ok((metadata_blocks, stream_info))
}

fn normalize_ogg_flac_header_packet<'a>(
    packet: &'a [u8],
    spec: &str,
) -> Result<ParsedOggFlacHeaderPacket<'a>, MuxError> {
    if packet.starts_with(b"fLaC") {
        return Ok(ParsedOggFlacHeaderPacket {
            native_header_bytes: packet,
            extra_header_packets_remaining: 0,
        });
    }
    if packet.len() >= 13 && packet[0] == 0x7F && &packet[1..5] == b"FLAC" {
        let major_version = packet[5];
        let minor_version = packet[6];
        let header_packet_count = u16::from_be_bytes([packet[7], packet[8]]);
        if major_version != 1 || minor_version != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "Ogg FLAC mapping header used unsupported version {}.{}",
                    major_version, minor_version
                ),
            });
        }
        if header_packet_count == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "Ogg FLAC mapping header declared zero FLAC metadata packets".to_string(),
            });
        }
        let native_packet = &packet[9..];
        if native_packet.starts_with(b"fLaC") {
            return Ok(ParsedOggFlacHeaderPacket {
                native_header_bytes: native_packet,
                extra_header_packets_remaining: header_packet_count,
            });
        }
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "Ogg FLAC identification packet did not use a supported native or mapping-header signature".to_string(),
    })
}

fn parse_flac_stream_info(block_data: &[u8], spec: &str) -> Result<ParsedFlacStreamInfo, MuxError> {
    if block_data.len() != 34 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "FLAC STREAMINFO metadata block must be 34 bytes, not {}",
                block_data.len()
            ),
        });
    }
    let sample_rate = (u32::from(block_data[10]) << 12)
        | (u32::from(block_data[11]) << 4)
        | (u32::from(block_data[12] >> 4));
    let channel_count = u16::from(((block_data[12] >> 1) & 0x07) + 1);
    let bits_per_sample = u16::from((((block_data[12] & 0x01) << 4) | (block_data[13] >> 4)) + 1);
    let total_samples = ((u64::from(block_data[13] & 0x0F)) << 32)
        | (u64::from(block_data[14]) << 24)
        | (u64::from(block_data[15]) << 16)
        | (u64::from(block_data[16]) << 8)
        | u64::from(block_data[17]);
    if sample_rate == 0 || channel_count == 0 || bits_per_sample == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "FLAC STREAMINFO declared a zero-valued audio parameter".to_string(),
        });
    }
    Ok(ParsedFlacStreamInfo {
        sample_rate,
        channel_count,
        bits_per_sample,
        total_samples,
    })
}

fn parse_flac_frame_packet(
    frame: &[u8],
    spec: &str,
    offset: u64,
    stream_info: &ParsedFlacStreamInfo,
) -> Result<ParsedFlacFrameHeader, MuxError> {
    if frame.len() < 6 {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet is truncated before the frame footer",
        ));
    }
    let mut reader = SliceBitReader::new(frame);
    if reader.read_bits_u32(15) != Some(0x7FFC) {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet did not start with the 14-bit sync code",
        ));
    }
    let _ = reader.read_bits_u32(1).ok_or_else(|| {
        invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet is truncated in its frame header",
        )
    })?;
    let block_size_code = reader.read_bits_u32(4).ok_or_else(|| {
        invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet is truncated in its frame header",
        )
    })?;
    if block_size_code == 0 {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet used the reserved block-size code 0",
        ));
    }
    let sample_rate_code = reader.read_bits_u32(4).ok_or_else(|| {
        invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet is truncated in its frame header",
        )
    })?;
    if sample_rate_code == 0x0F {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet used the reserved sample-rate code 15",
        ));
    }
    let channel_assignment = reader.read_bits_u32(4).ok_or_else(|| {
        invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet is truncated in its frame header",
        )
    })?;
    let channel_count = match channel_assignment {
        0..=7 => u16::try_from(channel_assignment + 1).unwrap(),
        8..=10 => 2,
        _ => {
            return Err(invalid_flac_frame(
                spec,
                offset,
                "FLAC frame packet used an unsupported channel-assignment code",
            ));
        }
    };
    if channel_count != stream_info.channel_count {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet changed the declared channel count",
        ));
    }
    let bits_per_sample_code = reader.read_bits_u32(3).ok_or_else(|| {
        invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet is truncated in its frame header",
        )
    })?;
    let frame_bits_per_sample = match bits_per_sample_code {
        0 => stream_info.bits_per_sample,
        1 => 8,
        2 => 12,
        3 => {
            return Err(invalid_flac_frame(
                spec,
                offset,
                "FLAC frame packet used the reserved bits-per-sample code 3",
            ));
        }
        4 => 16,
        5 => 20,
        6 => 24,
        7 => {
            return Err(invalid_flac_frame(
                spec,
                offset,
                "FLAC frame packet used the reserved bits-per-sample code 7",
            ));
        }
        _ => unreachable!(),
    };
    if frame_bits_per_sample != stream_info.bits_per_sample {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet changed the declared bits-per-sample value",
        ));
    }
    if reader.read_bits_u32(1) != Some(0) {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet set the reserved frame-header bit",
        ));
    }
    read_flac_utf8_like_value(&mut reader, spec, offset)?;
    let block_size = match block_size_code {
        6 => u32::from(read_flac_aligned_u8(&mut reader, spec, offset)?) + 1,
        7 => u32::from(read_flac_aligned_u16(&mut reader, spec, offset)?) + 1,
        value => FLAC_BLOCK_SIZE_TABLE[usize::try_from(value).unwrap()],
    };
    let sample_rate = match sample_rate_code {
        0 => stream_info.sample_rate,
        12 => u32::from(read_flac_aligned_u8(&mut reader, spec, offset)?),
        13 => u32::from(read_flac_aligned_u16(&mut reader, spec, offset)?),
        14 => u32::from(read_flac_aligned_u16(&mut reader, spec, offset)?) * 10,
        value => FLAC_SAMPLE_RATE_TABLE
            .get(usize::try_from(value).unwrap())
            .copied()
            .unwrap_or(0),
    };
    if sample_rate == 0 || sample_rate != stream_info.sample_rate {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet changed or omitted the declared sample rate",
        ));
    }
    let header_crc_position = reader.position_byte().ok_or_else(|| {
        invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet lost byte alignment before the header CRC",
        )
    })?;
    let stored_crc8 = read_flac_aligned_u8(&mut reader, spec, offset)?;
    if stored_crc8 != flac_crc8(&frame[..header_crc_position]) {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet failed its header CRC8 check",
        ));
    }
    if reader.read_bits_u32(1) != Some(0) {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet set the reserved subframe bit",
        ));
    }
    let subframe_type = reader.read_bits_u32(6).ok_or_else(|| {
        invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet is truncated in its first subframe",
        )
    })?;
    if !matches!(subframe_type, 0 | 1 | 8..=12 | 32..=63) {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet used an unsupported first-subframe type",
        ));
    }
    let stored_crc16 = u16::from_be_bytes([frame[frame.len() - 2], frame[frame.len() - 1]]);
    if stored_crc16 != flac_crc16(&frame[..frame.len() - 2]) {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet failed its frame CRC16 check",
        ));
    }
    Ok(ParsedFlacFrameHeader { block_size })
}

fn read_flac_utf8_like_value(
    reader: &mut SliceBitReader<'_>,
    spec: &str,
    offset: u64,
) -> Result<(), MuxError> {
    let mut value = u32::from(read_flac_aligned_u8(reader, spec, offset)?);
    let mut top = (value & 0x80) >> 1;
    if (value & 0xC0) == 0x80 || value >= 0xFE {
        return Err(invalid_flac_frame(
            spec,
            offset,
            "FLAC frame packet used an invalid UTF-8 coded frame or sample number",
        ));
    }
    while value & top != 0 {
        let continuation = read_flac_aligned_u8(reader, spec, offset)?;
        if continuation & 0xC0 != 0x80 {
            return Err(invalid_flac_frame(
                spec,
                offset,
                "FLAC frame packet used a malformed UTF-8 continuation byte",
            ));
        }
        value = (value << 6) | u32::from(continuation & 0x3F);
        top <<= 5;
    }
    Ok(())
}

fn read_flac_aligned_u8(
    reader: &mut SliceBitReader<'_>,
    spec: &str,
    offset: u64,
) -> Result<u8, MuxError> {
    reader
        .read_aligned_u8()
        .ok_or_else(|| invalid_flac_frame(spec, offset, "FLAC frame packet is truncated"))
}

fn read_flac_aligned_u16(
    reader: &mut SliceBitReader<'_>,
    spec: &str,
    offset: u64,
) -> Result<u16, MuxError> {
    let high = read_flac_aligned_u8(reader, spec, offset)?;
    let low = read_flac_aligned_u8(reader, spec, offset)?;
    Ok(u16::from_be_bytes([high, low]))
}

fn invalid_flac_frame(spec: &str, offset: u64, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("{message} at byte offset {offset}"),
    }
}

fn flac_crc8(data: &[u8]) -> u8 {
    let mut crc = 0_u8;
    for byte in data {
        crc ^= *byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn flac_crc16(data: &[u8]) -> u16 {
    let mut crc = 0_u16;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

struct SliceBitReader<'a> {
    bytes: &'a [u8],
    bit_offset: usize,
}

impl<'a> SliceBitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            bit_offset: 0,
        }
    }

    fn read_bits_u32(&mut self, width: usize) -> Option<u32> {
        let mut value = 0_u32;
        for _ in 0..width {
            value = (value << 1) | u32::from(self.read_bit()?);
        }
        Some(value)
    }

    fn read_bit(&mut self) -> Option<u8> {
        let byte = *self.bytes.get(self.bit_offset / 8)?;
        let shift = 7 - (self.bit_offset % 8);
        self.bit_offset += 1;
        Some((byte >> shift) & 0x01)
    }

    fn read_aligned_u8(&mut self) -> Option<u8> {
        if !self.bit_offset.is_multiple_of(8) {
            return None;
        }
        let byte = *self.bytes.get(self.bit_offset / 8)?;
        self.bit_offset += 8;
        Some(byte)
    }

    fn position_byte(&self) -> Option<usize> {
        self.bit_offset
            .is_multiple_of(8)
            .then_some(self.bit_offset / 8)
    }
}
