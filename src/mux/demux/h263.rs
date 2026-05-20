use std::fs::File;
use std::io::Cursor;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::bitio::BitReader;
use crate::boxes::iso14496_12::{Btrt, Colr, Pasp, SampleEntry, VisualSampleEntry};
use crate::boxes::threegpp::D263;

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    StagedSample, build_visual_sample_entry_box_with_compressor_name, read_exact_at_sync,
};
use super::annexb_common::{read_bits_u8_labeled, read_bits_u32_labeled};

const SAMPLE_ENTRY_S263: FourCc = FourCc::from_bytes(*b"s263");
const AVI_SAMPLE_ENTRY_H263: FourCc = FourCc::from_bytes(*b"H263");
const DEFAULT_TIMESCALE: u32 = 1_200_000;
const DEFAULT_SAMPLE_DURATION: u32 = 40_040;
const DEFAULT_FIRST_SAMPLE_DURATION: u32 = 48_000;
const DEFAULT_H263_LEVEL: u8 = 10;
const DEFAULT_H263_PROFILE: u8 = 0;
const DEFAULT_H263_COLOUR_PRIMARIES: u16 = 2;
const DEFAULT_H263_TRANSFER_CHARACTERISTICS: u16 = 2;
const DEFAULT_H263_MATRIX_COEFFICIENTS: u16 = 0;
const H263_HEADER_BYTES: usize = 5;
const SCAN_CHUNK_SIZE: usize = 16 * 1024;

pub(in crate::mux) struct ParsedH263Track {
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) timescale: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

#[derive(Clone, Copy)]
struct ParsedH263PictureHeader {
    width: u16,
    height: u16,
    is_sync_sample: bool,
}

pub(in crate::mux) fn scan_h263_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedH263Track, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_h263_stream_sync(&mut file, file_size, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_h263_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedH263Track, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_h263_stream_async(&mut file, file_size, spec).await
}

pub(in crate::mux) fn parse_h263_picture_bytes(
    spec: &str,
    bytes: &[u8],
) -> Result<(u16, u16, bool), MuxError> {
    if bytes.len() < H263_HEADER_BYTES {
        return Err(invalid_h263(spec, "H.263 picture header is truncated"));
    }
    let mut header = [0_u8; H263_HEADER_BYTES];
    header.copy_from_slice(&bytes[..H263_HEADER_BYTES]);
    let parsed = parse_picture_header_bytes(&header, spec)?;
    Ok((parsed.width, parsed.height, parsed.is_sync_sample))
}

fn parse_h263_stream_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<ParsedH263Track, MuxError> {
    if file_size < u64::try_from(H263_HEADER_BYTES).unwrap() {
        return Err(invalid_h263(
            spec,
            "H.263 input is truncated before the first picture header",
        ));
    }

    let mut samples = Vec::new();
    let mut current_sample_start = None::<u64>;
    let mut current_sync_sample = false;
    let mut width = None::<u16>;
    let mut height = None::<u16>;
    let mut carry = Vec::new();
    let mut offset = 0_u64;

    while offset < file_size {
        let read_len =
            usize::try_from((file_size - offset).min(u64::try_from(SCAN_CHUNK_SIZE).unwrap()))
                .map_err(|_| MuxError::LayoutOverflow("H.263 scan chunk length"))?;
        let mut chunk = vec![0_u8; read_len];
        read_exact_at_sync(
            file,
            offset,
            &mut chunk,
            spec,
            "H.263 scan chunk is truncated",
        )?;

        let combined_offset = offset
            .checked_sub(u64::try_from(carry.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow("H.263 combined scan offset"))?;
        let mut combined = carry;
        combined.extend_from_slice(&chunk);

        if combined.len() >= 4 {
            for index in 0..=combined.len() - 4 {
                if !looks_like_h263_start_code(&combined[index..index + 4]) {
                    continue;
                }
                let picture_start = combined_offset
                    .checked_add(u64::try_from(index).unwrap())
                    .ok_or(MuxError::LayoutOverflow("H.263 picture start"))?;
                let parsed = parse_picture_header_sync(file, file_size, picture_start, spec)?;
                let Some(current_width) = width else {
                    width = Some(parsed.width);
                    height = Some(parsed.height);
                    current_sample_start = Some(picture_start);
                    current_sync_sample = parsed.is_sync_sample;
                    continue;
                };
                if current_width != parsed.width || height.unwrap() != parsed.height {
                    return Err(invalid_h263(
                        spec,
                        "H.263 input changed coded picture size mid-stream",
                    ));
                }
                if let Some(sample_start) = current_sample_start
                    && picture_start > sample_start
                {
                    samples.push(StagedSample {
                        data_offset: sample_start,
                        data_size: u32::try_from(picture_start - sample_start)
                            .map_err(|_| MuxError::LayoutOverflow("H.263 frame size"))?,
                        duration: DEFAULT_SAMPLE_DURATION,
                        composition_time_offset: 0,
                        is_sync_sample: current_sync_sample,
                    });
                }
                current_sample_start = Some(picture_start);
                current_sync_sample = parsed.is_sync_sample;
            }
        }

        carry = if combined.len() > 3 {
            combined[combined.len() - 3..].to_vec()
        } else {
            combined
        };
        offset = offset
            .checked_add(u64::try_from(read_len).unwrap())
            .ok_or(MuxError::LayoutOverflow("H.263 scan offset"))?;
    }

    finalize_h263_track(
        spec,
        file_size,
        width,
        height,
        current_sample_start,
        current_sync_sample,
        samples,
    )
}

#[cfg(feature = "async")]
async fn parse_h263_stream_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<ParsedH263Track, MuxError> {
    if file_size < u64::try_from(H263_HEADER_BYTES).unwrap() {
        return Err(invalid_h263(
            spec,
            "H.263 input is truncated before the first picture header",
        ));
    }

    let mut samples = Vec::new();
    let mut current_sample_start = None::<u64>;
    let mut current_sync_sample = false;
    let mut width = None::<u16>;
    let mut height = None::<u16>;
    let mut carry = Vec::new();
    let mut offset = 0_u64;

    while offset < file_size {
        let read_len =
            usize::try_from((file_size - offset).min(u64::try_from(SCAN_CHUNK_SIZE).unwrap()))
                .map_err(|_| MuxError::LayoutOverflow("H.263 scan chunk length"))?;
        let mut chunk = vec![0_u8; read_len];
        read_exact_at_async(
            file,
            offset,
            &mut chunk,
            spec,
            "H.263 scan chunk is truncated",
        )
        .await?;

        let combined_offset = offset
            .checked_sub(u64::try_from(carry.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow("H.263 combined scan offset"))?;
        let mut combined = carry;
        combined.extend_from_slice(&chunk);

        if combined.len() >= 4 {
            for index in 0..=combined.len() - 4 {
                if !looks_like_h263_start_code(&combined[index..index + 4]) {
                    continue;
                }
                let picture_start = combined_offset
                    .checked_add(u64::try_from(index).unwrap())
                    .ok_or(MuxError::LayoutOverflow("H.263 picture start"))?;
                let parsed =
                    parse_picture_header_async(file, file_size, picture_start, spec).await?;
                let Some(current_width) = width else {
                    width = Some(parsed.width);
                    height = Some(parsed.height);
                    current_sample_start = Some(picture_start);
                    current_sync_sample = parsed.is_sync_sample;
                    continue;
                };
                if current_width != parsed.width || height.unwrap() != parsed.height {
                    return Err(invalid_h263(
                        spec,
                        "H.263 input changed coded picture size mid-stream",
                    ));
                }
                if let Some(sample_start) = current_sample_start
                    && picture_start > sample_start
                {
                    samples.push(StagedSample {
                        data_offset: sample_start,
                        data_size: u32::try_from(picture_start - sample_start)
                            .map_err(|_| MuxError::LayoutOverflow("H.263 frame size"))?,
                        duration: DEFAULT_SAMPLE_DURATION,
                        composition_time_offset: 0,
                        is_sync_sample: current_sync_sample,
                    });
                }
                current_sample_start = Some(picture_start);
                current_sync_sample = parsed.is_sync_sample;
            }
        }

        carry = if combined.len() > 3 {
            combined[combined.len() - 3..].to_vec()
        } else {
            combined
        };
        offset = offset
            .checked_add(u64::try_from(read_len).unwrap())
            .ok_or(MuxError::LayoutOverflow("H.263 scan offset"))?;
    }

    finalize_h263_track(
        spec,
        file_size,
        width,
        height,
        current_sample_start,
        current_sync_sample,
        samples,
    )
}

fn finalize_h263_track(
    spec: &str,
    file_size: u64,
    width: Option<u16>,
    height: Option<u16>,
    current_sample_start: Option<u64>,
    current_sync_sample: bool,
    mut samples: Vec<StagedSample>,
) -> Result<ParsedH263Track, MuxError> {
    let width = width.ok_or_else(|| {
        invalid_h263(
            spec,
            "H.263 input did not expose a supported picture-start header",
        )
    })?;
    let height = height.ok_or_else(|| {
        invalid_h263(
            spec,
            "H.263 input did not expose a supported picture-start header",
        )
    })?;
    let Some(sample_start) = current_sample_start else {
        return Err(invalid_h263(
            spec,
            "H.263 input did not contain any complete picture starts",
        ));
    };
    if sample_start >= file_size {
        return Err(invalid_h263(
            spec,
            "H.263 final frame start ran past the end of the file",
        ));
    }
    samples.push(StagedSample {
        data_offset: sample_start,
        data_size: u32::try_from(file_size - sample_start)
            .map_err(|_| MuxError::LayoutOverflow("H.263 trailing frame size"))?,
        duration: DEFAULT_SAMPLE_DURATION,
        composition_time_offset: 0,
        is_sync_sample: current_sync_sample,
    });
    for sample in &mut samples {
        sample.is_sync_sample = true;
    }
    if let Some(first_sample) = samples.first_mut() {
        first_sample.duration = DEFAULT_FIRST_SAMPLE_DURATION;
    }
    let (display_width, display_height) = display_dimensions_from_coded(width, height);
    Ok(ParsedH263Track {
        width: display_width,
        height: display_height,
        timescale: DEFAULT_TIMESCALE,
        sample_entry_box: build_h263_sample_entry_box(width, height)?,
        samples,
    })
}

fn parse_picture_header_sync(
    file: &mut File,
    file_size: u64,
    picture_start: u64,
    spec: &str,
) -> Result<ParsedH263PictureHeader, MuxError> {
    if picture_start
        .checked_add(u64::try_from(H263_HEADER_BYTES).unwrap())
        .is_none_or(|end| end > file_size)
    {
        return Err(invalid_h263(spec, "H.263 picture header is truncated"));
    }
    let mut header = [0_u8; H263_HEADER_BYTES];
    read_exact_at_sync(
        file,
        picture_start,
        &mut header,
        spec,
        "H.263 picture header is truncated",
    )?;
    parse_picture_header_bytes(&header, spec)
}

#[cfg(feature = "async")]
async fn parse_picture_header_async(
    file: &mut TokioFile,
    file_size: u64,
    picture_start: u64,
    spec: &str,
) -> Result<ParsedH263PictureHeader, MuxError> {
    if picture_start
        .checked_add(u64::try_from(H263_HEADER_BYTES).unwrap())
        .is_none_or(|end| end > file_size)
    {
        return Err(invalid_h263(spec, "H.263 picture header is truncated"));
    }
    let mut header = [0_u8; H263_HEADER_BYTES];
    read_exact_at_async(
        file,
        picture_start,
        &mut header,
        spec,
        "H.263 picture header is truncated",
    )
    .await?;
    parse_picture_header_bytes(&header, spec)
}

fn parse_picture_header_bytes(
    bytes: &[u8; H263_HEADER_BYTES],
    spec: &str,
) -> Result<ParsedH263PictureHeader, MuxError> {
    let mut reader = BitReader::new(Cursor::new(bytes.as_slice()));
    let picture_start_code = read_bits_u32_labeled(&mut reader, 22, spec, "H.263")?;
    if picture_start_code != 0x20 {
        return Err(invalid_h263(
            spec,
            "H.263 picture header did not start with the mandatory PSC pattern",
        ));
    }
    let _temporal_reference = read_bits_u8_labeled(&mut reader, 8, spec, "H.263")?;
    let _mandatory_bits = read_bits_u8_labeled(&mut reader, 5, spec, "H.263")?;
    let picture_size_format = read_bits_u8_labeled(&mut reader, 3, spec, "H.263")?;
    let (width, height) = picture_size_from_format(picture_size_format).ok_or_else(|| {
        invalid_h263(
            spec,
            "H.263 picture header used an unsupported picture-size format",
        )
    })?;
    Ok(ParsedH263PictureHeader {
        width,
        height,
        is_sync_sample: bytes[4] & 0x02 == 0,
    })
}

fn picture_size_from_format(format: u8) -> Option<(u16, u16)> {
    match format {
        1 => Some((128, 96)),
        2 => Some((176, 144)),
        3 => Some((352, 288)),
        4 => Some((704, 576)),
        5 => Some((1408, 1152)),
        _ => None,
    }
}

pub(in crate::mux) fn build_h263_sample_entry_box(
    width: u16,
    height: u16,
) -> Result<Vec<u8>, MuxError> {
    let d263 = super::super::mp4::encode_typed_box(
        &D263 {
            vendor: 0,
            decoder_version: 0,
            h263_level: DEFAULT_H263_LEVEL,
            h263_profile: DEFAULT_H263_PROFILE,
        },
        &[],
    )?;
    let mut child_boxes = vec![d263];
    if let Some((h_spacing, v_spacing)) = default_h263_pixel_aspect_ratio(width, height) {
        child_boxes.push(super::super::mp4::encode_typed_box(
            &Pasp {
                h_spacing,
                v_spacing,
            },
            &[],
        )?);
    }
    child_boxes.push(super::super::mp4::encode_typed_box(
        &Colr {
            colour_type: FourCc::from_bytes(*b"nclx"),
            colour_primaries: DEFAULT_H263_COLOUR_PRIMARIES,
            transfer_characteristics: DEFAULT_H263_TRANSFER_CHARACTERISTICS,
            matrix_coefficients: DEFAULT_H263_MATRIX_COEFFICIENTS,
            full_range_flag: false,
            reserved: 0,
            profile: Vec::new(),
            unknown: Vec::new(),
        },
        &[],
    )?);
    build_visual_sample_entry_box_with_compressor_name(
        SAMPLE_ENTRY_S263,
        width,
        height,
        &[],
        &child_boxes,
    )
}

pub(in crate::mux) fn build_avi_h263_sample_entry_box(
    width: u16,
    height: u16,
    btrt: Btrt,
) -> Result<Vec<u8>, MuxError> {
    let btrt = super::super::mp4::encode_typed_box(&btrt, &[])?;
    let mut compressorname = [0_u8; 32];
    compressorname[0] = 4;
    compressorname[1..5].copy_from_slice(b"H263");
    super::super::mp4::encode_typed_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: AVI_SAMPLE_ENTRY_H263,
                data_reference_index: 1,
            },
            width,
            height,
            horizresolution: 72,
            vertresolution: 72,
            frame_count: 1,
            compressorname,
            depth: 0x0018,
            pre_defined3: -1,
            ..VisualSampleEntry::default()
        },
        &btrt,
    )
}

fn looks_like_h263_start_code(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && (u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) >> 10) == 0x20
}

fn default_h263_pixel_aspect_ratio(width: u16, height: u16) -> Option<(u32, u32)> {
    match (width, height) {
        (176, 144) | (352, 288) | (704, 576) | (1408, 1152) => Some((12, 11)),
        _ => None,
    }
}

fn display_dimensions_from_coded(width: u16, height: u16) -> (u16, u16) {
    if let Some((h_spacing, v_spacing)) = default_h263_pixel_aspect_ratio(width, height) {
        let widened = u64::from(width) * u64::from(h_spacing);
        let display_width = widened.div_ceil(u64::from(v_spacing));
        return (u16::try_from(display_width).unwrap_or(u16::MAX), height);
    }
    (width, height)
}

fn invalid_h263(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}
