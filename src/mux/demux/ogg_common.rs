use std::fs::File;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use super::super::import::{SourceFileSpan, read_exact_at_sync, read_spans_sync};
#[cfg(feature = "async")]
use super::super::import::{read_exact_at_async, read_spans_async};
use super::super::{MuxError, MuxRawCodec};
use super::detect::DetectedPathTrackKind;

#[derive(Clone)]
pub(super) struct OggPageHeader {
    pub(super) header_type: u8,
    pub(super) granule_position: u64,
    pub(super) lacing_values: Vec<u8>,
    pub(super) payload_offset: u64,
    pub(super) payload_size: u64,
}

#[derive(Default)]
pub(super) struct OggPacketBuilder {
    spans: Vec<SourceFileSpan>,
    total_size: u32,
}

pub(super) struct CompletedOggPacket {
    pub(super) spans: Vec<SourceFileSpan>,
    pub(super) total_size: u32,
}

impl OggPacketBuilder {
    pub(super) fn push_span(&mut self, source_offset: u64, size: u32) -> Result<(), MuxError> {
        if size == 0 {
            return Ok(());
        }
        self.total_size = self
            .total_size
            .checked_add(size)
            .ok_or(MuxError::LayoutOverflow("Ogg packet size"))?;
        self.spans.push(SourceFileSpan {
            source_offset,
            size,
        });
        Ok(())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.total_size == 0
    }

    pub(super) fn finish(&mut self) -> CompletedOggPacket {
        CompletedOggPacket {
            spans: std::mem::take(&mut self.spans),
            total_size: std::mem::take(&mut self.total_size),
        }
    }
}

pub(in crate::mux) fn detect_ogg_track_kind_sync(
    file: &mut File,
) -> Result<DetectedPathTrackKind, MuxError> {
    detect_ogg_track_kind_with_reader_sync(file, "Ogg path detection")
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn detect_ogg_track_kind_async(
    file: &mut TokioFile,
) -> Result<DetectedPathTrackKind, MuxError> {
    detect_ogg_track_kind_with_reader_async(file, "Ogg path detection").await
}

fn detect_ogg_track_kind_with_reader_sync(
    file: &mut File,
    spec: &str,
) -> Result<DetectedPathTrackKind, MuxError> {
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut packet_builder = OggPacketBuilder::default();
    while offset < file_size {
        let page = read_ogg_page_header_sync(file, offset, spec)?;
        offset = page
            .payload_offset
            .checked_add(page.payload_size)
            .ok_or(MuxError::LayoutOverflow("Ogg page range"))?;
        let mut page_cursor = page.payload_offset;
        for lacing in &page.lacing_values {
            packet_builder.push_span(page_cursor, u32::from(*lacing))?;
            page_cursor += u64::from(*lacing);
            if *lacing < 255 {
                let packet = packet_builder.finish();
                if packet.total_size == 0 {
                    continue;
                }
                let prefix = read_packet_prefix_sync(
                    file,
                    &packet.spans,
                    64,
                    spec,
                    "Ogg packet is truncated while reading the identification payload",
                )?;
                if prefix.starts_with(b"OpusHead") {
                    return Ok(DetectedPathTrackKind::Raw(MuxRawCodec::Opus));
                }
                if looks_like_vorbis_identification_packet(&prefix) {
                    return Ok(DetectedPathTrackKind::Raw(MuxRawCodec::Vorbis));
                }
                if looks_like_speex_identification_packet(&prefix) {
                    return Ok(DetectedPathTrackKind::Raw(MuxRawCodec::Speex));
                }
                if looks_like_ogg_flac_identification_packet(&prefix) {
                    return Ok(DetectedPathTrackKind::Raw(MuxRawCodec::Flac));
                }
                if looks_like_theora_identification_packet(&prefix) {
                    return Ok(DetectedPathTrackKind::Raw(MuxRawCodec::Theora));
                }
                return Ok(DetectedPathTrackKind::Unknown);
            }
        }
    }
    Ok(DetectedPathTrackKind::Unknown)
}

#[cfg(feature = "async")]
async fn detect_ogg_track_kind_with_reader_async(
    file: &mut TokioFile,
    spec: &str,
) -> Result<DetectedPathTrackKind, MuxError> {
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut packet_builder = OggPacketBuilder::default();
    while offset < file_size {
        let page = read_ogg_page_header_async(file, offset, spec).await?;
        offset = page
            .payload_offset
            .checked_add(page.payload_size)
            .ok_or(MuxError::LayoutOverflow("Ogg page range"))?;
        let mut page_cursor = page.payload_offset;
        for lacing in &page.lacing_values {
            packet_builder.push_span(page_cursor, u32::from(*lacing))?;
            page_cursor += u64::from(*lacing);
            if *lacing < 255 {
                let packet = packet_builder.finish();
                if packet.total_size == 0 {
                    continue;
                }
                let prefix = read_packet_prefix_async(
                    file,
                    &packet.spans,
                    64,
                    spec,
                    "Ogg packet is truncated while reading the identification payload",
                )
                .await?;
                if prefix.starts_with(b"OpusHead") {
                    return Ok(DetectedPathTrackKind::Raw(MuxRawCodec::Opus));
                }
                if looks_like_vorbis_identification_packet(&prefix) {
                    return Ok(DetectedPathTrackKind::Raw(MuxRawCodec::Vorbis));
                }
                if looks_like_speex_identification_packet(&prefix) {
                    return Ok(DetectedPathTrackKind::Raw(MuxRawCodec::Speex));
                }
                if looks_like_ogg_flac_identification_packet(&prefix) {
                    return Ok(DetectedPathTrackKind::Raw(MuxRawCodec::Flac));
                }
                if looks_like_theora_identification_packet(&prefix) {
                    return Ok(DetectedPathTrackKind::Raw(MuxRawCodec::Theora));
                }
                return Ok(DetectedPathTrackKind::Unknown);
            }
        }
    }
    Ok(DetectedPathTrackKind::Unknown)
}

fn looks_like_ogg_flac_identification_packet(packet: &[u8]) -> bool {
    packet.starts_with(b"fLaC")
        || (packet.len() >= 13
            && packet[0] == 0x7F
            && &packet[1..5] == b"FLAC"
            && &packet[9..13] == b"fLaC")
}

fn looks_like_vorbis_identification_packet(packet: &[u8]) -> bool {
    packet.len() >= 7 && packet[0] == 0x01 && &packet[1..7] == b"vorbis"
}

fn looks_like_speex_identification_packet(packet: &[u8]) -> bool {
    packet.starts_with(b"Speex")
}

fn looks_like_theora_identification_packet(packet: &[u8]) -> bool {
    packet.len() >= 7 && packet[0] == 0x80 && &packet[1..7] == b"theora"
}

fn ogg_page_crc(page_bytes: &[u8]) -> u32 {
    let mut crc = 0_u32;
    for byte in page_bytes {
        crc ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn validate_ogg_page_crc_sync(
    file: &mut File,
    offset: u64,
    header: &[u8; 27],
    lacing_values: &[u8],
    payload_offset: u64,
    payload_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    let total_page_size = 27_u64
        .checked_add(u64::try_from(lacing_values.len()).unwrap())
        .and_then(|value| value.checked_add(payload_size))
        .ok_or(MuxError::LayoutOverflow("Ogg page size"))?;
    let mut page = vec![
        0_u8;
        usize::try_from(total_page_size)
            .map_err(|_| MuxError::LayoutOverflow("Ogg page size"))?
    ];
    page[..27].copy_from_slice(header);
    page[27..27 + lacing_values.len()].copy_from_slice(lacing_values);
    if payload_size != 0 {
        read_exact_at_sync(
            file,
            payload_offset,
            &mut page[27 + lacing_values.len()..],
            spec,
            "Ogg page payload is truncated while validating CRC",
        )?;
    }
    let expected_crc = u32::from_le_bytes(header[22..26].try_into().unwrap());
    page[22..26].fill(0);
    if ogg_page_crc(&page) != expected_crc {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("Ogg page at byte offset {offset} failed CRC validation"),
        });
    }
    Ok(())
}

#[cfg(feature = "async")]
async fn validate_ogg_page_crc_async(
    file: &mut TokioFile,
    offset: u64,
    header: &[u8; 27],
    lacing_values: &[u8],
    payload_offset: u64,
    payload_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    let total_page_size = 27_u64
        .checked_add(u64::try_from(lacing_values.len()).unwrap())
        .and_then(|value| value.checked_add(payload_size))
        .ok_or(MuxError::LayoutOverflow("Ogg page size"))?;
    let mut page = vec![
        0_u8;
        usize::try_from(total_page_size)
            .map_err(|_| MuxError::LayoutOverflow("Ogg page size"))?
    ];
    page[..27].copy_from_slice(header);
    page[27..27 + lacing_values.len()].copy_from_slice(lacing_values);
    if payload_size != 0 {
        read_exact_at_async(
            file,
            payload_offset,
            &mut page[27 + lacing_values.len()..],
            spec,
            "Ogg page payload is truncated while validating CRC",
        )
        .await?;
    }
    let expected_crc = u32::from_le_bytes(header[22..26].try_into().unwrap());
    page[22..26].fill(0);
    if ogg_page_crc(&page) != expected_crc {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("Ogg page at byte offset {offset} failed CRC validation"),
        });
    }
    Ok(())
}

pub(super) fn read_ogg_page_header_sync(
    file: &mut File,
    offset: u64,
    spec: &str,
) -> Result<OggPageHeader, MuxError> {
    let mut header = [0_u8; 27];
    read_exact_at_sync(
        file,
        offset,
        &mut header,
        spec,
        "Ogg page header is truncated before 27 bytes",
    )?;
    if &header[..4] != b"OggS" {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("Ogg page at byte offset {offset} did not start with `OggS`"),
        });
    }
    if header[4] != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "Ogg page at byte offset {offset} used unsupported stream structure version {}",
                header[4]
            ),
        });
    }
    let segment_count = usize::from(header[26]);
    let mut lacing_values = vec![0_u8; segment_count];
    read_exact_at_sync(
        file,
        offset + 27,
        &mut lacing_values,
        spec,
        "Ogg page segment table is truncated",
    )?;
    let payload_offset = offset
        .checked_add(27)
        .and_then(|value| value.checked_add(u64::try_from(segment_count).unwrap()))
        .ok_or(MuxError::LayoutOverflow("Ogg payload offset"))?;
    let payload_size = lacing_values.iter().map(|value| u64::from(*value)).sum();
    validate_ogg_page_crc_sync(
        file,
        offset,
        &header,
        &lacing_values,
        payload_offset,
        payload_size,
        spec,
    )?;
    Ok(OggPageHeader {
        header_type: header[5],
        granule_position: u64::from_le_bytes(header[6..14].try_into().unwrap()),
        lacing_values,
        payload_offset,
        payload_size,
    })
}

#[cfg(feature = "async")]
pub(super) async fn read_ogg_page_header_async(
    file: &mut TokioFile,
    offset: u64,
    spec: &str,
) -> Result<OggPageHeader, MuxError> {
    let mut header = [0_u8; 27];
    read_exact_at_async(
        file,
        offset,
        &mut header,
        spec,
        "Ogg page header is truncated before 27 bytes",
    )
    .await?;
    if &header[..4] != b"OggS" {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("Ogg page at byte offset {offset} did not start with `OggS`"),
        });
    }
    if header[4] != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "Ogg page at byte offset {offset} used unsupported stream structure version {}",
                header[4]
            ),
        });
    }
    let segment_count = usize::from(header[26]);
    let mut lacing_values = vec![0_u8; segment_count];
    read_exact_at_async(
        file,
        offset + 27,
        &mut lacing_values,
        spec,
        "Ogg page segment table is truncated",
    )
    .await?;
    let payload_offset = offset
        .checked_add(27)
        .and_then(|value| value.checked_add(u64::try_from(segment_count).unwrap()))
        .ok_or(MuxError::LayoutOverflow("Ogg payload offset"))?;
    let payload_size = lacing_values.iter().map(|value| u64::from(*value)).sum();
    validate_ogg_page_crc_async(
        file,
        offset,
        &header,
        &lacing_values,
        payload_offset,
        payload_size,
        spec,
    )
    .await?;
    Ok(OggPageHeader {
        header_type: header[5],
        granule_position: u64::from_le_bytes(header[6..14].try_into().unwrap()),
        lacing_values,
        payload_offset,
        payload_size,
    })
}

pub(super) fn read_packet_prefix_sync(
    file: &mut File,
    spans: &[SourceFileSpan],
    max_len: usize,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let requested = spans
        .iter()
        .fold(0usize, |len: usize, span| {
            len.saturating_add(usize::try_from(span.size).unwrap())
        })
        .min(max_len);
    let mut bytes = read_spans_sync(
        file,
        spans,
        u32::try_from(requested).unwrap_or(u32::MAX),
        spec,
        truncated_message,
    )?;
    bytes.truncate(requested);
    Ok(bytes)
}

#[cfg(feature = "async")]
pub(super) async fn read_packet_prefix_async(
    file: &mut TokioFile,
    spans: &[SourceFileSpan],
    max_len: usize,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let requested = spans
        .iter()
        .fold(0usize, |len: usize, span| {
            len.saturating_add(usize::try_from(span.size).unwrap())
        })
        .min(max_len);
    let mut bytes: Vec<u8> = read_spans_async(
        file,
        spans,
        u32::try_from(requested).unwrap_or(u32::MAX),
        spec,
        truncated_message,
    )
    .await?;
    bytes.truncate(requested);
    Ok(bytes)
}
