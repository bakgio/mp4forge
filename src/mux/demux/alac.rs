use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::AnyTypeBox;
use crate::boxes::iso14496_12::{AudioSampleEntry, Btrt, SampleEntry};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{StagedSample, build_btrt_from_sample_sizes, read_exact_at_sync};
#[cfg(feature = "async")]
use super::caf_common::read_caf_chunk_header_async;
use super::caf_common::read_caf_chunk_header_sync;

const ALAC: FourCc = FourCc::from_bytes(*b"alac");

pub(in crate::mux) struct ParsedCafAlacTrack {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

struct ParsedCafDescription {
    sample_rate: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
}

struct ParsedCafPacketTable {
    number_packets: u64,
    number_valid_frames: u64,
    priming_frames: u32,
    remainder_frames: u32,
    packet_sizes: Vec<u32>,
}

struct ParsedAlacCookieConfig {
    frame_length: u32,
    bit_depth: u16,
    channel_count: u16,
    sample_entry_payload: Vec<u8>,
}

pub(in crate::mux) fn scan_caf_alac_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedCafAlacTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut description = None;
    let mut cookie = None;
    let mut data_chunk = None;
    let mut packet_table = None;
    let mut offset = 8_u64;
    while offset < file_size {
        let (chunk_type, chunk_size) = read_caf_chunk_header_sync(&mut file, offset, spec)?;
        let chunk_data_offset = offset + 12;
        let chunk_end = chunk_data_offset
            .checked_add(chunk_size)
            .ok_or(MuxError::LayoutOverflow("CAF chunk range"))?;
        if chunk_end > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("CAF chunk `{chunk_type}` overruns the input length"),
            });
        }
        match chunk_type {
            value if value == FourCc::from_bytes(*b"desc") => {
                description = Some(parse_caf_description_chunk_sync(
                    &mut file,
                    chunk_data_offset,
                    chunk_size,
                    spec,
                )?);
            }
            value if value == FourCc::from_bytes(*b"kuki") => {
                let mut bytes = vec![0_u8; usize::try_from(chunk_size).unwrap()];
                read_exact_at_sync(
                    &mut file,
                    chunk_data_offset,
                    &mut bytes,
                    spec,
                    "CAF `kuki` chunk is truncated",
                )?;
                cookie = Some(bytes);
            }
            value if value == FourCc::from_bytes(*b"data") => {
                data_chunk = Some((chunk_data_offset, chunk_size));
            }
            value if value == FourCc::from_bytes(*b"pakt") => {
                packet_table = Some(parse_caf_packet_table_sync(
                    &mut file,
                    chunk_data_offset,
                    chunk_size,
                    spec,
                )?);
            }
            _ => {}
        }
        offset = chunk_end;
    }
    finalize_caf_alac_track(spec, description, cookie, data_chunk, packet_table)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_caf_alac_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedCafAlacTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut description = None;
    let mut cookie = None;
    let mut data_chunk = None;
    let mut packet_table = None;
    let mut offset = 8_u64;
    while offset < file_size {
        let (chunk_type, chunk_size) = read_caf_chunk_header_async(&mut file, offset, spec).await?;
        let chunk_data_offset = offset + 12;
        let chunk_end = chunk_data_offset
            .checked_add(chunk_size)
            .ok_or(MuxError::LayoutOverflow("CAF chunk range"))?;
        if chunk_end > file_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("CAF chunk `{chunk_type}` overruns the input length"),
            });
        }
        match chunk_type {
            value if value == FourCc::from_bytes(*b"desc") => {
                description = Some(
                    parse_caf_description_chunk_async(
                        &mut file,
                        chunk_data_offset,
                        chunk_size,
                        spec,
                    )
                    .await?,
                );
            }
            value if value == FourCc::from_bytes(*b"kuki") => {
                let mut bytes = vec![0_u8; usize::try_from(chunk_size).unwrap()];
                read_exact_at_async(
                    &mut file,
                    chunk_data_offset,
                    &mut bytes,
                    spec,
                    "CAF `kuki` chunk is truncated",
                )
                .await?;
                cookie = Some(bytes);
            }
            value if value == FourCc::from_bytes(*b"data") => {
                data_chunk = Some((chunk_data_offset, chunk_size));
            }
            value if value == FourCc::from_bytes(*b"pakt") => {
                packet_table = Some(
                    parse_caf_packet_table_async(&mut file, chunk_data_offset, chunk_size, spec)
                        .await?,
                );
            }
            _ => {}
        }
        offset = chunk_end;
    }
    finalize_caf_alac_track(spec, description, cookie, data_chunk, packet_table)
}

fn parse_caf_description_chunk_sync(
    file: &mut File,
    offset: u64,
    chunk_size: u64,
    spec: &str,
) -> Result<ParsedCafDescription, MuxError> {
    if chunk_size < 32 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF `desc` chunk is shorter than the required 32-byte payload".to_string(),
        });
    }
    let mut bytes = [0_u8; 32];
    read_exact_at_sync(
        file,
        offset,
        &mut bytes,
        spec,
        "CAF `desc` chunk is truncated",
    )?;
    parse_caf_description_chunk_bytes(&bytes, spec)
}

#[cfg(feature = "async")]
async fn parse_caf_description_chunk_async(
    file: &mut TokioFile,
    offset: u64,
    chunk_size: u64,
    spec: &str,
) -> Result<ParsedCafDescription, MuxError> {
    if chunk_size < 32 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF `desc` chunk is shorter than the required 32-byte payload".to_string(),
        });
    }
    let mut bytes = [0_u8; 32];
    read_exact_at_async(
        file,
        offset,
        &mut bytes,
        spec,
        "CAF `desc` chunk is truncated",
    )
    .await?;
    parse_caf_description_chunk_bytes(&bytes, spec)
}

fn parse_caf_description_chunk_bytes(
    bytes: &[u8; 32],
    spec: &str,
) -> Result<ParsedCafDescription, MuxError> {
    let sample_rate_f64 = f64::from_bits(u64::from_be_bytes(bytes[..8].try_into().unwrap()));
    if !sample_rate_f64.is_finite() || sample_rate_f64 <= 0.0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF `desc` chunk carried an invalid sample rate".to_string(),
        });
    }
    let sample_rate = sample_rate_f64.round() as u32;
    let format_id = FourCc::from_bytes(bytes[8..12].try_into().unwrap());
    if format_id != ALAC {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "CAF `desc` chunk used unsupported format `{format_id}`; only `alac` is supported"
            ),
        });
    }
    let bytes_per_packet = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let frames_per_packet = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    let channels_per_frame = u32::from_be_bytes(bytes[24..28].try_into().unwrap());
    let bits_per_channel = u32::from_be_bytes(bytes[28..32].try_into().unwrap());
    Ok(ParsedCafDescription {
        sample_rate,
        bytes_per_packet,
        frames_per_packet,
        channels_per_frame,
        bits_per_channel,
    })
}

fn parse_caf_packet_table_sync(
    file: &mut File,
    offset: u64,
    chunk_size: u64,
    spec: &str,
) -> Result<ParsedCafPacketTable, MuxError> {
    if chunk_size < 24 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF `pakt` chunk is shorter than the required 24-byte header".to_string(),
        });
    }
    let mut bytes = vec![0_u8; usize::try_from(chunk_size).unwrap()];
    read_exact_at_sync(
        file,
        offset,
        &mut bytes,
        spec,
        "CAF `pakt` chunk is truncated",
    )?;
    parse_caf_packet_table_bytes(&bytes, spec)
}

#[cfg(feature = "async")]
async fn parse_caf_packet_table_async(
    file: &mut TokioFile,
    offset: u64,
    chunk_size: u64,
    spec: &str,
) -> Result<ParsedCafPacketTable, MuxError> {
    if chunk_size < 24 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF `pakt` chunk is shorter than the required 24-byte header".to_string(),
        });
    }
    let mut bytes = vec![0_u8; usize::try_from(chunk_size).unwrap()];
    read_exact_at_async(
        file,
        offset,
        &mut bytes,
        spec,
        "CAF `pakt` chunk is truncated",
    )
    .await?;
    parse_caf_packet_table_bytes(&bytes, spec)
}

fn parse_caf_packet_table_bytes(
    bytes: &[u8],
    spec: &str,
) -> Result<ParsedCafPacketTable, MuxError> {
    let number_packets = u64::from_be_bytes(bytes[..8].try_into().unwrap());
    let number_valid_frames = u64::from_be_bytes(bytes[8..16].try_into().unwrap());
    let priming_frames = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let remainder_frames = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    let packet_sizes = decode_caf_packet_sizes(&bytes[24..], number_packets, spec)?;
    Ok(ParsedCafPacketTable {
        number_packets,
        number_valid_frames,
        priming_frames,
        remainder_frames,
        packet_sizes,
    })
}

fn decode_caf_packet_sizes(
    bytes: &[u8],
    number_packets: u64,
    spec: &str,
) -> Result<Vec<u32>, MuxError> {
    let packet_count =
        usize::try_from(number_packets).map_err(|_| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF `pakt` packet count exceeds the supported in-memory sample table size"
                .to_string(),
        })?;
    let mut sizes = Vec::with_capacity(packet_count);
    let mut cursor = 0usize;
    while sizes.len() < packet_count {
        if cursor >= bytes.len() {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "CAF `pakt` packet-size table is truncated".to_string(),
            });
        }
        let mut value = 0_u64;
        loop {
            let byte = bytes[cursor];
            cursor += 1;
            value = (value << 7) | u64::from(byte & 0x7F);
            if byte & 0x80 == 0 {
                break;
            }
            if cursor >= bytes.len() {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "CAF `pakt` packet-size table ended in the middle of a variable-length integer".to_string(),
                });
            }
        }
        sizes.push(
            u32::try_from(value).map_err(|_| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "CAF `pakt` packet size does not fit in the current mux sample model"
                    .to_string(),
            })?,
        );
    }
    if bytes[cursor..].iter().any(|byte| *byte != 0) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF `pakt` packet-size table carried unexpected trailing bytes".to_string(),
        });
    }
    Ok(sizes)
}

fn finalize_caf_alac_track(
    spec: &str,
    description: Option<ParsedCafDescription>,
    cookie: Option<Vec<u8>>,
    data_chunk: Option<(u64, u64)>,
    packet_table: Option<ParsedCafPacketTable>,
) -> Result<ParsedCafAlacTrack, MuxError> {
    let mut description = description.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "CAF input did not contain a required `desc` chunk".to_string(),
    })?;
    let cookie = cookie.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "CAF ALAC input did not contain a required `kuki` chunk".to_string(),
    })?;
    let (data_offset, chunk_size) = data_chunk.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "CAF ALAC input did not contain a required `data` chunk".to_string(),
    })?;
    let parsed_cookie = parse_alac_cookie(&cookie, spec)?;
    if description.frames_per_packet == 0 {
        description.frames_per_packet = parsed_cookie.frame_length;
    }
    if description.channels_per_frame == 0 {
        description.channels_per_frame = u32::from(parsed_cookie.channel_count);
    }
    if description.bits_per_channel == 0 {
        description.bits_per_channel = u32::from(parsed_cookie.bit_depth);
    }
    if description.frames_per_packet == 0
        || description.channels_per_frame == 0
        || description.bits_per_channel == 0
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "CAF ALAC input did not carry enough non-zero audio parameters in `desc` or `kuki`"
                    .to_string(),
        });
    }
    if chunk_size < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF `data` chunk is too short to include the edit-count field".to_string(),
        });
    }
    let payload_offset = data_offset + 4;
    let payload_size = chunk_size - 4;
    if payload_size == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF ALAC `data` chunk did not contain any encoded packet payload".to_string(),
        });
    }
    let channel_count = u16::try_from(description.channels_per_frame).map_err(|_| {
        MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF ALAC channel count does not fit in the current MP4 sample-entry model"
                .to_string(),
        }
    })?;
    let sample_size_bits = u16::try_from(description.bits_per_channel).map_err(|_| {
        MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF ALAC bits-per-channel does not fit in the current MP4 sample-entry model"
                .to_string(),
        }
    })?;
    let samples = if description.bytes_per_packet != 0 {
        build_fixed_packet_alac_samples(spec, payload_offset, payload_size, &description)?
    } else {
        build_variable_packet_alac_samples(
            spec,
            payload_offset,
            payload_size,
            &description,
            packet_table.as_ref(),
        )?
    };
    let btrt = build_btrt_from_sample_sizes(
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        description.sample_rate,
    )?;
    let sample_entry_box = build_alac_sample_entry_box(
        description.sample_rate,
        channel_count,
        sample_size_bits,
        &parsed_cookie.sample_entry_payload,
        btrt,
    )?;
    Ok(ParsedCafAlacTrack {
        sample_rate: description.sample_rate,
        sample_entry_box,
        samples,
    })
}

fn build_fixed_packet_alac_samples(
    spec: &str,
    payload_offset: u64,
    payload_size: u64,
    description: &ParsedCafDescription,
) -> Result<Vec<StagedSample>, MuxError> {
    if !payload_size.is_multiple_of(u64::from(description.bytes_per_packet)) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "CAF ALAC `data` chunk size is not a whole-number multiple of `bytes_per_packet`"
                    .to_string(),
        });
    }
    let packet_count = payload_size / u64::from(description.bytes_per_packet);
    let packet_count_u32 =
        u32::try_from(packet_count).map_err(|_| MuxError::LayoutOverflow("CAF packet count"))?;
    let mut samples = Vec::with_capacity(usize::try_from(packet_count).unwrap_or(0));
    for index in 0..packet_count_u32 {
        let packet_offset = payload_offset
            .checked_add(u64::from(index) * u64::from(description.bytes_per_packet))
            .ok_or(MuxError::LayoutOverflow("CAF packet offset"))?;
        samples.push(StagedSample {
            data_offset: packet_offset,
            data_size: description.bytes_per_packet,
            duration: description.frames_per_packet,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    Ok(samples)
}

fn build_variable_packet_alac_samples(
    spec: &str,
    payload_offset: u64,
    payload_size: u64,
    description: &ParsedCafDescription,
    packet_table: Option<&ParsedCafPacketTable>,
) -> Result<Vec<StagedSample>, MuxError> {
    let packet_table = packet_table.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message:
            "CAF ALAC input used variable packet sizing but did not provide a required `pakt` chunk"
                .to_string(),
    })?;
    if packet_table.priming_frames != 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "CAF ALAC `pakt` chunk declared priming frames; encoder-delay trimming is not landed yet"
                    .to_string(),
        });
    }
    if packet_table.remainder_frames >= description.frames_per_packet
        && description.frames_per_packet != 0
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "CAF ALAC `pakt` chunk declared a remainder frame count that is not smaller than `frames_per_packet`"
                    .to_string(),
        });
    }
    if usize::try_from(packet_table.number_packets).ok() != Some(packet_table.packet_sizes.len()) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "CAF ALAC `pakt` packet count did not match the number of decoded packet sizes"
                    .to_string(),
        });
    }
    let total_size: u64 = packet_table
        .packet_sizes
        .iter()
        .map(|size| u64::from(*size))
        .sum();
    if total_size != payload_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "CAF ALAC packet-table sizes did not add up to the `data` chunk payload length"
                    .to_string(),
        });
    }
    let mut packet_offset = payload_offset;
    let mut samples = Vec::with_capacity(packet_table.packet_sizes.len());
    let packet_count = u64::try_from(packet_table.packet_sizes.len()).unwrap();
    for (index, packet_size) in packet_table.packet_sizes.iter().copied().enumerate() {
        let index_u64 = u64::try_from(index).unwrap();
        let duration = if index_u64 + 1 == packet_count {
            derive_last_packet_duration(spec, packet_table, description.frames_per_packet)?
        } else {
            description.frames_per_packet
        };
        samples.push(StagedSample {
            data_offset: packet_offset,
            data_size: packet_size,
            duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        packet_offset = packet_offset
            .checked_add(u64::from(packet_size))
            .ok_or(MuxError::LayoutOverflow("CAF packet offset"))?;
    }
    Ok(samples)
}

fn derive_last_packet_duration(
    spec: &str,
    packet_table: &ParsedCafPacketTable,
    frames_per_packet: u32,
) -> Result<u32, MuxError> {
    if packet_table.number_packets == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF ALAC `pakt` chunk declared zero packets".to_string(),
        });
    }
    if packet_table.remainder_frames != 0 {
        return Ok(packet_table.remainder_frames);
    }
    if packet_table.number_packets == 1 {
        return u32::try_from(packet_table.number_valid_frames)
            .map_err(|_| MuxError::LayoutOverflow("CAF valid frame count"));
    }
    let preceding_frames = u64::from(frames_per_packet)
        .checked_mul(packet_table.number_packets.saturating_sub(1))
        .ok_or(MuxError::LayoutOverflow("CAF preceding packet frames"))?;
    let remaining = packet_table
        .number_valid_frames
        .saturating_sub(preceding_frames);
    if remaining == 0 {
        Ok(frames_per_packet)
    } else {
        u32::try_from(remaining)
            .map_err(|_| MuxError::LayoutOverflow("CAF trailing packet duration"))
    }
}

fn parse_alac_cookie(cookie: &[u8], spec: &str) -> Result<ParsedAlacCookieConfig, MuxError> {
    let sample_entry_payload = extract_alac_sample_entry_payload(cookie, spec)?;
    if sample_entry_payload.len() < 28 {
        return Ok(ParsedAlacCookieConfig {
            frame_length: 0,
            bit_depth: 0,
            channel_count: 0,
            sample_entry_payload,
        });
    }
    let config = if sample_entry_payload.len() == 28 {
        &sample_entry_payload[..]
    } else {
        &sample_entry_payload[sample_entry_payload.len() - 28..]
    };
    Ok(ParsedAlacCookieConfig {
        frame_length: u32::from_be_bytes(config[4..8].try_into().unwrap()),
        bit_depth: u16::from(config[9]),
        channel_count: u16::from(config[13]),
        sample_entry_payload,
    })
}

fn extract_alac_sample_entry_payload(cookie: &[u8], spec: &str) -> Result<Vec<u8>, MuxError> {
    let mut offset = 0usize;
    let mut saw_box = false;
    while cookie.len().saturating_sub(offset) >= 8 {
        let size = u32::from_be_bytes(cookie[offset..offset + 4].try_into().unwrap()) as usize;
        if size < 8
            || offset
                .checked_add(size)
                .is_none_or(|end| end > cookie.len())
        {
            if !saw_box && offset == 0 {
                return Ok(cookie.to_vec());
            }
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "CAF ALAC `kuki` box layout is malformed".to_string(),
            });
        }
        saw_box = true;
        let box_type = FourCc::from_bytes(cookie[offset + 4..offset + 8].try_into().unwrap());
        if box_type == ALAC {
            return Ok(cookie[offset + 8..offset + size].to_vec());
        }
        offset += size;
    }
    if saw_box {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF ALAC `kuki` did not contain a required inner `alac` box".to_string(),
        });
    }
    Ok(cookie.to_vec())
}

fn build_alac_sample_entry_box(
    sample_rate: u32,
    channel_count: u16,
    sample_size: u16,
    cookie: &[u8],
    btrt: Btrt,
) -> Result<Vec<u8>, MuxError> {
    let mut sample_entry = AudioSampleEntry::default();
    sample_entry.set_box_type(ALAC);
    sample_entry.sample_entry = SampleEntry {
        box_type: ALAC,
        data_reference_index: 1,
    };
    sample_entry.channel_count = channel_count;
    sample_entry.sample_size = sample_size;
    sample_entry.sample_rate = sample_rate << 16;

    let mut child_boxes = vec![super::super::mp4::encode_raw_box(ALAC, cookie)?];
    child_boxes.push(super::super::mp4::encode_typed_box(&btrt, &[])?);

    super::super::mp4::encode_typed_box(&sample_entry, &child_boxes.concat())
}
