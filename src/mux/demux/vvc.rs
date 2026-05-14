use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::AsyncReadExt;

use crate::FourCc;
use crate::bitio::{BitReader, BitWriter};
use crate::boxes::AnyTypeBox;
use crate::boxes::iso14496_12::{SampleEntry, VisualSampleEntry};
use crate::boxes::iso14496_15::VVCDecoderConfiguration;

use super::super::MuxError;
use super::super::import::{
    SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData, SegmentedMuxSourceSpec, StagedSample,
};
use super::annexb_common::{
    AnnexBNal, AnnexBNalScanner, IndexedAnnexBTrack, nal_to_rbsp, push_unique_nal,
    read_bit_labeled, read_bits_u8_labeled, read_bits_u32_labeled, read_ue_labeled,
    skip_bits_labeled,
};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;

const VVC1: FourCc = FourCc::from_bytes(*b"vvc1");
const VVCC_LENGTH_SIZE_MINUS_ONE: u8 = 3;
const VVC_NAL_TYPE_OPI: u8 = 12;
const VVC_NAL_TYPE_DCI: u8 = 13;
const VVC_NAL_TYPE_VPS: u8 = 14;
const VVC_NAL_TYPE_SPS: u8 = 15;
const VVC_NAL_TYPE_PPS: u8 = 16;
const VVC_NAL_TYPE_PREFIX_APS: u8 = 17;
const VVC_NAL_TYPE_AUD: u8 = 20;
const DEFAULT_SINGLE_SAMPLE_VVC_TIMESCALE: u32 = 25;
const DEFAULT_SINGLE_SAMPLE_VVC_COMPOSITION_OFFSET: i32 = 1;
const DEFAULT_SINGLE_SAMPLE_VVC_EDIT_MEDIA_TIME: u64 = 1;
const VVC_GENERAL_CONSTRAINT_INFO_BYTES: usize = 12;

pub(in crate::mux) fn stage_annex_b_vvc_sync(
    path: &Path,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    let mut file = File::open(path)?;
    let mut scanner = AnnexBNalScanner::default();
    let mut state = VvcStageState::default();
    let mut chunk = [0_u8; 16 * 1024];

    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        scanner.push(&chunk[..read], |nal| stage_vvc_nal(&mut state, nal, spec))?;
    }
    scanner.finish(|nal| stage_vvc_nal(&mut state, nal, spec))?;
    finalize_vvc_staged_track(path, state, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn stage_annex_b_vvc_async(
    path: &Path,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let mut scanner = AnnexBNalScanner::default();
    let mut state = VvcStageState::default();
    let mut chunk = [0_u8; 16 * 1024];

    loop {
        let read = file.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        for nal in scanner.collect(&chunk[..read]) {
            stage_vvc_nal(&mut state, nal, spec)?;
        }
    }
    for nal in scanner.finish_collect() {
        stage_vvc_nal(&mut state, nal, spec)?;
    }
    finalize_vvc_staged_track(path, state, spec)
}

pub(in crate::mux) fn stage_annex_b_vvc_segmented_sync(
    path: &Path,
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    let mut scanner = AnnexBNalScanner::default();
    let mut state = VvcStageState::default();
    let mut offset = 0_u64;

    while offset < total_size {
        let read_len = usize::try_from((total_size - offset).min(16 * 1024))
            .map_err(|_| MuxError::LayoutOverflow("segmented VVC scan chunk length"))?;
        let mut chunk = vec![0_u8; read_len];
        read_segmented_bytes_sync(
            file,
            segments,
            total_size,
            offset,
            &mut chunk,
            spec,
            "segmented VVC scan chunk is truncated",
        )?;
        for nal in scanner.collect(&chunk) {
            stage_vvc_nal_segmented(&mut state, nal, spec)?;
        }
        offset = offset
            .checked_add(u64::try_from(read_len).unwrap())
            .ok_or(MuxError::LayoutOverflow("segmented VVC scan offset"))?;
    }
    for nal in scanner.finish_collect() {
        stage_vvc_nal_segmented(&mut state, nal, spec)?;
    }
    finalize_vvc_staged_track(path, state, spec)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn stage_annex_b_vvc_segmented_async(
    path: &Path,
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    let mut scanner = AnnexBNalScanner::default();
    let mut state = VvcStageState::default();
    let mut offset = 0_u64;

    while offset < total_size {
        let read_len = usize::try_from((total_size - offset).min(16 * 1024))
            .map_err(|_| MuxError::LayoutOverflow("segmented VVC scan chunk length"))?;
        let mut chunk = vec![0_u8; read_len];
        read_segmented_bytes_async(
            file,
            segments,
            total_size,
            offset,
            &mut chunk,
            spec,
            "segmented VVC scan chunk is truncated",
        )
        .await?;
        for nal in scanner.collect(&chunk) {
            stage_vvc_nal_segmented(&mut state, nal, spec)?;
        }
        offset = offset
            .checked_add(u64::try_from(read_len).unwrap())
            .ok_or(MuxError::LayoutOverflow("segmented VVC scan offset"))?;
    }
    for nal in scanner.finish_collect() {
        stage_vvc_nal_segmented(&mut state, nal, spec)?;
    }
    finalize_vvc_staged_track(path, state, spec)
}

#[derive(Default)]
struct VvcStageState {
    vps_list: Vec<Vec<u8>>,
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

impl VvcStageState {
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
            .ok_or(MuxError::LayoutOverflow("raw VVC transformed payload"))?;
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
                    .ok_or(MuxError::LayoutOverflow("raw VVC transformed sample size"))?,
            )
            .ok_or(MuxError::LayoutOverflow("raw VVC staged sample size"))?;
        self.logical_size = self
            .logical_size
            .checked_add(u64::from(source_size))
            .ok_or(MuxError::LayoutOverflow("raw VVC transformed payload"))?;
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
            .map_err(|_| MuxError::LayoutOverflow("segmented VVC NAL length"))?;
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
                "segmented VVC transformed payload",
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
                        "segmented VVC transformed sample size",
                    ))?,
            )
            .ok_or(MuxError::LayoutOverflow("segmented VVC staged sample size"))?;
        self.logical_size = self
            .logical_size
            .checked_add(u64::from(source_size))
            .ok_or(MuxError::LayoutOverflow(
                "segmented VVC transformed payload",
            ))?;
        self.current_sync |= is_sync_sample;
        self.current_has_vcl |= is_vcl;
        Ok(())
    }
}

fn stage_vvc_nal(state: &mut VvcStageState, nal: AnnexBNal, spec: &str) -> Result<(), MuxError> {
    if nal.bytes.len() < 2 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "VVC NAL units must be at least two bytes long".to_string(),
        });
    }
    let nal_type = vvc_nal_type(&nal.bytes);
    match nal_type {
        VVC_NAL_TYPE_VPS => push_unique_nal(&mut state.vps_list, nal.bytes),
        VVC_NAL_TYPE_SPS => push_unique_nal(&mut state.sps_list, nal.bytes),
        VVC_NAL_TYPE_PPS => push_unique_nal(&mut state.pps_list, nal.bytes),
        VVC_NAL_TYPE_PREFIX_APS => {
            let nal_len = u32::try_from(nal.bytes.len())
                .map_err(|_| MuxError::LayoutOverflow("VVC NAL length"))?;
            state.append_sample_nal(nal.source_offset, nal_len, false, false)?;
        }
        VVC_NAL_TYPE_AUD => {
            if state.current_sample_offset.is_some() {
                state.finish_current_sample();
            }
            let nal_len = u32::try_from(nal.bytes.len())
                .map_err(|_| MuxError::LayoutOverflow("VVC NAL length"))?;
            state.append_sample_nal(nal.source_offset, nal_len, false, false)?;
        }
        _ => {
            let is_vcl = is_vvc_vcl_nal_type(nal_type);
            let nal_len = u32::try_from(nal.bytes.len())
                .map_err(|_| MuxError::LayoutOverflow("VVC NAL length"))?;
            state.append_sample_nal(
                nal.source_offset,
                nal_len,
                is_vvc_sync_nal_type(nal_type),
                is_vcl,
            )?;
        }
    }
    Ok(())
}

fn stage_vvc_nal_segmented(
    state: &mut VvcStageState,
    nal: AnnexBNal,
    spec: &str,
) -> Result<(), MuxError> {
    if nal.bytes.len() < 2 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "VVC NAL units must be at least two bytes long".to_string(),
        });
    }
    let nal_type = vvc_nal_type(&nal.bytes);
    match nal_type {
        VVC_NAL_TYPE_VPS => push_unique_nal(&mut state.vps_list, nal.bytes),
        VVC_NAL_TYPE_SPS => push_unique_nal(&mut state.sps_list, nal.bytes),
        VVC_NAL_TYPE_PPS => push_unique_nal(&mut state.pps_list, nal.bytes),
        VVC_NAL_TYPE_PREFIX_APS => state.append_sample_bytes(nal.bytes, false, false)?,
        VVC_NAL_TYPE_AUD => {
            if state.current_sample_offset.is_some() {
                state.finish_current_sample();
            }
            state.append_sample_bytes(nal.bytes, false, false)?;
        }
        _ => {
            let is_vcl = is_vvc_vcl_nal_type(nal_type);
            state.append_sample_bytes(nal.bytes, is_vvc_sync_nal_type(nal_type), is_vcl)?;
        }
    }
    Ok(())
}

fn finalize_vvc_staged_track(
    path: &Path,
    mut state: VvcStageState,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    state.finish_current_sample();
    if state.sps_list.is_empty() || state.pps_list.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "VVC input must include SPS and PPS NAL units".to_string(),
        });
    }
    if state.samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "VVC input contained parameter sets but no media samples".to_string(),
        });
    }
    if state.samples.len() > 1 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "multi-sample VVC inputs are not supported on the native direct-ingest path yet"
                    .to_string(),
        });
    }

    let sps_info = parse_vvc_sps_configuration(&state.sps_list[0], spec)?;
    state.samples[0].duration = 1;
    state.samples[0].composition_time_offset = DEFAULT_SINGLE_SAMPLE_VVC_COMPOSITION_OFFSET;
    let sample_entry_box = build_vvc_sample_entry_box(
        sps_info.width,
        sps_info.height,
        build_vvc_decoder_configuration_record(
            &sps_info,
            &state.vps_list,
            &state.sps_list,
            &state.pps_list,
        )?,
    )?;

    Ok(IndexedAnnexBTrack {
        segmented_source: SegmentedMuxSourceSpec {
            path: path.to_path_buf(),
            segments: state.segments,
            total_size: state.logical_size,
        },
        track_width: sps_info.width,
        track_height: sps_info.height,
        timescale: DEFAULT_SINGLE_SAMPLE_VVC_TIMESCALE,
        sample_entry_box,
        source_edit_media_time: Some(DEFAULT_SINGLE_SAMPLE_VVC_EDIT_MEDIA_TIME),
        samples: state.samples,
    })
}

struct VvcSpsInfo {
    width: u16,
    height: u16,
    max_sublayers: u8,
    chroma_format_idc: u8,
    bit_depth: u8,
    profile_tier_level: Option<VvcProfileTierLevel>,
}

struct VvcProfileTierLevel {
    general_profile_idc: u8,
    general_tier_flag: bool,
    general_level_idc: u8,
    frame_only_constraint: bool,
    multilayer_enabled: bool,
    general_constraint_info: [u8; VVC_GENERAL_CONSTRAINT_INFO_BYTES],
    sublayer_present_mask: u8,
    sublayer_level_idc: [u8; 8],
    num_sub_profiles: u8,
    sub_profiles_idc: Vec<u32>,
}

fn parse_vvc_sps_configuration(nal: &[u8], spec: &str) -> Result<VvcSpsInfo, MuxError> {
    if nal.len() < 3 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "VVC SPS NAL is too short".to_string(),
        });
    }
    let rbsp = nal_to_rbsp(&nal[2..]);
    let mut reader = BitReader::new(Cursor::new(rbsp));

    skip_bits_labeled(&mut reader, 4, spec, "VVC SPS id")?;
    skip_bits_labeled(&mut reader, 4, spec, "VVC SPS VPS id")?;
    let max_sublayers = read_bits_u8_labeled(&mut reader, 3, spec, "VVC SPS max sublayers")?
        .checked_add(1)
        .ok_or(MuxError::LayoutOverflow("VVC SPS max sublayers"))?;
    let chroma_format_idc = read_bits_u8_labeled(&mut reader, 2, spec, "VVC SPS chroma format")?;
    let log2_ctu_size = read_bits_u8_labeled(&mut reader, 2, spec, "VVC SPS CTU size")?
        .checked_add(5)
        .ok_or(MuxError::LayoutOverflow("VVC SPS CTU size"))?;
    let profile_tier_level = if read_bit_labeled(&mut reader, spec, "VVC SPS PTL presence")? {
        Some(read_vvc_profile_tier_level(
            &mut reader,
            max_sublayers.saturating_sub(1),
            spec,
        )?)
    } else {
        None
    };
    let _gdr_enabled = read_bit_labeled(&mut reader, spec, "VVC SPS GDR enabled")?;
    let ref_pic_resampling = read_bit_labeled(&mut reader, spec, "VVC SPS ref pic resampling")?;
    if ref_pic_resampling {
        let _res_change_in_clvs =
            read_bit_labeled(&mut reader, spec, "VVC SPS res change in CLVS")?;
    }
    let mut width = read_ue_labeled(&mut reader, spec, "VVC SPS width")?;
    let mut height = read_ue_labeled(&mut reader, spec, "VVC SPS height")?;
    let conf_window_present = read_bit_labeled(&mut reader, spec, "VVC SPS conformance window")?;
    if conf_window_present {
        let left = read_ue_labeled(&mut reader, spec, "VVC SPS conformance left")?;
        let right = read_ue_labeled(&mut reader, spec, "VVC SPS conformance right")?;
        let top = read_ue_labeled(&mut reader, spec, "VVC SPS conformance top")?;
        let bottom = read_ue_labeled(&mut reader, spec, "VVC SPS conformance bottom")?;
        let (sub_width_c, sub_height_c) = match chroma_format_idc {
            1 => (2_u32, 2_u32),
            2 => (2_u32, 1_u32),
            _ => (1_u32, 1_u32),
        };
        let horizontal_crop = sub_width_c
            .checked_mul(
                left.checked_add(right)
                    .ok_or(MuxError::LayoutOverflow("VVC conformance width crop"))?,
            )
            .ok_or(MuxError::LayoutOverflow("VVC conformance width crop"))?;
        let vertical_crop = sub_height_c
            .checked_mul(
                top.checked_add(bottom)
                    .ok_or(MuxError::LayoutOverflow("VVC conformance height crop"))?,
            )
            .ok_or(MuxError::LayoutOverflow("VVC conformance height crop"))?;
        if horizontal_crop >= width || vertical_crop >= height {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "VVC SPS conformance window exceeds coded dimensions".to_string(),
            });
        }
        width -= horizontal_crop;
        height -= vertical_crop;
    }
    if width == 0 || height == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "VVC SPS coded dimensions resolved to zero".to_string(),
        });
    }
    let ctb_size_y = 1_u32
        .checked_shl(u32::from(log2_ctu_size))
        .ok_or(MuxError::LayoutOverflow("VVC CTU size"))?;
    let subpic_info_present = read_bit_labeled(&mut reader, spec, "VVC SPS subpic info")?;
    if subpic_info_present {
        let nb_subpics = read_ue_labeled(&mut reader, spec, "VVC SPS subpic count")?
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("VVC SPS subpic count"))?;
        if nb_subpics > 1 {
            let independent_subpic_flags =
                read_bit_labeled(&mut reader, spec, "VVC SPS independent subpics")?;
            let subpic_same_size =
                read_bit_labeled(&mut reader, spec, "VVC SPS equal-sized subpics")?;
            let tmp_width_bits = vvc_ceil_log2(
                width
                    .checked_add(ctb_size_y - 1)
                    .ok_or(MuxError::LayoutOverflow("VVC SPS width CTU count"))?
                    / ctb_size_y,
            );
            let tmp_height_bits = vvc_ceil_log2(
                height
                    .checked_add(ctb_size_y - 1)
                    .ok_or(MuxError::LayoutOverflow("VVC SPS height CTU count"))?
                    / ctb_size_y,
            );
            for index in 0..nb_subpics {
                if !subpic_same_size || index == 0 {
                    if index != 0 && width > ctb_size_y {
                        skip_bits_labeled(
                            &mut reader,
                            usize::try_from(tmp_width_bits).map_err(|_| {
                                MuxError::LayoutOverflow("VVC SPS subpic width bits")
                            })?,
                            spec,
                            "VVC SPS subpic CTU x",
                        )?;
                    }
                    if index != 0 && height > ctb_size_y {
                        skip_bits_labeled(
                            &mut reader,
                            usize::try_from(tmp_height_bits).map_err(|_| {
                                MuxError::LayoutOverflow("VVC SPS subpic height bits")
                            })?,
                            spec,
                            "VVC SPS subpic CTU y",
                        )?;
                    }
                    if index + 1 < nb_subpics && width > ctb_size_y {
                        skip_bits_labeled(
                            &mut reader,
                            usize::try_from(tmp_width_bits).map_err(|_| {
                                MuxError::LayoutOverflow("VVC SPS subpic width bits")
                            })?,
                            spec,
                            "VVC SPS subpic width",
                        )?;
                    }
                    if index + 1 < nb_subpics && height > ctb_size_y {
                        skip_bits_labeled(
                            &mut reader,
                            usize::try_from(tmp_height_bits).map_err(|_| {
                                MuxError::LayoutOverflow("VVC SPS subpic height bits")
                            })?,
                            spec,
                            "VVC SPS subpic height",
                        )?;
                    }
                }
                if !independent_subpic_flags {
                    let _ = read_bit_labeled(
                        &mut reader,
                        spec,
                        "VVC SPS subpic treated-as-picture flag",
                    )?;
                    let _ = read_bit_labeled(&mut reader, spec, "VVC SPS subpic loop-filter flag")?;
                }
            }
        }
        let subpic_id_len = read_ue_labeled(&mut reader, spec, "VVC SPS subpic id len")?
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("VVC SPS subpic id len"))?;
        let subpicid_mapping_explicit =
            read_bit_labeled(&mut reader, spec, "VVC SPS subpic id mapping explicit")?;
        if subpicid_mapping_explicit {
            let subpicid_mapping_present =
                read_bit_labeled(&mut reader, spec, "VVC SPS subpic id mapping present")?;
            if subpicid_mapping_present {
                for _ in 0..nb_subpics {
                    skip_bits_labeled(
                        &mut reader,
                        usize::try_from(subpic_id_len)
                            .map_err(|_| MuxError::LayoutOverflow("VVC SPS subpic id len"))?,
                        spec,
                        "VVC SPS subpic id",
                    )?;
                }
            }
        }
    }
    let bit_depth = read_ue_labeled(&mut reader, spec, "VVC SPS bitdepth minus 8")?
        .checked_add(8)
        .ok_or(MuxError::LayoutOverflow("VVC bit depth"))?;
    Ok(VvcSpsInfo {
        width: u16::try_from(width).map_err(|_| MuxError::LayoutOverflow("VVC coded width"))?,
        height: u16::try_from(height).map_err(|_| MuxError::LayoutOverflow("VVC coded height"))?,
        max_sublayers,
        chroma_format_idc,
        bit_depth: u8::try_from(bit_depth)
            .map_err(|_| MuxError::LayoutOverflow("VVC coded bit depth"))?,
        profile_tier_level,
    })
}

fn read_vvc_profile_tier_level<R>(
    reader: &mut BitReader<R>,
    max_tid: u8,
    spec: &str,
) -> Result<VvcProfileTierLevel, MuxError>
where
    R: Read,
{
    let general_profile_idc = read_bits_u8_labeled(reader, 7, spec, "VVC PTL general profile idc")?;
    let general_tier_flag = read_bit_labeled(reader, spec, "VVC PTL general tier flag")?;
    let general_level_idc = read_bits_u8_labeled(reader, 8, spec, "VVC PTL general level idc")?;
    let frame_only_constraint = read_bit_labeled(reader, spec, "VVC PTL frame-only constraint")?;
    let multilayer_enabled = read_bit_labeled(reader, spec, "VVC PTL multilayer enabled")?;
    let gci_present = read_bit_labeled(reader, spec, "VVC PTL constraint presence")?;
    let mut general_constraint_info = [0_u8; VVC_GENERAL_CONSTRAINT_INFO_BYTES];
    if gci_present {
        general_constraint_info[0] =
            0x80 | read_bits_u8_labeled(reader, 7, spec, "VVC PTL constraint prefix")?;
        for byte in &mut general_constraint_info[1..9] {
            *byte = read_bits_u8_labeled(reader, 8, spec, "VVC PTL constraint payload")?;
        }
        general_constraint_info[10] =
            read_bits_u8_labeled(reader, 2, spec, "VVC PTL constraint suffix")? << 6;
        let extension_bits = read_bits_u8_labeled(reader, 8, spec, "VVC PTL extension length")?;
        if extension_bits != 0 {
            skip_bits_labeled(
                reader,
                usize::from(extension_bits),
                spec,
                "VVC PTL extension payload",
            )?;
        }
    }
    while !reader.is_aligned() {
        let _ = read_bit_labeled(reader, spec, "VVC PTL alignment")?;
    }
    let mut sublayer_present_mask = 0_u8;
    for layer_index in (0..max_tid).rev() {
        if read_bit_labeled(reader, spec, "VVC PTL sublayer level flag")? {
            sublayer_present_mask |= 1 << layer_index;
        }
    }
    while !reader.is_aligned() {
        let _ = read_bit_labeled(reader, spec, "VVC PTL alignment")?;
    }
    let mut sublayer_level_idc = [0_u8; 8];
    for layer_index in (0..max_tid).rev() {
        if sublayer_present_mask & (1 << layer_index) != 0 {
            sublayer_level_idc[usize::from(layer_index)] =
                read_bits_u8_labeled(reader, 8, spec, "VVC PTL sublayer level idc")?;
        }
    }
    let num_sub_profiles = read_bits_u8_labeled(reader, 8, spec, "VVC PTL sub-profile count")?;
    let mut sub_profiles_idc = Vec::with_capacity(usize::from(num_sub_profiles));
    for _ in 0..num_sub_profiles {
        sub_profiles_idc.push(read_bits_u32_labeled(
            reader,
            32,
            spec,
            "VVC PTL sub-profile idc",
        )?);
    }
    Ok(VvcProfileTierLevel {
        general_profile_idc,
        general_tier_flag,
        general_level_idc,
        frame_only_constraint,
        multilayer_enabled,
        general_constraint_info,
        sublayer_present_mask,
        sublayer_level_idc,
        num_sub_profiles,
        sub_profiles_idc,
    })
}

fn build_vvc_decoder_configuration_record(
    sps_info: &VvcSpsInfo,
    vps_list: &[Vec<u8>],
    sps_list: &[Vec<u8>],
    pps_list: &[Vec<u8>],
) -> Result<Vec<u8>, MuxError> {
    let mut writer = BitWriter::new(Vec::new());
    write_u8_bits(&mut writer, 0x1F, 5)?;
    write_u8_bits(&mut writer, VVCC_LENGTH_SIZE_MINUS_ONE, 2)?;
    writer
        .write_bit(sps_info.profile_tier_level.is_some())
        .map_err(|_| MuxError::LayoutOverflow("VVC decoder configuration record"))?;
    if let Some(ptl) = &sps_info.profile_tier_level {
        write_u16_bits(&mut writer, 0, 9)?;
        write_u8_bits(&mut writer, sps_info.max_sublayers, 3)?;
        write_u8_bits(&mut writer, 1, 2)?;
        write_u8_bits(&mut writer, sps_info.chroma_format_idc, 2)?;
        let bit_depth_minus_eight =
            sps_info
                .bit_depth
                .checked_sub(8)
                .ok_or(MuxError::UnsupportedTrackImport {
                    spec: "VVC".to_string(),
                    message: "VVC bit depth must be at least 8".to_string(),
                })?;
        write_u8_bits(&mut writer, bit_depth_minus_eight, 3)?;
        write_u8_bits(&mut writer, 0x1F, 5)?;
        write_u8_bits(&mut writer, 0, 2)?;
        write_u8_bits(
            &mut writer,
            u8::try_from(VVC_GENERAL_CONSTRAINT_INFO_BYTES)
                .map_err(|_| MuxError::LayoutOverflow("VVC constraint info length"))?,
            6,
        )?;
        write_u8_bits(&mut writer, ptl.general_profile_idc, 7)?;
        writer
            .write_bit(ptl.general_tier_flag)
            .map_err(|_| MuxError::LayoutOverflow("VVC decoder configuration record"))?;
        write_u8_bits(&mut writer, ptl.general_level_idc, 8)?;
        writer
            .write_bit(ptl.frame_only_constraint)
            .map_err(|_| MuxError::LayoutOverflow("VVC decoder configuration record"))?;
        writer
            .write_bit(ptl.multilayer_enabled)
            .map_err(|_| MuxError::LayoutOverflow("VVC decoder configuration record"))?;
        for &byte in &ptl.general_constraint_info[..VVC_GENERAL_CONSTRAINT_INFO_BYTES - 1] {
            write_u8_bits(&mut writer, byte, 8)?;
        }
        write_u8_bits(
            &mut writer,
            ptl.general_constraint_info[VVC_GENERAL_CONSTRAINT_INFO_BYTES - 1],
            6,
        )?;
        for layer_index in (0..sps_info.max_sublayers.saturating_sub(1)).rev() {
            writer
                .write_bit(ptl.sublayer_present_mask & (1 << layer_index) != 0)
                .map_err(|_| MuxError::LayoutOverflow("VVC decoder configuration record"))?;
        }
        if sps_info.max_sublayers > 1 {
            for _ in sps_info.max_sublayers..=8 {
                writer
                    .write_bit(false)
                    .map_err(|_| MuxError::LayoutOverflow("VVC decoder configuration record"))?;
            }
        }
        for layer_index in (0..sps_info.max_sublayers.saturating_sub(1)).rev() {
            if ptl.sublayer_present_mask & (1 << layer_index) != 0 {
                write_u8_bits(
                    &mut writer,
                    ptl.sublayer_level_idc[usize::from(layer_index)],
                    8,
                )?;
            }
        }
        write_u8_bits(&mut writer, ptl.num_sub_profiles, 8)?;
        for &sub_profile_idc in &ptl.sub_profiles_idc {
            write_u32_bits(&mut writer, sub_profile_idc, 32)?;
        }
        write_u16_bits(&mut writer, sps_info.width, 16)?;
        write_u16_bits(&mut writer, sps_info.height, 16)?;
        write_u16_bits(&mut writer, 0, 16)?;
    }
    let mut arrays = Vec::new();
    if !vps_list.is_empty() {
        arrays.push((VVC_NAL_TYPE_VPS, vps_list));
    }
    arrays.push((VVC_NAL_TYPE_SPS, sps_list));
    arrays.push((VVC_NAL_TYPE_PPS, pps_list));
    write_u8_bits(
        &mut writer,
        u8::try_from(arrays.len()).map_err(|_| MuxError::LayoutOverflow("VVC NAL array count"))?,
        8,
    )?;
    for (nal_type, nalus) in arrays {
        writer
            .write_bit(true)
            .map_err(|_| MuxError::LayoutOverflow("VVC decoder configuration record"))?;
        write_u8_bits(&mut writer, 0, 2)?;
        write_u8_bits(&mut writer, nal_type, 5)?;
        if nal_type != VVC_NAL_TYPE_OPI && nal_type != VVC_NAL_TYPE_DCI {
            write_u16_bits(
                &mut writer,
                u16::try_from(nalus.len())
                    .map_err(|_| MuxError::LayoutOverflow("VVC NAL count"))?,
                16,
            )?;
        }
        for nal in nalus {
            write_u16_bits(
                &mut writer,
                u16::try_from(nal.len()).map_err(|_| MuxError::LayoutOverflow("VVC NAL length"))?,
                16,
            )?;
            writer
                .write_all(nal)
                .map_err(|_| MuxError::LayoutOverflow("VVC decoder configuration record"))?;
        }
    }
    writer
        .into_inner()
        .map_err(|_| MuxError::LayoutOverflow("VVC decoder configuration record"))
}

fn write_u8_bits(writer: &mut BitWriter<Vec<u8>>, value: u8, width: usize) -> Result<(), MuxError> {
    writer
        .write_bits(&[value], width)
        .map_err(|_| MuxError::LayoutOverflow("VVC decoder configuration record"))
}

fn write_u16_bits(
    writer: &mut BitWriter<Vec<u8>>,
    value: u16,
    width: usize,
) -> Result<(), MuxError> {
    writer
        .write_bits(&value.to_be_bytes(), width)
        .map_err(|_| MuxError::LayoutOverflow("VVC decoder configuration record"))
}

fn write_u32_bits(
    writer: &mut BitWriter<Vec<u8>>,
    value: u32,
    width: usize,
) -> Result<(), MuxError> {
    writer
        .write_bits(&value.to_be_bytes(), width)
        .map_err(|_| MuxError::LayoutOverflow("VVC decoder configuration record"))
}

fn vvc_ceil_log2(value: u32) -> u32 {
    let mut bits = 0_u32;
    while value > (1_u32 << bits) {
        bits = bits.saturating_add(1);
    }
    bits
}

fn build_vvc_sample_entry_box(
    width: u16,
    height: u16,
    decoder_configuration_record: Vec<u8>,
) -> Result<Vec<u8>, MuxError> {
    let mut sample_entry = VisualSampleEntry::default();
    sample_entry.set_box_type(VVC1);
    sample_entry.sample_entry = SampleEntry {
        box_type: VVC1,
        data_reference_index: 1,
    };
    sample_entry.width = width;
    sample_entry.height = height;
    sample_entry.horizresolution = 72_u32 << 16;
    sample_entry.vertresolution = 72_u32 << 16;
    sample_entry.frame_count = 1;
    sample_entry.depth = 0x0018;
    sample_entry.pre_defined3 = -1;

    let child_boxes = super::super::mp4::encode_typed_box(
        &VVCDecoderConfiguration {
            version: 0,
            flags: 0,
            decoder_configuration_record,
        },
        &[],
    )?;

    super::super::mp4::encode_typed_box(&sample_entry, &child_boxes)
}

fn vvc_nal_type(nal: &[u8]) -> u8 {
    nal[1] >> 3
}

fn is_vvc_vcl_nal_type(nal_type: u8) -> bool {
    nal_type <= 11
}

fn is_vvc_sync_nal_type(nal_type: u8) -> bool {
    matches!(nal_type, 7..=11)
}
