use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::iso14496_12::{SampleEntry, VisualSampleEntry};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::read_exact_at_sync;

const PNG_ENTRY: FourCc = FourCc::from_bytes(*b"png ");
const AVI_PNG_ENTRY: FourCc = FourCc::from_bytes(*b"PNG ");
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const IHDR: FourCc = FourCc::from_bytes(*b"IHDR");
const IEND: FourCc = FourCc::from_bytes(*b"IEND");

pub(in crate::mux) struct ParsedPngTrack {
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) data_size: u32,
}

pub(in crate::mux) fn scan_png_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedPngTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_png_stream_sync(&mut file, file_size, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_png_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedPngTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_png_stream_async(&mut file, file_size, spec).await
}

pub(in crate::mux) fn parse_png_bytes(
    spec: &str,
    bytes: &[u8],
) -> Result<ParsedPngTrack, MuxError> {
    let file_size =
        u64::try_from(bytes.len()).map_err(|_| MuxError::LayoutOverflow("PNG bytes length"))?;
    if bytes.len() < 8 {
        return Err(invalid_png(
            spec,
            "PNG input is truncated before the 8-byte signature",
        ));
    }
    let mut signature = [0_u8; 8];
    signature.copy_from_slice(&bytes[..8]);
    validate_png_signature(&signature, spec)?;

    let mut offset = 8_u64;
    let mut width = None::<u32>;
    let mut height = None::<u32>;
    let mut saw_iend = false;
    let mut first_chunk = true;
    while offset < file_size {
        let offset_usize =
            usize::try_from(offset).map_err(|_| MuxError::LayoutOverflow("PNG chunk offset"))?;
        if bytes.len() - offset_usize < 12 {
            return Err(invalid_png(spec, "PNG chunk header is truncated"));
        }
        let mut header = [0_u8; 8];
        header.copy_from_slice(&bytes[offset_usize..offset_usize + 8]);
        let (chunk_type, data_offset, data_size, next_offset) =
            decode_png_chunk_header(file_size, offset, header, spec)?;
        if first_chunk && chunk_type != IHDR {
            return Err(invalid_png(
                spec,
                "PNG input did not start its chunk stream with IHDR",
            ));
        }
        first_chunk = false;
        match chunk_type {
            IHDR => {
                if width.is_some() || height.is_some() {
                    return Err(invalid_png(
                        spec,
                        "PNG input carried more than one IHDR chunk",
                    ));
                }
                if data_size != 13 {
                    return Err(invalid_png(
                        spec,
                        "PNG IHDR chunk did not carry the required 13-byte payload",
                    ));
                }
                let data_offset_usize = usize::try_from(data_offset)
                    .map_err(|_| MuxError::LayoutOverflow("PNG IHDR data offset"))?;
                let parsed_width = u32::from_be_bytes(
                    bytes[data_offset_usize..data_offset_usize + 4]
                        .try_into()
                        .unwrap(),
                );
                let parsed_height = u32::from_be_bytes(
                    bytes[data_offset_usize + 4..data_offset_usize + 8]
                        .try_into()
                        .unwrap(),
                );
                if parsed_width == 0 || parsed_height == 0 {
                    return Err(invalid_png(
                        spec,
                        "PNG IHDR declared zero width or zero height",
                    ));
                }
                width = Some(parsed_width);
                height = Some(parsed_height);
            }
            IEND => {
                if data_size != 0 {
                    return Err(invalid_png(spec, "PNG IEND chunk must be empty"));
                }
                saw_iend = true;
                if next_offset != file_size {
                    return Err(invalid_png(
                        spec,
                        "PNG input carried trailing bytes after the IEND chunk",
                    ));
                }
                break;
            }
            _ => {}
        }
        offset = next_offset;
    }

    finalize_png_track(spec, file_size, width, height, saw_iend)
}

fn parse_png_stream_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<ParsedPngTrack, MuxError> {
    validate_png_prefix_sync(file, file_size, spec)?;
    parse_png_chunks_sync(file, file_size, spec)
}

#[cfg(feature = "async")]
async fn parse_png_stream_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<ParsedPngTrack, MuxError> {
    validate_png_prefix_async(file, file_size, spec).await?;
    parse_png_chunks_async(file, file_size, spec).await
}

fn validate_png_prefix_sync(file: &mut File, file_size: u64, spec: &str) -> Result<(), MuxError> {
    if file_size < 8 {
        return Err(invalid_png(
            spec,
            "PNG input is truncated before the 8-byte signature",
        ));
    }
    let mut signature = [0_u8; 8];
    read_exact_at_sync(
        file,
        0,
        &mut signature,
        spec,
        "PNG input is truncated before the 8-byte signature",
    )?;
    validate_png_signature(&signature, spec)
}

#[cfg(feature = "async")]
async fn validate_png_prefix_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if file_size < 8 {
        return Err(invalid_png(
            spec,
            "PNG input is truncated before the 8-byte signature",
        ));
    }
    let mut signature = [0_u8; 8];
    read_exact_at_async(
        file,
        0,
        &mut signature,
        spec,
        "PNG input is truncated before the 8-byte signature",
    )
    .await?;
    validate_png_signature(&signature, spec)
}

fn validate_png_signature(signature: &[u8; 8], spec: &str) -> Result<(), MuxError> {
    if *signature != PNG_SIGNATURE {
        return Err(invalid_png(
            spec,
            "input does not carry the PNG file signature",
        ));
    }
    Ok(())
}

fn parse_png_chunks_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<ParsedPngTrack, MuxError> {
    let mut offset = 8_u64;
    let mut width = None::<u32>;
    let mut height = None::<u32>;
    let mut saw_iend = false;
    let mut first_chunk = true;
    while offset < file_size {
        let (chunk_type, data_offset, data_size, next_offset) =
            read_png_chunk_header_sync(file, file_size, offset, spec)?;
        if first_chunk && chunk_type != IHDR {
            return Err(invalid_png(
                spec,
                "PNG input did not start its chunk stream with IHDR",
            ));
        }
        first_chunk = false;
        match chunk_type {
            IHDR => {
                if width.is_some() || height.is_some() {
                    return Err(invalid_png(
                        spec,
                        "PNG input carried more than one IHDR chunk",
                    ));
                }
                if data_size != 13 {
                    return Err(invalid_png(
                        spec,
                        "PNG IHDR chunk did not carry the required 13-byte payload",
                    ));
                }
                let mut ihdr = [0_u8; 13];
                read_exact_at_sync(
                    file,
                    data_offset,
                    &mut ihdr,
                    spec,
                    "PNG IHDR payload is truncated",
                )?;
                let parsed_width = u32::from_be_bytes(ihdr[0..4].try_into().unwrap());
                let parsed_height = u32::from_be_bytes(ihdr[4..8].try_into().unwrap());
                if parsed_width == 0 || parsed_height == 0 {
                    return Err(invalid_png(
                        spec,
                        "PNG IHDR declared zero width or zero height",
                    ));
                }
                width = Some(parsed_width);
                height = Some(parsed_height);
            }
            IEND => {
                if data_size != 0 {
                    return Err(invalid_png(spec, "PNG IEND chunk must be empty"));
                }
                saw_iend = true;
                if next_offset != file_size {
                    return Err(invalid_png(
                        spec,
                        "PNG input carried trailing bytes after the IEND chunk",
                    ));
                }
                break;
            }
            _ => {}
        }
        offset = next_offset;
    }
    finalize_png_track(spec, file_size, width, height, saw_iend)
}

#[cfg(feature = "async")]
async fn parse_png_chunks_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<ParsedPngTrack, MuxError> {
    let mut offset = 8_u64;
    let mut width = None::<u32>;
    let mut height = None::<u32>;
    let mut saw_iend = false;
    let mut first_chunk = true;
    while offset < file_size {
        let (chunk_type, data_offset, data_size, next_offset) =
            read_png_chunk_header_async(file, file_size, offset, spec).await?;
        if first_chunk && chunk_type != IHDR {
            return Err(invalid_png(
                spec,
                "PNG input did not start its chunk stream with IHDR",
            ));
        }
        first_chunk = false;
        match chunk_type {
            IHDR => {
                if width.is_some() || height.is_some() {
                    return Err(invalid_png(
                        spec,
                        "PNG input carried more than one IHDR chunk",
                    ));
                }
                if data_size != 13 {
                    return Err(invalid_png(
                        spec,
                        "PNG IHDR chunk did not carry the required 13-byte payload",
                    ));
                }
                let mut ihdr = [0_u8; 13];
                read_exact_at_async(
                    file,
                    data_offset,
                    &mut ihdr,
                    spec,
                    "PNG IHDR payload is truncated",
                )
                .await?;
                let parsed_width = u32::from_be_bytes(ihdr[0..4].try_into().unwrap());
                let parsed_height = u32::from_be_bytes(ihdr[4..8].try_into().unwrap());
                if parsed_width == 0 || parsed_height == 0 {
                    return Err(invalid_png(
                        spec,
                        "PNG IHDR declared zero width or zero height",
                    ));
                }
                width = Some(parsed_width);
                height = Some(parsed_height);
            }
            IEND => {
                if data_size != 0 {
                    return Err(invalid_png(spec, "PNG IEND chunk must be empty"));
                }
                saw_iend = true;
                if next_offset != file_size {
                    return Err(invalid_png(
                        spec,
                        "PNG input carried trailing bytes after the IEND chunk",
                    ));
                }
                break;
            }
            _ => {}
        }
        offset = next_offset;
    }
    finalize_png_track(spec, file_size, width, height, saw_iend)
}

fn finalize_png_track(
    spec: &str,
    file_size: u64,
    width: Option<u32>,
    height: Option<u32>,
    saw_iend: bool,
) -> Result<ParsedPngTrack, MuxError> {
    if !saw_iend {
        return Err(invalid_png(
            spec,
            "PNG input did not terminate with an IEND chunk",
        ));
    }
    let width = width.ok_or_else(|| invalid_png(spec, "PNG input did not carry an IHDR chunk"))?;
    let height =
        height.ok_or_else(|| invalid_png(spec, "PNG input did not carry an IHDR chunk"))?;
    let width = u16::try_from(width)
        .map_err(|_| invalid_png(spec, "PNG width does not fit in an MP4 visual sample entry"))?;
    let height = u16::try_from(height).map_err(|_| {
        invalid_png(
            spec,
            "PNG height does not fit in an MP4 visual sample entry",
        )
    })?;
    let data_size = u32::try_from(file_size)
        .map_err(|_| MuxError::LayoutOverflow("PNG file size exceeds MP4 sample limits"))?;
    let sample_entry_box = build_png_sample_entry_box(width, height)?;
    Ok(ParsedPngTrack {
        width,
        height,
        sample_entry_box,
        data_size,
    })
}

fn read_png_chunk_header_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<(FourCc, u64, u32, u64), MuxError> {
    if file_size - offset < 12 {
        return Err(invalid_png(spec, "PNG chunk header is truncated"));
    }
    let mut header = [0_u8; 8];
    read_exact_at_sync(
        file,
        offset,
        &mut header,
        spec,
        "PNG chunk header is truncated",
    )?;
    decode_png_chunk_header(file_size, offset, header, spec)
}

#[cfg(feature = "async")]
async fn read_png_chunk_header_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<(FourCc, u64, u32, u64), MuxError> {
    if file_size - offset < 12 {
        return Err(invalid_png(spec, "PNG chunk header is truncated"));
    }
    let mut header = [0_u8; 8];
    read_exact_at_async(
        file,
        offset,
        &mut header,
        spec,
        "PNG chunk header is truncated",
    )
    .await?;
    decode_png_chunk_header(file_size, offset, header, spec)
}

fn decode_png_chunk_header(
    file_size: u64,
    offset: u64,
    header: [u8; 8],
    spec: &str,
) -> Result<(FourCc, u64, u32, u64), MuxError> {
    let data_size = u32::from_be_bytes(header[0..4].try_into().unwrap());
    let chunk_type = FourCc::from_bytes(header[4..8].try_into().unwrap());
    let data_offset = offset + 8;
    let next_offset = data_offset
        .checked_add(u64::from(data_size))
        .and_then(|value| value.checked_add(4))
        .ok_or(MuxError::LayoutOverflow("PNG chunk range"))?;
    if next_offset > file_size {
        return Err(invalid_png(
            spec,
            &format!("PNG chunk `{chunk_type}` overruns the input length"),
        ));
    }
    Ok((chunk_type, data_offset, data_size, next_offset))
}

fn invalid_png(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}

fn build_png_sample_entry_box(width: u16, height: u16) -> Result<Vec<u8>, MuxError> {
    let mut compressorname = [0_u8; 32];
    compressorname[0] = 3;
    compressorname[1..4].copy_from_slice(b"PNG");
    super::super::mp4::encode_typed_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: PNG_ENTRY,
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
        &[],
    )
}

pub(in crate::mux) fn build_avi_png_sample_entry_box(
    width: u16,
    height: u16,
) -> Result<Vec<u8>, MuxError> {
    let mut compressorname = [0_u8; 32];
    compressorname[0] = 3;
    compressorname[1..4].copy_from_slice(b"PNG");
    super::super::mp4::encode_typed_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: AVI_PNG_ENTRY,
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
        &[],
    )
}
