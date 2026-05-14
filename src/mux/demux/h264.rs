use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::AsyncReadExt;

use crate::FourCc;
use crate::bitio::BitReader;
use crate::boxes::AnyTypeBox;
use crate::boxes::iso14496_12::{
    AVCDecoderConfiguration, AVCParameterSet, Colr, Pasp, SampleEntry, VisualSampleEntry,
};

use super::super::MuxError;
use super::super::import::{
    SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData, SegmentedMuxSourceSpec, StagedSample,
    build_btrt_from_sample_sizes,
};
use super::annexb_common::{
    AnnexBNal, AnnexBNalScanner, IndexedAnnexBTrack, nal_to_rbsp, push_unique_nal,
    read_bit_labeled, read_bits_u8_labeled, read_bits_u16_labeled, read_bits_u32_labeled,
    read_se_labeled, read_ue_labeled,
};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;

pub(in crate::mux) fn stage_annex_b_h264_sync(
    path: &Path,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    let mut file = File::open(path)?;
    let mut scanner = AnnexBNalScanner::default();
    let mut state = H264StageState::new();
    let mut chunk = [0_u8; 16 * 1024];

    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        scanner.push(&chunk[..read], |nal| stage_h264_nal(&mut state, nal))?;
    }
    scanner.finish(|nal| stage_h264_nal(&mut state, nal))?;
    finalize_h264_staged_track(path, state, spec)
}

pub(in crate::mux) fn build_h264_sample_entry_from_avc_config_with_options(
    avcc: &AVCDecoderConfiguration,
    spec: &str,
    include_colr: bool,
) -> Result<(Vec<u8>, u16, u16), MuxError> {
    if avcc.sequence_parameter_sets.is_empty() || avcc.picture_parameter_sets.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.264 configuration input must include SPS and PPS parameter sets"
                .to_string(),
        });
    }
    let sequence_parameter_sets = avcc
        .sequence_parameter_sets
        .iter()
        .map(|parameter_set| parameter_set.nal_unit.clone())
        .collect::<Vec<_>>();
    let sps_info = parse_h264_sps(&sequence_parameter_sets[0], spec)?;
    let mut authored_avcc = avcc.clone();
    if h264_profile_supports_config_extensions(authored_avcc.profile)
        && !authored_avcc.high_profile_fields_enabled
    {
        authored_avcc.high_profile_fields_enabled = true;
    }
    let sample_entry_box =
        build_h264_sample_entry_box_from_avc_config(&sps_info, authored_avcc, include_colr)?;
    Ok((sample_entry_box, sps_info.width, sps_info.height))
}

pub(in crate::mux) fn stage_annex_b_h264_segmented_sync(
    path: &Path,
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    let mut scanner = AnnexBNalScanner::default();
    let mut state = H264StageState::new();
    let mut offset = 0_u64;

    while offset < total_size {
        let read_len = usize::try_from((total_size - offset).min(16 * 1024))
            .map_err(|_| MuxError::LayoutOverflow("segmented H.264 scan chunk length"))?;
        let mut chunk = vec![0_u8; read_len];
        read_segmented_bytes_sync(
            file,
            segments,
            total_size,
            offset,
            &mut chunk,
            spec,
            "segmented H.264 scan chunk is truncated",
        )?;
        for nal in scanner.collect(&chunk) {
            stage_h264_nal_segmented(&mut state, nal)?;
        }
        offset = offset
            .checked_add(u64::try_from(read_len).unwrap())
            .ok_or(MuxError::LayoutOverflow("segmented H.264 scan offset"))?;
    }
    for nal in scanner.finish_collect() {
        stage_h264_nal_segmented(&mut state, nal)?;
    }
    finalize_h264_staged_track(path, state, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn stage_annex_b_h264_async(
    path: &Path,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let mut scanner = AnnexBNalScanner::default();
    let mut state = H264StageState::new();
    let mut chunk = [0_u8; 16 * 1024];

    loop {
        let read = file.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        for nal in scanner.collect(&chunk[..read]) {
            stage_h264_nal(&mut state, nal)?;
        }
    }
    for nal in scanner.finish_collect() {
        stage_h264_nal(&mut state, nal)?;
    }
    finalize_h264_staged_track(path, state, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn stage_annex_b_h264_segmented_async(
    path: &Path,
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    let mut scanner = AnnexBNalScanner::default();
    let mut state = H264StageState::new();
    let mut offset = 0_u64;

    while offset < total_size {
        let read_len = usize::try_from((total_size - offset).min(16 * 1024))
            .map_err(|_| MuxError::LayoutOverflow("segmented H.264 scan chunk length"))?;
        let mut chunk = vec![0_u8; read_len];
        read_segmented_bytes_async(
            file,
            segments,
            total_size,
            offset,
            &mut chunk,
            spec,
            "segmented H.264 scan chunk is truncated",
        )
        .await?;
        for nal in scanner.collect(&chunk) {
            stage_h264_nal_segmented(&mut state, nal)?;
        }
        offset = offset
            .checked_add(u64::try_from(read_len).unwrap())
            .ok_or(MuxError::LayoutOverflow("segmented H.264 scan offset"))?;
    }
    for nal in scanner.finish_collect() {
        stage_h264_nal_segmented(&mut state, nal)?;
    }
    finalize_h264_staged_track(path, state, spec)
}

struct H264StageState {
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
    samples: Vec<StagedSample>,
    segments: Vec<SegmentedMuxSourceSegment>,
    current_sample_offset: Option<u64>,
    current_sample_size: u32,
    current_sync: bool,
    current_has_vcl: bool,
    logical_size: u64,
}

impl H264StageState {
    fn new() -> Self {
        Self {
            sps_list: Vec::new(),
            pps_list: Vec::new(),
            samples: Vec::new(),
            segments: Vec::new(),
            current_sample_offset: None,
            current_sample_size: 0,
            current_sync: false,
            current_has_vcl: false,
            logical_size: 0,
        }
    }

    fn finish_current_sample(&mut self) {
        if let Some(data_offset) = self.current_sample_offset.take() {
            self.samples.push(StagedSample {
                data_offset,
                data_size: self.current_sample_size,
                duration: 0,
                composition_time_offset: 0,
                is_sync_sample: self.current_sync,
            });
            self.current_sample_size = 0;
            self.current_sync = false;
            self.current_has_vcl = false;
        }
    }

    fn append_sample_nal(
        &mut self,
        source_offset: u64,
        source_size: u32,
        is_sync_sample: bool,
        is_vcl: bool,
    ) -> Result<(), MuxError> {
        if self.current_sample_offset.is_none() {
            self.current_sample_offset = Some(self.logical_size);
        }
        let prefix = source_size.to_be_bytes();
        self.segments.push(SegmentedMuxSourceSegment {
            logical_offset: self.logical_size,
            data: SegmentedMuxSourceSegmentData::Prefix(prefix),
        });
        self.logical_size = self
            .logical_size
            .checked_add(4)
            .ok_or(MuxError::LayoutOverflow("raw H.264 transformed payload"))?;
        self.segments.push(SegmentedMuxSourceSegment {
            logical_offset: self.logical_size,
            data: SegmentedMuxSourceSegmentData::FileRange {
                source_offset,
                size: source_size,
            },
        });
        self.current_sample_size = self
            .current_sample_size
            .checked_add(
                4_u32
                    .checked_add(source_size)
                    .ok_or(MuxError::LayoutOverflow(
                        "raw H.264 transformed sample size",
                    ))?,
            )
            .ok_or(MuxError::LayoutOverflow("raw H.264 staged sample size"))?;
        self.logical_size = self
            .logical_size
            .checked_add(u64::from(source_size))
            .ok_or(MuxError::LayoutOverflow("raw H.264 transformed payload"))?;
        self.current_sync |= is_sync_sample;
        self.current_has_vcl |= is_vcl;
        Ok(())
    }

    fn append_sample_bytes(
        &mut self,
        bytes: Vec<u8>,
        is_sync_sample: bool,
        is_vcl: bool,
    ) -> Result<(), MuxError> {
        let source_size = u32::try_from(bytes.len())
            .map_err(|_| MuxError::LayoutOverflow("segmented H.264 NAL length"))?;
        if self.current_sample_offset.is_none() {
            self.current_sample_offset = Some(self.logical_size);
        }
        let prefix = source_size.to_be_bytes();
        self.segments.push(SegmentedMuxSourceSegment {
            logical_offset: self.logical_size,
            data: SegmentedMuxSourceSegmentData::Prefix(prefix),
        });
        self.logical_size = self
            .logical_size
            .checked_add(4)
            .ok_or(MuxError::LayoutOverflow(
                "segmented H.264 transformed payload",
            ))?;
        self.segments.push(SegmentedMuxSourceSegment {
            logical_offset: self.logical_size,
            data: SegmentedMuxSourceSegmentData::Bytes(bytes),
        });
        self.current_sample_size = self
            .current_sample_size
            .checked_add(
                4_u32
                    .checked_add(source_size)
                    .ok_or(MuxError::LayoutOverflow(
                        "segmented H.264 transformed sample size",
                    ))?,
            )
            .ok_or(MuxError::LayoutOverflow(
                "segmented H.264 staged sample size",
            ))?;
        self.logical_size = self
            .logical_size
            .checked_add(u64::from(source_size))
            .ok_or(MuxError::LayoutOverflow(
                "segmented H.264 transformed payload",
            ))?;
        self.current_sync |= is_sync_sample;
        self.current_has_vcl |= is_vcl;
        Ok(())
    }
}

fn stage_h264_nal(state: &mut H264StageState, nal: AnnexBNal) -> Result<(), MuxError> {
    if nal.bytes.is_empty() {
        return Ok(());
    }
    let nal_type = nal.bytes[0] & 0x1F;
    match nal_type {
        7 => push_unique_nal(&mut state.sps_list, nal.bytes),
        8 => push_unique_nal(&mut state.pps_list, nal.bytes),
        9 => state.finish_current_sample(),
        _ => {
            let is_vcl = is_h264_vcl_nal_type(nal_type);
            if is_vcl && h264_first_mb_in_slice(&nal.bytes, "h264")? == 0 && state.current_has_vcl {
                state.finish_current_sample();
            }
            let nal_len = u32::try_from(nal.bytes.len())
                .map_err(|_| MuxError::LayoutOverflow("H.264 NAL length"))?;
            state.append_sample_nal(nal.source_offset, nal_len, nal_type == 5, is_vcl)?;
        }
    }
    Ok(())
}

fn stage_h264_nal_segmented(state: &mut H264StageState, nal: AnnexBNal) -> Result<(), MuxError> {
    if nal.bytes.is_empty() {
        return Ok(());
    }
    let nal_type = nal.bytes[0] & 0x1F;
    match nal_type {
        7 => push_unique_nal(&mut state.sps_list, nal.bytes),
        8 => push_unique_nal(&mut state.pps_list, nal.bytes),
        9 => state.finish_current_sample(),
        _ => {
            let is_vcl = is_h264_vcl_nal_type(nal_type);
            if is_vcl && h264_first_mb_in_slice(&nal.bytes, "h264")? == 0 && state.current_has_vcl {
                state.finish_current_sample();
            }
            state.append_sample_bytes(nal.bytes, nal_type == 5, is_vcl)?;
        }
    }
    Ok(())
}

fn finalize_h264_staged_track(
    path: &Path,
    mut state: H264StageState,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    state.finish_current_sample();
    if state.sps_list.is_empty() || state.pps_list.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.264 input must include SPS and PPS NAL units".to_string(),
        });
    }
    if state.samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.264 input contained parameter sets but no media samples".to_string(),
        });
    }

    let sps_info = parse_h264_sps(&state.sps_list[0], spec)?;
    let (timescale, sample_duration) = match (
        sps_info.timing_time_scale,
        sps_info.timing_num_units_in_tick,
    ) {
        (Some(time_scale), Some(num_units_in_tick))
            if time_scale != 0 && num_units_in_tick != 0 =>
        {
            (time_scale, num_units_in_tick.saturating_mul(2))
        }
        _ if state.samples.len() == 1 => (25_000, 1_000),
        _ => {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message:
                    "multi-sample H.264 inputs currently require timing info in SPS VUI parameters"
                        .to_string(),
            });
        }
    };
    for sample in &mut state.samples {
        sample.duration = sample_duration;
    }

    let sample_entry_box =
        build_h264_sample_entry_box(&sps_info, &state.sps_list, &state.pps_list, true)?;
    let track_width = display_track_width(sps_info.width, sps_info.pixel_aspect_ratio.as_ref());
    Ok(IndexedAnnexBTrack {
        segmented_source: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: state.segments,
            total_size: state.logical_size,
        },
        track_width,
        track_height: sps_info.height,
        timescale,
        sample_entry_box,
        source_edit_media_time: None,
        samples: state.samples,
    })
}

const fn is_h264_vcl_nal_type(nal_type: u8) -> bool {
    matches!(nal_type, 1..=5)
}

fn h264_first_mb_in_slice(nal: &[u8], spec: &str) -> Result<u64, MuxError> {
    if nal.len() < 2 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.264 VCL NAL is too short".to_string(),
        });
    }
    let rbsp = nal_to_rbsp(&nal[1..]);
    let mut reader = BitReader::new(Cursor::new(rbsp));
    Ok(u64::from(read_ue(&mut reader, spec)?))
}

fn build_h264_sample_entry_box(
    sps_info: &H264SpsInfo,
    sequence_parameter_sets: &[Vec<u8>],
    picture_parameter_sets: &[Vec<u8>],
    include_colr: bool,
) -> Result<Vec<u8>, MuxError> {
    let avcc = AVCDecoderConfiguration {
        configuration_version: 1,
        profile: sps_info.profile,
        profile_compatibility: sps_info.profile_compatibility,
        level: sps_info.level,
        length_size_minus_one: 3,
        num_of_sequence_parameter_sets: u8::try_from(sequence_parameter_sets.len())
            .map_err(|_| MuxError::LayoutOverflow("AVC SPS count"))?,
        sequence_parameter_sets: sequence_parameter_sets
            .iter()
            .map(|nal| -> Result<AVCParameterSet, MuxError> {
                Ok(AVCParameterSet {
                    length: u16::try_from(nal.len())
                        .map_err(|_| MuxError::LayoutOverflow("AVC SPS length"))?,
                    nal_unit: nal.clone(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        num_of_picture_parameter_sets: u8::try_from(picture_parameter_sets.len())
            .map_err(|_| MuxError::LayoutOverflow("AVC PPS count"))?,
        picture_parameter_sets: picture_parameter_sets
            .iter()
            .map(|nal| -> Result<AVCParameterSet, MuxError> {
                Ok(AVCParameterSet {
                    length: u16::try_from(nal.len())
                        .map_err(|_| MuxError::LayoutOverflow("AVC PPS length"))?,
                    nal_unit: nal.clone(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        high_profile_fields_enabled: sps_info.high_profile_fields_enabled,
        chroma_format: sps_info.chroma_format,
        bit_depth_luma_minus8: sps_info.bit_depth_luma_minus8,
        bit_depth_chroma_minus8: sps_info.bit_depth_chroma_minus8,
        num_of_sequence_parameter_set_ext: 0,
        sequence_parameter_sets_ext: Vec::new(),
    };

    build_h264_sample_entry_box_from_avc_config(sps_info, avcc, include_colr)
}

fn build_h264_sample_entry_box_from_avc_config(
    sps_info: &H264SpsInfo,
    avcc: AVCDecoderConfiguration,
    include_colr: bool,
) -> Result<Vec<u8>, MuxError> {
    let mut avc1 = VisualSampleEntry::default();
    avc1.set_box_type(FourCc::from_bytes(*b"avc1"));
    avc1.sample_entry = SampleEntry {
        box_type: FourCc::from_bytes(*b"avc1"),
        data_reference_index: 1,
    };
    avc1.width = sps_info.width;
    avc1.height = sps_info.height;
    avc1.horizresolution = 72_u32 << 16;
    avc1.vertresolution = 72_u32 << 16;
    avc1.frame_count = 1;
    avc1.depth = 0x0018;
    avc1.pre_defined3 = -1;

    let mut child_boxes = vec![super::super::mp4::encode_typed_box(&avcc, &[])?];
    if let Some(pixel_aspect_ratio) = sps_info.pixel_aspect_ratio.as_ref() {
        child_boxes.push(super::super::mp4::encode_typed_box(
            &Pasp {
                h_spacing: pixel_aspect_ratio.h_spacing,
                v_spacing: pixel_aspect_ratio.v_spacing,
            },
            &[],
        )?);
    }
    if include_colr {
        let color_info = sps_info.color_info.as_ref().map_or(
            Colr {
                colour_type: FourCc::from_bytes(*b"nclx"),
                colour_primaries: 1,
                transfer_characteristics: 1,
                matrix_coefficients: 1,
                full_range_flag: false,
                reserved: 0,
                profile: Vec::new(),
                unknown: Vec::new(),
            },
            |color_info| Colr {
                colour_type: FourCc::from_bytes(*b"nclx"),
                colour_primaries: color_info.colour_primaries,
                transfer_characteristics: color_info.transfer_characteristics,
                matrix_coefficients: color_info.matrix_coefficients,
                full_range_flag: color_info.full_range_flag,
                reserved: 0,
                profile: Vec::new(),
                unknown: Vec::new(),
            },
        );
        child_boxes.push(super::super::mp4::encode_typed_box(&color_info, &[])?);
    }

    super::super::mp4::encode_typed_box(&avc1, &child_boxes.concat())
}

pub(in crate::mux) fn retune_carried_h264_sample_entry_box<I>(
    sample_entry_box: &[u8],
    timescale: u32,
    samples: I,
) -> Result<Vec<u8>, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    const VISUAL_SAMPLE_ENTRY_HEADER_SIZE: usize = 86;

    if sample_entry_box.len() < VISUAL_SAMPLE_ENTRY_HEADER_SIZE {
        return Err(MuxError::UnsupportedTrackImport {
            spec: "h264".to_string(),
            message:
                "carried H.264 sample entry is truncated before the visual sample entry header"
                    .to_string(),
        });
    }
    if &sample_entry_box[4..8] != b"avc1" {
        return Err(MuxError::UnsupportedTrackImport {
            spec: "h264".to_string(),
            message: "carried H.264 sample entry did not use the `avc1` sample entry type"
                .to_string(),
        });
    }

    let mut avcc_box = None::<Vec<u8>>;
    let mut child_offset = VISUAL_SAMPLE_ENTRY_HEADER_SIZE;
    while sample_entry_box.len().saturating_sub(child_offset) >= 8 {
        let child_size = u32::from_be_bytes(
            sample_entry_box[child_offset..child_offset + 4]
                .try_into()
                .unwrap(),
        );
        let child_size = usize::try_from(child_size)
            .map_err(|_| MuxError::LayoutOverflow("H.264 sample-entry child size"))?;
        if child_size < 8 || child_offset + child_size > sample_entry_box.len() {
            return Err(MuxError::UnsupportedTrackImport {
                spec: "h264".to_string(),
                message: "carried H.264 sample entry contained one truncated child box".to_string(),
            });
        }
        if &sample_entry_box[child_offset + 4..child_offset + 8] == b"avcC" {
            avcc_box = Some(sample_entry_box[child_offset..child_offset + child_size].to_vec());
        }
        child_offset += child_size;
    }

    let avcc_box = avcc_box.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: "h264".to_string(),
        message: "carried H.264 sample entry did not contain an `avcC` decoder configuration box"
            .to_string(),
    })?;
    let pasp_box = super::super::mp4::encode_typed_box(
        &Pasp {
            h_spacing: 1,
            v_spacing: 1,
        },
        &[],
    )?;
    let btrt_box = super::super::mp4::encode_typed_box(
        &build_btrt_from_sample_sizes(samples, timescale).map_err(|error| match error {
            MuxError::LayoutOverflow(_) => error,
            _ => MuxError::LayoutOverflow("carried H.264 bitrate box"),
        })?,
        &[],
    )?;
    let rebuilt_size = VISUAL_SAMPLE_ENTRY_HEADER_SIZE
        .checked_add(avcc_box.len())
        .and_then(|size| size.checked_add(pasp_box.len()))
        .and_then(|size| size.checked_add(btrt_box.len()))
        .ok_or(MuxError::LayoutOverflow("carried H.264 sample-entry size"))?;
    let rebuilt_size = u32::try_from(rebuilt_size)
        .map_err(|_| MuxError::LayoutOverflow("carried H.264 sample-entry size"))?;

    let mut rebuilt = Vec::with_capacity(usize::try_from(rebuilt_size).unwrap());
    rebuilt.extend_from_slice(&rebuilt_size.to_be_bytes());
    rebuilt.extend_from_slice(&sample_entry_box[4..VISUAL_SAMPLE_ENTRY_HEADER_SIZE]);
    rebuilt.extend_from_slice(&avcc_box);
    rebuilt.extend_from_slice(&pasp_box);
    rebuilt.extend_from_slice(&btrt_box);
    Ok(rebuilt)
}

const fn h264_profile_supports_config_extensions(profile: u8) -> bool {
    matches!(profile, 100 | 110 | 122 | 144)
}

struct H264SpsInfo {
    width: u16,
    height: u16,
    profile: u8,
    profile_compatibility: u8,
    level: u8,
    high_profile_fields_enabled: bool,
    chroma_format: u8,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
    timing_time_scale: Option<u32>,
    timing_num_units_in_tick: Option<u32>,
    pixel_aspect_ratio: Option<H264PixelAspectRatio>,
    color_info: Option<H264ColorInfo>,
}

struct H264PixelAspectRatio {
    h_spacing: u32,
    v_spacing: u32,
}

struct H264ColorInfo {
    colour_primaries: u16,
    transfer_characteristics: u16,
    matrix_coefficients: u16,
    full_range_flag: bool,
}

type H264VuiInfo = (
    Option<u32>,
    Option<u32>,
    Option<H264PixelAspectRatio>,
    Option<H264ColorInfo>,
);

fn parse_h264_sps(nal: &[u8], spec: &str) -> Result<H264SpsInfo, MuxError> {
    if nal.len() < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.264 SPS NAL is too short".to_string(),
        });
    }
    let profile = nal[1];
    let rbsp = nal_to_rbsp(&nal[1..]);
    let mut reader = BitReader::new(Cursor::new(rbsp));
    let profile_idc = read_bits_u8(&mut reader, 8, spec)?;
    let profile_compatibility_bits = read_bits_u8(&mut reader, 8, spec)?;
    let level_idc = read_bits_u8(&mut reader, 8, spec)?;
    let _seq_parameter_set_id = read_ue(&mut reader, spec)?;

    let mut chroma_format_idc = 1_u8;
    let mut bit_depth_luma_minus8 = 0_u8;
    let mut bit_depth_chroma_minus8 = 0_u8;
    let mut high_profile_fields_enabled = false;
    if matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        high_profile_fields_enabled = true;
        chroma_format_idc = u8::try_from(read_ue(&mut reader, spec)?).map_err(|_| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "H.264 chroma format does not fit in u8".to_string(),
            }
        })?;
        if chroma_format_idc == 3 {
            let _separate_colour_plane_flag = read_bit(&mut reader, spec)?;
        }
        bit_depth_luma_minus8 = u8::try_from(read_ue(&mut reader, spec)?).map_err(|_| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "H.264 luma bit depth does not fit in u8".to_string(),
            }
        })?;
        bit_depth_chroma_minus8 = u8::try_from(read_ue(&mut reader, spec)?).map_err(|_| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "H.264 chroma bit depth does not fit in u8".to_string(),
            }
        })?;
        let _qpprime_y_zero_transform_bypass_flag = read_bit(&mut reader, spec)?;
        let seq_scaling_matrix_present_flag = read_bit(&mut reader, spec)?;
        if seq_scaling_matrix_present_flag {
            let count = if chroma_format_idc != 3 { 8 } else { 12 };
            for index in 0..count {
                if read_bit(&mut reader, spec)? {
                    skip_scaling_list(&mut reader, if index < 6 { 16 } else { 64 }, spec)?;
                }
            }
        }
    }

    let _log2_max_frame_num_minus4 = read_ue(&mut reader, spec)?;
    let pic_order_cnt_type = read_ue(&mut reader, spec)?;
    if pic_order_cnt_type == 0 {
        let _log2_max_pic_order_cnt_lsb_minus4 = read_ue(&mut reader, spec)?;
    } else if pic_order_cnt_type == 1 {
        let _delta_pic_order_always_zero_flag = read_bit(&mut reader, spec)?;
        let _offset_for_non_ref_pic = read_se(&mut reader, spec)?;
        let _offset_for_top_to_bottom_field = read_se(&mut reader, spec)?;
        let cycle = read_ue(&mut reader, spec)?;
        for _ in 0..cycle {
            let _ = read_se(&mut reader, spec)?;
        }
    }
    let _max_num_ref_frames = read_ue(&mut reader, spec)?;
    let _gaps_in_frame_num_value_allowed_flag = read_bit(&mut reader, spec)?;
    let pic_width_in_mbs_minus1 = read_ue(&mut reader, spec)?;
    let pic_height_in_map_units_minus1 = read_ue(&mut reader, spec)?;
    let frame_mbs_only_flag = read_bit(&mut reader, spec)?;
    if !frame_mbs_only_flag {
        let _mb_adaptive_frame_field_flag = read_bit(&mut reader, spec)?;
    }
    let _direct_8x8_inference_flag = read_bit(&mut reader, spec)?;
    let frame_cropping_flag = read_bit(&mut reader, spec)?;
    let (
        frame_crop_left_offset,
        frame_crop_right_offset,
        frame_crop_top_offset,
        frame_crop_bottom_offset,
    ) = if frame_cropping_flag {
        (
            read_ue(&mut reader, spec)?,
            read_ue(&mut reader, spec)?,
            read_ue(&mut reader, spec)?,
            read_ue(&mut reader, spec)?,
        )
    } else {
        (0, 0, 0, 0)
    };

    let vui_parameters_present_flag = read_bit(&mut reader, spec)?;
    let (timing_num_units_in_tick, timing_time_scale, pixel_aspect_ratio, color_info) =
        if vui_parameters_present_flag {
            parse_vui_timing(&mut reader, spec)?
        } else {
            (None, None, None, None)
        };

    let sub_width_c = match chroma_format_idc {
        0 | 3 => 1_u32,
        _ => 2_u32,
    };
    let sub_height_c = match chroma_format_idc {
        0 => {
            if frame_mbs_only_flag {
                1
            } else {
                2
            }
        }
        1 => {
            if frame_mbs_only_flag {
                2
            } else {
                4
            }
        }
        2 | 3 => {
            if frame_mbs_only_flag {
                1
            } else {
                2
            }
        }
        _ => 1,
    };
    let crop_unit_x = if chroma_format_idc == 0 {
        1
    } else {
        sub_width_c
    };
    let crop_unit_y = if chroma_format_idc == 0 {
        if frame_mbs_only_flag { 2 } else { 4 }
    } else {
        sub_height_c
    };

    let width = ((pic_width_in_mbs_minus1 + 1) * 16)
        .saturating_sub((frame_crop_left_offset + frame_crop_right_offset) * crop_unit_x);
    let height =
        ((pic_height_in_map_units_minus1 + 1) * 16 * if frame_mbs_only_flag { 1 } else { 2 })
            .saturating_sub((frame_crop_top_offset + frame_crop_bottom_offset) * crop_unit_y);

    Ok(H264SpsInfo {
        width: u16::try_from(width).map_err(|_| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.264 SPS width does not fit in u16".to_string(),
        })?,
        height: u16::try_from(height).map_err(|_| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.264 SPS height does not fit in u16".to_string(),
        })?,
        profile,
        profile_compatibility: profile_compatibility_bits,
        level: level_idc,
        high_profile_fields_enabled,
        chroma_format: chroma_format_idc,
        bit_depth_luma_minus8,
        bit_depth_chroma_minus8,
        timing_time_scale,
        timing_num_units_in_tick,
        pixel_aspect_ratio,
        color_info,
    })
}

fn display_track_width(width: u16, pixel_aspect_ratio: Option<&H264PixelAspectRatio>) -> u16 {
    let Some(pixel_aspect_ratio) = pixel_aspect_ratio else {
        return width;
    };
    let numerator = u64::from(width)
        .saturating_mul(u64::from(pixel_aspect_ratio.h_spacing))
        .saturating_add(u64::from(pixel_aspect_ratio.v_spacing / 2));
    let display_width = numerator / u64::from(pixel_aspect_ratio.v_spacing);
    u16::try_from(display_width).unwrap_or(width)
}

fn parse_vui_timing<R>(reader: &mut BitReader<R>, spec: &str) -> Result<H264VuiInfo, MuxError>
where
    R: Read,
{
    let mut pixel_aspect_ratio = None;
    if read_bit(reader, spec)? {
        let aspect_ratio_idc = read_bits_u8(reader, 8, spec)?;
        if aspect_ratio_idc == 255 {
            let sar_width = read_bits_u16(reader, 16, spec)?;
            let sar_height = read_bits_u16(reader, 16, spec)?;
            if sar_width != 0 && sar_height != 0 && sar_width != sar_height {
                pixel_aspect_ratio = Some(H264PixelAspectRatio {
                    h_spacing: u32::from(sar_width),
                    v_spacing: u32::from(sar_height),
                });
            }
        } else {
            pixel_aspect_ratio = h264_pixel_aspect_ratio_from_idc(aspect_ratio_idc);
        }
    }
    if read_bit(reader, spec)? {
        let _overscan_appropriate_flag = read_bit(reader, spec)?;
    }
    let mut color_info = None;
    if read_bit(reader, spec)? {
        let _video_format = read_bits_u8(reader, 3, spec)?;
        let video_full_range_flag = read_bit(reader, spec)?;
        if read_bit(reader, spec)? {
            color_info = Some(H264ColorInfo {
                colour_primaries: u16::from(read_bits_u8(reader, 8, spec)?),
                transfer_characteristics: u16::from(read_bits_u8(reader, 8, spec)?),
                matrix_coefficients: u16::from(read_bits_u8(reader, 8, spec)?),
                full_range_flag: video_full_range_flag,
            });
        }
    }
    if read_bit(reader, spec)? {
        let _chroma_sample_loc_type_top_field = read_ue(reader, spec)?;
        let _chroma_sample_loc_type_bottom_field = read_ue(reader, spec)?;
    }
    if read_bit(reader, spec)? {
        let num_units_in_tick = read_bits_u32(reader, 32, spec)?;
        let time_scale = read_bits_u32(reader, 32, spec)?;
        let _fixed_frame_rate_flag = read_bit(reader, spec)?;
        return Ok((
            Some(num_units_in_tick),
            Some(time_scale),
            pixel_aspect_ratio,
            color_info,
        ));
    }
    Ok((None, None, pixel_aspect_ratio, color_info))
}

fn h264_pixel_aspect_ratio_from_idc(aspect_ratio_idc: u8) -> Option<H264PixelAspectRatio> {
    let (h_spacing, v_spacing) = match aspect_ratio_idc {
        1 => (1, 1),
        2 => (12, 11),
        3 => (10, 11),
        4 => (16, 11),
        5 => (40, 33),
        6 => (24, 11),
        7 => (20, 11),
        8 => (32, 11),
        9 => (80, 33),
        10 => (18, 11),
        11 => (15, 11),
        12 => (64, 33),
        13 => (160, 99),
        14 => (4, 3),
        15 => (3, 2),
        16 => (2, 1),
        _ => return None,
    };
    (h_spacing != v_spacing).then_some(H264PixelAspectRatio {
        h_spacing,
        v_spacing,
    })
}

fn skip_scaling_list<R>(reader: &mut BitReader<R>, size: usize, spec: &str) -> Result<(), MuxError>
where
    R: Read,
{
    let mut last_scale = 8_i32;
    let mut next_scale = 8_i32;
    for _ in 0..size {
        if next_scale != 0 {
            let delta_scale = read_se(reader, spec)?;
            next_scale = (last_scale + delta_scale + 256) % 256;
        }
        last_scale = if next_scale == 0 {
            last_scale
        } else {
            next_scale
        };
    }
    Ok(())
}

fn read_bit<R>(reader: &mut BitReader<R>, spec: &str) -> Result<bool, MuxError>
where
    R: Read,
{
    read_bit_labeled(reader, spec, "H.264")
}

fn read_bits_u8<R>(reader: &mut BitReader<R>, width: usize, spec: &str) -> Result<u8, MuxError>
where
    R: Read,
{
    read_bits_u8_labeled(reader, width, spec, "H.264")
}

fn read_bits_u16<R>(reader: &mut BitReader<R>, width: usize, spec: &str) -> Result<u16, MuxError>
where
    R: Read,
{
    read_bits_u16_labeled(reader, width, spec, "H.264")
}

fn read_bits_u32<R>(reader: &mut BitReader<R>, width: usize, spec: &str) -> Result<u32, MuxError>
where
    R: Read,
{
    read_bits_u32_labeled(reader, width, spec, "H.264")
}

fn read_ue<R>(reader: &mut BitReader<R>, spec: &str) -> Result<u32, MuxError>
where
    R: Read,
{
    read_ue_labeled(reader, spec, "H.264")
}

fn read_se<R>(reader: &mut BitReader<R>, spec: &str) -> Result<i32, MuxError>
where
    R: Read,
{
    read_se_labeled(reader, spec, "H.264")
}
