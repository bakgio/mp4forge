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

const JPEG_ENTRY: FourCc = FourCc::from_bytes(*b"jpeg");
const AVI_JPEG_ENTRY: FourCc = FourCc::from_bytes(*b"MJPG");
const JPEG_SOI: [u8; 2] = [0xFF, 0xD8];
const JPEG_MARKER_SOI: u8 = 0xD8;
const JPEG_MARKER_EOI: u8 = 0xD9;
const JPEG_MARKER_SOS: u8 = 0xDA;
const JPEG_MARKER_TEM: u8 = 0x01;

pub(in crate::mux) struct ParsedJpegTrack {
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) data_size: u32,
}

pub(in crate::mux) fn scan_jpeg_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedJpegTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_jpeg_stream_sync(&mut file, file_size, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_jpeg_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedJpegTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_jpeg_stream_async(&mut file, file_size, spec).await
}

pub(in crate::mux) fn parse_jpeg_bytes(
    spec: &str,
    bytes: &[u8],
) -> Result<ParsedJpegTrack, MuxError> {
    let file_size =
        u64::try_from(bytes.len()).map_err(|_| MuxError::LayoutOverflow("JPEG bytes length"))?;
    if bytes.len() < 4 {
        return Err(invalid_jpeg(
            spec,
            "JPEG input is truncated before the first marker header",
        ));
    }
    let mut prefix = [0_u8; 2];
    prefix.copy_from_slice(&bytes[..2]);
    validate_jpeg_prefix_bytes(&prefix, spec)?;

    let mut offset = 2_u64;
    let mut width = None::<u32>;
    let mut height = None::<u32>;
    let mut saw_sof = false;
    let mut saw_sos = false;
    let mut saw_eoi = false;
    while offset < file_size {
        let (marker, marker_offset) = read_next_marker_bytes(bytes, offset, spec)?;
        match marker {
            JPEG_MARKER_SOI => {
                return Err(invalid_jpeg(
                    spec,
                    "JPEG input carried an unexpected embedded SOI marker",
                ));
            }
            JPEG_MARKER_EOI => {
                saw_eoi = true;
                if marker_offset + 2 != file_size {
                    return Err(invalid_jpeg(
                        spec,
                        "JPEG input carried trailing bytes after the EOI marker",
                    ));
                }
                break;
            }
            JPEG_MARKER_SOS => {
                let (_, data_size, next_offset) =
                    read_segment_bounds_bytes(bytes, marker_offset, spec)?;
                if data_size < 6 {
                    return Err(invalid_jpeg(spec, "JPEG SOS segment is too short"));
                }
                saw_sos = true;
                let (_, next_marker_offset) =
                    scan_entropy_coded_data_bytes(bytes, next_offset, spec)?;
                offset = next_marker_offset;
            }
            marker if marker_has_standalone_layout(marker) => {
                offset = marker_offset + 2;
            }
            marker => {
                let (data_offset, data_size, next_offset) =
                    read_segment_bounds_bytes(bytes, marker_offset, spec)?;
                if is_sof_marker(marker) {
                    if saw_sof {
                        return Err(invalid_jpeg(
                            spec,
                            "JPEG input carried more than one frame header marker",
                        ));
                    }
                    let data_offset_usize = usize::try_from(data_offset)
                        .map_err(|_| MuxError::LayoutOverflow("JPEG SOF data offset"))?;
                    if data_offset_usize + 6 > bytes.len() {
                        return Err(invalid_jpeg(spec, "JPEG frame header is truncated"));
                    }
                    let mut header = [0_u8; 6];
                    header.copy_from_slice(&bytes[data_offset_usize..data_offset_usize + 6]);
                    let (parsed_width, parsed_height) =
                        decode_sof_dimensions(header, data_size, spec)?;
                    width = Some(parsed_width);
                    height = Some(parsed_height);
                    saw_sof = true;
                }
                offset = next_offset;
            }
        }
    }

    finalize_jpeg_track(spec, file_size, width, height, saw_sof, saw_sos, saw_eoi)
}

fn parse_jpeg_stream_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<ParsedJpegTrack, MuxError> {
    validate_jpeg_prefix_sync(file, file_size, spec)?;
    parse_jpeg_markers_sync(file, file_size, spec)
}

#[cfg(feature = "async")]
async fn parse_jpeg_stream_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<ParsedJpegTrack, MuxError> {
    validate_jpeg_prefix_async(file, file_size, spec).await?;
    parse_jpeg_markers_async(file, file_size, spec).await
}

fn validate_jpeg_prefix_sync(file: &mut File, file_size: u64, spec: &str) -> Result<(), MuxError> {
    if file_size < 4 {
        return Err(invalid_jpeg(
            spec,
            "JPEG input is truncated before the first marker header",
        ));
    }
    let mut prefix = [0_u8; 2];
    read_exact_at_sync(
        file,
        0,
        &mut prefix,
        spec,
        "JPEG input is truncated before the SOI marker",
    )?;
    validate_jpeg_prefix_bytes(&prefix, spec)
}

#[cfg(feature = "async")]
async fn validate_jpeg_prefix_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if file_size < 4 {
        return Err(invalid_jpeg(
            spec,
            "JPEG input is truncated before the first marker header",
        ));
    }
    let mut prefix = [0_u8; 2];
    read_exact_at_async(
        file,
        0,
        &mut prefix,
        spec,
        "JPEG input is truncated before the SOI marker",
    )
    .await?;
    validate_jpeg_prefix_bytes(&prefix, spec)
}

fn validate_jpeg_prefix_bytes(prefix: &[u8; 2], spec: &str) -> Result<(), MuxError> {
    if *prefix != JPEG_SOI {
        return Err(invalid_jpeg(
            spec,
            "input does not begin with the JPEG SOI marker",
        ));
    }
    Ok(())
}

fn parse_jpeg_markers_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<ParsedJpegTrack, MuxError> {
    let mut offset = 2_u64;
    let mut width = None::<u32>;
    let mut height = None::<u32>;
    let mut saw_sof = false;
    let mut saw_sos = false;
    let mut saw_eoi = false;
    while offset < file_size {
        let (marker, marker_offset) = read_next_marker_sync(file, file_size, offset, spec)?;
        match marker {
            JPEG_MARKER_SOI => {
                return Err(invalid_jpeg(
                    spec,
                    "JPEG input carried an unexpected embedded SOI marker",
                ));
            }
            JPEG_MARKER_EOI => {
                saw_eoi = true;
                if marker_offset + 2 != file_size {
                    return Err(invalid_jpeg(
                        spec,
                        "JPEG input carried trailing bytes after the EOI marker",
                    ));
                }
                break;
            }
            JPEG_MARKER_SOS => {
                let (_, data_size, next_offset) =
                    read_segment_bounds_sync(file, file_size, marker_offset, spec)?;
                if data_size < 6 {
                    return Err(invalid_jpeg(spec, "JPEG SOS segment is too short"));
                }
                saw_sos = true;
                let (_, next_marker_offset) =
                    scan_entropy_coded_data_sync(file, file_size, next_offset, spec)?;
                offset = next_marker_offset;
            }
            marker if marker_has_standalone_layout(marker) => {
                offset = marker_offset + 2;
            }
            marker => {
                let (data_offset, data_size, next_offset) =
                    read_segment_bounds_sync(file, file_size, marker_offset, spec)?;
                if is_sof_marker(marker) {
                    if saw_sof {
                        return Err(invalid_jpeg(
                            spec,
                            "JPEG input carried more than one frame header marker",
                        ));
                    }
                    let (parsed_width, parsed_height) =
                        parse_sof_dimensions_sync(file, data_offset, data_size, spec)?;
                    width = Some(parsed_width);
                    height = Some(parsed_height);
                    saw_sof = true;
                }
                offset = next_offset;
            }
        }
    }
    finalize_jpeg_track(spec, file_size, width, height, saw_sof, saw_sos, saw_eoi)
}

#[cfg(feature = "async")]
async fn parse_jpeg_markers_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<ParsedJpegTrack, MuxError> {
    let mut offset = 2_u64;
    let mut width = None::<u32>;
    let mut height = None::<u32>;
    let mut saw_sof = false;
    let mut saw_sos = false;
    let mut saw_eoi = false;
    while offset < file_size {
        let (marker, marker_offset) = read_next_marker_async(file, file_size, offset, spec).await?;
        match marker {
            JPEG_MARKER_SOI => {
                return Err(invalid_jpeg(
                    spec,
                    "JPEG input carried an unexpected embedded SOI marker",
                ));
            }
            JPEG_MARKER_EOI => {
                saw_eoi = true;
                if marker_offset + 2 != file_size {
                    return Err(invalid_jpeg(
                        spec,
                        "JPEG input carried trailing bytes after the EOI marker",
                    ));
                }
                break;
            }
            JPEG_MARKER_SOS => {
                let (_, data_size, next_offset) =
                    read_segment_bounds_async(file, file_size, marker_offset, spec).await?;
                if data_size < 6 {
                    return Err(invalid_jpeg(spec, "JPEG SOS segment is too short"));
                }
                saw_sos = true;
                let (_, next_marker_offset) =
                    scan_entropy_coded_data_async(file, file_size, next_offset, spec).await?;
                offset = next_marker_offset;
            }
            marker if marker_has_standalone_layout(marker) => {
                offset = marker_offset + 2;
            }
            marker => {
                let (data_offset, data_size, next_offset) =
                    read_segment_bounds_async(file, file_size, marker_offset, spec).await?;
                if is_sof_marker(marker) {
                    if saw_sof {
                        return Err(invalid_jpeg(
                            spec,
                            "JPEG input carried more than one frame header marker",
                        ));
                    }
                    let (parsed_width, parsed_height) =
                        parse_sof_dimensions_async(file, data_offset, data_size, spec).await?;
                    width = Some(parsed_width);
                    height = Some(parsed_height);
                    saw_sof = true;
                }
                offset = next_offset;
            }
        }
    }
    finalize_jpeg_track(spec, file_size, width, height, saw_sof, saw_sos, saw_eoi)
}

fn read_next_marker_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<(u8, u64), MuxError> {
    let mut cursor = offset;
    if cursor + 2 > file_size {
        return Err(invalid_jpeg(spec, "JPEG marker header is truncated"));
    }
    let mut prefix = [0_u8; 1];
    read_exact_at_sync(
        file,
        cursor,
        &mut prefix,
        spec,
        "JPEG marker header is truncated",
    )?;
    if prefix[0] != 0xFF {
        return Err(invalid_jpeg(
            spec,
            "JPEG marker stream contained non-marker bytes between segments",
        ));
    }
    cursor += 1;
    loop {
        if cursor >= file_size {
            return Err(invalid_jpeg(spec, "JPEG marker header is truncated"));
        }
        let mut marker = [0_u8; 1];
        read_exact_at_sync(
            file,
            cursor,
            &mut marker,
            spec,
            "JPEG marker header is truncated",
        )?;
        if marker[0] == 0xFF {
            cursor += 1;
            continue;
        }
        if marker[0] == 0x00 {
            return Err(invalid_jpeg(
                spec,
                "JPEG marker stream carried a stuffed zero outside entropy-coded data",
            ));
        }
        return Ok((marker[0], cursor - 1));
    }
}

fn read_next_marker_bytes(bytes: &[u8], offset: u64, spec: &str) -> Result<(u8, u64), MuxError> {
    let file_size =
        u64::try_from(bytes.len()).map_err(|_| MuxError::LayoutOverflow("JPEG bytes length"))?;
    let mut cursor = offset;
    if cursor + 2 > file_size {
        return Err(invalid_jpeg(spec, "JPEG marker header is truncated"));
    }
    let offset_usize =
        usize::try_from(cursor).map_err(|_| MuxError::LayoutOverflow("JPEG marker offset"))?;
    if bytes[offset_usize] != 0xFF {
        return Err(invalid_jpeg(
            spec,
            "JPEG marker stream contained non-marker bytes between segments",
        ));
    }
    cursor += 1;
    loop {
        if cursor >= file_size {
            return Err(invalid_jpeg(spec, "JPEG marker header is truncated"));
        }
        let cursor_usize =
            usize::try_from(cursor).map_err(|_| MuxError::LayoutOverflow("JPEG marker offset"))?;
        let marker = bytes[cursor_usize];
        if marker == 0xFF {
            cursor += 1;
            continue;
        }
        if marker == 0x00 {
            return Err(invalid_jpeg(
                spec,
                "JPEG marker stream carried a stuffed zero outside entropy-coded data",
            ));
        }
        return Ok((marker, cursor - 1));
    }
}

#[cfg(feature = "async")]
async fn read_next_marker_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<(u8, u64), MuxError> {
    let mut cursor = offset;
    if cursor + 2 > file_size {
        return Err(invalid_jpeg(spec, "JPEG marker header is truncated"));
    }
    let mut prefix = [0_u8; 1];
    read_exact_at_async(
        file,
        cursor,
        &mut prefix,
        spec,
        "JPEG marker header is truncated",
    )
    .await?;
    if prefix[0] != 0xFF {
        return Err(invalid_jpeg(
            spec,
            "JPEG marker stream contained non-marker bytes between segments",
        ));
    }
    cursor += 1;
    loop {
        if cursor >= file_size {
            return Err(invalid_jpeg(spec, "JPEG marker header is truncated"));
        }
        let mut marker = [0_u8; 1];
        read_exact_at_async(
            file,
            cursor,
            &mut marker,
            spec,
            "JPEG marker header is truncated",
        )
        .await?;
        if marker[0] == 0xFF {
            cursor += 1;
            continue;
        }
        if marker[0] == 0x00 {
            return Err(invalid_jpeg(
                spec,
                "JPEG marker stream carried a stuffed zero outside entropy-coded data",
            ));
        }
        return Ok((marker[0], cursor - 1));
    }
}

fn read_segment_bounds_sync(
    file: &mut File,
    file_size: u64,
    marker_offset: u64,
    spec: &str,
) -> Result<(u64, u64, u64), MuxError> {
    let mut length = [0_u8; 2];
    read_exact_at_sync(
        file,
        marker_offset + 2,
        &mut length,
        spec,
        "JPEG segment length is truncated",
    )?;
    decode_segment_bounds(file_size, marker_offset, u16::from_be_bytes(length), spec)
}

#[cfg(feature = "async")]
async fn read_segment_bounds_async(
    file: &mut TokioFile,
    file_size: u64,
    marker_offset: u64,
    spec: &str,
) -> Result<(u64, u64, u64), MuxError> {
    let mut length = [0_u8; 2];
    read_exact_at_async(
        file,
        marker_offset + 2,
        &mut length,
        spec,
        "JPEG segment length is truncated",
    )
    .await?;
    decode_segment_bounds(file_size, marker_offset, u16::from_be_bytes(length), spec)
}

fn decode_segment_bounds(
    file_size: u64,
    marker_offset: u64,
    length: u16,
    spec: &str,
) -> Result<(u64, u64, u64), MuxError> {
    if length < 2 {
        return Err(invalid_jpeg(
            spec,
            "JPEG segment length field was smaller than the required 2-byte minimum",
        ));
    }
    let data_offset = marker_offset + 4;
    let data_size = u64::from(length - 2);
    let next_offset = data_offset
        .checked_add(data_size)
        .ok_or(MuxError::LayoutOverflow("JPEG segment range"))?;
    if next_offset > file_size {
        return Err(invalid_jpeg(spec, "JPEG segment overruns the input length"));
    }
    Ok((data_offset, data_size, next_offset))
}

fn read_segment_bounds_bytes(
    bytes: &[u8],
    marker_offset: u64,
    spec: &str,
) -> Result<(u64, u64, u64), MuxError> {
    let file_size =
        u64::try_from(bytes.len()).map_err(|_| MuxError::LayoutOverflow("JPEG bytes length"))?;
    let length_offset = usize::try_from(marker_offset + 2)
        .map_err(|_| MuxError::LayoutOverflow("JPEG segment length offset"))?;
    if length_offset + 2 > bytes.len() {
        return Err(invalid_jpeg(spec, "JPEG segment length is truncated"));
    }
    let length = u16::from_be_bytes(bytes[length_offset..length_offset + 2].try_into().unwrap());
    decode_segment_bounds(file_size, marker_offset, length, spec)
}

fn parse_sof_dimensions_sync(
    file: &mut File,
    data_offset: u64,
    data_size: u64,
    spec: &str,
) -> Result<(u32, u32), MuxError> {
    if data_size < 6 {
        return Err(invalid_jpeg(spec, "JPEG frame header is too short"));
    }
    let mut header = [0_u8; 6];
    read_exact_at_sync(
        file,
        data_offset,
        &mut header,
        spec,
        "JPEG frame header is truncated",
    )?;
    decode_sof_dimensions(header, data_size, spec)
}

#[cfg(feature = "async")]
async fn parse_sof_dimensions_async(
    file: &mut TokioFile,
    data_offset: u64,
    data_size: u64,
    spec: &str,
) -> Result<(u32, u32), MuxError> {
    if data_size < 6 {
        return Err(invalid_jpeg(spec, "JPEG frame header is too short"));
    }
    let mut header = [0_u8; 6];
    read_exact_at_async(
        file,
        data_offset,
        &mut header,
        spec,
        "JPEG frame header is truncated",
    )
    .await?;
    decode_sof_dimensions(header, data_size, spec)
}

fn decode_sof_dimensions(
    header: [u8; 6],
    data_size: u64,
    spec: &str,
) -> Result<(u32, u32), MuxError> {
    let sample_precision = header[0];
    let height = u16::from_be_bytes([header[1], header[2]]);
    let width = u16::from_be_bytes([header[3], header[4]]);
    let component_count = header[5];
    if sample_precision == 0 {
        return Err(invalid_jpeg(
            spec,
            "JPEG frame header declared a zero sample precision",
        ));
    }
    if width == 0 || height == 0 {
        return Err(invalid_jpeg(
            spec,
            "JPEG frame header declared zero width or zero height",
        ));
    }
    if component_count == 0 {
        return Err(invalid_jpeg(
            spec,
            "JPEG frame header declared zero image components",
        ));
    }
    let required_size = 6_u64
        .checked_add(u64::from(component_count) * 3)
        .ok_or(MuxError::LayoutOverflow("JPEG frame header size"))?;
    if data_size < required_size {
        return Err(invalid_jpeg(
            spec,
            "JPEG frame header does not contain every declared component entry",
        ));
    }
    Ok((u32::from(width), u32::from(height)))
}

fn scan_entropy_coded_data_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<(u8, u64), MuxError> {
    let mut cursor = offset;
    let mut previous_was_ff = false;
    let mut buffer = [0_u8; 4096];
    while cursor < file_size {
        let chunk_len = usize::try_from((file_size - cursor).min(buffer.len() as u64)).unwrap();
        read_exact_at_sync(
            file,
            cursor,
            &mut buffer[..chunk_len],
            spec,
            "JPEG entropy-coded data is truncated",
        )?;
        for (index, byte) in buffer[..chunk_len].iter().copied().enumerate() {
            if previous_was_ff {
                match byte {
                    0x00 => previous_was_ff = false,
                    0xFF => previous_was_ff = true,
                    0xD0..=0xD7 => previous_was_ff = false,
                    marker => {
                        let marker_offset = cursor
                            .checked_add(u64::try_from(index).unwrap())
                            .and_then(|value| value.checked_sub(1))
                            .ok_or(MuxError::LayoutOverflow("JPEG marker offset"))?;
                        return Ok((marker, marker_offset));
                    }
                }
            } else if byte == 0xFF {
                previous_was_ff = true;
            }
        }
        cursor += u64::try_from(chunk_len).unwrap();
    }
    Err(invalid_jpeg(
        spec,
        "JPEG entropy-coded data did not terminate with a marker",
    ))
}

fn scan_entropy_coded_data_bytes(
    bytes: &[u8],
    offset: u64,
    spec: &str,
) -> Result<(u8, u64), MuxError> {
    let file_size =
        u64::try_from(bytes.len()).map_err(|_| MuxError::LayoutOverflow("JPEG bytes length"))?;
    let mut cursor =
        usize::try_from(offset).map_err(|_| MuxError::LayoutOverflow("JPEG marker offset"))?;
    let mut previous_was_ff = false;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if previous_was_ff {
            match byte {
                0x00 => previous_was_ff = false,
                0xFF => previous_was_ff = true,
                0xD0..=0xD7 => previous_was_ff = false,
                marker => {
                    let marker_offset = u64::try_from(cursor)
                        .map_err(|_| MuxError::LayoutOverflow("JPEG marker offset"))?
                        .checked_sub(1)
                        .ok_or(MuxError::LayoutOverflow("JPEG marker offset"))?;
                    return Ok((marker, marker_offset));
                }
            }
        } else if byte == 0xFF {
            previous_was_ff = true;
        }
        cursor += 1;
    }
    let _ = file_size;
    Err(invalid_jpeg(
        spec,
        "JPEG entropy-coded data did not terminate with a marker",
    ))
}

#[cfg(feature = "async")]
async fn scan_entropy_coded_data_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<(u8, u64), MuxError> {
    let mut cursor = offset;
    let mut previous_was_ff = false;
    let mut buffer = [0_u8; 4096];
    while cursor < file_size {
        let chunk_len = usize::try_from((file_size - cursor).min(buffer.len() as u64)).unwrap();
        read_exact_at_async(
            file,
            cursor,
            &mut buffer[..chunk_len],
            spec,
            "JPEG entropy-coded data is truncated",
        )
        .await?;
        for (index, byte) in buffer[..chunk_len].iter().copied().enumerate() {
            if previous_was_ff {
                match byte {
                    0x00 => previous_was_ff = false,
                    0xFF => previous_was_ff = true,
                    0xD0..=0xD7 => previous_was_ff = false,
                    marker => {
                        let marker_offset = cursor
                            .checked_add(u64::try_from(index).unwrap())
                            .and_then(|value| value.checked_sub(1))
                            .ok_or(MuxError::LayoutOverflow("JPEG marker offset"))?;
                        return Ok((marker, marker_offset));
                    }
                }
            } else if byte == 0xFF {
                previous_was_ff = true;
            }
        }
        cursor += u64::try_from(chunk_len).unwrap();
    }
    Err(invalid_jpeg(
        spec,
        "JPEG entropy-coded data did not terminate with a marker",
    ))
}

fn finalize_jpeg_track(
    spec: &str,
    file_size: u64,
    width: Option<u32>,
    height: Option<u32>,
    saw_sof: bool,
    saw_sos: bool,
    saw_eoi: bool,
) -> Result<ParsedJpegTrack, MuxError> {
    if !saw_sof {
        return Err(invalid_jpeg(
            spec,
            "JPEG input did not carry a supported frame header marker",
        ));
    }
    if !saw_sos {
        return Err(invalid_jpeg(
            spec,
            "JPEG input did not carry a start-of-scan segment",
        ));
    }
    if !saw_eoi {
        return Err(invalid_jpeg(
            spec,
            "JPEG input did not terminate with an EOI marker",
        ));
    }
    let width = width.ok_or_else(|| {
        invalid_jpeg(
            spec,
            "JPEG input did not expose image dimensions before scan data",
        )
    })?;
    let height = height.ok_or_else(|| {
        invalid_jpeg(
            spec,
            "JPEG input did not expose image dimensions before scan data",
        )
    })?;
    let width = u16::try_from(width).map_err(|_| {
        invalid_jpeg(
            spec,
            "JPEG width does not fit in an MP4 visual sample entry",
        )
    })?;
    let height = u16::try_from(height).map_err(|_| {
        invalid_jpeg(
            spec,
            "JPEG height does not fit in an MP4 visual sample entry",
        )
    })?;
    let data_size = u32::try_from(file_size)
        .map_err(|_| MuxError::LayoutOverflow("JPEG file size exceeds MP4 sample limits"))?;
    let sample_entry_box = build_jpeg_sample_entry_box(width, height)?;
    Ok(ParsedJpegTrack {
        width,
        height,
        sample_entry_box,
        data_size,
    })
}

const fn marker_has_standalone_layout(marker: u8) -> bool {
    matches!(marker, JPEG_MARKER_TEM | 0xD0..=0xD7)
}

fn build_jpeg_sample_entry_box(width: u16, height: u16) -> Result<Vec<u8>, MuxError> {
    let mut compressorname = [0_u8; 32];
    compressorname[0] = 4;
    compressorname[1..5].copy_from_slice(b"JPEG");
    super::super::mp4::encode_typed_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: JPEG_ENTRY,
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

pub(in crate::mux) fn build_avi_jpeg_sample_entry_box(
    width: u16,
    height: u16,
) -> Result<Vec<u8>, MuxError> {
    let mut compressorname = [0_u8; 32];
    compressorname[0] = 19;
    compressorname[1..20].copy_from_slice(b"Codec Not Supported");
    super::super::mp4::encode_typed_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: AVI_JPEG_ENTRY,
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

const fn is_sof_marker(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

fn invalid_jpeg(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}
