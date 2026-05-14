use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::iso14496_12::Btrt;

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    SegmentedMuxSourceSegment, StagedSample, build_btrt_from_sample_sizes,
    build_generic_audio_sample_entry_box, read_exact_at_sync,
};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;

const MHM1: FourCc = FourCc::from_bytes(*b"mhm1");
const MHAS_SAMPLE_RATE_TABLE: [u32; 28] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350, 0, 0, 57_600, 51_200, 40_000, 38_400, 34_150, 28_800, 25_600, 20_000, 19_200, 17_075,
    14_400, 12_800, 9_600,
];

pub(in crate::mux) struct ParsedMhasTrack {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedMhasConfig {
    sample_rate: u32,
    frame_length: u32,
    channel_count: u16,
}

#[derive(Clone, Copy)]
struct MhasPacketHeader {
    packet_type: u32,
    payload_size: u64,
    header_size: u64,
}

struct MhasBitCursor<'a> {
    data: &'a [u8],
    bit_offset: usize,
}

impl<'a> MhasBitCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_offset: 0,
        }
    }

    fn read_bits(&mut self, width: usize, spec: &str, context: &str) -> Result<u64, MuxError> {
        if width > 64 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("MHAS parser requested invalid bit width {width} for {context}"),
            });
        }
        let end = self
            .bit_offset
            .checked_add(width)
            .ok_or(MuxError::LayoutOverflow("MHAS bit reader position"))?;
        if end > self.data.len() * 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated MHAS data while reading {context}"),
            });
        }

        let mut value = 0_u64;
        for _ in 0..width {
            let byte = self.data[self.bit_offset / 8];
            let shift = 7 - (self.bit_offset % 8);
            value = (value << 1) | u64::from((byte >> shift) & 0x01);
            self.bit_offset += 1;
        }
        Ok(value)
    }

    fn read_bool(&mut self, spec: &str, context: &str) -> Result<bool, MuxError> {
        Ok(self.read_bits(1, spec, context)? != 0)
    }

    fn bytes_consumed(&self) -> usize {
        self.bit_offset.div_ceil(8)
    }

    fn read_escaped_value(
        &mut self,
        first_width: usize,
        escape_width: usize,
        final_width: usize,
        spec: &str,
        context: &str,
    ) -> Result<u64, MuxError> {
        let value = self.read_bits(first_width, spec, context)?;
        let max_first = (1_u64 << first_width) - 1;
        if value != max_first {
            return Ok(value);
        }
        let escape = self.read_bits(escape_width, spec, context)?;
        let max_escape = (1_u64 << escape_width) - 1;
        if escape != max_escape {
            return value
                .checked_add(escape)
                .ok_or(MuxError::LayoutOverflow("MHAS escaped value"));
        }
        let final_value = self.read_bits(final_width, spec, context)?;
        value
            .checked_add(escape)
            .and_then(|prefix| prefix.checked_add(final_value))
            .ok_or(MuxError::LayoutOverflow("MHAS escaped value"))
    }
}

pub(in crate::mux) fn scan_mhas_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedMhasTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    if file_size < 3 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MHAS input is truncated before the required leading sync packet".to_string(),
        });
    }

    let mut offset = 0_u64;
    let mut sample_start = 0_u64;
    let mut config = None::<ParsedMhasConfig>;
    let mut saw_frame = false;
    let mut samples = Vec::new();
    while offset < file_size {
        let header = read_mhas_packet_header_sync(&mut file, file_size, offset, spec)?;
        let payload_offset = offset
            .checked_add(header.header_size)
            .ok_or(MuxError::LayoutOverflow("MHAS payload offset"))?;
        let packet_end = payload_offset
            .checked_add(header.payload_size)
            .ok_or(MuxError::LayoutOverflow("MHAS packet range"))?;
        if packet_end > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("MHAS packet at byte offset {offset} overruns the input length"),
            });
        }
        if header.packet_type > 18 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "MHAS packet at byte offset {offset} used unsupported packet type {}",
                    header.packet_type
                ),
            });
        }
        if header.payload_size == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("MHAS packet at byte offset {offset} declared a zero payload"),
            });
        }
        if offset == 0 {
            let sync_byte = read_mhas_sync_marker_sync(&mut file, payload_offset, spec)?;
            if header.packet_type != 6 || header.payload_size != 1 || sync_byte != 0xA5 {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "MHAS direct input currently requires a leading sync packet with marker 0xA5"
                            .to_string(),
                });
            }
        }

        match header.packet_type {
            1 => {
                let payload = read_mhas_packet_payload_sync(
                    &mut file,
                    payload_offset,
                    header.payload_size,
                    spec,
                    "MHAS config packet payload is truncated",
                )?;
                let parsed = parse_mhas_config_packet(&payload, spec)?;
                if let Some(existing) = &config {
                    if existing != &parsed {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "MHAS input changed profile, sample rate, frame length, channel layout, or configuration bytes mid-stream"
                                    .to_string(),
                        });
                    }
                } else {
                    config = Some(parsed);
                }
            }
            2 => {
                let current_config =
                    config
                        .as_ref()
                        .ok_or_else(|| MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message: "MHAS frame packet appeared before any configuration packet"
                                .to_string(),
                        })?;
                let is_sync_sample = read_mhas_frame_sap_sync(&mut file, payload_offset, spec)?;
                let data_size = u32::try_from(packet_end - sample_start)
                    .map_err(|_| MuxError::LayoutOverflow("MHAS access unit size"))?;
                samples.push(StagedSample {
                    data_offset: sample_start,
                    data_size,
                    duration: current_config.frame_length,
                    composition_time_offset: 0,
                    is_sync_sample: is_sync_sample && samples.is_empty(),
                });
                sample_start = packet_end;
                saw_frame = true;
            }
            17 => {
                let payload = read_mhas_packet_payload_sync(
                    &mut file,
                    payload_offset,
                    header.payload_size,
                    spec,
                    "MHAS truncation packet payload is truncated",
                )?;
                parse_mhas_truncation_packet(&payload, spec)?;
            }
            _ => {}
        }
        offset = packet_end;
    }

    finalize_mhas_track(spec, config, samples, sample_start, saw_frame, offset)
}

pub(in crate::mux) fn scan_mhas_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedMhasTrack, MuxError> {
    parse_mhas_segmented_stream_sync(file, segments, total_size, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_mhas_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedMhasTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    if file_size < 3 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MHAS input is truncated before the required leading sync packet".to_string(),
        });
    }

    let mut offset = 0_u64;
    let mut sample_start = 0_u64;
    let mut config = None::<ParsedMhasConfig>;
    let mut saw_frame = false;
    let mut samples = Vec::new();
    while offset < file_size {
        let header = read_mhas_packet_header_async(&mut file, file_size, offset, spec).await?;
        let payload_offset = offset
            .checked_add(header.header_size)
            .ok_or(MuxError::LayoutOverflow("MHAS payload offset"))?;
        let packet_end = payload_offset
            .checked_add(header.payload_size)
            .ok_or(MuxError::LayoutOverflow("MHAS packet range"))?;
        if packet_end > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("MHAS packet at byte offset {offset} overruns the input length"),
            });
        }
        if header.packet_type > 18 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "MHAS packet at byte offset {offset} used unsupported packet type {}",
                    header.packet_type
                ),
            });
        }
        if header.payload_size == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("MHAS packet at byte offset {offset} declared a zero payload"),
            });
        }
        if offset == 0 {
            let sync_byte = read_mhas_sync_marker_async(&mut file, payload_offset, spec).await?;
            if header.packet_type != 6 || header.payload_size != 1 || sync_byte != 0xA5 {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "MHAS direct input currently requires a leading sync packet with marker 0xA5"
                            .to_string(),
                });
            }
        }

        match header.packet_type {
            1 => {
                let payload = read_mhas_packet_payload_async(
                    &mut file,
                    payload_offset,
                    header.payload_size,
                    spec,
                    "MHAS config packet payload is truncated",
                )
                .await?;
                let parsed = parse_mhas_config_packet(&payload, spec)?;
                if let Some(existing) = &config {
                    if existing != &parsed {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "MHAS input changed profile, sample rate, frame length, channel layout, or configuration bytes mid-stream"
                                    .to_string(),
                        });
                    }
                } else {
                    config = Some(parsed);
                }
            }
            2 => {
                let current_config =
                    config
                        .as_ref()
                        .ok_or_else(|| MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message: "MHAS frame packet appeared before any configuration packet"
                                .to_string(),
                        })?;
                let is_sync_sample =
                    read_mhas_frame_sap_async(&mut file, payload_offset, spec).await?;
                let data_size = u32::try_from(packet_end - sample_start)
                    .map_err(|_| MuxError::LayoutOverflow("MHAS access unit size"))?;
                samples.push(StagedSample {
                    data_offset: sample_start,
                    data_size,
                    duration: current_config.frame_length,
                    composition_time_offset: 0,
                    is_sync_sample: is_sync_sample && samples.is_empty(),
                });
                sample_start = packet_end;
                saw_frame = true;
            }
            17 => {
                let payload = read_mhas_packet_payload_async(
                    &mut file,
                    payload_offset,
                    header.payload_size,
                    spec,
                    "MHAS truncation packet payload is truncated",
                )
                .await?;
                parse_mhas_truncation_packet(&payload, spec)?;
            }
            _ => {}
        }
        offset = packet_end;
    }

    finalize_mhas_track(spec, config, samples, sample_start, saw_frame, offset)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_mhas_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedMhasTrack, MuxError> {
    parse_mhas_segmented_stream_async(file, segments, total_size, spec).await
}

fn parse_mhas_segmented_stream_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedMhasTrack, MuxError> {
    let mut offset = 0_u64;
    let mut sample_start = 0_u64;
    let mut config = None::<ParsedMhasConfig>;
    let mut saw_frame = false;
    let mut samples = Vec::new();
    while offset < total_size {
        let header =
            read_mhas_packet_header_segmented_sync(file, segments, total_size, offset, spec)?;
        let payload_offset = offset
            .checked_add(header.header_size)
            .ok_or(MuxError::LayoutOverflow("MHAS payload offset"))?;
        let packet_end = payload_offset
            .checked_add(header.payload_size)
            .ok_or(MuxError::LayoutOverflow("MHAS packet range"))?;
        if packet_end > total_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "MHAS packet at logical byte offset {offset} overruns the carried input length"
                ),
            });
        }
        if header.packet_type > 18 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "MHAS packet at logical byte offset {offset} used unsupported packet type {}",
                    header.packet_type
                ),
            });
        }
        if header.payload_size == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "MHAS packet at logical byte offset {offset} declared a zero payload"
                ),
            });
        }
        if offset == 0 {
            let sync_byte =
                read_mhas_sync_marker_segmented_sync(file, segments, payload_offset, spec)?;
            if header.packet_type != 6 || header.payload_size != 1 || sync_byte != 0xA5 {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "MHAS direct input currently requires a leading sync packet with marker 0xA5"
                            .to_string(),
                });
            }
        }

        match header.packet_type {
            1 => {
                let payload = read_mhas_packet_payload_segmented_sync(
                    file,
                    segments,
                    payload_offset,
                    header.payload_size,
                    spec,
                    "MHAS config packet payload is truncated",
                )?;
                let parsed = parse_mhas_config_packet(&payload, spec)?;
                if let Some(existing) = &config {
                    if existing != &parsed {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "MHAS input changed profile, sample rate, frame length, channel layout, or configuration bytes mid-stream"
                                    .to_string(),
                        });
                    }
                } else {
                    config = Some(parsed);
                }
            }
            2 => {
                let current_config =
                    config
                        .as_ref()
                        .ok_or_else(|| MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message: "MHAS frame packet appeared before any configuration packet"
                                .to_string(),
                        })?;
                let is_sync_sample =
                    read_mhas_frame_sap_segmented_sync(file, segments, payload_offset, spec)?;
                let data_size = u32::try_from(packet_end - sample_start)
                    .map_err(|_| MuxError::LayoutOverflow("MHAS access unit size"))?;
                samples.push(StagedSample {
                    data_offset: sample_start,
                    data_size,
                    duration: current_config.frame_length,
                    composition_time_offset: 0,
                    is_sync_sample: is_sync_sample && samples.is_empty(),
                });
                sample_start = packet_end;
                saw_frame = true;
            }
            17 => {
                let payload = read_mhas_packet_payload_segmented_sync(
                    file,
                    segments,
                    payload_offset,
                    header.payload_size,
                    spec,
                    "MHAS truncation packet payload is truncated",
                )?;
                parse_mhas_truncation_packet(&payload, spec)?;
            }
            _ => {}
        }
        offset = packet_end;
    }

    finalize_mhas_track(spec, config, samples, sample_start, saw_frame, offset)
}

#[cfg(feature = "async")]
async fn parse_mhas_segmented_stream_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedMhasTrack, MuxError> {
    let mut offset = 0_u64;
    let mut sample_start = 0_u64;
    let mut config = None::<ParsedMhasConfig>;
    let mut saw_frame = false;
    let mut samples = Vec::new();
    while offset < total_size {
        let header =
            read_mhas_packet_header_segmented_async(file, segments, total_size, offset, spec)
                .await?;
        let payload_offset = offset
            .checked_add(header.header_size)
            .ok_or(MuxError::LayoutOverflow("MHAS payload offset"))?;
        let packet_end = payload_offset
            .checked_add(header.payload_size)
            .ok_or(MuxError::LayoutOverflow("MHAS packet range"))?;
        if packet_end > total_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "MHAS packet at logical byte offset {offset} overruns the carried input length"
                ),
            });
        }
        if header.packet_type > 18 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "MHAS packet at logical byte offset {offset} used unsupported packet type {}",
                    header.packet_type
                ),
            });
        }
        if header.payload_size == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "MHAS packet at logical byte offset {offset} declared a zero payload"
                ),
            });
        }
        if offset == 0 {
            let sync_byte =
                read_mhas_sync_marker_segmented_async(file, segments, payload_offset, spec).await?;
            if header.packet_type != 6 || header.payload_size != 1 || sync_byte != 0xA5 {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "MHAS direct input currently requires a leading sync packet with marker 0xA5"
                            .to_string(),
                });
            }
        }

        match header.packet_type {
            1 => {
                let payload = read_mhas_packet_payload_segmented_async(
                    file,
                    segments,
                    payload_offset,
                    header.payload_size,
                    spec,
                    "MHAS config packet payload is truncated",
                )
                .await?;
                let parsed = parse_mhas_config_packet(&payload, spec)?;
                if let Some(existing) = &config {
                    if existing != &parsed {
                        return Err(MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message:
                                "MHAS input changed profile, sample rate, frame length, channel layout, or configuration bytes mid-stream"
                                    .to_string(),
                        });
                    }
                } else {
                    config = Some(parsed);
                }
            }
            2 => {
                let current_config =
                    config
                        .as_ref()
                        .ok_or_else(|| MuxError::UnsupportedTrackImport {
                            spec: spec.to_string(),
                            message: "MHAS frame packet appeared before any configuration packet"
                                .to_string(),
                        })?;
                let is_sync_sample =
                    read_mhas_frame_sap_segmented_async(file, segments, payload_offset, spec)
                        .await?;
                let data_size = u32::try_from(packet_end - sample_start)
                    .map_err(|_| MuxError::LayoutOverflow("MHAS access unit size"))?;
                samples.push(StagedSample {
                    data_offset: sample_start,
                    data_size,
                    duration: current_config.frame_length,
                    composition_time_offset: 0,
                    is_sync_sample: is_sync_sample && samples.is_empty(),
                });
                sample_start = packet_end;
                saw_frame = true;
            }
            17 => {
                let payload = read_mhas_packet_payload_segmented_async(
                    file,
                    segments,
                    payload_offset,
                    header.payload_size,
                    spec,
                    "MHAS truncation packet payload is truncated",
                )
                .await?;
                parse_mhas_truncation_packet(&payload, spec)?;
            }
            _ => {}
        }
        offset = packet_end;
    }

    finalize_mhas_track(spec, config, samples, sample_start, saw_frame, offset)
}

fn finalize_mhas_track(
    spec: &str,
    config: Option<ParsedMhasConfig>,
    samples: Vec<StagedSample>,
    sample_start: u64,
    saw_frame: bool,
    final_offset: u64,
) -> Result<ParsedMhasTrack, MuxError> {
    let config = config.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "MHAS input did not contain a required configuration packet".to_string(),
    })?;
    if !saw_frame || samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MHAS input did not contain any frame packets".to_string(),
        });
    }
    if sample_start != final_offset {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MHAS input ended with non-frame packets after the last frame packet"
                .to_string(),
        });
    }
    let btrt = build_btrt_from_sample_sizes(
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        config.sample_rate,
    )?;
    Ok(ParsedMhasTrack {
        sample_rate: config.sample_rate,
        sample_entry_box: build_mhas_sample_entry_box(&config, btrt)?,
        samples,
    })
}

fn build_mhas_sample_entry_box(config: &ParsedMhasConfig, btrt: Btrt) -> Result<Vec<u8>, MuxError> {
    build_mhas_sample_entry_box_with_btrt(config.sample_rate, btrt)
}

pub(in crate::mux) fn build_mhas_sample_entry_box_with_btrt(
    sample_rate: u32,
    btrt: Btrt,
) -> Result<Vec<u8>, MuxError> {
    let btrt_bytes = super::super::mp4::encode_typed_box(&btrt, &[])?;
    build_generic_audio_sample_entry_box(MHM1, sample_rate, 0, 16, &[btrt_bytes])
}

fn read_mhas_packet_header_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<MhasPacketHeader, MuxError> {
    let available = usize::try_from((file_size - offset).min(15))
        .map_err(|_| MuxError::LayoutOverflow("MHAS header probe size"))?;
    let mut header = vec![0_u8; available];
    read_exact_at_sync(
        file,
        offset,
        &mut header,
        spec,
        "MHAS packet header is truncated",
    )?;
    parse_mhas_packet_header(&header, spec)
}

fn read_mhas_packet_header_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    offset: u64,
    spec: &str,
) -> Result<MhasPacketHeader, MuxError> {
    let available = usize::try_from((total_size - offset).min(15))
        .map_err(|_| MuxError::LayoutOverflow("MHAS header probe size"))?;
    let mut header = vec![0_u8; available];
    read_segmented_bytes_sync(
        file,
        segments,
        total_size,
        offset,
        &mut header,
        spec,
        "MHAS packet header is truncated",
    )?;
    parse_mhas_packet_header(&header, spec)
}

#[cfg(feature = "async")]
async fn read_mhas_packet_header_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<MhasPacketHeader, MuxError> {
    let available = usize::try_from((file_size - offset).min(15))
        .map_err(|_| MuxError::LayoutOverflow("MHAS header probe size"))?;
    let mut header = vec![0_u8; available];
    read_exact_at_async(
        file,
        offset,
        &mut header,
        spec,
        "MHAS packet header is truncated",
    )
    .await?;
    parse_mhas_packet_header(&header, spec)
}

#[cfg(feature = "async")]
async fn read_mhas_packet_header_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    offset: u64,
    spec: &str,
) -> Result<MhasPacketHeader, MuxError> {
    let available = usize::try_from((total_size - offset).min(15))
        .map_err(|_| MuxError::LayoutOverflow("MHAS header probe size"))?;
    let mut header = vec![0_u8; available];
    read_segmented_bytes_async(
        file,
        segments,
        total_size,
        offset,
        &mut header,
        spec,
        "MHAS packet header is truncated",
    )
    .await?;
    parse_mhas_packet_header(&header, spec)
}

fn parse_mhas_packet_header(header: &[u8], spec: &str) -> Result<MhasPacketHeader, MuxError> {
    let mut reader = MhasBitCursor::new(header);
    let packet_type =
        u32::try_from(reader.read_escaped_value(3, 8, 8, spec, "MHAS packet type")?)
            .map_err(|_| MuxError::LayoutOverflow("MHAS packet type"))?;
    let _label = reader.read_escaped_value(2, 8, 32, spec, "MHAS packet label")?;
    let payload_size = reader.read_escaped_value(11, 24, 24, spec, "MHAS packet size")?;
    Ok(MhasPacketHeader {
        packet_type,
        payload_size,
        header_size: u64::try_from(reader.bytes_consumed())
            .map_err(|_| MuxError::LayoutOverflow("MHAS header size"))?,
    })
}

fn read_mhas_packet_payload_sync(
    file: &mut File,
    offset: u64,
    size: u64,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let len =
        usize::try_from(size).map_err(|_| MuxError::LayoutOverflow("MHAS packet payload size"))?;
    let mut payload = vec![0_u8; len];
    read_exact_at_sync(file, offset, &mut payload, spec, truncated_message)?;
    Ok(payload)
}

fn read_mhas_packet_payload_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    offset: u64,
    size: u64,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let len =
        usize::try_from(size).map_err(|_| MuxError::LayoutOverflow("MHAS packet payload size"))?;
    let mut payload = vec![0_u8; len];
    read_segmented_bytes_sync(
        file,
        segments,
        u64::try_from(len)
            .map_err(|_| MuxError::LayoutOverflow("MHAS packet payload size"))?
            .checked_add(offset)
            .ok_or(MuxError::LayoutOverflow("MHAS packet payload range"))?,
        offset,
        &mut payload,
        spec,
        truncated_message,
    )?;
    Ok(payload)
}

#[cfg(feature = "async")]
async fn read_mhas_packet_payload_async(
    file: &mut TokioFile,
    offset: u64,
    size: u64,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let len =
        usize::try_from(size).map_err(|_| MuxError::LayoutOverflow("MHAS packet payload size"))?;
    let mut payload = vec![0_u8; len];
    read_exact_at_async(file, offset, &mut payload, spec, truncated_message).await?;
    Ok(payload)
}

#[cfg(feature = "async")]
async fn read_mhas_packet_payload_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    offset: u64,
    size: u64,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let len =
        usize::try_from(size).map_err(|_| MuxError::LayoutOverflow("MHAS packet payload size"))?;
    let mut payload = vec![0_u8; len];
    read_segmented_bytes_async(
        file,
        segments,
        u64::try_from(len)
            .map_err(|_| MuxError::LayoutOverflow("MHAS packet payload size"))?
            .checked_add(offset)
            .ok_or(MuxError::LayoutOverflow("MHAS packet payload range"))?,
        offset,
        &mut payload,
        spec,
        truncated_message,
    )
    .await?;
    Ok(payload)
}

fn read_mhas_sync_marker_sync(file: &mut File, offset: u64, spec: &str) -> Result<u8, MuxError> {
    let mut marker = [0_u8; 1];
    read_exact_at_sync(
        file,
        offset,
        &mut marker,
        spec,
        "MHAS sync payload is truncated",
    )?;
    Ok(marker[0])
}

fn read_mhas_sync_marker_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    offset: u64,
    spec: &str,
) -> Result<u8, MuxError> {
    let mut marker = [0_u8; 1];
    read_segmented_bytes_sync(
        file,
        segments,
        offset
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("MHAS sync payload range"))?,
        offset,
        &mut marker,
        spec,
        "MHAS sync payload is truncated",
    )?;
    Ok(marker[0])
}

#[cfg(feature = "async")]
async fn read_mhas_sync_marker_async(
    file: &mut TokioFile,
    offset: u64,
    spec: &str,
) -> Result<u8, MuxError> {
    let mut marker = [0_u8; 1];
    read_exact_at_async(
        file,
        offset,
        &mut marker,
        spec,
        "MHAS sync payload is truncated",
    )
    .await?;
    Ok(marker[0])
}

#[cfg(feature = "async")]
async fn read_mhas_sync_marker_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    offset: u64,
    spec: &str,
) -> Result<u8, MuxError> {
    let mut marker = [0_u8; 1];
    read_segmented_bytes_async(
        file,
        segments,
        offset
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("MHAS sync payload range"))?,
        offset,
        &mut marker,
        spec,
        "MHAS sync payload is truncated",
    )
    .await?;
    Ok(marker[0])
}

fn read_mhas_frame_sap_sync(file: &mut File, offset: u64, spec: &str) -> Result<bool, MuxError> {
    let mut byte = [0_u8; 1];
    read_exact_at_sync(
        file,
        offset,
        &mut byte,
        spec,
        "MHAS frame payload is truncated before the SAP flag",
    )?;
    Ok(byte[0] & 0x80 != 0)
}

fn read_mhas_frame_sap_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    offset: u64,
    spec: &str,
) -> Result<bool, MuxError> {
    let mut byte = [0_u8; 1];
    read_segmented_bytes_sync(
        file,
        segments,
        offset
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("MHAS frame SAP range"))?,
        offset,
        &mut byte,
        spec,
        "MHAS frame payload is truncated before the SAP flag",
    )?;
    Ok(byte[0] & 0x80 != 0)
}

#[cfg(feature = "async")]
async fn read_mhas_frame_sap_async(
    file: &mut TokioFile,
    offset: u64,
    spec: &str,
) -> Result<bool, MuxError> {
    let mut byte = [0_u8; 1];
    read_exact_at_async(
        file,
        offset,
        &mut byte,
        spec,
        "MHAS frame payload is truncated before the SAP flag",
    )
    .await?;
    Ok(byte[0] & 0x80 != 0)
}

#[cfg(feature = "async")]
async fn read_mhas_frame_sap_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    offset: u64,
    spec: &str,
) -> Result<bool, MuxError> {
    let mut byte = [0_u8; 1];
    read_segmented_bytes_async(
        file,
        segments,
        offset
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("MHAS frame SAP range"))?,
        offset,
        &mut byte,
        spec,
        "MHAS frame payload is truncated before the SAP flag",
    )
    .await?;
    Ok(byte[0] & 0x80 != 0)
}

fn parse_mhas_config_packet(payload: &[u8], spec: &str) -> Result<ParsedMhasConfig, MuxError> {
    let mut reader = MhasBitCursor::new(payload);
    let _profile_level =
        u8::try_from(reader.read_bits(8, spec, "MHAS profile-level indication")?).unwrap();
    let sample_rate_index =
        usize::try_from(reader.read_bits(5, spec, "MHAS sample-rate index")?).unwrap();
    let sample_rate = if sample_rate_index == 0x1F {
        u32::try_from(reader.read_bits(24, spec, "MHAS explicit sample rate")?)
            .map_err(|_| MuxError::LayoutOverflow("MHAS explicit sample rate"))?
    } else {
        let value = *MHAS_SAMPLE_RATE_TABLE
            .get(sample_rate_index)
            .ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "MHAS configuration used unsupported sample-rate index {sample_rate_index}"
                ),
            })?;
        if value == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "MHAS configuration used reserved sample-rate index {sample_rate_index}"
                ),
            });
        }
        value
    };
    let frame_length = match reader.read_bits(3, spec, "MHAS frame-length index")? {
        0 | 2 => 768,
        _ => 1024,
    };
    let _core_sbr_flag = reader.read_bool(spec, "MHAS core-SBR flag")?;
    let _resilient_flag = reader.read_bool(spec, "MHAS resilient flag")?;
    let speaker_layout_type =
        u8::try_from(reader.read_bits(2, spec, "MHAS speaker-layout type")?).unwrap();
    let (_reference_channel_layout, channel_count) = if speaker_layout_type == 0 {
        let cicp_layout =
            u8::try_from(reader.read_bits(6, spec, "MHAS CICP speaker layout")?).unwrap();
        (
            cicp_layout,
            mhas_channel_count_from_cicp(cicp_layout, spec)?,
        )
    } else {
        let count =
            reader.read_escaped_value(5, 8, 16, spec, "MHAS explicit speaker count minus one")?;
        let count = count
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("MHAS explicit speaker count"))?;
        let channel_count = u16::try_from(count)
            .map_err(|_| MuxError::LayoutOverflow("MHAS explicit speaker count"))?;
        (0xFF, channel_count)
    };
    if sample_rate == 0 || channel_count == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MHAS configuration did not yield a usable sample rate or channel count"
                .to_string(),
        });
    }
    Ok(ParsedMhasConfig {
        sample_rate,
        frame_length,
        channel_count,
    })
}

fn parse_mhas_truncation_packet(payload: &[u8], spec: &str) -> Result<(), MuxError> {
    let mut reader = MhasBitCursor::new(payload);
    let is_active = reader.read_bool(spec, "MHAS truncation active flag")?;
    let _reserved = reader.read_bool(spec, "MHAS truncation reserved flag")?;
    let _trunc_from_begin = reader.read_bool(spec, "MHAS truncation direction flag")?;
    let truncated_samples =
        u16::try_from(reader.read_bits(13, spec, "MHAS truncated sample count")?).unwrap();
    if is_active && truncated_samples != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MHAS truncation packets with active sample trimming are not supported yet"
                .to_string(),
        });
    }
    Ok(())
}

fn mhas_channel_count_from_cicp(cicp_layout: u8, spec: &str) -> Result<u16, MuxError> {
    let count = match cicp_layout {
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 5,
        6 => 6,
        7 => 8,
        8 => 2,
        9 => 3,
        10 => 4,
        11 => 7,
        12 => 8,
        13 => 24,
        14 => 8,
        15 => 12,
        16 => 10,
        17 => 12,
        18 => 14,
        19 => 10,
        20 => 14,
        _ => {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "MHAS configuration used unsupported CICP speaker layout {cicp_layout}"
                ),
            });
        }
    };
    Ok(count)
}
