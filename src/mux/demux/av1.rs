use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::FourCc;
use crate::boxes::av1::AV1CodecConfiguration;
use crate::boxes::iso14496_12::{Colr, Pasp};

use super::super::import::build_visual_sample_entry_box;
use super::super::import::{
    SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData, SegmentedMuxSourceSpec, StagedSample,
};
use super::super::{MuxError, MuxRawCodec};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;
#[cfg(feature = "async")]
use super::ivf_common::read_indexed_sample_async;
#[cfg(feature = "async")]
use super::ivf_common::scan_ivf_video_file_async;
use super::ivf_common::{read_indexed_sample_sync, scan_ivf_video_file_sync};

const AV1_COLOUR_TYPE_NCLX: FourCc = FourCc::from_bytes(*b"nclx");
const OBU_SEQUENCE_HEADER: u8 = 1;
const OBU_TEMPORAL_DELIMITER: u8 = 2;
const RAW_AV1_OBU_TIMESCALE: u32 = 1_200_000;
const RAW_AV1_OBU_SAMPLE_DURATION: u32 = 48_000;
const RAW_AV1_ANNEX_B_TIMESCALE: u32 = 25_000;
const RAW_AV1_ANNEX_B_SAMPLE_DURATION: u32 = 1_000;
const TRANSPORT_AV1_TIMESCALE: u32 = 90_000;

pub(in crate::mux) enum ParsedAv1TrackSource {
    File,
    Segmented(SegmentedMuxSourceSpec),
}

pub(in crate::mux) struct ParsedAv1Track {
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) timescale: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
    pub(in crate::mux) source: ParsedAv1TrackSource,
}

pub(in crate::mux) fn scan_av1_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedAv1Track, MuxError> {
    if path_starts_with(path, b"DKIF")? {
        return scan_av1_ivf_sync(path, spec);
    }
    match av1_non_ivf_input_form(path) {
        Some(Av1InputForm::Section5Obu) => scan_av1_section5_sync(path, spec),
        Some(Av1InputForm::AnnexB) => scan_av1_annex_b_sync(path, spec),
        Some(Av1InputForm::GenericAv1) => match scan_av1_annex_b_sync(path, spec) {
            Ok(track) => Ok(track),
            Err(MuxError::UnsupportedTrackImport { .. }) => scan_av1_section5_sync(path, spec),
            Err(error) => Err(error),
        },
        None => Err(unsupported(
            spec,
            "raw AV1 direct ingest currently expects IVF, `.obu`, `.av1`, or `.av1b` inputs",
        )),
    }
}

pub(in crate::mux) fn scan_transport_av1_segmented_sync(
    path: &Path,
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    sample_offsets: &[u64],
    carried_descriptor: [u8; 4],
    spec: &str,
) -> Result<ParsedAv1Track, MuxError> {
    scan_transport_av1_segmented_inner(
        path,
        total_size,
        sample_offsets,
        carried_descriptor,
        spec,
        |offset, bytes, message| {
            read_segmented_bytes_sync(file, segments, total_size, offset, bytes, spec, message)
        },
    )
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_av1_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedAv1Track, MuxError> {
    if path_starts_with_async(path, b"DKIF").await? {
        return scan_av1_ivf_async(path, spec).await;
    }
    match av1_non_ivf_input_form(path) {
        Some(Av1InputForm::Section5Obu) => scan_av1_section5_async(path, spec).await,
        Some(Av1InputForm::AnnexB) => scan_av1_annex_b_async(path, spec).await,
        Some(Av1InputForm::GenericAv1) => match scan_av1_annex_b_async(path, spec).await {
            Ok(track) => Ok(track),
            Err(MuxError::UnsupportedTrackImport { .. }) => {
                scan_av1_section5_async(path, spec).await
            }
            Err(error) => Err(error),
        },
        None => Err(unsupported(
            spec,
            "raw AV1 direct ingest currently expects IVF, `.obu`, `.av1`, or `.av1b` inputs",
        )),
    }
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_transport_av1_segmented_async(
    path: &Path,
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    sample_offsets: &[u64],
    carried_descriptor: [u8; 4],
    spec: &str,
) -> Result<ParsedAv1Track, MuxError> {
    if sample_offsets.is_empty() {
        return Err(unsupported(
            spec,
            "transport-stream AV1 carriage did not expose any PES access-unit boundaries",
        ));
    }

    let mut logical_size = 0_u64;
    let mut transformed_segments = Vec::with_capacity(sample_offsets.len());
    let mut samples = Vec::with_capacity(sample_offsets.len());
    let mut first_sample_bytes = None::<Vec<u8>>;

    for (index, &sample_offset) in sample_offsets.iter().enumerate() {
        let sample_end = sample_offsets.get(index + 1).copied().unwrap_or(total_size);
        if sample_end <= sample_offset || sample_end > total_size {
            return Err(unsupported(
                spec,
                "transport-stream AV1 PES access-unit boundaries were malformed",
            ));
        }
        let sample_size = usize::try_from(sample_end - sample_offset)
            .map_err(|_| MuxError::LayoutOverflow("transport-stream AV1 sample size"))?;
        let mut sample_bytes = vec![0_u8; sample_size];
        read_segmented_bytes_async(
            file,
            segments,
            total_size,
            sample_offset,
            &mut sample_bytes,
            spec,
            "transport-stream AV1 sample payload is truncated",
        )
        .await?;
        let normalized = normalize_transport_av1_sample(&sample_bytes, spec)?;
        if normalized.is_empty() {
            return Err(unsupported(
                spec,
                "transport-stream AV1 sample payload did not contain any decodable OBUs",
            ));
        }
        if first_sample_bytes.is_none() {
            first_sample_bytes = Some(normalized.clone());
        }
        let normalized_size = u32::try_from(normalized.len())
            .map_err(|_| MuxError::LayoutOverflow("transport-stream AV1 normalized sample"))?;
        transformed_segments.push(SegmentedMuxSourceSegment {
            logical_offset: logical_size,
            data: SegmentedMuxSourceSegmentData::Bytes(normalized),
        });
        samples.push(StagedSample {
            data_offset: logical_size,
            data_size: normalized_size,
            duration: 0,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        logical_size = logical_size.checked_add(u64::from(normalized_size)).ok_or(
            MuxError::LayoutOverflow("transport-stream AV1 transformed payload"),
        )?;
    }

    let first_sample_bytes = first_sample_bytes.ok_or_else(|| {
        unsupported(
            spec,
            "transport-stream AV1 input did not produce any decodable sample payloads",
        )
    })?;
    let (sample_entry_box, width, height) = build_transport_av1_sample_entry_box_from_sample(
        &first_sample_bytes,
        carried_descriptor,
        spec,
    )?;
    Ok(ParsedAv1Track {
        width,
        height,
        timescale: TRANSPORT_AV1_TIMESCALE,
        sample_entry_box,
        samples,
        source: ParsedAv1TrackSource::Segmented(SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: transformed_segments,
            total_size: logical_size,
        }),
    })
}

#[derive(Clone, Copy)]
enum Av1InputForm {
    Section5Obu,
    AnnexB,
    GenericAv1,
}

#[derive(Clone, Copy)]
enum RawAv1TrackProfile {
    Section5Obu,
    AnnexB,
}

fn av1_non_ivf_input_form(path: &Path) -> Option<Av1InputForm> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("obu") {
        return Some(Av1InputForm::Section5Obu);
    }
    if extension.eq_ignore_ascii_case("av1b") {
        return Some(Av1InputForm::AnnexB);
    }
    if extension.eq_ignore_ascii_case("av1") {
        return Some(Av1InputForm::GenericAv1);
    }
    None
}

fn path_starts_with(path: &Path, signature: &[u8]) -> Result<bool, MuxError> {
    let mut file = File::open(path)?;
    let mut prefix = vec![0_u8; signature.len()];
    let read = file.read(&mut prefix)?;
    Ok(read == signature.len() && prefix == signature)
}

#[cfg(feature = "async")]
async fn path_starts_with_async(path: &Path, signature: &[u8]) -> Result<bool, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let mut prefix = vec![0_u8; signature.len()];
    let read = file.read(&mut prefix).await?;
    Ok(read == signature.len() && prefix == signature)
}

fn scan_av1_ivf_sync(path: &Path, spec: &str) -> Result<ParsedAv1Track, MuxError> {
    let mut indexed = scan_ivf_video_file_sync(path, MuxRawCodec::Av1, spec)?;
    normalize_av1_ivf_sample_spans_sync(path, &mut indexed, spec)?;
    let first_sample = read_indexed_sample_sync(
        path,
        indexed.first_sample_span,
        spec,
        "IVF AV1 sample payload is truncated",
    )?;
    let (sample_entry_box, width, height) =
        build_av1_sample_entry_box_from_sample(&first_sample, spec)?;
    Ok(ParsedAv1Track {
        width,
        height,
        timescale: indexed.timescale,
        sample_entry_box,
        samples: indexed.samples,
        source: ParsedAv1TrackSource::File,
    })
}

#[cfg(feature = "async")]
async fn scan_av1_ivf_async(path: &Path, spec: &str) -> Result<ParsedAv1Track, MuxError> {
    let mut indexed = scan_ivf_video_file_async(path, MuxRawCodec::Av1, spec).await?;
    normalize_av1_ivf_sample_spans_async(path, &mut indexed, spec).await?;
    let first_sample = read_indexed_sample_async(
        path,
        indexed.first_sample_span,
        spec,
        "IVF AV1 sample payload is truncated",
    )
    .await?;
    let (sample_entry_box, width, height) =
        build_av1_sample_entry_box_from_sample(&first_sample, spec)?;
    Ok(ParsedAv1Track {
        width,
        height,
        timescale: indexed.timescale,
        sample_entry_box,
        samples: indexed.samples,
        source: ParsedAv1TrackSource::File,
    })
}

fn scan_av1_section5_sync(path: &Path, spec: &str) -> Result<ParsedAv1Track, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut saw_temporal_delimiter = false;
    let mut first_sample_bytes = Vec::new();
    let mut current_sample_offset = None::<u64>;
    let mut current_sample_size = 0_u32;
    let mut samples = Vec::new();

    while offset < file_size {
        let obu = read_section5_obu_sync(&mut file, file_size, offset, spec, samples.is_empty())?;
        offset = offset
            .checked_add(u64::from(obu.total_size))
            .ok_or(MuxError::LayoutOverflow("raw AV1 OBU offset"))?;
        if obu.obu_type == OBU_TEMPORAL_DELIMITER {
            saw_temporal_delimiter = true;
            if let Some(sample_offset) = current_sample_offset.take() {
                if current_sample_size == 0 {
                    return Err(unsupported(
                        spec,
                        "raw AV1 OBU streams contained an empty temporal unit",
                    ));
                }
                samples.push(StagedSample {
                    data_offset: sample_offset,
                    data_size: current_sample_size,
                    duration: RAW_AV1_OBU_SAMPLE_DURATION,
                    composition_time_offset: 0,
                    is_sync_sample: true,
                });
                current_sample_size = 0;
            }
            continue;
        }
        if !saw_temporal_delimiter && current_sample_offset.is_none() && samples.is_empty() {
            return Err(unsupported(
                spec,
                "raw AV1 OBU streams must begin each temporal unit with a temporal-delimiter OBU",
            ));
        }
        if current_sample_offset.is_none() {
            current_sample_offset = Some(obu.file_offset);
        }
        current_sample_size = current_sample_size
            .checked_add(obu.total_size)
            .ok_or(MuxError::LayoutOverflow("raw AV1 sample size"))?;
        if samples.is_empty() {
            first_sample_bytes.extend_from_slice(&obu.normalized_bytes);
        }
    }

    if let Some(sample_offset) = current_sample_offset.take() {
        if current_sample_size == 0 {
            return Err(unsupported(
                spec,
                "raw AV1 OBU streams contained an empty trailing temporal unit",
            ));
        }
        samples.push(StagedSample {
            data_offset: sample_offset,
            data_size: current_sample_size,
            duration: RAW_AV1_OBU_SAMPLE_DURATION,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }

    finalize_raw_av1_track(
        path,
        spec,
        RawAv1TrackProfile::Section5Obu,
        RAW_AV1_OBU_TIMESCALE,
        first_sample_bytes,
        samples,
        ParsedAv1TrackSource::File,
    )
}

#[cfg(feature = "async")]
async fn scan_av1_section5_async(path: &Path, spec: &str) -> Result<ParsedAv1Track, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut saw_temporal_delimiter = false;
    let mut first_sample_bytes = Vec::new();
    let mut current_sample_offset = None::<u64>;
    let mut current_sample_size = 0_u32;
    let mut samples = Vec::new();

    while offset < file_size {
        let obu =
            read_section5_obu_async(&mut file, file_size, offset, spec, samples.is_empty()).await?;
        offset = offset
            .checked_add(u64::from(obu.total_size))
            .ok_or(MuxError::LayoutOverflow("raw AV1 OBU offset"))?;
        if obu.obu_type == OBU_TEMPORAL_DELIMITER {
            saw_temporal_delimiter = true;
            if let Some(sample_offset) = current_sample_offset.take() {
                if current_sample_size == 0 {
                    return Err(unsupported(
                        spec,
                        "raw AV1 OBU streams contained an empty temporal unit",
                    ));
                }
                samples.push(StagedSample {
                    data_offset: sample_offset,
                    data_size: current_sample_size,
                    duration: RAW_AV1_OBU_SAMPLE_DURATION,
                    composition_time_offset: 0,
                    is_sync_sample: true,
                });
                current_sample_size = 0;
            }
            continue;
        }
        if !saw_temporal_delimiter && current_sample_offset.is_none() && samples.is_empty() {
            return Err(unsupported(
                spec,
                "raw AV1 OBU streams must begin each temporal unit with a temporal-delimiter OBU",
            ));
        }
        if current_sample_offset.is_none() {
            current_sample_offset = Some(obu.file_offset);
        }
        current_sample_size = current_sample_size
            .checked_add(obu.total_size)
            .ok_or(MuxError::LayoutOverflow("raw AV1 sample size"))?;
        if samples.is_empty() {
            first_sample_bytes.extend_from_slice(&obu.normalized_bytes);
        }
    }

    if let Some(sample_offset) = current_sample_offset.take() {
        if current_sample_size == 0 {
            return Err(unsupported(
                spec,
                "raw AV1 OBU streams contained an empty trailing temporal unit",
            ));
        }
        samples.push(StagedSample {
            data_offset: sample_offset,
            data_size: current_sample_size,
            duration: RAW_AV1_OBU_SAMPLE_DURATION,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }

    finalize_raw_av1_track(
        path,
        spec,
        RawAv1TrackProfile::Section5Obu,
        RAW_AV1_OBU_TIMESCALE,
        first_sample_bytes,
        samples,
        ParsedAv1TrackSource::File,
    )
}

fn scan_av1_annex_b_sync(path: &Path, spec: &str) -> Result<ParsedAv1Track, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut logical_size = 0_u64;
    let mut segments = Vec::new();
    let mut samples = Vec::new();
    let mut first_sample_bytes = Vec::new();

    while offset < file_size {
        let (temporal_unit_size, temporal_unit_leb_size) =
            read_leb128_from_file_sync(&mut file, offset, spec, "AV1 temporal-unit size")?;
        offset = offset
            .checked_add(u64::try_from(temporal_unit_leb_size).unwrap())
            .ok_or(MuxError::LayoutOverflow("AV1 temporal-unit offset"))?;
        if temporal_unit_size == 0 {
            return Err(unsupported(
                spec,
                "AV1 Annex B temporal units must have non-zero sizes",
            ));
        }
        let temporal_unit_end = offset
            .checked_add(temporal_unit_size)
            .ok_or(MuxError::LayoutOverflow("AV1 temporal-unit size"))?;
        if temporal_unit_end > file_size {
            return Err(unsupported(
                spec,
                "AV1 Annex B temporal-unit payload overruns the input length",
            ));
        }

        let sample_offset = logical_size;
        let mut sample_size = 0_u32;
        let capture_first_sample = samples.is_empty();

        while offset < temporal_unit_end {
            let (frame_unit_size, frame_unit_leb_size) =
                read_leb128_from_file_sync(&mut file, offset, spec, "AV1 frame-unit size")?;
            offset = offset
                .checked_add(u64::try_from(frame_unit_leb_size).unwrap())
                .ok_or(MuxError::LayoutOverflow("AV1 frame-unit offset"))?;
            if frame_unit_size == 0 {
                return Err(unsupported(
                    spec,
                    "AV1 Annex B frame units must have non-zero sizes",
                ));
            }
            let frame_unit_end = offset
                .checked_add(frame_unit_size)
                .ok_or(MuxError::LayoutOverflow("AV1 frame-unit size"))?;
            if frame_unit_end > temporal_unit_end {
                return Err(unsupported(
                    spec,
                    "AV1 Annex B frame-unit payload overruns its temporal unit",
                ));
            }
            while offset < frame_unit_end {
                let (obu_size, obu_leb_size) =
                    read_leb128_from_file_sync(&mut file, offset, spec, "AV1 OBU size")?;
                offset = offset
                    .checked_add(u64::try_from(obu_leb_size).unwrap())
                    .ok_or(MuxError::LayoutOverflow("AV1 OBU offset"))?;
                if obu_size == 0 {
                    return Err(unsupported(
                        spec,
                        "AV1 Annex B OBUs must have non-zero sizes",
                    ));
                }
                let obu_end = offset
                    .checked_add(obu_size)
                    .ok_or(MuxError::LayoutOverflow("AV1 OBU size"))?;
                if obu_end > frame_unit_end {
                    return Err(unsupported(
                        spec,
                        "AV1 Annex B OBU payload overruns its frame unit",
                    ));
                }
                let parsed_obu = read_annex_b_obu_sync(
                    &mut file,
                    offset,
                    u32::try_from(obu_size)
                        .map_err(|_| MuxError::LayoutOverflow("AV1 Annex B OBU size"))?,
                    spec,
                    capture_first_sample,
                )?;
                offset = obu_end;
                if parsed_obu.obu_type == OBU_TEMPORAL_DELIMITER {
                    continue;
                }
                if sample_size == 0 && logical_size != sample_offset {
                    return Err(MuxError::LayoutOverflow("AV1 sample logical offset"));
                }
                append_segmented_av1_bytes(
                    &mut segments,
                    &mut logical_size,
                    &mut sample_size,
                    parsed_obu.segment_prefix,
                    parsed_obu.file_range,
                )?;
                if capture_first_sample {
                    first_sample_bytes.extend_from_slice(&parsed_obu.normalized_bytes);
                }
            }
        }
        if offset != temporal_unit_end || sample_size == 0 {
            return Err(unsupported(
                spec,
                "AV1 Annex B temporal units must contribute at least one non-delimiter OBU sample payload",
            ));
        }
        samples.push(StagedSample {
            data_offset: sample_offset,
            data_size: sample_size,
            duration: RAW_AV1_ANNEX_B_SAMPLE_DURATION,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }

    finalize_raw_av1_track(
        path,
        spec,
        RawAv1TrackProfile::AnnexB,
        RAW_AV1_ANNEX_B_TIMESCALE,
        first_sample_bytes,
        samples,
        ParsedAv1TrackSource::Segmented(SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments,
            total_size: logical_size,
        }),
    )
}

#[cfg(feature = "async")]
async fn scan_av1_annex_b_async(path: &Path, spec: &str) -> Result<ParsedAv1Track, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut logical_size = 0_u64;
    let mut segments = Vec::new();
    let mut samples = Vec::new();
    let mut first_sample_bytes = Vec::new();

    while offset < file_size {
        let (temporal_unit_size, temporal_unit_leb_size) =
            read_leb128_from_file_async(&mut file, offset, spec, "AV1 temporal-unit size").await?;
        offset = offset
            .checked_add(u64::try_from(temporal_unit_leb_size).unwrap())
            .ok_or(MuxError::LayoutOverflow("AV1 temporal-unit offset"))?;
        if temporal_unit_size == 0 {
            return Err(unsupported(
                spec,
                "AV1 Annex B temporal units must have non-zero sizes",
            ));
        }
        let temporal_unit_end = offset
            .checked_add(temporal_unit_size)
            .ok_or(MuxError::LayoutOverflow("AV1 temporal-unit size"))?;
        if temporal_unit_end > file_size {
            return Err(unsupported(
                spec,
                "AV1 Annex B temporal-unit payload overruns the input length",
            ));
        }

        let sample_offset = logical_size;
        let mut sample_size = 0_u32;
        let capture_first_sample = samples.is_empty();

        while offset < temporal_unit_end {
            let (frame_unit_size, frame_unit_leb_size) =
                read_leb128_from_file_async(&mut file, offset, spec, "AV1 frame-unit size").await?;
            offset = offset
                .checked_add(u64::try_from(frame_unit_leb_size).unwrap())
                .ok_or(MuxError::LayoutOverflow("AV1 frame-unit offset"))?;
            if frame_unit_size == 0 {
                return Err(unsupported(
                    spec,
                    "AV1 Annex B frame units must have non-zero sizes",
                ));
            }
            let frame_unit_end = offset
                .checked_add(frame_unit_size)
                .ok_or(MuxError::LayoutOverflow("AV1 frame-unit size"))?;
            if frame_unit_end > temporal_unit_end {
                return Err(unsupported(
                    spec,
                    "AV1 Annex B frame-unit payload overruns its temporal unit",
                ));
            }
            while offset < frame_unit_end {
                let (obu_size, obu_leb_size) =
                    read_leb128_from_file_async(&mut file, offset, spec, "AV1 OBU size").await?;
                offset = offset
                    .checked_add(u64::try_from(obu_leb_size).unwrap())
                    .ok_or(MuxError::LayoutOverflow("AV1 OBU offset"))?;
                if obu_size == 0 {
                    return Err(unsupported(
                        spec,
                        "AV1 Annex B OBUs must have non-zero sizes",
                    ));
                }
                let obu_end = offset
                    .checked_add(obu_size)
                    .ok_or(MuxError::LayoutOverflow("AV1 OBU size"))?;
                if obu_end > frame_unit_end {
                    return Err(unsupported(
                        spec,
                        "AV1 Annex B OBU payload overruns its frame unit",
                    ));
                }
                let parsed_obu = read_annex_b_obu_async(
                    &mut file,
                    offset,
                    u32::try_from(obu_size)
                        .map_err(|_| MuxError::LayoutOverflow("AV1 Annex B OBU size"))?,
                    spec,
                    capture_first_sample,
                )
                .await?;
                offset = obu_end;
                if parsed_obu.obu_type == OBU_TEMPORAL_DELIMITER {
                    continue;
                }
                append_segmented_av1_bytes(
                    &mut segments,
                    &mut logical_size,
                    &mut sample_size,
                    parsed_obu.segment_prefix,
                    parsed_obu.file_range,
                )?;
                if capture_first_sample {
                    first_sample_bytes.extend_from_slice(&parsed_obu.normalized_bytes);
                }
            }
        }
        if offset != temporal_unit_end || sample_size == 0 {
            return Err(unsupported(
                spec,
                "AV1 Annex B temporal units must contribute at least one non-delimiter OBU sample payload",
            ));
        }
        samples.push(StagedSample {
            data_offset: sample_offset,
            data_size: sample_size,
            duration: RAW_AV1_ANNEX_B_SAMPLE_DURATION,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }

    finalize_raw_av1_track(
        path,
        spec,
        RawAv1TrackProfile::AnnexB,
        RAW_AV1_ANNEX_B_TIMESCALE,
        first_sample_bytes,
        samples,
        ParsedAv1TrackSource::Segmented(SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments,
            total_size: logical_size,
        }),
    )
}

fn finalize_raw_av1_track(
    _path: &Path,
    spec: &str,
    profile: RawAv1TrackProfile,
    timescale: u32,
    first_sample_bytes: Vec<u8>,
    samples: Vec<StagedSample>,
    source: ParsedAv1TrackSource,
) -> Result<ParsedAv1Track, MuxError> {
    if samples.is_empty() || first_sample_bytes.is_empty() {
        return Err(unsupported(
            spec,
            "raw AV1 input did not produce any decodable sample payloads",
        ));
    }
    let (sample_entry_box, width, height) = build_raw_av1_sample_entry_box_from_sample(
        profile,
        &first_sample_bytes,
        &samples,
        timescale,
        spec,
    )?;
    Ok(ParsedAv1Track {
        width,
        height,
        timescale,
        sample_entry_box,
        samples,
        source,
    })
}

fn scan_transport_av1_segmented_inner<F>(
    path: &Path,
    total_size: u64,
    sample_offsets: &[u64],
    carried_descriptor: [u8; 4],
    spec: &str,
    mut read_exact: F,
) -> Result<ParsedAv1Track, MuxError>
where
    F: FnMut(u64, &mut [u8], &'static str) -> Result<(), MuxError>,
{
    if sample_offsets.is_empty() {
        return Err(unsupported(
            spec,
            "transport-stream AV1 carriage did not expose any PES access-unit boundaries",
        ));
    }

    let mut logical_size = 0_u64;
    let mut transformed_segments = Vec::with_capacity(sample_offsets.len());
    let mut samples = Vec::with_capacity(sample_offsets.len());
    let mut first_sample_bytes = None::<Vec<u8>>;

    for (index, &sample_offset) in sample_offsets.iter().enumerate() {
        let sample_end = sample_offsets.get(index + 1).copied().unwrap_or(total_size);
        if sample_end <= sample_offset || sample_end > total_size {
            return Err(unsupported(
                spec,
                "transport-stream AV1 PES access-unit boundaries were malformed",
            ));
        }
        let sample_size = usize::try_from(sample_end - sample_offset)
            .map_err(|_| MuxError::LayoutOverflow("transport-stream AV1 sample size"))?;
        let mut sample_bytes = vec![0_u8; sample_size];
        read_exact(
            sample_offset,
            &mut sample_bytes,
            "transport-stream AV1 sample payload is truncated",
        )?;
        let normalized = normalize_transport_av1_sample(&sample_bytes, spec)?;
        if normalized.is_empty() {
            return Err(unsupported(
                spec,
                "transport-stream AV1 sample payload did not contain any decodable OBUs",
            ));
        }
        if first_sample_bytes.is_none() {
            first_sample_bytes = Some(normalized.clone());
        }
        let normalized_size = u32::try_from(normalized.len())
            .map_err(|_| MuxError::LayoutOverflow("transport-stream AV1 normalized sample"))?;
        transformed_segments.push(SegmentedMuxSourceSegment {
            logical_offset: logical_size,
            data: SegmentedMuxSourceSegmentData::Bytes(normalized),
        });
        samples.push(StagedSample {
            data_offset: logical_size,
            data_size: normalized_size,
            duration: 0,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        logical_size = logical_size.checked_add(u64::from(normalized_size)).ok_or(
            MuxError::LayoutOverflow("transport-stream AV1 transformed payload"),
        )?;
    }

    let first_sample_bytes = first_sample_bytes.ok_or_else(|| {
        unsupported(
            spec,
            "transport-stream AV1 input did not produce any decodable sample payloads",
        )
    })?;
    let (sample_entry_box, width, height) = build_transport_av1_sample_entry_box_from_sample(
        &first_sample_bytes,
        carried_descriptor,
        spec,
    )?;
    Ok(ParsedAv1Track {
        width,
        height,
        timescale: TRANSPORT_AV1_TIMESCALE,
        sample_entry_box,
        samples,
        source: ParsedAv1TrackSource::Segmented(SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: transformed_segments,
            total_size: logical_size,
        }),
    })
}

fn build_av1_sample_entry_box_from_sample(
    sample: &[u8],
    spec: &str,
) -> Result<(Vec<u8>, u16, u16), MuxError> {
    let (config, colr, width, height) = parse_av1_sample_entry_details(sample, spec)?;
    let child_boxes = vec![
        super::super::mp4::encode_typed_box(&config, &[])?,
        super::super::mp4::encode_typed_box(&colr, &[])?,
    ];
    let sample_entry_box =
        build_visual_sample_entry_box(FourCc::from_bytes(*b"av01"), width, height, &child_boxes)?;
    Ok((sample_entry_box, width, height))
}

fn build_transport_av1_sample_entry_box_from_sample(
    sample: &[u8],
    _carried_descriptor: [u8; 4],
    spec: &str,
) -> Result<(Vec<u8>, u16, u16), MuxError> {
    let (config, colr, width, height) = parse_av1_sample_entry_details(sample, spec)?;
    let child_boxes = vec![
        super::super::mp4::encode_typed_box(&config, &[])?,
        super::super::mp4::encode_typed_box(&colr, &[])?,
    ];
    let sample_entry_box =
        build_visual_sample_entry_box(FourCc::from_bytes(*b"av01"), width, height, &child_boxes)?;
    Ok((sample_entry_box, width, height))
}

fn build_raw_av1_sample_entry_box_from_sample(
    profile: RawAv1TrackProfile,
    sample: &[u8],
    _samples: &[StagedSample],
    _timescale: u32,
    spec: &str,
) -> Result<(Vec<u8>, u16, u16), MuxError> {
    let (config, colr, width, height) = parse_av1_sample_entry_details(sample, spec)?;
    let mut child_boxes = vec![super::super::mp4::encode_typed_box(&config, &[])?];
    if matches!(profile, RawAv1TrackProfile::Section5Obu) {
        child_boxes.push(super::super::mp4::encode_typed_box(
            &Pasp {
                h_spacing: 1,
                v_spacing: 1,
            },
            &[],
        )?);
    }
    child_boxes.push(super::super::mp4::encode_typed_box(&colr, &[])?);
    let sample_entry_box =
        build_visual_sample_entry_box(FourCc::from_bytes(*b"av01"), width, height, &child_boxes)?;
    Ok((sample_entry_box, width, height))
}

fn normalize_transport_av1_sample(sample: &[u8], spec: &str) -> Result<Vec<u8>, MuxError> {
    let mut offset = 0usize;
    let mut normalized = Vec::new();
    while offset < sample.len() {
        let start = find_transport_av1_start_code(sample, offset).ok_or_else(|| {
            unsupported(
                spec,
                "transport-stream AV1 sample payload did not begin with MPEG-TS start-code framing",
            )
        })?;
        let payload_start = start + 3;
        let next_start =
            find_transport_av1_start_code(sample, payload_start).unwrap_or(sample.len());
        if next_start <= payload_start {
            return Err(unsupported(
                spec,
                "transport-stream AV1 sample payload carried an empty start-code framed OBU",
            ));
        }
        let encoded_obu = &sample[payload_start..next_start];
        let obu = remove_transport_av1_emulation_bytes(encoded_obu);
        validate_transport_av1_obu(&obu, spec)?;
        normalized.extend_from_slice(&obu);
        offset = next_start;
    }
    Ok(normalized)
}

fn find_transport_av1_start_code(bytes: &[u8], offset: usize) -> Option<usize> {
    let mut cursor = offset;
    while cursor + 3 <= bytes.len() {
        if bytes[cursor..].starts_with(&[0x00, 0x00, 0x01]) {
            return Some(cursor);
        }
        cursor += 1;
    }
    None
}

fn remove_transport_av1_emulation_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if index + 2 < bytes.len()
            && bytes[index] == 0x00
            && bytes[index + 1] == 0x00
            && bytes[index + 2] == 0x03
            && bytes.get(index + 3).is_some_and(|next| *next <= 0x03)
        {
            normalized.push(0x00);
            normalized.push(0x00);
            index += 3;
            continue;
        }
        normalized.push(bytes[index]);
        index += 1;
    }
    normalized
}

fn validate_transport_av1_obu(obu: &[u8], spec: &str) -> Result<(), MuxError> {
    if obu.is_empty() {
        return Err(unsupported(
            spec,
            "transport-stream AV1 carried an empty OBU after removing MPEG-TS framing",
        ));
    }
    let header = obu[0];
    let extension_flag = (header >> 2) & 0x01 != 0;
    let has_size_field = (header >> 1) & 0x01 != 0;
    if !has_size_field {
        return Err(unsupported(
            spec,
            "transport-stream AV1 OBUs must keep explicit size fields on the native direct-ingest path",
        ));
    }
    let payload_size_offset = if extension_flag { 2 } else { 1 };
    if payload_size_offset >= obu.len() {
        return Err(unsupported(
            spec,
            "transport-stream AV1 OBU header was truncated before the payload-size field",
        ));
    }
    let (payload_size, leb_size) = read_leb128_from_slice(
        &obu[payload_size_offset..],
        spec,
        "transport-stream AV1 OBU size",
        u64::try_from(payload_size_offset).unwrap(),
    )?;
    let payload_offset =
        payload_size_offset
            .checked_add(leb_size)
            .ok_or(MuxError::LayoutOverflow(
                "transport-stream AV1 OBU payload offset",
            ))?;
    let payload_end = payload_offset
        .checked_add(usize::try_from(payload_size).unwrap())
        .ok_or(MuxError::LayoutOverflow(
            "transport-stream AV1 OBU payload length",
        ))?;
    if payload_end != obu.len() {
        return Err(unsupported(
            spec,
            "transport-stream AV1 OBU size fields did not match the start-code framed payload length",
        ));
    }
    Ok(())
}

struct ParsedSection5Obu {
    file_offset: u64,
    total_size: u32,
    obu_type: u8,
    normalized_bytes: Vec<u8>,
}

struct ParsedAnnexBObu {
    obu_type: u8,
    segment_prefix: Option<Vec<u8>>,
    file_range: Option<(u64, u32)>,
    normalized_bytes: Vec<u8>,
}

fn parse_av1_sample_entry_details(
    sample: &[u8],
    spec: &str,
) -> Result<(AV1CodecConfiguration, Colr, u16, u16), MuxError> {
    let (config_obus, sequence_header) = find_av1_sequence_header_obu(sample, spec)?;
    let mut config = AV1CodecConfiguration {
        seq_profile: sequence_header.seq_profile,
        seq_level_idx_0: sequence_header.seq_level_idx_0,
        seq_tier_0: sequence_header.seq_tier_0,
        high_bitdepth: u8::from(sequence_header.high_bitdepth),
        twelve_bit: u8::from(sequence_header.twelve_bit),
        monochrome: u8::from(sequence_header.monochrome),
        chroma_subsampling_x: sequence_header.chroma_subsampling_x,
        chroma_subsampling_y: sequence_header.chroma_subsampling_y,
        chroma_sample_position: sequence_header.chroma_sample_position,
        initial_presentation_delay_present: u8::from(
            sequence_header
                .initial_presentation_delay_minus_one
                .is_some(),
        ),
        initial_presentation_delay_minus_one: sequence_header
            .initial_presentation_delay_minus_one
            .unwrap_or(0),
        config_obus,
    };
    if config.initial_presentation_delay_present == 0 {
        config.initial_presentation_delay_minus_one = 0;
    }
    let colr = Colr {
        colour_type: AV1_COLOUR_TYPE_NCLX,
        colour_primaries: sequence_header.colour_primaries,
        transfer_characteristics: sequence_header.transfer_characteristics,
        matrix_coefficients: sequence_header.matrix_coefficients,
        full_range_flag: sequence_header.full_range_flag,
        reserved: 0,
        profile: Vec::new(),
        unknown: Vec::new(),
    };
    Ok((config, colr, sequence_header.width, sequence_header.height))
}

fn normalize_av1_ivf_sample_spans_sync(
    path: &Path,
    indexed: &mut super::ivf_common::IndexedIvfTrack,
    spec: &str,
) -> Result<(), MuxError> {
    let mut file = File::open(path)?;
    for sample in &mut indexed.samples {
        let trim = scan_leading_temporal_delimiter_bytes_sync(
            &mut file,
            sample.data_offset,
            sample.data_size,
            spec,
        )?;
        apply_av1_sample_trim(sample, trim, spec)?;
    }
    let trim = scan_leading_temporal_delimiter_bytes_sync(
        &mut file,
        indexed.first_sample_span.data_offset,
        indexed.first_sample_span.data_size,
        spec,
    )?;
    apply_av1_ivf_span_trim(&mut indexed.first_sample_span, trim, spec)?;
    Ok(())
}

#[cfg(feature = "async")]
async fn normalize_av1_ivf_sample_spans_async(
    path: &Path,
    indexed: &mut super::ivf_common::IndexedIvfTrack,
    spec: &str,
) -> Result<(), MuxError> {
    let mut file = TokioFile::open(path).await?;
    for sample in &mut indexed.samples {
        let trim = scan_leading_temporal_delimiter_bytes_async(
            &mut file,
            sample.data_offset,
            sample.data_size,
            spec,
        )
        .await?;
        apply_av1_sample_trim(sample, trim, spec)?;
    }
    let trim = scan_leading_temporal_delimiter_bytes_async(
        &mut file,
        indexed.first_sample_span.data_offset,
        indexed.first_sample_span.data_size,
        spec,
    )
    .await?;
    apply_av1_ivf_span_trim(&mut indexed.first_sample_span, trim, spec)?;
    Ok(())
}

fn apply_av1_sample_trim(sample: &mut StagedSample, trim: u32, spec: &str) -> Result<(), MuxError> {
    if trim == 0 {
        return Ok(());
    }
    if trim >= sample.data_size {
        return Err(unsupported(
            spec,
            "AV1 sample payload only contained temporal-delimiter OBUs",
        ));
    }
    sample.data_offset = sample
        .data_offset
        .checked_add(u64::from(trim))
        .ok_or(MuxError::LayoutOverflow("AV1 sample trim offset"))?;
    sample.data_size -= trim;
    Ok(())
}

fn apply_av1_ivf_span_trim(
    sample: &mut super::ivf_common::IndexedIvfSample,
    trim: u32,
    spec: &str,
) -> Result<(), MuxError> {
    if trim == 0 {
        return Ok(());
    }
    if trim >= sample.data_size {
        return Err(unsupported(
            spec,
            "AV1 sample payload only contained temporal-delimiter OBUs",
        ));
    }
    sample.data_offset = sample
        .data_offset
        .checked_add(u64::from(trim))
        .ok_or(MuxError::LayoutOverflow("AV1 sample trim offset"))?;
    sample.data_size -= trim;
    Ok(())
}

fn read_section5_obu_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
    capture_bytes: bool,
) -> Result<ParsedSection5Obu, MuxError> {
    let (parsed, total_size) = read_av1_obu_header_sync(file, file_size, offset, spec)?;
    if parsed.payload_end > file_size {
        return Err(unsupported(
            spec,
            "raw AV1 OBU payload overruns the input length",
        ));
    }
    let normalized_bytes = if capture_bytes {
        read_bytes_at_sync(
            file,
            offset,
            total_size,
            spec,
            "raw AV1 OBU payload is truncated",
        )?
    } else {
        Vec::new()
    };
    Ok(ParsedSection5Obu {
        file_offset: offset,
        total_size,
        obu_type: parsed.obu_type,
        normalized_bytes,
    })
}

#[cfg(feature = "async")]
async fn read_section5_obu_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
    capture_bytes: bool,
) -> Result<ParsedSection5Obu, MuxError> {
    let (parsed, total_size) = read_av1_obu_header_async(file, file_size, offset, spec).await?;
    if parsed.payload_end > file_size {
        return Err(unsupported(
            spec,
            "raw AV1 OBU payload overruns the input length",
        ));
    }
    let normalized_bytes = if capture_bytes {
        read_bytes_at_async(
            file,
            offset,
            total_size,
            spec,
            "raw AV1 OBU payload is truncated",
        )
        .await?
    } else {
        Vec::new()
    };
    Ok(ParsedSection5Obu {
        file_offset: offset,
        total_size,
        obu_type: parsed.obu_type,
        normalized_bytes,
    })
}

fn read_annex_b_obu_sync(
    file: &mut File,
    obu_offset: u64,
    obu_size: u32,
    spec: &str,
    capture_bytes: bool,
) -> Result<ParsedAnnexBObu, MuxError> {
    let obu_end = obu_offset
        .checked_add(u64::from(obu_size))
        .ok_or(MuxError::LayoutOverflow("AV1 Annex B OBU end"))?;
    let (parsed, _) = read_av1_obu_header_sync(file, obu_end, obu_offset, spec)?;
    if parsed.payload_end != obu_end {
        return Err(unsupported(
            spec,
            "AV1 Annex B OBU used an internal size field that disagrees with the wrapper size",
        ));
    }
    let normalized_bytes = if capture_bytes {
        if parsed.has_size_field {
            read_bytes_at_sync(
                file,
                obu_offset,
                obu_size,
                spec,
                "AV1 Annex B OBU payload is truncated",
            )?
        } else {
            let mut bytes = parsed.normalized_prefix.clone();
            bytes.extend_from_slice(&read_bytes_at_sync(
                file,
                parsed.payload_offset,
                parsed.payload_size,
                spec,
                "AV1 Annex B OBU payload is truncated",
            )?);
            bytes
        }
    } else {
        Vec::new()
    };
    let (segment_prefix, file_range) = if parsed.obu_type == OBU_TEMPORAL_DELIMITER {
        (None, None)
    } else if parsed.has_size_field {
        (None, Some((obu_offset, obu_size)))
    } else {
        (
            Some(parsed.normalized_prefix),
            Some((parsed.payload_offset, parsed.payload_size)),
        )
    };
    Ok(ParsedAnnexBObu {
        obu_type: parsed.obu_type,
        segment_prefix,
        file_range,
        normalized_bytes,
    })
}

#[cfg(feature = "async")]
async fn read_annex_b_obu_async(
    file: &mut TokioFile,
    obu_offset: u64,
    obu_size: u32,
    spec: &str,
    capture_bytes: bool,
) -> Result<ParsedAnnexBObu, MuxError> {
    let obu_end = obu_offset
        .checked_add(u64::from(obu_size))
        .ok_or(MuxError::LayoutOverflow("AV1 Annex B OBU end"))?;
    let (parsed, _) = read_av1_obu_header_async(file, obu_end, obu_offset, spec).await?;
    if parsed.payload_end != obu_end {
        return Err(unsupported(
            spec,
            "AV1 Annex B OBU used an internal size field that disagrees with the wrapper size",
        ));
    }
    let normalized_bytes = if capture_bytes {
        if parsed.has_size_field {
            read_bytes_at_async(
                file,
                obu_offset,
                obu_size,
                spec,
                "AV1 Annex B OBU payload is truncated",
            )
            .await?
        } else {
            let mut bytes = parsed.normalized_prefix.clone();
            bytes.extend_from_slice(
                &read_bytes_at_async(
                    file,
                    parsed.payload_offset,
                    parsed.payload_size,
                    spec,
                    "AV1 Annex B OBU payload is truncated",
                )
                .await?,
            );
            bytes
        }
    } else {
        Vec::new()
    };
    let (segment_prefix, file_range) = if parsed.obu_type == OBU_TEMPORAL_DELIMITER {
        (None, None)
    } else if parsed.has_size_field {
        (None, Some((obu_offset, obu_size)))
    } else {
        (
            Some(parsed.normalized_prefix),
            Some((parsed.payload_offset, parsed.payload_size)),
        )
    };
    Ok(ParsedAnnexBObu {
        obu_type: parsed.obu_type,
        segment_prefix,
        file_range,
        normalized_bytes,
    })
}

struct ParsedAv1ObuHeader {
    obu_type: u8,
    has_size_field: bool,
    payload_offset: u64,
    payload_size: u32,
    payload_end: u64,
    normalized_prefix: Vec<u8>,
}

fn read_av1_obu_header_sync(
    file: &mut File,
    limit_end: u64,
    obu_offset: u64,
    spec: &str,
) -> Result<(ParsedAv1ObuHeader, u32), MuxError> {
    file.seek(SeekFrom::Start(obu_offset))?;
    let header = read_byte_sync(file, spec, "AV1 OBU header is truncated")?;
    parse_av1_obu_header_prefix_sync(file, limit_end, obu_offset, header, spec)
}

#[cfg(feature = "async")]
async fn read_av1_obu_header_async(
    file: &mut TokioFile,
    limit_end: u64,
    obu_offset: u64,
    spec: &str,
) -> Result<(ParsedAv1ObuHeader, u32), MuxError> {
    file.seek(SeekFrom::Start(obu_offset)).await?;
    let header = read_byte_async(file, spec, "AV1 OBU header is truncated").await?;
    parse_av1_obu_header_prefix_async(file, limit_end, obu_offset, header, spec).await
}

fn parse_av1_obu_header_prefix_sync(
    file: &mut File,
    limit_end: u64,
    obu_offset: u64,
    header: u8,
    spec: &str,
) -> Result<(ParsedAv1ObuHeader, u32), MuxError> {
    let (obu_type, extension_flag, has_size_field) = parse_av1_obu_header_bits(header, spec)?;
    let mut normalized_prefix = vec![header];
    let mut cursor = obu_offset
        .checked_add(1)
        .ok_or(MuxError::LayoutOverflow("AV1 OBU header cursor"))?;
    if extension_flag {
        let extension = read_byte_sync(file, spec, "AV1 OBU extension header is truncated")?;
        normalized_prefix.push(extension);
        cursor = cursor
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("AV1 OBU extension cursor"))?;
    }
    let (payload_offset, payload_size) = if has_size_field {
        let (obu_payload_size, leb_size, leb_bytes) =
            read_leb128_from_current_sync(file, spec, "AV1 OBU size")?;
        normalized_prefix.extend_from_slice(&leb_bytes);
        let payload_offset = cursor
            .checked_add(u64::try_from(leb_size).unwrap())
            .ok_or(MuxError::LayoutOverflow("AV1 OBU payload offset"))?;
        let payload_size = u32::try_from(obu_payload_size)
            .map_err(|_| MuxError::LayoutOverflow("AV1 OBU payload size"))?;
        (payload_offset, payload_size)
    } else {
        let payload_size = u32::try_from(limit_end.saturating_sub(cursor))
            .map_err(|_| MuxError::LayoutOverflow("AV1 OBU payload size"))?;
        normalized_prefix[0] |= 0x02;
        normalized_prefix.extend_from_slice(&encode_leb128(payload_size));
        (cursor, payload_size)
    };
    validate_av1_temporal_delimiter_payload(obu_type, payload_size, spec)?;
    let payload_end = payload_offset
        .checked_add(u64::from(payload_size))
        .ok_or(MuxError::LayoutOverflow("AV1 OBU payload end"))?;
    let total_size = u32::try_from(payload_end.saturating_sub(obu_offset))
        .map_err(|_| MuxError::LayoutOverflow("AV1 OBU total size"))?;
    Ok((
        ParsedAv1ObuHeader {
            obu_type,
            has_size_field,
            payload_offset,
            payload_size,
            payload_end,
            normalized_prefix,
        },
        total_size,
    ))
}

#[cfg(feature = "async")]
async fn parse_av1_obu_header_prefix_async(
    file: &mut TokioFile,
    limit_end: u64,
    obu_offset: u64,
    header: u8,
    spec: &str,
) -> Result<(ParsedAv1ObuHeader, u32), MuxError> {
    let (obu_type, extension_flag, has_size_field) = parse_av1_obu_header_bits(header, spec)?;
    let mut normalized_prefix = vec![header];
    let mut cursor = obu_offset
        .checked_add(1)
        .ok_or(MuxError::LayoutOverflow("AV1 OBU header cursor"))?;
    if extension_flag {
        let extension =
            read_byte_async(file, spec, "AV1 OBU extension header is truncated").await?;
        normalized_prefix.push(extension);
        cursor = cursor
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("AV1 OBU extension cursor"))?;
    }
    let (payload_offset, payload_size) = if has_size_field {
        let (obu_payload_size, leb_size, leb_bytes) =
            read_leb128_from_current_async(file, spec, "AV1 OBU size").await?;
        normalized_prefix.extend_from_slice(&leb_bytes);
        let payload_offset = cursor
            .checked_add(u64::try_from(leb_size).unwrap())
            .ok_or(MuxError::LayoutOverflow("AV1 OBU payload offset"))?;
        let payload_size = u32::try_from(obu_payload_size)
            .map_err(|_| MuxError::LayoutOverflow("AV1 OBU payload size"))?;
        (payload_offset, payload_size)
    } else {
        let payload_size = u32::try_from(limit_end.saturating_sub(cursor))
            .map_err(|_| MuxError::LayoutOverflow("AV1 OBU payload size"))?;
        normalized_prefix[0] |= 0x02;
        normalized_prefix.extend_from_slice(&encode_leb128(payload_size));
        (cursor, payload_size)
    };
    validate_av1_temporal_delimiter_payload(obu_type, payload_size, spec)?;
    let payload_end = payload_offset
        .checked_add(u64::from(payload_size))
        .ok_or(MuxError::LayoutOverflow("AV1 OBU payload end"))?;
    let total_size = u32::try_from(payload_end.saturating_sub(obu_offset))
        .map_err(|_| MuxError::LayoutOverflow("AV1 OBU total size"))?;
    Ok((
        ParsedAv1ObuHeader {
            obu_type,
            has_size_field,
            payload_offset,
            payload_size,
            payload_end,
            normalized_prefix,
        },
        total_size,
    ))
}

fn parse_av1_obu_header_bits(header: u8, spec: &str) -> Result<(u8, bool, bool), MuxError> {
    if header >> 7 != 0 {
        return Err(unsupported(
            spec,
            "AV1 OBU header used a non-zero forbidden bit",
        ));
    }
    if header & 0x01 != 0 {
        return Err(unsupported(
            spec,
            "AV1 OBU header used a non-zero reserved bit",
        ));
    }
    Ok((
        (header >> 3) & 0x0F,
        (header >> 2) & 0x01 != 0,
        (header >> 1) & 0x01 != 0,
    ))
}

fn validate_av1_temporal_delimiter_payload(
    obu_type: u8,
    payload_size: u32,
    spec: &str,
) -> Result<(), MuxError> {
    if obu_type == OBU_TEMPORAL_DELIMITER && payload_size != 0 {
        return Err(unsupported(
            spec,
            "AV1 temporal-delimiter OBU payloads must have zero length",
        ));
    }
    Ok(())
}

fn append_segmented_av1_bytes(
    segments: &mut Vec<SegmentedMuxSourceSegment>,
    logical_size: &mut u64,
    sample_size: &mut u32,
    segment_prefix: Option<Vec<u8>>,
    file_range: Option<(u64, u32)>,
) -> Result<(), MuxError> {
    if let Some(prefix) = segment_prefix
        && !prefix.is_empty()
    {
        let prefix_len = u64::try_from(prefix.len())
            .map_err(|_| MuxError::LayoutOverflow("AV1 prefix length"))?;
        *sample_size = sample_size
            .checked_add(u32::try_from(prefix.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow("AV1 segmented sample size"))?;
        segments.push(SegmentedMuxSourceSegment {
            logical_offset: *logical_size,
            data: SegmentedMuxSourceSegmentData::Bytes(prefix),
        });
        *logical_size = logical_size
            .checked_add(prefix_len)
            .ok_or(MuxError::LayoutOverflow("AV1 segmented logical size"))?;
    }
    if let Some((source_offset, size)) = file_range {
        *sample_size = sample_size
            .checked_add(size)
            .ok_or(MuxError::LayoutOverflow("AV1 segmented sample size"))?;
        segments.push(SegmentedMuxSourceSegment {
            logical_offset: *logical_size,
            data: SegmentedMuxSourceSegmentData::FileRange {
                source_offset,
                size,
            },
        });
        *logical_size = logical_size
            .checked_add(u64::from(size))
            .ok_or(MuxError::LayoutOverflow("AV1 segmented logical size"))?;
    }
    Ok(())
}

fn read_leb128_from_file_sync(
    file: &mut File,
    offset: u64,
    spec: &str,
    field_name: &str,
) -> Result<(u64, usize), MuxError> {
    file.seek(SeekFrom::Start(offset))?;
    let (value, size, _) = read_leb128_from_current_sync(file, spec, field_name)?;
    Ok((value, size))
}

#[cfg(feature = "async")]
async fn read_leb128_from_file_async(
    file: &mut TokioFile,
    offset: u64,
    spec: &str,
    field_name: &str,
) -> Result<(u64, usize), MuxError> {
    file.seek(SeekFrom::Start(offset)).await?;
    let (value, size, _) = read_leb128_from_current_async(file, spec, field_name).await?;
    Ok((value, size))
}

fn read_leb128_from_current_sync(
    file: &mut File,
    spec: &str,
    field_name: &str,
) -> Result<(u64, usize, Vec<u8>), MuxError> {
    let mut value = 0_u64;
    let mut shift = 0_u32;
    let mut bytes = Vec::new();
    loop {
        let byte = read_byte_sync(file, spec, "AV1 leb128 field is truncated")?;
        bytes.push(byte);
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, bytes.len(), bytes));
        }
        shift = shift
            .checked_add(7)
            .ok_or(MuxError::LayoutOverflow("AV1 leb128 shift"))?;
        if shift >= 63 {
            return Err(unsupported(
                spec,
                &format!("{field_name} used an unterminated or unsupported leb128 value"),
            ));
        }
    }
}

#[cfg(feature = "async")]
async fn read_leb128_from_current_async(
    file: &mut TokioFile,
    spec: &str,
    field_name: &str,
) -> Result<(u64, usize, Vec<u8>), MuxError> {
    let mut value = 0_u64;
    let mut shift = 0_u32;
    let mut bytes = Vec::new();
    loop {
        let byte = read_byte_async(file, spec, "AV1 leb128 field is truncated").await?;
        bytes.push(byte);
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, bytes.len(), bytes));
        }
        shift = shift
            .checked_add(7)
            .ok_or(MuxError::LayoutOverflow("AV1 leb128 shift"))?;
        if shift >= 63 {
            return Err(unsupported(
                spec,
                &format!("{field_name} used an unterminated or unsupported leb128 value"),
            ));
        }
    }
}

fn read_bytes_at_sync(
    file: &mut File,
    offset: u64,
    size: u32,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![
        0_u8;
        usize::try_from(size)
            .map_err(|_| MuxError::LayoutOverflow("AV1 byte range size"))?
    ];
    file.read_exact(&mut bytes)
        .map_err(|error| map_temporal_delimiter_io_error(error, spec, truncated_message))?;
    Ok(bytes)
}

#[cfg(feature = "async")]
async fn read_bytes_at_async(
    file: &mut TokioFile,
    offset: u64,
    size: u32,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    file.seek(SeekFrom::Start(offset)).await?;
    let mut bytes = vec![
        0_u8;
        usize::try_from(size)
            .map_err(|_| MuxError::LayoutOverflow("AV1 byte range size"))?
    ];
    file.read_exact(&mut bytes)
        .await
        .map_err(|error| map_temporal_delimiter_io_error(error, spec, truncated_message))?;
    Ok(bytes)
}

fn read_byte_sync(
    file: &mut File,
    spec: &str,
    truncated_message: &'static str,
) -> Result<u8, MuxError> {
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte)
        .map_err(|error| map_temporal_delimiter_io_error(error, spec, truncated_message))?;
    Ok(byte[0])
}

#[cfg(feature = "async")]
async fn read_byte_async(
    file: &mut TokioFile,
    spec: &str,
    truncated_message: &'static str,
) -> Result<u8, MuxError> {
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte)
        .await
        .map_err(|error| map_temporal_delimiter_io_error(error, spec, truncated_message))?;
    Ok(byte[0])
}

fn encode_leb128(mut value: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = u8::try_from(value & 0x7F).unwrap();
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if value == 0 {
            return bytes;
        }
    }
}

fn scan_leading_temporal_delimiter_bytes_sync(
    file: &mut File,
    sample_offset: u64,
    sample_size: u32,
    spec: &str,
) -> Result<u32, MuxError> {
    let mut trimmed = 0_u32;
    loop {
        let remaining = sample_size
            .checked_sub(trimmed)
            .ok_or(MuxError::LayoutOverflow("AV1 sample trim remainder"))?;
        if remaining == 0 {
            return Err(unsupported(
                spec,
                "AV1 sample payload only contained temporal-delimiter OBUs",
            ));
        }
        file.seek(SeekFrom::Start(
            sample_offset
                .checked_add(u64::from(trimmed))
                .ok_or(MuxError::LayoutOverflow("AV1 sample trim seek"))?,
        ))?;
        let prefix_len = usize::try_from(remaining.min(8))
            .map_err(|_| MuxError::LayoutOverflow("AV1 prefix read size"))?;
        let mut prefix = vec![0_u8; prefix_len];
        file.read_exact(&mut prefix).map_err(|error| {
            map_temporal_delimiter_io_error(
                error,
                spec,
                "AV1 sample payload is truncated while reading a temporal-delimiter prefix",
            )
        })?;
        match leading_temporal_delimiter_len(&prefix, spec, sample_offset + u64::from(trimmed))? {
            Some(length) => {
                trimmed = trimmed
                    .checked_add(length)
                    .ok_or(MuxError::LayoutOverflow("AV1 sample trim length"))?;
            }
            None => return Ok(trimmed),
        }
    }
}

#[cfg(feature = "async")]
async fn scan_leading_temporal_delimiter_bytes_async(
    file: &mut TokioFile,
    sample_offset: u64,
    sample_size: u32,
    spec: &str,
) -> Result<u32, MuxError> {
    let mut trimmed = 0_u32;
    loop {
        let remaining = sample_size
            .checked_sub(trimmed)
            .ok_or(MuxError::LayoutOverflow("AV1 sample trim remainder"))?;
        if remaining == 0 {
            return Err(unsupported(
                spec,
                "AV1 sample payload only contained temporal-delimiter OBUs",
            ));
        }
        file.seek(SeekFrom::Start(
            sample_offset
                .checked_add(u64::from(trimmed))
                .ok_or(MuxError::LayoutOverflow("AV1 sample trim seek"))?,
        ))
        .await?;
        let prefix_len = usize::try_from(remaining.min(8))
            .map_err(|_| MuxError::LayoutOverflow("AV1 prefix read size"))?;
        let mut prefix = vec![0_u8; prefix_len];
        file.read_exact(&mut prefix).await.map_err(|error| {
            map_temporal_delimiter_io_error(
                error,
                spec,
                "AV1 sample payload is truncated while reading a temporal-delimiter prefix",
            )
        })?;
        match leading_temporal_delimiter_len(&prefix, spec, sample_offset + u64::from(trimmed))? {
            Some(length) => {
                trimmed = trimmed
                    .checked_add(length)
                    .ok_or(MuxError::LayoutOverflow("AV1 sample trim length"))?;
            }
            None => return Ok(trimmed),
        }
    }
}

fn leading_temporal_delimiter_len(
    prefix: &[u8],
    spec: &str,
    offset: u64,
) -> Result<Option<u32>, MuxError> {
    let mut cursor = 0usize;
    let header = *prefix.get(cursor).ok_or_else(|| {
        unsupported(
            spec,
            "AV1 temporal-delimiter prefix is truncated before the OBU header",
        )
    })?;
    if header >> 7 != 0 {
        return Err(unsupported(
            spec,
            "AV1 OBU header used a non-zero forbidden bit",
        ));
    }
    let obu_type = (header >> 3) & 0x0F;
    if obu_type != OBU_TEMPORAL_DELIMITER {
        return Ok(None);
    }
    cursor += 1;
    let extension_flag = (header >> 2) & 0x01 != 0;
    let has_size_field = (header >> 1) & 0x01 != 0;
    if header & 0x01 != 0 {
        return Err(unsupported(
            spec,
            "AV1 OBU header used a non-zero reserved bit",
        ));
    }
    if extension_flag {
        if prefix.get(cursor).is_none() {
            return Err(unsupported(
                spec,
                "AV1 temporal-delimiter OBU extension header is truncated",
            ));
        }
        cursor += 1;
    }
    if !has_size_field {
        return Err(unsupported(
            spec,
            "AV1 temporal-delimiter OBUs without explicit size fields are not supported",
        ));
    }
    let (obu_size, leb_bytes) = read_leb128_from_slice(
        prefix.get(cursor..).unwrap_or_default(),
        spec,
        "AV1 temporal-delimiter OBU size",
        offset + u64::try_from(cursor).unwrap_or(u64::MAX),
    )?;
    if obu_size != 0 {
        return Err(unsupported(
            spec,
            "AV1 temporal-delimiter OBU payloads must have zero length",
        ));
    }
    cursor = cursor
        .checked_add(leb_bytes)
        .ok_or(MuxError::LayoutOverflow(
            "AV1 temporal-delimiter size field",
        ))?;
    Ok(Some(u32::try_from(cursor).map_err(|_| {
        MuxError::LayoutOverflow("AV1 temporal delimiter")
    })?))
}

fn map_temporal_delimiter_io_error(
    error: std::io::Error,
    spec: &str,
    truncated_message: &'static str,
) -> MuxError {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        unsupported(spec, truncated_message)
    } else {
        MuxError::Io(error)
    }
}

fn find_av1_sequence_header_obu(
    sample: &[u8],
    spec: &str,
) -> Result<(Vec<u8>, ParsedAv1SequenceHeader), MuxError> {
    let mut offset = 0usize;
    while offset < sample.len() {
        let start = offset;
        let header = *sample
            .get(offset)
            .ok_or_else(|| unsupported(spec, "AV1 OBU header is truncated"))?;
        offset += 1;
        if header >> 7 != 0 {
            return Err(unsupported(
                spec,
                "AV1 OBU header used a non-zero forbidden bit",
            ));
        }
        let obu_type = (header >> 3) & 0x0F;
        let extension_flag = (header >> 2) & 0x01 != 0;
        let has_size_field = (header >> 1) & 0x01 != 0;
        if header & 0x01 != 0 {
            return Err(unsupported(
                spec,
                "AV1 OBU header used a non-zero reserved bit",
            ));
        }
        if extension_flag {
            if sample.get(offset).is_none() {
                return Err(unsupported(spec, "AV1 OBU extension header is truncated"));
            }
            offset += 1;
        }
        if !has_size_field {
            return Err(unsupported(
                spec,
                "AV1 sequence OBUs without explicit size fields are not supported",
            ));
        }
        let (obu_size, leb_bytes) = read_leb128_from_slice(
            sample.get(offset..).unwrap_or_default(),
            spec,
            "AV1 OBU size",
            u64::try_from(offset).unwrap_or(u64::MAX),
        )?;
        offset = offset
            .checked_add(leb_bytes)
            .ok_or(MuxError::LayoutOverflow("AV1 OBU header size"))?;
        let payload_end = offset
            .checked_add(
                usize::try_from(obu_size).map_err(|_| MuxError::LayoutOverflow("AV1 OBU size"))?,
            )
            .ok_or(MuxError::LayoutOverflow("AV1 OBU size"))?;
        if payload_end > sample.len() {
            return Err(unsupported(
                spec,
                "AV1 OBU payload overruns the sample payload",
            ));
        }
        if obu_type == OBU_SEQUENCE_HEADER {
            let obu_bytes = sample[start..payload_end].to_vec();
            let sequence_header = parse_av1_sequence_header(&sample[offset..payload_end], spec)?;
            return Ok((obu_bytes, sequence_header));
        }
        offset = payload_end;
    }
    Err(unsupported(
        spec,
        "AV1 input did not contain a sequence-header OBU in its first sample",
    ))
}

#[derive(Clone, Copy)]
struct ParsedAv1SequenceHeader {
    seq_profile: u8,
    seq_level_idx_0: u8,
    seq_tier_0: u8,
    width: u16,
    height: u16,
    high_bitdepth: bool,
    twelve_bit: bool,
    monochrome: bool,
    chroma_subsampling_x: u8,
    chroma_subsampling_y: u8,
    chroma_sample_position: u8,
    initial_presentation_delay_minus_one: Option<u8>,
    colour_primaries: u16,
    transfer_characteristics: u16,
    matrix_coefficients: u16,
    full_range_flag: bool,
}

fn parse_av1_sequence_header(
    bytes: &[u8],
    spec: &str,
) -> Result<ParsedAv1SequenceHeader, MuxError> {
    let mut bits = BitCursor::new(bytes);
    let seq_profile = bits.read_bits_u8(3, spec, "AV1 seq_profile")?;
    let still_picture = bits.read_bit(spec, "AV1 still_picture")?;
    let reduced_still_picture_header = bits.read_bit(spec, "AV1 reduced_still_picture_header")?;
    if reduced_still_picture_header && !still_picture {
        return Err(unsupported(
            spec,
            "AV1 reduced still-picture headers must also set the still-picture flag",
        ));
    }

    let mut seq_tier_0 = 0;
    let mut initial_presentation_delay_minus_one = None;
    let seq_level_idx_0;
    let decoder_model_info = if reduced_still_picture_header {
        seq_level_idx_0 = bits.read_bits_u8(5, spec, "AV1 seq_level_idx_0")?;
        None
    } else {
        let timing_info_present_flag = bits.read_bit(spec, "AV1 timing_info_present_flag")?;
        let decoder_model_info_present_flag = if timing_info_present_flag {
            bits.read_bit(spec, "AV1 decoder_model_info_present_flag")?
        } else {
            false
        };
        let decoder_model_info = if timing_info_present_flag && decoder_model_info_present_flag {
            skip_timing_info_and_decoder_model(&mut bits, spec)?
        } else if timing_info_present_flag {
            skip_timing_info_only(&mut bits, spec)?;
            None
        } else {
            None
        };
        let initial_display_delay_present_flag =
            bits.read_bit(spec, "AV1 initial_display_delay_present_flag")?;
        let operating_points_cnt_minus_1 =
            bits.read_bits_u8(5, spec, "AV1 operating_points_cnt_minus_1")?;
        let mut seq_level = 0;
        let mut seq_tier = 0;
        let mut initial_delay = None;
        for index in 0..=operating_points_cnt_minus_1 {
            bits.skip_bits(12, spec, "AV1 operating_point_idc")?;
            let level = bits.read_bits_u8(5, spec, "AV1 seq_level_idx")?;
            let tier = if level > 7 {
                u8::from(bits.read_bit(spec, "AV1 seq_tier")?)
            } else {
                0
            };
            if let Some(info) = decoder_model_info
                && bits.read_bit(spec, "AV1 decoder_model_present_for_this_op")?
            {
                bits.skip_bits(
                    usize::from(info.buffer_delay_length_minus_one) + 1,
                    spec,
                    "AV1 decoder_buffer_delay",
                )?;
                bits.skip_bits(
                    usize::from(info.buffer_delay_length_minus_one) + 1,
                    spec,
                    "AV1 encoder_buffer_delay",
                )?;
                bits.skip_bits(1, spec, "AV1 low_delay_mode_flag")?;
            }
            let op_delay = if initial_display_delay_present_flag
                && bits.read_bit(spec, "AV1 initial_display_delay_present_for_this_op")?
            {
                Some(bits.read_bits_u8(4, spec, "AV1 initial_display_delay_minus_one")?)
            } else {
                None
            };
            if index == 0 {
                seq_level = level;
                seq_tier = tier;
                initial_delay = op_delay;
            }
        }
        seq_level_idx_0 = seq_level;
        seq_tier_0 = seq_tier;
        initial_presentation_delay_minus_one = initial_delay;
        decoder_model_info
    };

    let frame_width_bits_minus_1 = bits.read_bits_u8(4, spec, "AV1 frame_width_bits_minus_1")?;
    let frame_height_bits_minus_1 = bits.read_bits_u8(4, spec, "AV1 frame_height_bits_minus_1")?;
    let width = u16::try_from(
        bits.read_bits_u32(
            usize::from(frame_width_bits_minus_1) + 1,
            spec,
            "AV1 max_frame_width_minus_1",
        )?
        .checked_add(1)
        .ok_or(MuxError::LayoutOverflow("AV1 frame width"))?,
    )
    .map_err(|_| MuxError::LayoutOverflow("AV1 frame width"))?;
    let height = u16::try_from(
        bits.read_bits_u32(
            usize::from(frame_height_bits_minus_1) + 1,
            spec,
            "AV1 max_frame_height_minus_one",
        )?
        .checked_add(1)
        .ok_or(MuxError::LayoutOverflow("AV1 frame height"))?,
    )
    .map_err(|_| MuxError::LayoutOverflow("AV1 frame height"))?;
    if width == 0 || height == 0 {
        return Err(unsupported(
            spec,
            "AV1 sequence header signaled a zero-sized coded frame",
        ));
    }
    if !reduced_still_picture_header && bits.read_bit(spec, "AV1 frame_id_numbers_present_flag")? {
        bits.skip_bits(4, spec, "AV1 delta_frame_id_length_minus_2")?;
        bits.skip_bits(3, spec, "AV1 additional_frame_id_length_minus_1")?;
    }

    bits.skip_bits(1, spec, "AV1 use_128x128_superblock")?;
    bits.skip_bits(1, spec, "AV1 enable_filter_intra")?;
    bits.skip_bits(1, spec, "AV1 enable_intra_edge_filter")?;
    if !reduced_still_picture_header {
        bits.skip_bits(1, spec, "AV1 enable_interintra_compound")?;
        bits.skip_bits(1, spec, "AV1 enable_masked_compound")?;
        bits.skip_bits(1, spec, "AV1 enable_warped_motion")?;
        let enable_dual_filter = bits.read_bit(spec, "AV1 enable_dual_filter")?;
        let enable_order_hint = bits.read_bit(spec, "AV1 enable_order_hint")?;
        if enable_order_hint {
            bits.skip_bits(1, spec, "AV1 enable_jnt_comp")?;
            bits.skip_bits(1, spec, "AV1 enable_ref_frame_mvs")?;
        }
        let seq_choose_screen_content_tools =
            bits.read_bit(spec, "AV1 seq_choose_screen_content_tools")?;
        let seq_force_screen_content_tools = if seq_choose_screen_content_tools {
            None
        } else {
            Some(bits.read_bit(spec, "AV1 seq_force_screen_content_tools")?)
        };
        if seq_force_screen_content_tools == Some(true) {
            let seq_choose_integer_mv = bits.read_bit(spec, "AV1 seq_choose_integer_mv")?;
            if !seq_choose_integer_mv {
                bits.skip_bits(1, spec, "AV1 seq_force_integer_mv")?;
            }
        }
        if enable_order_hint || enable_dual_filter {
            let _ = decoder_model_info;
        }
        if enable_order_hint {
            bits.skip_bits(3, spec, "AV1 order_hint_bits_minus_1")?;
        }
    }
    bits.skip_bits(1, spec, "AV1 enable_superres")?;
    bits.skip_bits(1, spec, "AV1 enable_cdef")?;
    bits.skip_bits(1, spec, "AV1 enable_restoration")?;

    let color_info = parse_av1_color_config(&mut bits, seq_profile, spec)?;
    bits.skip_bits(1, spec, "AV1 film_grain_params_present")?;

    Ok(ParsedAv1SequenceHeader {
        seq_profile,
        seq_level_idx_0,
        seq_tier_0,
        width,
        height,
        high_bitdepth: color_info.high_bitdepth,
        twelve_bit: color_info.twelve_bit,
        monochrome: color_info.monochrome,
        chroma_subsampling_x: color_info.chroma_subsampling_x,
        chroma_subsampling_y: color_info.chroma_subsampling_y,
        chroma_sample_position: color_info.chroma_sample_position,
        initial_presentation_delay_minus_one,
        colour_primaries: color_info.colour_primaries,
        transfer_characteristics: color_info.transfer_characteristics,
        matrix_coefficients: color_info.matrix_coefficients,
        full_range_flag: color_info.full_range_flag,
    })
}

#[derive(Clone, Copy)]
struct Av1DecoderModelInfo {
    buffer_delay_length_minus_one: u8,
}

fn skip_timing_info_and_decoder_model(
    bits: &mut BitCursor<'_>,
    spec: &str,
) -> Result<Option<Av1DecoderModelInfo>, MuxError> {
    skip_timing_info_only(bits, spec)?;
    let buffer_delay_length_minus_one =
        bits.read_bits_u8(5, spec, "AV1 buffer_delay_length_minus_one")?;
    bits.skip_bits(32, spec, "AV1 num_units_in_decoding_tick")?;
    bits.skip_bits(5, spec, "AV1 buffer_removal_time_length_minus_1")?;
    bits.skip_bits(5, spec, "AV1 frame_presentation_time_length_minus_1")?;
    Ok(Some(Av1DecoderModelInfo {
        buffer_delay_length_minus_one,
    }))
}

fn skip_timing_info_only(bits: &mut BitCursor<'_>, spec: &str) -> Result<(), MuxError> {
    bits.skip_bits(32, spec, "AV1 num_units_in_display_tick")?;
    bits.skip_bits(32, spec, "AV1 time_scale")?;
    if bits.read_bit(spec, "AV1 equal_picture_interval")? {
        let _ = read_uvlc(bits, spec, "AV1 num_ticks_per_picture_minus_1")?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ParsedAv1ColorInfo {
    high_bitdepth: bool,
    twelve_bit: bool,
    monochrome: bool,
    chroma_subsampling_x: u8,
    chroma_subsampling_y: u8,
    chroma_sample_position: u8,
    colour_primaries: u16,
    transfer_characteristics: u16,
    matrix_coefficients: u16,
    full_range_flag: bool,
}

fn parse_av1_color_config(
    bits: &mut BitCursor<'_>,
    seq_profile: u8,
    spec: &str,
) -> Result<ParsedAv1ColorInfo, MuxError> {
    let high_bitdepth = bits.read_bit(spec, "AV1 high_bitdepth")?;
    let twelve_bit = seq_profile == 2 && high_bitdepth && bits.read_bit(spec, "AV1 twelve_bit")?;
    let monochrome = if seq_profile == 1 {
        false
    } else {
        bits.read_bit(spec, "AV1 monochrome")?
    };
    let mut colour_primaries = 2_u16;
    let mut transfer_characteristics = 2_u16;
    let mut matrix_coefficients = 2_u16;
    if bits.read_bit(spec, "AV1 color_description_present_flag")? {
        colour_primaries = u16::from(bits.read_bits_u8(8, spec, "AV1 colour_primaries")?);
        transfer_characteristics =
            u16::from(bits.read_bits_u8(8, spec, "AV1 transfer_characteristics")?);
        matrix_coefficients = u16::from(bits.read_bits_u8(8, spec, "AV1 matrix_coefficients")?);
    }

    let full_range_flag;
    let (chroma_subsampling_x, chroma_subsampling_y, chroma_sample_position) = if monochrome {
        full_range_flag = bits.read_bit(spec, "AV1 color_range")?;
        (1, 1, 0)
    } else if colour_primaries == 1 && transfer_characteristics == 13 && matrix_coefficients == 0 {
        full_range_flag = true;
        (0, 0, 0)
    } else {
        full_range_flag = bits.read_bit(spec, "AV1 color_range")?;
        let chroma = if seq_profile == 0 {
            (1, 1)
        } else if seq_profile == 1 {
            (0, 0)
        } else if twelve_bit {
            let chroma_x = u8::from(bits.read_bit(spec, "AV1 chroma_subsampling_x")?);
            let chroma_y = if chroma_x == 1 {
                u8::from(bits.read_bit(spec, "AV1 chroma_subsampling_y")?)
            } else {
                0
            };
            (chroma_x, chroma_y)
        } else {
            (1, 0)
        };
        let chroma_sample_position = if chroma.0 == 1 && chroma.1 == 1 {
            bits.read_bits_u8(2, spec, "AV1 chroma_sample_position")?
        } else {
            0
        };
        bits.skip_bits(1, spec, "AV1 separate_uv_delta_q")?;
        return Ok(ParsedAv1ColorInfo {
            high_bitdepth,
            twelve_bit,
            monochrome,
            chroma_subsampling_x: chroma.0,
            chroma_subsampling_y: chroma.1,
            chroma_sample_position,
            colour_primaries,
            transfer_characteristics,
            matrix_coefficients,
            full_range_flag,
        });
    };
    bits.skip_bits(1, spec, "AV1 separate_uv_delta_q")?;
    Ok(ParsedAv1ColorInfo {
        high_bitdepth,
        twelve_bit,
        monochrome,
        chroma_subsampling_x,
        chroma_subsampling_y,
        chroma_sample_position,
        colour_primaries,
        transfer_characteristics,
        matrix_coefficients,
        full_range_flag,
    })
}

fn read_leb128_from_slice(
    bytes: &[u8],
    spec: &str,
    field_name: &str,
    offset: u64,
) -> Result<(u64, usize), MuxError> {
    let mut value = 0_u64;
    let mut shift = 0_u32;
    for (index, byte) in bytes.iter().copied().enumerate() {
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
        shift += 7;
        if shift >= 63 {
            break;
        }
    }
    Err(unsupported(
        spec,
        &format!(
            "{field_name} at byte offset {offset} used an unterminated or unsupported leb128 value"
        ),
    ))
}

fn read_uvlc(bits: &mut BitCursor<'_>, spec: &str, label: &str) -> Result<u32, MuxError> {
    let mut leading_zeroes = 0usize;
    while !bits.read_bit(spec, label)? {
        leading_zeroes += 1;
    }
    if leading_zeroes == 0 {
        return Ok(0);
    }
    let remainder = bits.read_bits_u32(leading_zeroes, spec, label)?;
    Ok((1_u32 << leading_zeroes) - 1 + remainder)
}

struct BitCursor<'a> {
    data: &'a [u8],
    bit_offset: usize,
}

impl<'a> BitCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_offset: 0,
        }
    }

    fn read_bit(&mut self, spec: &str, label: &str) -> Result<bool, MuxError> {
        Ok(self.read_bits_u32(1, spec, label)? != 0)
    }

    fn read_bits_u8(&mut self, width: usize, spec: &str, label: &str) -> Result<u8, MuxError> {
        u8::try_from(self.read_bits_u32(width, spec, label)?)
            .map_err(|_| MuxError::LayoutOverflow("AV1 bit width conversion"))
    }

    fn read_bits_u32(&mut self, width: usize, spec: &str, label: &str) -> Result<u32, MuxError> {
        let end = self
            .bit_offset
            .checked_add(width)
            .ok_or(MuxError::LayoutOverflow("AV1 bit reader position"))?;
        if end > self.data.len() * 8 {
            return Err(unsupported(
                spec,
                &format!("{label} is truncated in the AV1 sequence header"),
            ));
        }

        let mut value = 0_u32;
        for _ in 0..width {
            let byte = self.data[self.bit_offset / 8];
            let shift = 7 - (self.bit_offset % 8);
            value = (value << 1) | u32::from((byte >> shift) & 1);
            self.bit_offset += 1;
        }
        Ok(value)
    }

    fn skip_bits(&mut self, width: usize, spec: &str, label: &str) -> Result<(), MuxError> {
        let _ = self.read_bits_u32(width, spec, label)?;
        Ok(())
    }
}

fn unsupported(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}
