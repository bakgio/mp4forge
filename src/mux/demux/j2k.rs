use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs;

use crate::FourCc;

use super::super::MuxError;
use super::super::import::StagedSample;
use super::raw_visual::build_mjp2_sample_entry_box;

const JP_SIGNATURE: FourCc = FourCc::from_bytes(*b"jP  ");
const JP2H: FourCc = FourCc::from_bytes(*b"jp2h");
const IHDR: FourCc = FourCc::from_bytes(*b"ihdr");
const JP2C: FourCc = FourCc::from_bytes(*b"jp2c");

pub(in crate::mux) struct ParsedJ2kTrack {
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

pub(in crate::mux) fn scan_j2k_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedJ2kTrack, MuxError> {
    let bytes = std::fs::read(path)?;
    parse_j2k_bytes(spec, &bytes)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_j2k_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedJ2kTrack, MuxError> {
    let bytes = fs::read(path).await?;
    parse_j2k_bytes(spec, &bytes)
}

fn parse_j2k_bytes(spec: &str, bytes: &[u8]) -> Result<ParsedJ2kTrack, MuxError> {
    if bytes.len() >= 12
        && u32::from_be_bytes(bytes[0..4].try_into().unwrap()) == 12
        && bytes[4..8] == *JP_SIGNATURE.as_bytes()
        && u32::from_be_bytes(bytes[8..12].try_into().unwrap()) == 0x0D0A_870A
    {
        return parse_jp2_bytes(spec, bytes);
    }
    if bytes.len() >= 16 && u32::from_be_bytes(bytes[0..4].try_into().unwrap()) == 0xFF4F_FF51 {
        return parse_j2k_codestream_bytes(spec, bytes);
    }
    Err(invalid_j2k(
        spec,
        "input is neither a JP2 image file nor a raw JPEG 2000 codestream",
    ))
}

fn parse_jp2_bytes(spec: &str, bytes: &[u8]) -> Result<ParsedJ2kTrack, MuxError> {
    let file_size =
        u64::try_from(bytes.len()).map_err(|_| MuxError::LayoutOverflow("JP2 byte length"))?;
    let mut offset = 0_u64;
    let mut width = None::<u32>;
    let mut height = None::<u32>;
    let mut jp2h_payload = None::<Vec<u8>>;
    let mut jp2c_offset = None::<u64>;
    while offset < file_size {
        let (box_type, data_offset, end_offset) = read_be_box_range(bytes, offset, spec)?;
        match box_type {
            JP2H => {
                let payload = bytes
                    [usize::try_from(data_offset).unwrap()..usize::try_from(end_offset).unwrap()]
                    .to_vec();
                let (parsed_width, parsed_height) = parse_jp2h_dimensions(spec, &payload)?;
                width = Some(parsed_width);
                height = Some(parsed_height);
                jp2h_payload = Some(payload);
            }
            JP2C => {
                jp2c_offset = Some(offset);
                break;
            }
            _ => {}
        }
        offset = end_offset;
    }

    let width = width
        .ok_or_else(|| invalid_j2k(spec, "JP2 input did not carry a jp2h/ihdr image header"))?;
    let height = height.ok_or_else(|| {
        invalid_j2k(
            spec,
            "JP2 input did not expose image dimensions before codestream data",
        )
    })?;
    let jp2h_payload = jp2h_payload.ok_or_else(|| {
        invalid_j2k(
            spec,
            "JP2 input did not carry a jp2h decoder-configuration box",
        )
    })?;
    let jp2c_offset = jp2c_offset
        .ok_or_else(|| invalid_j2k(spec, "JP2 input did not carry a jp2c codestream box"))?;
    let width_u16 = u16::try_from(width)
        .map_err(|_| invalid_j2k(spec, "JP2 width does not fit in an MP4 visual sample entry"))?;
    let height_u16 = u16::try_from(height).map_err(|_| {
        invalid_j2k(
            spec,
            "JP2 height does not fit in an MP4 visual sample entry",
        )
    })?;
    let sample_size = u32::try_from(file_size - jp2c_offset).map_err(|_| {
        MuxError::LayoutOverflow("JP2 codestream payload exceeds MP4 sample limits")
    })?;
    let sample_entry_box =
        build_mjp2_sample_entry_box(width_u16, height_u16, b"", Some(&jp2h_payload))?;
    Ok(ParsedJ2kTrack {
        width: width_u16,
        height: height_u16,
        sample_entry_box,
        samples: vec![StagedSample {
            data_offset: jp2c_offset,
            data_size: sample_size,
            duration: 1_000,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    })
}

fn parse_jp2h_dimensions(spec: &str, payload: &[u8]) -> Result<(u32, u32), MuxError> {
    let mut offset = 0_u64;
    let payload_size =
        u64::try_from(payload.len()).map_err(|_| MuxError::LayoutOverflow("JP2H byte length"))?;
    while offset < payload_size {
        let (box_type, data_offset, end_offset) = read_be_box_range(payload, offset, spec)?;
        if box_type == IHDR {
            if end_offset - data_offset < 14 {
                return Err(invalid_j2k(spec, "JP2 ihdr payload is truncated"));
            }
            let data_offset_usize = usize::try_from(data_offset)
                .map_err(|_| MuxError::LayoutOverflow("JP2 ihdr offset"))?;
            let height = u32::from_be_bytes(
                payload[data_offset_usize..data_offset_usize + 4]
                    .try_into()
                    .unwrap(),
            );
            let width = u32::from_be_bytes(
                payload[data_offset_usize + 4..data_offset_usize + 8]
                    .try_into()
                    .unwrap(),
            );
            if width == 0 || height == 0 {
                return Err(invalid_j2k(
                    spec,
                    "JP2 ihdr declared zero width or zero height",
                ));
            }
            return Ok((width, height));
        }
        offset = end_offset;
    }
    Err(invalid_j2k(
        spec,
        "JP2 input did not carry an ihdr image header inside jp2h",
    ))
}

fn parse_j2k_codestream_bytes(spec: &str, bytes: &[u8]) -> Result<ParsedJ2kTrack, MuxError> {
    if bytes.len() < 16 {
        return Err(invalid_j2k(
            spec,
            "JPEG 2000 codestream is truncated before the SIZ image dimensions",
        ));
    }
    let width = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[12..16].try_into().unwrap());
    if width == 0 || height == 0 {
        return Err(invalid_j2k(
            spec,
            "JPEG 2000 codestream declared zero width or zero height",
        ));
    }
    let width_u16 = u16::try_from(width).map_err(|_| {
        invalid_j2k(
            spec,
            "JPEG 2000 codestream width does not fit in an MP4 visual sample entry",
        )
    })?;
    let height_u16 = u16::try_from(height).map_err(|_| {
        invalid_j2k(
            spec,
            "JPEG 2000 codestream height does not fit in an MP4 visual sample entry",
        )
    })?;
    let sample_entry_box = build_mjp2_sample_entry_box(width_u16, height_u16, b"", None)?;
    let data_size = u32::try_from(bytes.len())
        .map_err(|_| MuxError::LayoutOverflow("JPEG 2000 codestream exceeds MP4 sample limits"))?;
    Ok(ParsedJ2kTrack {
        width: width_u16,
        height: height_u16,
        sample_entry_box,
        samples: vec![StagedSample {
            data_offset: 0,
            data_size,
            duration: 1_000,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    })
}

fn read_be_box_range(
    bytes: &[u8],
    offset: u64,
    spec: &str,
) -> Result<(FourCc, u64, u64), MuxError> {
    let offset_usize =
        usize::try_from(offset).map_err(|_| MuxError::LayoutOverflow("JPEG 2000 box offset"))?;
    if bytes.len().saturating_sub(offset_usize) < 8 {
        return Err(invalid_j2k(spec, "JPEG 2000 box header is truncated"));
    }
    let size32 = u32::from_be_bytes(bytes[offset_usize..offset_usize + 4].try_into().unwrap());
    let box_type = FourCc::from_bytes(
        bytes[offset_usize + 4..offset_usize + 8]
            .try_into()
            .unwrap(),
    );
    let (header_size, end_offset) = if size32 == 1 {
        if bytes.len().saturating_sub(offset_usize) < 16 {
            return Err(invalid_j2k(
                spec,
                "JPEG 2000 extended-size box header is truncated",
            ));
        }
        let size64 = u64::from_be_bytes(
            bytes[offset_usize + 8..offset_usize + 16]
                .try_into()
                .unwrap(),
        );
        if size64 < 16 {
            return Err(invalid_j2k(
                spec,
                &format!("JPEG 2000 box `{box_type}` declared an invalid extended size"),
            ));
        }
        (16_u64, offset + size64)
    } else if size32 == 0 {
        (
            8_u64,
            u64::try_from(bytes.len())
                .map_err(|_| MuxError::LayoutOverflow("JPEG 2000 byte length"))?,
        )
    } else {
        if size32 < 8 {
            return Err(invalid_j2k(
                spec,
                &format!("JPEG 2000 box `{box_type}` declared a size smaller than its header"),
            ));
        }
        (8_u64, offset + u64::from(size32))
    };
    if end_offset
        > u64::try_from(bytes.len())
            .map_err(|_| MuxError::LayoutOverflow("JPEG 2000 byte length"))?
    {
        return Err(invalid_j2k(
            spec,
            &format!("JPEG 2000 box `{box_type}` overruns the input length"),
        ));
    }
    Ok((box_type, offset + header_size, end_offset))
}

fn invalid_j2k(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}
