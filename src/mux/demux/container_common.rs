use std::fs::File;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData, read_exact_at_sync,
};

fn segment_logical_end(segment: &SegmentedMuxSourceSegment) -> u64 {
    segment.logical_offset
        + match &segment.data {
            SegmentedMuxSourceSegmentData::Prefix(_) => 4,
            SegmentedMuxSourceSegmentData::Bytes(bytes) => u64::try_from(bytes.len()).unwrap(),
            SegmentedMuxSourceSegmentData::FileRange { size, .. } => u64::from(*size),
        }
}

pub(in crate::mux) fn append_file_range_segment(
    segments: &mut Vec<SegmentedMuxSourceSegment>,
    logical_size: &mut u64,
    source_offset: u64,
    size: u32,
) {
    if size == 0 {
        return;
    }
    if let Some(previous) = segments.last_mut() {
        let previous_end = segment_logical_end(previous);
        if previous_end == *logical_size
            && let SegmentedMuxSourceSegmentData::FileRange {
                source_offset: previous_source_offset,
                size: previous_size,
            } = &mut previous.data
            && *previous_source_offset + u64::from(*previous_size) == source_offset
        {
            *previous_size = previous_size.checked_add(size).unwrap();
            *logical_size += u64::from(size);
            return;
        }
    }
    segments.push(SegmentedMuxSourceSegment {
        logical_offset: *logical_size,
        data: SegmentedMuxSourceSegmentData::FileRange {
            source_offset,
            size,
        },
    });
    *logical_size += u64::from(size);
}

pub(in crate::mux) fn read_segmented_bytes_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    offset: u64,
    buf: &mut [u8],
    spec: &str,
    truncated_message: &'static str,
) -> Result<(), MuxError> {
    if offset
        .checked_add(u64::try_from(buf.len()).unwrap_or(u64::MAX))
        .is_none_or(|end| end > total_size)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: truncated_message.to_string(),
        });
    }

    let mut written = 0usize;
    let mut logical_offset = offset;
    for segment in segments {
        if written == buf.len() {
            break;
        }
        if segment_logical_end(segment) <= logical_offset || segment.logical_offset > logical_offset
        {
            if segment.logical_offset > logical_offset {
                break;
            }
            continue;
        }
        let segment_offset = usize::try_from(logical_offset - segment.logical_offset)
            .map_err(|_| MuxError::LayoutOverflow("segmented logical offset"))?;
        match &segment.data {
            SegmentedMuxSourceSegmentData::Prefix(prefix) => {
                let available = prefix.len().saturating_sub(segment_offset);
                let to_copy = available.min(buf.len() - written);
                buf[written..written + to_copy]
                    .copy_from_slice(&prefix[segment_offset..segment_offset + to_copy]);
                written += to_copy;
                logical_offset += u64::try_from(to_copy).unwrap();
            }
            SegmentedMuxSourceSegmentData::Bytes(bytes) => {
                let available = bytes.len().saturating_sub(segment_offset);
                let to_copy = available.min(buf.len() - written);
                buf[written..written + to_copy]
                    .copy_from_slice(&bytes[segment_offset..segment_offset + to_copy]);
                written += to_copy;
                logical_offset += u64::try_from(to_copy).unwrap();
            }
            SegmentedMuxSourceSegmentData::FileRange {
                source_offset,
                size,
            } => {
                let available =
                    usize::try_from(u64::from(*size) - u64::try_from(segment_offset).unwrap())
                        .map_err(|_| MuxError::LayoutOverflow("segmented file range"))?;
                let to_copy = available.min(buf.len() - written);
                read_exact_at_sync(
                    file,
                    source_offset + u64::try_from(segment_offset).unwrap(),
                    &mut buf[written..written + to_copy],
                    spec,
                    truncated_message,
                )?;
                written += to_copy;
                logical_offset += u64::try_from(to_copy).unwrap();
            }
        }
    }

    if written != buf.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: truncated_message.to_string(),
        });
    }
    Ok(())
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn read_segmented_bytes_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    offset: u64,
    buf: &mut [u8],
    spec: &str,
    truncated_message: &'static str,
) -> Result<(), MuxError> {
    if offset
        .checked_add(u64::try_from(buf.len()).unwrap_or(u64::MAX))
        .is_none_or(|end| end > total_size)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: truncated_message.to_string(),
        });
    }

    let mut written = 0usize;
    let mut logical_offset = offset;
    for segment in segments {
        if written == buf.len() {
            break;
        }
        if segment_logical_end(segment) <= logical_offset || segment.logical_offset > logical_offset
        {
            if segment.logical_offset > logical_offset {
                break;
            }
            continue;
        }
        let segment_offset = usize::try_from(logical_offset - segment.logical_offset)
            .map_err(|_| MuxError::LayoutOverflow("segmented logical offset"))?;
        match &segment.data {
            SegmentedMuxSourceSegmentData::Prefix(prefix) => {
                let available = prefix.len().saturating_sub(segment_offset);
                let to_copy = available.min(buf.len() - written);
                buf[written..written + to_copy]
                    .copy_from_slice(&prefix[segment_offset..segment_offset + to_copy]);
                written += to_copy;
                logical_offset += u64::try_from(to_copy).unwrap();
            }
            SegmentedMuxSourceSegmentData::Bytes(bytes) => {
                let available = bytes.len().saturating_sub(segment_offset);
                let to_copy = available.min(buf.len() - written);
                buf[written..written + to_copy]
                    .copy_from_slice(&bytes[segment_offset..segment_offset + to_copy]);
                written += to_copy;
                logical_offset += u64::try_from(to_copy).unwrap();
            }
            SegmentedMuxSourceSegmentData::FileRange {
                source_offset,
                size,
            } => {
                let available =
                    usize::try_from(u64::from(*size) - u64::try_from(segment_offset).unwrap())
                        .map_err(|_| MuxError::LayoutOverflow("segmented file range"))?;
                let to_copy = available.min(buf.len() - written);
                read_exact_at_async(
                    file,
                    source_offset + u64::try_from(segment_offset).unwrap(),
                    &mut buf[written..written + to_copy],
                    spec,
                    truncated_message,
                )
                .await?;
                written += to_copy;
                logical_offset += u64::try_from(to_copy).unwrap();
            }
        }
    }

    if written != buf.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: truncated_message.to_string(),
        });
    }
    Ok(())
}
