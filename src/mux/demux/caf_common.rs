use std::fs::File;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;

#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::read_exact_at_sync;
use super::super::{MuxError, MuxRawCodec};
use super::detect::DetectedPathTrackKind;

pub(super) struct CafDetectionDescription {
    pub(super) format_id: FourCc,
}

pub(in crate::mux) fn detect_caf_track_kind_sync(
    file: &mut File,
) -> Result<DetectedPathTrackKind, MuxError> {
    detect_caf_track_kind_with_reader_sync(file, "CAF path detection")
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn detect_caf_track_kind_async(
    file: &mut TokioFile,
) -> Result<DetectedPathTrackKind, MuxError> {
    detect_caf_track_kind_with_reader_async(file, "CAF path detection").await
}

fn detect_caf_track_kind_with_reader_sync(
    file: &mut File,
    spec: &str,
) -> Result<DetectedPathTrackKind, MuxError> {
    let description = read_caf_description_sync(file, spec)?;
    if description.format_id == FourCc::from_bytes(*b"alac") {
        Ok(DetectedPathTrackKind::Raw(MuxRawCodec::Alac))
    } else {
        Ok(DetectedPathTrackKind::Unknown)
    }
}

#[cfg(feature = "async")]
async fn detect_caf_track_kind_with_reader_async(
    file: &mut TokioFile,
    spec: &str,
) -> Result<DetectedPathTrackKind, MuxError> {
    let description = read_caf_description_async(file, spec).await?;
    if description.format_id == FourCc::from_bytes(*b"alac") {
        Ok(DetectedPathTrackKind::Raw(MuxRawCodec::Alac))
    } else {
        Ok(DetectedPathTrackKind::Unknown)
    }
}

pub(super) fn read_caf_description_sync(
    file: &mut File,
    spec: &str,
) -> Result<CafDetectionDescription, MuxError> {
    let file_size = file.metadata()?.len();
    if file_size < 8 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF input is truncated before the 8-byte file header".to_string(),
        });
    }
    let mut header = [0_u8; 8];
    read_exact_at_sync(
        file,
        0,
        &mut header,
        spec,
        "CAF input is truncated before the 8-byte file header",
    )?;
    if &header[..4] != b"caff" {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF input did not start with the `caff` signature".to_string(),
        });
    }
    let mut offset = 8_u64;
    while offset < file_size {
        let (chunk_type, chunk_size) = read_caf_chunk_header_sync(file, offset, spec)?;
        let chunk_data_offset = offset + 12;
        if chunk_type == FourCc::from_bytes(*b"desc") {
            if chunk_size < 12 {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "CAF `desc` chunk is too short to include a format identifier"
                        .to_string(),
                });
            }
            let mut desc = [0_u8; 12];
            read_exact_at_sync(
                file,
                chunk_data_offset,
                &mut desc,
                spec,
                "CAF `desc` chunk is truncated",
            )?;
            return Ok(CafDetectionDescription {
                format_id: FourCc::from_bytes(desc[8..12].try_into().unwrap()),
            });
        }
        offset = chunk_data_offset
            .checked_add(chunk_size)
            .ok_or(MuxError::LayoutOverflow("CAF chunk range"))?;
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "CAF input did not contain a required `desc` chunk".to_string(),
    })
}

#[cfg(feature = "async")]
pub(super) async fn read_caf_description_async(
    file: &mut TokioFile,
    spec: &str,
) -> Result<CafDetectionDescription, MuxError> {
    let file_size = file.metadata().await?.len();
    if file_size < 8 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF input is truncated before the 8-byte file header".to_string(),
        });
    }
    let mut header = [0_u8; 8];
    read_exact_at_async(
        file,
        0,
        &mut header,
        spec,
        "CAF input is truncated before the 8-byte file header",
    )
    .await?;
    if &header[..4] != b"caff" {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "CAF input did not start with the `caff` signature".to_string(),
        });
    }
    let mut offset = 8_u64;
    while offset < file_size {
        let (chunk_type, chunk_size) = read_caf_chunk_header_async(file, offset, spec).await?;
        let chunk_data_offset = offset + 12;
        if chunk_type == FourCc::from_bytes(*b"desc") {
            if chunk_size < 12 {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "CAF `desc` chunk is too short to include a format identifier"
                        .to_string(),
                });
            }
            let mut desc = [0_u8; 12];
            read_exact_at_async(
                file,
                chunk_data_offset,
                &mut desc,
                spec,
                "CAF `desc` chunk is truncated",
            )
            .await?;
            return Ok(CafDetectionDescription {
                format_id: FourCc::from_bytes(desc[8..12].try_into().unwrap()),
            });
        }
        offset = chunk_data_offset
            .checked_add(chunk_size)
            .ok_or(MuxError::LayoutOverflow("CAF chunk range"))?;
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "CAF input did not contain a required `desc` chunk".to_string(),
    })
}

pub(super) fn read_caf_chunk_header_sync(
    file: &mut File,
    offset: u64,
    spec: &str,
) -> Result<(FourCc, u64), MuxError> {
    let mut header = [0_u8; 12];
    read_exact_at_sync(
        file,
        offset,
        &mut header,
        spec,
        "CAF chunk header is truncated before 12 bytes",
    )?;
    Ok((
        FourCc::from_bytes(header[..4].try_into().unwrap()),
        u64::from_be_bytes(header[4..12].try_into().unwrap()),
    ))
}

#[cfg(feature = "async")]
pub(super) async fn read_caf_chunk_header_async(
    file: &mut TokioFile,
    offset: u64,
    spec: &str,
) -> Result<(FourCc, u64), MuxError> {
    let mut header = [0_u8; 12];
    read_exact_at_async(
        file,
        offset,
        &mut header,
        spec,
        "CAF chunk header is truncated before 12 bytes",
    )
    .await?;
    Ok((
        FourCc::from_bytes(header[..4].try_into().unwrap()),
        u64::from_be_bytes(header[4..12].try_into().unwrap()),
    ))
}
