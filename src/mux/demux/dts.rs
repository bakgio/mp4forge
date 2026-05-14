use std::fs::File;
use std::io::Cursor;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::bitio::BitReader;
use crate::boxes::AnyTypeBox;
use crate::boxes::iso14496_12::{AudioSampleEntry, Btrt, SampleEntry};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData, SegmentedMuxSourceSpec,
};
use super::super::import::{StagedSample, build_btrt_from_sample_sizes, read_exact_at_sync};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;

const DTSC: FourCc = FourCc::from_bytes(*b"dtsc");
const DTSX: FourCc = FourCc::from_bytes(*b"dtsx");
const DTS_SYNC_WORD: u32 = 0x7FFE_8001;
const DTS_MIN_HEADER_BYTES: u64 = 11;
const DTS_MEDIA_TIMESCALE: u32 = 90_000;
const DTS_FAMILY_WRAPPER_HEADER: &[u8; 8] = b"DTSHDHDR";
const DTS_FAMILY_CORE_SCAN_LIMIT: u64 = 1_048_576;
const DTS_SAMPLE_RATE_BY_CODE: [Option<u32>; 16] = [
    None,
    Some(8_000),
    Some(16_000),
    Some(32_000),
    None,
    None,
    Some(11_025),
    Some(22_050),
    Some(44_100),
    None,
    None,
    Some(12_000),
    Some(24_000),
    Some(48_000),
    None,
    None,
];
const DTS_EXT_AUDIO_ID_VALID: [bool; 8] = [true, false, true, false, false, false, true, false];
const DTS_CORE_CHANNELS_BY_AMODE: [u16; 16] = [1, 2, 2, 2, 2, 3, 3, 4, 4, 5, 6, 6, 7, 7, 7, 8];
const RAW_DTS_DIRECT_INGEST_NOTE: &str = "native raw direct-ingest currently supports big-endian core DTS sync frames, little-endian core DTS sync frames, transformed 14-bit core DTS sync frames, and DTS-family wrappers that expose one contiguous core substream";

pub(in crate::mux) struct ParsedDtsTrack {
    pub(in crate::mux) media_timescale: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
    pub(in crate::mux) transformed_source: Option<SegmentedMuxSourceSpec>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DtsTrackDescriptor {
    sample_rate: u32,
    sample_duration: u32,
    channel_count: u16,
    sample_depth: u8,
}

#[derive(Clone, Copy)]
struct ParsedDtsFrame {
    descriptor: DtsTrackDescriptor,
    frame_size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DtsInputEncoding {
    CoreBigEndian16,
    CoreLittleEndian16,
    CoreBigEndian14,
    CoreLittleEndian14,
}

struct NormalizedDtsStream {
    bytes: Vec<u8>,
    descriptor: Option<DtsTrackDescriptor>,
    samples: Vec<StagedSample>,
    consumed_input_size: usize,
    frame_input_sizes: Vec<u32>,
}

pub(in crate::mux) fn scan_dts_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let (start_offset, encoding) = sniff_dts_payload_sync(&mut file, file_size, spec)?;
    if start_offset == 0 && matches!(encoding, DtsInputEncoding::CoreBigEndian16) {
        parse_dts_stream_sync(&mut file, start_offset, file_size, spec)
    } else {
        parse_transformed_dts_stream_sync(path, &mut file, start_offset, file_size, encoding, spec)
    }
}

pub(in crate::mux) fn scan_dts_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    parse_dts_segmented_stream_sync(file, segments, total_size, spec)
}

fn sniff_dts_input_encoding_sync(
    file: &mut File,
    spec: &str,
) -> Result<DtsInputEncoding, MuxError> {
    let mut sync = [0_u8; 4];
    read_exact_at_sync(file, 0, &mut sync, spec, "truncated DTS frame header")?;
    dts_input_encoding_from_sync(sync, spec)
}

pub(in crate::mux) fn wrapped_dts_family_has_native_core_sync_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<bool, MuxError> {
    Ok(find_wrapped_dts_payload_sync(file, file_size, spec)?.is_some())
}

fn sniff_dts_payload_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<(u64, DtsInputEncoding), MuxError> {
    match sniff_dts_input_encoding_sync(file, spec) {
        Ok(encoding) => Ok((0, encoding)),
        Err(error) => {
            if let Some((start_offset, encoding)) =
                find_wrapped_dts_payload_sync(file, file_size, spec)?
            {
                Ok((start_offset, encoding))
            } else {
                Err(error)
            }
        }
    }
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_dts_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let (start_offset, encoding) = sniff_dts_payload_async(&mut file, file_size, spec).await?;
    if start_offset == 0 && matches!(encoding, DtsInputEncoding::CoreBigEndian16) {
        parse_dts_stream_async(&mut file, start_offset, file_size, spec).await
    } else {
        parse_transformed_dts_stream_async(path, &mut file, start_offset, file_size, encoding, spec)
            .await
    }
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn wrapped_dts_family_has_native_core_sync_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<bool, MuxError> {
    Ok(find_wrapped_dts_payload_async(file, file_size, spec)
        .await?
        .is_some())
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_dts_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    parse_dts_segmented_stream_async(file, segments, total_size, spec).await
}

fn parse_dts_stream_sync(
    file: &mut File,
    start_offset: u64,
    file_size: u64,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    let mut offset = start_offset;
    let mut samples = Vec::new();
    let mut descriptor = None::<DtsTrackDescriptor>;

    while offset < file_size {
        if file_size - offset < DTS_MIN_HEADER_BYTES {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated DTS frame header".to_string(),
            });
        }
        let mut header = [0_u8; DTS_MIN_HEADER_BYTES as usize];
        read_exact_at_sync(
            file,
            offset,
            &mut header,
            spec,
            "truncated DTS frame header",
        )?;
        let parsed = parse_dts_frame_header(&header, offset, spec)?;
        let frame_size_u64 = u64::from(parsed.frame_size);
        if offset
            .checked_add(frame_size_u64)
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated DTS frame at byte offset {offset}"),
            });
        }
        if let Some(current) = descriptor {
            if current != parsed.descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "DTS frames changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            descriptor = Some(parsed.descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: parsed.frame_size,
            duration: parsed.descriptor.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size_u64)
            .ok_or(MuxError::LayoutOverflow("DTS frame offset"))?;
    }

    finalize_parsed_dts_track(spec, descriptor, samples, None, DTSC)
}

fn parse_dts_segmented_stream_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut descriptor = None::<DtsTrackDescriptor>;

    while offset < total_size {
        if total_size - offset < DTS_MIN_HEADER_BYTES {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated DTS frame header".to_string(),
            });
        }
        let mut header = [0_u8; DTS_MIN_HEADER_BYTES as usize];
        read_segmented_bytes_sync(
            file,
            segments,
            total_size,
            offset,
            &mut header,
            spec,
            "truncated DTS frame header",
        )?;
        let parsed = parse_dts_frame_header(&header, offset, spec)?;
        let frame_size_u64 = u64::from(parsed.frame_size);
        if offset
            .checked_add(frame_size_u64)
            .is_none_or(|end| end > total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated DTS frame at logical byte offset {offset}"),
            });
        }
        if let Some(current) = descriptor {
            if current != parsed.descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "DTS frames changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            descriptor = Some(parsed.descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: parsed.frame_size,
            duration: parsed.descriptor.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size_u64)
            .ok_or(MuxError::LayoutOverflow("DTS frame offset"))?;
    }

    finalize_parsed_dts_track(spec, descriptor, samples, None, DTSC)
}

#[cfg(feature = "async")]
async fn parse_dts_stream_async(
    file: &mut TokioFile,
    start_offset: u64,
    file_size: u64,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    let mut offset = start_offset;
    let mut samples = Vec::new();
    let mut descriptor = None::<DtsTrackDescriptor>;

    while offset < file_size {
        if file_size - offset < DTS_MIN_HEADER_BYTES {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated DTS frame header".to_string(),
            });
        }
        let mut header = [0_u8; DTS_MIN_HEADER_BYTES as usize];
        read_exact_at_async(
            file,
            offset,
            &mut header,
            spec,
            "truncated DTS frame header",
        )
        .await?;
        let parsed = parse_dts_frame_header(&header, offset, spec)?;
        let frame_size_u64 = u64::from(parsed.frame_size);
        if offset
            .checked_add(frame_size_u64)
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated DTS frame at byte offset {offset}"),
            });
        }
        if let Some(current) = descriptor {
            if current != parsed.descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "DTS frames changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            descriptor = Some(parsed.descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: parsed.frame_size,
            duration: parsed.descriptor.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size_u64)
            .ok_or(MuxError::LayoutOverflow("DTS frame offset"))?;
    }

    finalize_parsed_dts_track(spec, descriptor, samples, None, DTSC)
}

#[cfg(feature = "async")]
async fn parse_dts_segmented_stream_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut descriptor = None::<DtsTrackDescriptor>;

    while offset < total_size {
        if total_size - offset < DTS_MIN_HEADER_BYTES {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated DTS frame header".to_string(),
            });
        }
        let mut header = [0_u8; DTS_MIN_HEADER_BYTES as usize];
        read_segmented_bytes_async(
            file,
            segments,
            total_size,
            offset,
            &mut header,
            spec,
            "truncated DTS frame header",
        )
        .await?;
        let parsed = parse_dts_frame_header(&header, offset, spec)?;
        let frame_size_u64 = u64::from(parsed.frame_size);
        if offset
            .checked_add(frame_size_u64)
            .is_none_or(|end| end > total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated DTS frame at logical byte offset {offset}"),
            });
        }
        if let Some(current) = descriptor {
            if current != parsed.descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "DTS frames changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            descriptor = Some(parsed.descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: parsed.frame_size,
            duration: parsed.descriptor.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size_u64)
            .ok_or(MuxError::LayoutOverflow("DTS frame offset"))?;
    }

    finalize_parsed_dts_track(spec, descriptor, samples, None, DTSC)
}

#[cfg(feature = "async")]
async fn sniff_dts_input_encoding_async(
    file: &mut TokioFile,
    spec: &str,
) -> Result<DtsInputEncoding, MuxError> {
    let mut sync = [0_u8; 4];
    read_exact_at_async(file, 0, &mut sync, spec, "truncated DTS frame header").await?;
    dts_input_encoding_from_sync(sync, spec)
}

#[cfg(feature = "async")]
async fn sniff_dts_payload_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<(u64, DtsInputEncoding), MuxError> {
    match sniff_dts_input_encoding_async(file, spec).await {
        Ok(encoding) => Ok((0, encoding)),
        Err(error) => {
            if let Some((start_offset, encoding)) =
                find_wrapped_dts_payload_async(file, file_size, spec).await?
            {
                Ok((start_offset, encoding))
            } else {
                Err(error)
            }
        }
    }
}

fn parse_transformed_dts_stream_sync(
    path: &Path,
    file: &mut File,
    start_offset: u64,
    file_size: u64,
    encoding: DtsInputEncoding,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    let wrapped_family = start_offset != 0;
    if wrapped_family {
        let wrapped_input = read_dts_stream_range_sync(file, 0, file_size, spec)?;
        let start_offset_usize = usize::try_from(start_offset)
            .map_err(|_| MuxError::LayoutOverflow("DTS wrapped start offset"))?;
        let normalized =
            normalize_dts_stream_bytes(&wrapped_input[start_offset_usize..], encoding, spec, true)?;
        let wrapped_samples = rebuild_wrapped_dts_family_samples(
            &normalized.samples,
            &normalized.frame_input_sizes,
            start_offset_usize,
            wrapped_input.len(),
            normalized.consumed_input_size,
        )?;
        let total_size = u64::try_from(wrapped_input.len())
            .map_err(|_| MuxError::LayoutOverflow("DTS wrapped source size"))?;
        let transformed_source = SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: vec![SegmentedMuxSourceSegment {
                logical_offset: 0,
                data: SegmentedMuxSourceSegmentData::Bytes(wrapped_input),
            }],
            total_size,
        };
        return finalize_parsed_dts_track(
            spec,
            normalized.descriptor,
            wrapped_samples,
            Some(transformed_source),
            DTSX,
        );
    }

    let input = read_dts_stream_range_sync(file, start_offset, file_size, spec)?;
    let normalized = normalize_dts_stream_bytes(&input, encoding, spec, false)?;
    let staged_bytes = match encoding {
        // Keep the original little-endian core wire bytes in mdat so flat direct-ingest parity
        // matches the retained path-only importer behavior while we still parse headers through a
        // normalized big-endian view.
        DtsInputEncoding::CoreLittleEndian16 => input[..normalized.consumed_input_size].to_vec(),
        _ => normalized.bytes,
    };
    let total_size = u64::try_from(staged_bytes.len())
        .map_err(|_| MuxError::LayoutOverflow("DTS transformed source size"))?;
    let transformed_source = SegmentedMuxSourceSpec {
        path: path.to_path_buf(),
        segments: vec![SegmentedMuxSourceSegment {
            logical_offset: 0,
            data: SegmentedMuxSourceSegmentData::Bytes(staged_bytes),
        }],
        total_size,
    };
    finalize_parsed_dts_track(
        spec,
        normalized.descriptor,
        normalized.samples,
        Some(transformed_source),
        DTSC,
    )
}

#[cfg(feature = "async")]
async fn parse_transformed_dts_stream_async(
    path: &Path,
    file: &mut TokioFile,
    start_offset: u64,
    file_size: u64,
    encoding: DtsInputEncoding,
    spec: &str,
) -> Result<ParsedDtsTrack, MuxError> {
    let wrapped_family = start_offset != 0;
    if wrapped_family {
        let wrapped_input = read_dts_stream_range_async(file, 0, file_size, spec).await?;
        let start_offset_usize = usize::try_from(start_offset)
            .map_err(|_| MuxError::LayoutOverflow("DTS wrapped start offset"))?;
        let normalized =
            normalize_dts_stream_bytes(&wrapped_input[start_offset_usize..], encoding, spec, true)?;
        let wrapped_samples = rebuild_wrapped_dts_family_samples(
            &normalized.samples,
            &normalized.frame_input_sizes,
            start_offset_usize,
            wrapped_input.len(),
            normalized.consumed_input_size,
        )?;
        let total_size = u64::try_from(wrapped_input.len())
            .map_err(|_| MuxError::LayoutOverflow("DTS wrapped source size"))?;
        let transformed_source = SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: vec![SegmentedMuxSourceSegment {
                logical_offset: 0,
                data: SegmentedMuxSourceSegmentData::Bytes(wrapped_input),
            }],
            total_size,
        };
        return finalize_parsed_dts_track(
            spec,
            normalized.descriptor,
            wrapped_samples,
            Some(transformed_source),
            DTSX,
        );
    }

    let input = read_dts_stream_range_async(file, start_offset, file_size, spec).await?;
    let normalized = normalize_dts_stream_bytes(&input, encoding, spec, false)?;
    let staged_bytes = match encoding {
        DtsInputEncoding::CoreLittleEndian16 => input[..normalized.consumed_input_size].to_vec(),
        _ => normalized.bytes,
    };
    let total_size = u64::try_from(staged_bytes.len())
        .map_err(|_| MuxError::LayoutOverflow("DTS transformed source size"))?;
    let transformed_source = SegmentedMuxSourceSpec {
        path: path.to_path_buf(),
        segments: vec![SegmentedMuxSourceSegment {
            logical_offset: 0,
            data: SegmentedMuxSourceSegmentData::Bytes(staged_bytes),
        }],
        total_size,
    };
    finalize_parsed_dts_track(
        spec,
        normalized.descriptor,
        normalized.samples,
        Some(transformed_source),
        DTSC,
    )
}

fn read_dts_stream_range_sync(
    file: &mut File,
    start_offset: u64,
    file_size: u64,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    let payload_size = file_size
        .checked_sub(start_offset)
        .ok_or(MuxError::LayoutOverflow("DTS input range"))?;
    let mut bytes = vec![
        0_u8;
        usize::try_from(payload_size)
            .map_err(|_| MuxError::LayoutOverflow("DTS input byte capacity"))?
    ];
    if !bytes.is_empty() {
        read_exact_at_sync(
            file,
            start_offset,
            &mut bytes,
            spec,
            "truncated DTS input stream",
        )?;
    }
    Ok(bytes)
}

#[cfg(feature = "async")]
async fn read_dts_stream_range_async(
    file: &mut TokioFile,
    start_offset: u64,
    file_size: u64,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    let payload_size = file_size
        .checked_sub(start_offset)
        .ok_or(MuxError::LayoutOverflow("DTS input range"))?;
    let mut bytes = vec![
        0_u8;
        usize::try_from(payload_size)
            .map_err(|_| MuxError::LayoutOverflow("DTS input byte capacity"))?
    ];
    if !bytes.is_empty() {
        read_exact_at_async(
            file,
            start_offset,
            &mut bytes,
            spec,
            "truncated DTS input stream",
        )
        .await?;
    }
    Ok(bytes)
}

fn normalize_dts_stream_bytes(
    input: &[u8],
    encoding: DtsInputEncoding,
    spec: &str,
    allow_non_core_tail: bool,
) -> Result<NormalizedDtsStream, MuxError> {
    let mut input_offset = 0usize;
    let mut output = Vec::new();
    let mut samples = Vec::new();
    let mut descriptor = None::<DtsTrackDescriptor>;
    let mut frame_input_sizes = Vec::new();

    while input_offset < input.len() {
        if allow_non_core_tail
            && descriptor.is_some()
            && !input_starts_with_dts_encoding(input, input_offset, encoding)
        {
            break;
        }
        let (normalized_frame, parsed, frame_input_size) =
            normalize_one_dts_frame(input, input_offset, encoding, spec)?;
        if let Some(current) = descriptor {
            if current != parsed.descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "DTS frames changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            descriptor = Some(parsed.descriptor);
        }
        let data_offset = u64::try_from(output.len())
            .map_err(|_| MuxError::LayoutOverflow("DTS transformed output offset"))?;
        output.extend_from_slice(&normalized_frame);
        frame_input_sizes.push(
            u32::try_from(frame_input_size)
                .map_err(|_| MuxError::LayoutOverflow("DTS transformed frame input size"))?,
        );
        samples.push(StagedSample {
            data_offset,
            data_size: parsed.frame_size,
            duration: parsed.descriptor.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        input_offset = input_offset
            .checked_add(frame_input_size)
            .ok_or(MuxError::LayoutOverflow("DTS transformed input offset"))?;
    }

    Ok(NormalizedDtsStream {
        bytes: output,
        descriptor,
        samples,
        consumed_input_size: input_offset,
        frame_input_sizes,
    })
}

fn input_starts_with_dts_encoding(
    input: &[u8],
    input_offset: usize,
    encoding: DtsInputEncoding,
) -> bool {
    let Some(prefix) = input.get(input_offset..input_offset.saturating_add(4)) else {
        return false;
    };
    prefix == dts_encoding_sync_bytes(encoding)
}

fn dts_encoding_sync_bytes(encoding: DtsInputEncoding) -> &'static [u8; 4] {
    match encoding {
        DtsInputEncoding::CoreBigEndian16 => b"\x7F\xFE\x80\x01",
        DtsInputEncoding::CoreLittleEndian16 => b"\xFE\x7F\x01\x80",
        DtsInputEncoding::CoreBigEndian14 => b"\x1F\xFF\xE8\x00",
        DtsInputEncoding::CoreLittleEndian14 => b"\xFF\x1F\x00\xE8",
    }
}

fn normalize_one_dts_frame(
    input: &[u8],
    input_offset: usize,
    encoding: DtsInputEncoding,
    spec: &str,
) -> Result<(Vec<u8>, ParsedDtsFrame, usize), MuxError> {
    let normalized_header = normalize_dts_header_prefix(input, input_offset, encoding, spec)?;
    let parsed = parse_dts_frame_header(
        &normalized_header,
        u64::try_from(input_offset).map_err(|_| MuxError::LayoutOverflow("DTS input offset"))?,
        spec,
    )?;
    let normalized_frame_size = usize::try_from(parsed.frame_size)
        .map_err(|_| MuxError::LayoutOverflow("DTS frame size"))?;
    let frame_input_size = match encoding {
        DtsInputEncoding::CoreBigEndian16 | DtsInputEncoding::CoreLittleEndian16 => {
            normalized_frame_size
        }
        DtsInputEncoding::CoreBigEndian14 | DtsInputEncoding::CoreLittleEndian14 => {
            packed_14bit_frame_size(normalized_frame_size)?
        }
    };
    let frame_end = input_offset
        .checked_add(frame_input_size)
        .ok_or(MuxError::LayoutOverflow("DTS frame end"))?;
    if frame_end > input.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("truncated DTS frame at byte offset {input_offset}"),
        });
    }
    let normalized_frame = match encoding {
        DtsInputEncoding::CoreBigEndian16 => input[input_offset..frame_end].to_vec(),
        DtsInputEncoding::CoreLittleEndian16 => {
            swap_dts_16bit_words(&input[input_offset..frame_end], spec, input_offset)?
        }
        DtsInputEncoding::CoreBigEndian14 => unpack_dts_14bit_words(
            &input[input_offset..frame_end],
            false,
            normalized_frame_size,
        )?,
        DtsInputEncoding::CoreLittleEndian14 => {
            unpack_dts_14bit_words(&input[input_offset..frame_end], true, normalized_frame_size)?
        }
    };
    Ok((normalized_frame, parsed, frame_input_size))
}

fn normalize_dts_header_prefix(
    input: &[u8],
    input_offset: usize,
    encoding: DtsInputEncoding,
    spec: &str,
) -> Result<[u8; DTS_MIN_HEADER_BYTES as usize], MuxError> {
    match encoding {
        DtsInputEncoding::CoreBigEndian16 => {
            let header_end = input_offset
                .checked_add(DTS_MIN_HEADER_BYTES as usize)
                .ok_or(MuxError::LayoutOverflow("DTS header end"))?;
            let Some(header) = input.get(input_offset..header_end) else {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "truncated DTS frame header".to_string(),
                });
            };
            Ok(header.try_into().unwrap())
        }
        DtsInputEncoding::CoreLittleEndian16 => {
            let header_end = input_offset
                .checked_add(DTS_MIN_HEADER_BYTES as usize + 1)
                .ok_or(MuxError::LayoutOverflow("DTS header end"))?;
            let Some(header) = input.get(input_offset..header_end) else {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "truncated DTS frame header".to_string(),
                });
            };
            let swapped = swap_dts_16bit_words(header, spec, input_offset)?;
            Ok(swapped[..DTS_MIN_HEADER_BYTES as usize].try_into().unwrap())
        }
        DtsInputEncoding::CoreBigEndian14 | DtsInputEncoding::CoreLittleEndian14 => {
            let header_input_size = packed_14bit_frame_size(DTS_MIN_HEADER_BYTES as usize)?;
            let header_end = input_offset
                .checked_add(header_input_size)
                .ok_or(MuxError::LayoutOverflow("DTS header end"))?;
            let Some(header) = input.get(input_offset..header_end) else {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "truncated DTS frame header".to_string(),
                });
            };
            let unpacked = unpack_dts_14bit_words(
                header,
                matches!(encoding, DtsInputEncoding::CoreLittleEndian14),
                DTS_MIN_HEADER_BYTES as usize,
            )?;
            Ok(unpacked.try_into().unwrap())
        }
    }
}

fn find_wrapped_dts_payload_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
) -> Result<Option<(u64, DtsInputEncoding)>, MuxError> {
    let scan_size = file_size.min(DTS_FAMILY_CORE_SCAN_LIMIT);
    let scan_size = usize::try_from(scan_size)
        .map_err(|_| MuxError::LayoutOverflow("DTS wrapped scan size"))?;
    if scan_size < DTS_FAMILY_WRAPPER_HEADER.len() + 4 {
        return Ok(None);
    }
    let mut prefix = vec![0_u8; scan_size];
    read_exact_at_sync(
        file,
        0,
        &mut prefix,
        spec,
        "truncated DTS-family input stream",
    )?;
    Ok(find_wrapped_dts_payload_in_bytes(&prefix))
}

#[cfg(feature = "async")]
async fn find_wrapped_dts_payload_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<Option<(u64, DtsInputEncoding)>, MuxError> {
    let scan_size = file_size.min(DTS_FAMILY_CORE_SCAN_LIMIT);
    let scan_size = usize::try_from(scan_size)
        .map_err(|_| MuxError::LayoutOverflow("DTS wrapped scan size"))?;
    if scan_size < DTS_FAMILY_WRAPPER_HEADER.len() + 4 {
        return Ok(None);
    }
    let mut prefix = vec![0_u8; scan_size];
    read_exact_at_async(
        file,
        0,
        &mut prefix,
        spec,
        "truncated DTS-family input stream",
    )
    .await?;
    Ok(find_wrapped_dts_payload_in_bytes(&prefix))
}

fn find_wrapped_dts_payload_in_bytes(bytes: &[u8]) -> Option<(u64, DtsInputEncoding)> {
    if !bytes.starts_with(DTS_FAMILY_WRAPPER_HEADER) {
        return None;
    }
    (DTS_FAMILY_WRAPPER_HEADER.len()..bytes.len().saturating_sub(3)).find_map(|offset| {
        let encoding = dts_input_encoding_from_sync_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])?;
        Some((u64::try_from(offset).unwrap(), encoding))
    })
}

fn dts_input_encoding_from_sync_bytes(sync: [u8; 4]) -> Option<DtsInputEncoding> {
    match sync {
        [0x7F, 0xFE, 0x80, 0x01] => Some(DtsInputEncoding::CoreBigEndian16),
        [0xFE, 0x7F, 0x01, 0x80] => Some(DtsInputEncoding::CoreLittleEndian16),
        [0x1F, 0xFF, 0xE8, 0x00] => Some(DtsInputEncoding::CoreBigEndian14),
        [0xFF, 0x1F, 0x00, 0xE8] => Some(DtsInputEncoding::CoreLittleEndian14),
        _ => None,
    }
}

fn dts_input_encoding_from_sync(sync: [u8; 4], spec: &str) -> Result<DtsInputEncoding, MuxError> {
    dts_input_encoding_from_sync_bytes(sync).ok_or_else(|| {
        unsupported_raw_dts(
            spec,
            "missing core DTS sync word at byte offset 0".to_string(),
        )
    })
}

fn swap_dts_16bit_words(
    input: &[u8],
    spec: &str,
    input_offset: usize,
) -> Result<Vec<u8>, MuxError> {
    if !input.len().is_multiple_of(2) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "little-endian DTS frame at byte offset {input_offset} had an odd byte length"
            ),
        });
    }
    let mut output = vec![0_u8; input.len()];
    for (word, chunk) in input.chunks_exact(2).enumerate() {
        output[word * 2] = chunk[1];
        output[word * 2 + 1] = chunk[0];
    }
    Ok(output)
}

fn packed_14bit_frame_size(normalized_frame_size: usize) -> Result<usize, MuxError> {
    let payload_bits = normalized_frame_size
        .checked_mul(8)
        .ok_or(MuxError::LayoutOverflow("DTS 14-bit payload bits"))?;
    let words = payload_bits.div_ceil(14);
    words
        .checked_mul(2)
        .ok_or(MuxError::LayoutOverflow("DTS 14-bit packed frame size"))
}

fn unpack_dts_14bit_words(
    input: &[u8],
    little_endian: bool,
    output_size: usize,
) -> Result<Vec<u8>, MuxError> {
    if !input.len().is_multiple_of(2) {
        return Err(MuxError::LayoutOverflow("DTS 14-bit input word size"));
    }

    let mut output = Vec::with_capacity(output_size.saturating_add(2));
    let mut bit_buffer = 0_u64;
    let mut buffered_bits = 0usize;
    for chunk in input.chunks_exact(2) {
        let word = if little_endian {
            u16::from_le_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_be_bytes([chunk[0], chunk[1]])
        };
        bit_buffer = (bit_buffer << 14) | u64::from(word & 0x3FFF);
        buffered_bits += 14;
        while buffered_bits >= 8 {
            buffered_bits -= 8;
            output.push(((bit_buffer >> buffered_bits) & 0xFF) as u8);
        }
    }
    output.truncate(output_size);
    Ok(output)
}

fn finalize_parsed_dts_track(
    spec: &str,
    descriptor: Option<DtsTrackDescriptor>,
    samples: Vec<StagedSample>,
    transformed_source: Option<SegmentedMuxSourceSpec>,
    sample_entry_type: FourCc,
) -> Result<ParsedDtsTrack, MuxError> {
    let descriptor = descriptor.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "DTS input contained no frames".to_string(),
    })?;
    let samples = samples
        .into_iter()
        .map(|sample| {
            let duration = u64::from(sample.duration)
                .checked_mul(u64::from(DTS_MEDIA_TIMESCALE))
                .ok_or(MuxError::LayoutOverflow("DTS media duration"))?
                / u64::from(descriptor.sample_rate);
            let duration = u32::try_from(duration)
                .map_err(|_| MuxError::LayoutOverflow("DTS media duration"))?;
            if duration == 0 {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "DTS frame duration underflowed after media-timescale normalization"
                        .to_string(),
                });
            }
            Ok(StagedSample { duration, ..sample })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if samples.iter().all(|sample| sample.duration == 0) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "DTS input contained frames with zero duration".to_string(),
        });
    }
    let btrt = build_btrt_from_sample_sizes(
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        DTS_MEDIA_TIMESCALE,
    )?;
    Ok(ParsedDtsTrack {
        media_timescale: DTS_MEDIA_TIMESCALE,
        sample_entry_box: build_dts_sample_entry_box(descriptor, btrt, sample_entry_type)?,
        samples,
        transformed_source,
    })
}

fn rebuild_wrapped_dts_family_samples(
    normalized_samples: &[StagedSample],
    frame_input_sizes: &[u32],
    wrapper_prefix_size: usize,
    wrapped_input_size: usize,
    consumed_core_input_size: usize,
) -> Result<Vec<StagedSample>, MuxError> {
    if normalized_samples.len() != frame_input_sizes.len() {
        return Err(MuxError::LayoutOverflow(
            "wrapped DTS sample and frame-size mismatch",
        ));
    }
    let wrapper_prefix_size = u32::try_from(wrapper_prefix_size)
        .map_err(|_| MuxError::LayoutOverflow("wrapped DTS prefix size"))?;
    let trailing_family_tail_size = wrapped_input_size
        .checked_sub(usize::try_from(wrapper_prefix_size).unwrap())
        .and_then(|value| value.checked_sub(consumed_core_input_size))
        .ok_or(MuxError::LayoutOverflow("wrapped DTS trailing tail size"))?;
    let trailing_family_tail_size = u32::try_from(trailing_family_tail_size)
        .map_err(|_| MuxError::LayoutOverflow("wrapped DTS trailing tail size"))?;
    let mut data_offset = 0_u64;
    normalized_samples
        .iter()
        .zip(frame_input_sizes)
        .enumerate()
        .map(|(index, (sample, frame_input_size))| {
            let mut data_size = *frame_input_size;
            if index == 0 {
                data_size = data_size
                    .checked_add(wrapper_prefix_size)
                    .ok_or(MuxError::LayoutOverflow("wrapped DTS first-sample size"))?;
            }
            if index + 1 == normalized_samples.len() {
                data_size = data_size
                    .checked_add(trailing_family_tail_size)
                    .ok_or(MuxError::LayoutOverflow("wrapped DTS last-sample size"))?;
            }
            let rebuilt = StagedSample {
                data_offset,
                data_size,
                duration: sample.duration,
                composition_time_offset: sample.composition_time_offset,
                is_sync_sample: sample.is_sync_sample,
            };
            data_offset = data_offset
                .checked_add(u64::from(data_size))
                .ok_or(MuxError::LayoutOverflow("wrapped DTS sample offset"))?;
            Ok(rebuilt)
        })
        .collect()
}

fn parse_dts_frame_header(
    header: &[u8; DTS_MIN_HEADER_BYTES as usize],
    offset: u64,
    spec: &str,
) -> Result<ParsedDtsFrame, MuxError> {
    let mut reader = BitReader::new(Cursor::new(header.as_slice()));
    let sync_word = u32::from_be_bytes(read_bits_exact::<4, _>(&mut reader, spec, "DTS")?);
    if sync_word != DTS_SYNC_WORD {
        return Err(unsupported_raw_dts(
            spec,
            format!("missing core DTS sync word at byte offset {offset}"),
        ));
    }
    skip_bits_labeled(&mut reader, 1 + 5, spec, "DTS")?;
    if read_bit_labeled(&mut reader, spec, "DTS")? {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "DTS frames with CRC protection are not supported".to_string(),
        });
    }
    let blocks_per_frame_minus_one = read_bits_u8_labeled(&mut reader, 7, spec, "DTS")?;
    if blocks_per_frame_minus_one < 5 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "unsupported DTS PCM sample-block count {}",
                blocks_per_frame_minus_one + 1
            ),
        });
    }
    let frame_size_minus_one = read_bits_u16_labeled(&mut reader, 14, spec, "DTS")?;
    if frame_size_minus_one < 95 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "unsupported DTS frame size {}",
                u32::from(frame_size_minus_one) + 1
            ),
        });
    }
    let amode = read_bits_u8_labeled(&mut reader, 6, spec, "DTS")?;
    let sample_rate_code = read_bits_u8_labeled(&mut reader, 4, spec, "DTS")?;
    let sample_rate = DTS_SAMPLE_RATE_BY_CODE
        .get(usize::from(sample_rate_code))
        .and_then(|value| *value)
        .ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported DTS sample-rate code {sample_rate_code}"),
        })?;
    let bitrate_code = read_bits_u8_labeled(&mut reader, 5, spec, "DTS")?;
    if bitrate_code > 25 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported DTS bitrate code {bitrate_code}"),
        });
    }
    let reserved = read_bit_labeled(&mut reader, spec, "DTS")?;
    if reserved {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "reserved DTS header bit was set".to_string(),
        });
    }
    skip_bits_labeled(&mut reader, 1 + 1 + 1 + 1, spec, "DTS")?;
    let ext_audio_id = read_bits_u8_labeled(&mut reader, 3, spec, "DTS")?;
    if !DTS_EXT_AUDIO_ID_VALID[usize::from(ext_audio_id)] {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported DTS extension-audio descriptor flag {ext_audio_id}"),
        });
    }
    skip_bits_labeled(&mut reader, 1 + 1, spec, "DTS")?;
    let lfe_flag = read_bits_u8_labeled(&mut reader, 2, spec, "DTS")?;
    if lfe_flag == 3 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "reserved DTS low-frequency-effects flag value".to_string(),
        });
    }

    let sample_duration = u32::from(blocks_per_frame_minus_one + 1) * 32;
    dts_frame_duration_code(sample_duration).ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("unsupported DTS frame duration {sample_duration}"),
    })?;
    let channel_count = dts_channel_count(amode, lfe_flag != 0).ok_or_else(|| {
        MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("unsupported DTS channel arrangement code {amode}"),
        }
    })?;
    let frame_size = u32::from(frame_size_minus_one) + 1;
    Ok(ParsedDtsFrame {
        descriptor: DtsTrackDescriptor {
            sample_rate,
            sample_duration,
            channel_count,
            sample_depth: 16,
        },
        frame_size,
    })
}

fn build_dts_sample_entry_box(
    descriptor: DtsTrackDescriptor,
    btrt: Btrt,
    sample_entry_type: FourCc,
) -> Result<Vec<u8>, MuxError> {
    let mut sample_entry = AudioSampleEntry::default();
    sample_entry.set_box_type(sample_entry_type);
    sample_entry.sample_entry = SampleEntry {
        box_type: sample_entry_type,
        data_reference_index: 1,
    };
    sample_entry.channel_count = descriptor.channel_count;
    sample_entry.sample_size = u16::from(descriptor.sample_depth);
    sample_entry.sample_rate = if sample_entry_type == DTSX {
        0
    } else {
        descriptor.sample_rate << 16
    };

    let btrt_bytes = super::super::mp4::encode_typed_box(&btrt, &[])?;
    super::super::mp4::encode_typed_box(&sample_entry, &btrt_bytes)
}

/// Rewrites carried transport DTS sample entries onto the transport-oriented box type.
pub(in crate::mux) fn retune_carried_dts_sample_entry_box(
    sample_entry_box: &[u8],
) -> Result<Vec<u8>, MuxError> {
    if sample_entry_box.len() < 36 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: "dts".to_string(),
            message:
                "carried DTS sample entry is truncated before the fixed audio sample-entry fields"
                    .to_string(),
        });
    }
    if sample_entry_box[4..8] != DTSC.into_bytes() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: "dts".to_string(),
            message: "carried DTS sample entry did not use the expected core DTS box type"
                .to_string(),
        });
    }

    let mut rebuilt = sample_entry_box.to_vec();
    rebuilt[4..8].copy_from_slice(&DTSX.into_bytes());
    rebuilt[24..26].copy_from_slice(&2_u16.to_be_bytes());
    rebuilt[32..36].copy_from_slice(&0_u32.to_be_bytes());
    Ok(rebuilt)
}

const fn dts_frame_duration_code(sample_duration: u32) -> Option<u8> {
    match sample_duration {
        512 => Some(0),
        1024 => Some(1),
        2048 => Some(2),
        4096 => Some(3),
        _ => None,
    }
}

const fn dts_channel_count(amode: u8, lfe_present: bool) -> Option<u16> {
    if amode > 15 {
        return None;
    }
    Some(DTS_CORE_CHANNELS_BY_AMODE[amode as usize] + if lfe_present { 1 } else { 0 })
}

fn skip_bits_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<(), MuxError>
where
    R: std::io::Read,
{
    reader
        .read_bits(width)
        .map(|_| ())
        .map_err(|error| truncated_dts_error(spec, label, error))
}

fn read_bit_labeled<R>(reader: &mut BitReader<R>, spec: &str, label: &str) -> Result<bool, MuxError>
where
    R: std::io::Read,
{
    reader
        .read_bit()
        .map_err(|error| truncated_dts_error(spec, label, error))
}

fn read_bits_u8_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u8, MuxError>
where
    R: std::io::Read,
{
    let bytes = reader
        .read_bits(width)
        .map_err(|error| truncated_dts_error(spec, label, error))?;
    Ok(bytes[0])
}

fn read_bits_u16_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u16, MuxError>
where
    R: std::io::Read,
{
    let bytes = reader
        .read_bits(width)
        .map_err(|error| truncated_dts_error(spec, label, error))?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_bits_exact<const N: usize, R>(
    reader: &mut BitReader<R>,
    spec: &str,
    label: &str,
) -> Result<[u8; N], MuxError>
where
    R: std::io::Read,
{
    let bytes = reader
        .read_bits(N * 8)
        .map_err(|error| truncated_dts_error(spec, label, error))?;
    Ok(bytes.try_into().unwrap())
}

fn truncated_dts_error(spec: &str, label: &str, error: std::io::Error) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("{label} parsing failed: {error}"),
    }
}

fn unsupported_raw_dts(spec: &str, message: String) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("{message}; {RAW_DTS_DIRECT_INGEST_NOTE}"),
    }
}
