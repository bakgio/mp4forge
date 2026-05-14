use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs;

use super::super::MuxError;
use super::super::import::{
    SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData, SegmentedMuxSourceSpec,
};
use super::raw_visual::{UncvPixelLayout, build_uncv_sample_entry_box};

pub(in crate::mux) struct ParsedBmpTrack {
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) segmented_source: SegmentedMuxSourceSpec,
}

pub(in crate::mux) fn scan_bmp_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedBmpTrack, MuxError> {
    let bytes = std::fs::read(path)?;
    parse_bmp_bytes(path, spec, &bytes)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_bmp_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedBmpTrack, MuxError> {
    let bytes = fs::read(path).await?;
    parse_bmp_bytes(path, spec, &bytes)
}

fn parse_bmp_bytes(path: &Path, spec: &str, bytes: &[u8]) -> Result<ParsedBmpTrack, MuxError> {
    if bytes.len() < 54 {
        return Err(invalid_bmp(
            spec,
            "BMP input is truncated before the 54-byte file and info headers",
        ));
    }
    if &bytes[..2] != b"BM" {
        return Err(invalid_bmp(
            spec,
            "input does not start with the BMP file signature",
        ));
    }

    let data_offset = usize::try_from(read_le_u32(bytes, 10, spec, "pixel data offset")?)
        .map_err(|_| MuxError::LayoutOverflow("BMP data offset"))?;
    let dib_header_size = read_le_u32(bytes, 14, spec, "DIB header size")?;
    if dib_header_size < 40 {
        return Err(invalid_bmp(
            spec,
            "BMP DIB header is smaller than the required 40-byte BITMAPINFOHEADER layout",
        ));
    }

    let width_signed = read_le_i32(bytes, 18, spec, "width")?;
    let height_signed = read_le_i32(bytes, 22, spec, "height")?;
    if width_signed <= 0 || height_signed == 0 {
        return Err(invalid_bmp(
            spec,
            "BMP header declared a non-positive width or a zero height",
        ));
    }
    let top_down = height_signed < 0;
    let width_abs = u32::try_from(width_signed)
        .map_err(|_| invalid_bmp(spec, "BMP width does not fit in a positive 32-bit size"))?;
    let height_abs = height_signed.unsigned_abs();
    let width = u16::try_from(width_abs)
        .map_err(|_| invalid_bmp(spec, "BMP width does not fit in an MP4 visual sample entry"))?;
    let height = u16::try_from(height_abs).map_err(|_| {
        invalid_bmp(
            spec,
            "BMP height does not fit in an MP4 visual sample entry",
        )
    })?;

    let planes = read_le_u16(bytes, 26, spec, "plane count")?;
    if planes != 1 {
        return Err(invalid_bmp(
            spec,
            "BMP input declared a plane count other than one",
        ));
    }
    let bits_per_pixel = read_le_u16(bytes, 28, spec, "bits per pixel")?;
    let compression = read_le_u32(bytes, 30, spec, "compression")?;
    if compression != 0 {
        return Err(invalid_bmp(
            spec,
            "BMP input used a compressed pixel layout; only BI_RGB carriage is supported",
        ));
    }

    let (layout, bytes_per_pixel) = match bits_per_pixel {
        24 => (UncvPixelLayout::Rgb24, 3_u64),
        32 => (UncvPixelLayout::Rgbx32, 4_u64),
        _ => {
            return Err(invalid_bmp(
                spec,
                "BMP input declared an unsupported bit depth; only 24-bit and 32-bit BI_RGB carriage is supported",
            ));
        }
    };

    let row_stride = (u64::from(width_abs) * u64::from(bits_per_pixel)).div_ceil(32) * 4;
    let data_end = u64::try_from(data_offset)
        .unwrap()
        .checked_add(
            row_stride
                .checked_mul(u64::from(height_abs))
                .ok_or(MuxError::LayoutOverflow("BMP pixel payload size"))?,
        )
        .ok_or(MuxError::LayoutOverflow("BMP pixel payload range"))?;
    if data_end
        > u64::try_from(bytes.len()).map_err(|_| MuxError::LayoutOverflow("BMP byte length"))?
    {
        return Err(invalid_bmp(
            spec,
            "BMP pixel payload overruns the input length",
        ));
    }

    let transformed_len = u64::from(width_abs)
        .checked_mul(u64::from(height_abs))
        .and_then(|value| value.checked_mul(bytes_per_pixel))
        .ok_or(MuxError::LayoutOverflow("BMP transformed payload size"))?;
    let transformed_capacity = usize::try_from(transformed_len)
        .map_err(|_| MuxError::LayoutOverflow("BMP transformed payload size"))?;
    let mut transformed = Vec::with_capacity(transformed_capacity);
    for output_row in 0..height_abs {
        let source_row = if top_down {
            output_row
        } else {
            height_abs - 1 - output_row
        };
        let row_start = data_offset
            + usize::try_from(u64::from(source_row) * row_stride)
                .map_err(|_| MuxError::LayoutOverflow("BMP row offset"))?;
        for column in 0..width_abs {
            let pixel_offset = row_start
                + usize::try_from(u64::from(column) * bytes_per_pixel)
                    .map_err(|_| MuxError::LayoutOverflow("BMP pixel offset"))?;
            match bits_per_pixel {
                24 => {
                    transformed.push(bytes[pixel_offset + 2]);
                    transformed.push(bytes[pixel_offset + 1]);
                    transformed.push(bytes[pixel_offset]);
                }
                32 => {
                    transformed.push(bytes[pixel_offset + 2]);
                    transformed.push(bytes[pixel_offset + 1]);
                    transformed.push(bytes[pixel_offset]);
                    transformed.push(bytes[pixel_offset + 3]);
                }
                _ => unreachable!(),
            }
        }
    }

    let sample_entry_box = build_uncv_sample_entry_box(width, height, layout, false, false)?;
    let total_size = u64::try_from(transformed.len())
        .map_err(|_| MuxError::LayoutOverflow("BMP transformed payload size"))?;
    Ok(ParsedBmpTrack {
        width,
        height,
        sample_entry_box,
        segmented_source: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: vec![SegmentedMuxSourceSegment {
                logical_offset: 0,
                data: SegmentedMuxSourceSegmentData::Bytes(transformed),
            }],
            total_size,
        },
    })
}

fn read_le_u16(bytes: &[u8], offset: usize, spec: &str, field: &str) -> Result<u16, MuxError> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| invalid_bmp(spec, &format!("BMP header is truncated before the {field}")))?;
    Ok(u16::from_le_bytes(slice.try_into().unwrap()))
}

fn read_le_u32(bytes: &[u8], offset: usize, spec: &str, field: &str) -> Result<u32, MuxError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_bmp(spec, &format!("BMP header is truncated before the {field}")))?;
    Ok(u32::from_le_bytes(slice.try_into().unwrap()))
}

fn read_le_i32(bytes: &[u8], offset: usize, spec: &str, field: &str) -> Result<i32, MuxError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_bmp(spec, &format!("BMP header is truncated before the {field}")))?;
    Ok(i32::from_le_bytes(slice.try_into().unwrap()))
}

fn invalid_bmp(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}
