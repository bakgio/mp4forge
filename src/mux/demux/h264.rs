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
    build_btrt_from_sample_sizes_with_total_duration,
};
use super::annexb_common::{
    AnnexBNal, AnnexBNalScanner, IndexedAnnexBTrack, nal_to_rbsp, push_unique_nal,
    read_bit_labeled, read_bits_u8_labeled, read_bits_u16_labeled, read_bits_u32_labeled,
    read_se_labeled, read_ue_labeled,
};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;

const DEFAULT_RAW_H264_TIMESCALE: u32 = 25_000;
const DEFAULT_RAW_H264_SAMPLE_DURATION: u32 = 1_000;

pub(in crate::mux) fn stage_annex_b_h264_sync(
    path: &Path,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    let mut file = File::open(path)?;
    let mut scanner = AnnexBNalScanner::default();
    let (sps_list, pps_list) = collect_h264_parameter_sets_sync(path)?;
    let mut state = H264StageState::with_parameter_sets(sps_list, pps_list, false);
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
    build_h264_sample_entry_from_avc_config_with_box_type_and_options(
        avcc,
        FourCc::from_bytes(*b"avc1"),
        spec,
        include_colr,
    )
}

pub(in crate::mux) fn build_h264_sample_entry_from_avc_config_with_box_type_and_options(
    avcc: &AVCDecoderConfiguration,
    sample_entry_type: FourCc,
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
        authored_avcc.chroma_format = sps_info.chroma_format;
        authored_avcc.bit_depth_luma_minus8 = sps_info.bit_depth_luma_minus8;
        authored_avcc.bit_depth_chroma_minus8 = sps_info.bit_depth_chroma_minus8;
    }
    let sample_entry_box = build_h264_sample_entry_box_from_avc_config(
        &sps_info,
        authored_avcc,
        sample_entry_type,
        include_colr,
    )?;
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
    let (sps_list, pps_list) =
        collect_h264_parameter_sets_segmented_sync(file, segments, total_size, spec)?;
    let mut state = H264StageState::with_parameter_sets(sps_list, pps_list, true);
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
    let (sps_list, pps_list) = collect_h264_parameter_sets_async(path).await?;
    let mut state = H264StageState::with_parameter_sets(sps_list, pps_list, false);
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
    let (sps_list, pps_list) =
        collect_h264_parameter_sets_segmented_async(file, segments, total_size, spec).await?;
    let mut state = H264StageState::with_parameter_sets(sps_list, pps_list, true);
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

fn collect_h264_parameter_sets_sync(path: &Path) -> Result<H264ParameterSetLists, MuxError> {
    let mut file = File::open(path)?;
    let mut scanner = AnnexBNalScanner::default();
    let mut sps_list = Vec::new();
    let mut pps_list = Vec::new();
    let mut chunk = [0_u8; 16 * 1024];

    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        scanner.push(&chunk[..read], |nal| {
            collect_h264_parameter_set_nal(&mut sps_list, &mut pps_list, nal);
            Ok(())
        })?;
    }
    scanner.finish(|nal| {
        collect_h264_parameter_set_nal(&mut sps_list, &mut pps_list, nal);
        Ok(())
    })?;
    Ok((sps_list, pps_list))
}

fn collect_h264_parameter_sets_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<H264ParameterSetLists, MuxError> {
    let mut scanner = AnnexBNalScanner::default();
    let mut sps_list = Vec::new();
    let mut pps_list = Vec::new();
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
            collect_h264_parameter_set_nal(&mut sps_list, &mut pps_list, nal);
        }
        offset = offset
            .checked_add(u64::try_from(read_len).unwrap())
            .ok_or(MuxError::LayoutOverflow("segmented H.264 scan offset"))?;
    }
    for nal in scanner.finish_collect() {
        collect_h264_parameter_set_nal(&mut sps_list, &mut pps_list, nal);
    }
    Ok((sps_list, pps_list))
}

#[cfg(feature = "async")]
async fn collect_h264_parameter_sets_async(
    path: &Path,
) -> Result<(Vec<Vec<u8>>, Vec<Vec<u8>>), MuxError> {
    let mut file = TokioFile::open(path).await?;
    let mut scanner = AnnexBNalScanner::default();
    let mut sps_list = Vec::new();
    let mut pps_list = Vec::new();
    let mut chunk = [0_u8; 16 * 1024];

    loop {
        let read = file.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        for nal in scanner.collect(&chunk[..read]) {
            collect_h264_parameter_set_nal(&mut sps_list, &mut pps_list, nal);
        }
    }
    for nal in scanner.finish_collect() {
        collect_h264_parameter_set_nal(&mut sps_list, &mut pps_list, nal);
    }
    Ok((sps_list, pps_list))
}

#[cfg(feature = "async")]
async fn collect_h264_parameter_sets_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<(Vec<Vec<u8>>, Vec<Vec<u8>>), MuxError> {
    let mut scanner = AnnexBNalScanner::default();
    let mut sps_list = Vec::new();
    let mut pps_list = Vec::new();
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
            collect_h264_parameter_set_nal(&mut sps_list, &mut pps_list, nal);
        }
        offset = offset
            .checked_add(u64::try_from(read_len).unwrap())
            .ok_or(MuxError::LayoutOverflow("segmented H.264 scan offset"))?;
    }
    for nal in scanner.finish_collect() {
        collect_h264_parameter_set_nal(&mut sps_list, &mut pps_list, nal);
    }
    Ok((sps_list, pps_list))
}

fn collect_h264_parameter_set_nal(
    sps_list: &mut Vec<Vec<u8>>,
    pps_list: &mut Vec<Vec<u8>>,
    nal: AnnexBNal,
) {
    if nal.bytes.is_empty() {
        return;
    }
    match nal.bytes[0] & 0x1F {
        7 => push_unique_nal(sps_list, nal.bytes),
        8 => push_unique_nal(pps_list, nal.bytes),
        _ => {}
    }
}

struct H264StageState {
    segmented_mode: bool,
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
    samples: Vec<StagedSample>,
    sample_first_vcl_nals: Vec<Vec<u8>>,
    segments: Vec<SegmentedMuxSourceSegment>,
    pending_prefix_nals: Vec<AnnexBNal>,
    current_sample_offset: Option<u64>,
    current_sample_first_vcl_nal: Option<Vec<u8>>,
    current_access_unit_info: Option<ParsedH264AccessUnitInfo>,
    current_sample_poc: Option<i32>,
    current_sample_size: u32,
    current_sync: bool,
    current_has_vcl: bool,
    logical_size: u64,
    prev_poc_lsb: u32,
    prev_poc_msb: i32,
}

type H264ParameterSetLists = (Vec<Vec<u8>>, Vec<Vec<u8>>);

impl H264StageState {
    fn with_parameter_sets(
        sps_list: Vec<Vec<u8>>,
        pps_list: Vec<Vec<u8>>,
        segmented_mode: bool,
    ) -> Self {
        Self {
            segmented_mode,
            sps_list,
            pps_list,
            samples: Vec::new(),
            sample_first_vcl_nals: Vec::new(),
            segments: Vec::new(),
            pending_prefix_nals: Vec::new(),
            current_sample_offset: None,
            current_sample_first_vcl_nal: None,
            current_access_unit_info: None,
            current_sample_poc: None,
            current_sample_size: 0,
            current_sync: false,
            current_has_vcl: false,
            logical_size: 0,
            prev_poc_lsb: 0,
            prev_poc_msb: 0,
        }
    }

    fn finish_current_sample(&mut self) {
        if let Some(data_offset) = self.current_sample_offset.take() {
            if self.current_has_vcl {
                self.samples.push(StagedSample {
                    data_offset,
                    data_size: self.current_sample_size,
                    duration: 0,
                    composition_time_offset: 0,
                    is_sync_sample: self.current_sync,
                });
                self.sample_first_vcl_nals
                    .push(self.current_sample_first_vcl_nal.take().unwrap_or_default());
            } else {
                self.current_sample_first_vcl_nal = None;
            }
            self.current_access_unit_info = None;
            self.current_sample_poc = None;
            self.current_sample_size = 0;
            self.current_sync = false;
            self.current_has_vcl = false;
        }
    }

    fn flush_pending_prefix_nals(&mut self) -> Result<(), MuxError> {
        for nal in std::mem::take(&mut self.pending_prefix_nals) {
            if self.segmented_mode {
                self.append_sample_bytes(nal.bytes, false, false)?;
            } else {
                let source_size = u32::try_from(nal.bytes.len())
                    .map_err(|_| MuxError::LayoutOverflow("raw H.264 NAL length"))?;
                self.append_sample_nal(nal.source_offset, source_size, false, false)?;
            }
        }
        Ok(())
    }

    fn trim_leading_non_sync_samples(&mut self, spec: &str) -> Result<(), MuxError> {
        let Some(first_sync_index) = self.samples.iter().position(|sample| sample.is_sync_sample)
        else {
            return Ok(());
        };
        if first_sync_index == 0 {
            return Ok(());
        }

        let trim_offset = self.samples[first_sync_index].data_offset;
        self.samples.drain(0..first_sync_index);
        self.sample_first_vcl_nals.drain(0..first_sync_index);
        for sample in &mut self.samples {
            sample.data_offset = sample
                .data_offset
                .checked_sub(trim_offset)
                .ok_or(MuxError::LayoutOverflow("raw H.264 trimmed sample offset"))?;
        }

        let mut rebased_segments = Vec::with_capacity(self.segments.len());
        for mut segment in self.segments.drain(..) {
            if segment.logical_end() <= trim_offset {
                continue;
            }
            if segment.logical_offset < trim_offset {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "raw H.264 leading trim crossed a transformed sample boundary"
                        .to_string(),
                });
            }
            segment.logical_offset = segment
                .logical_offset
                .checked_sub(trim_offset)
                .ok_or(MuxError::LayoutOverflow("raw H.264 trimmed segment offset"))?;
            rebased_segments.push(segment);
        }
        self.segments = rebased_segments;
        self.logical_size = self
            .logical_size
            .checked_sub(trim_offset)
            .ok_or(MuxError::LayoutOverflow("raw H.264 trimmed logical size"))?;
        Ok(())
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
            if is_vcl {
                let access_unit_info = parse_h264_stage_access_unit_info(state, &nal.bytes, "h264");
                if let Some(access_unit_info) = access_unit_info {
                    if state.current_has_vcl
                        && state
                            .current_access_unit_info
                            .as_ref()
                            .is_some_and(|current| h264_starts_new_access_unit(current, &access_unit_info))
                    {
                        state.finish_current_sample();
                    }
                    if state.current_access_unit_info.is_none() {
                        state.current_access_unit_info = Some(access_unit_info);
                    }
                    if let Some(parsed_poc) = access_unit_info.poc {
                        if state.current_sample_poc.is_none() {
                            state.current_sample_poc = Some(parsed_poc.poc);
                        }
                        state.prev_poc_lsb = parsed_poc.poc_lsb;
                        state.prev_poc_msb = parsed_poc.poc_msb;
                    }
                } else if h264_first_mb_in_slice(&nal.bytes, "h264")? == 0
                    && state.current_has_vcl
                {
                    state.finish_current_sample();
                }
                state.flush_pending_prefix_nals()?;
            }
            if is_vcl && state.current_sample_first_vcl_nal.is_none() {
                state.current_sample_first_vcl_nal = Some(nal.bytes.clone());
            }
            if is_vcl {
                let nal_len = u32::try_from(nal.bytes.len())
                    .map_err(|_| MuxError::LayoutOverflow("H.264 NAL length"))?;
                state.append_sample_nal(nal.source_offset, nal_len, nal_type == 5, true)?;
            } else {
                state.pending_prefix_nals.push(nal);
            }
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
            if is_vcl {
                let access_unit_info = parse_h264_stage_access_unit_info(state, &nal.bytes, "h264");
                if let Some(access_unit_info) = access_unit_info {
                    if state.current_has_vcl
                        && state
                            .current_access_unit_info
                            .as_ref()
                            .is_some_and(|current| h264_starts_new_access_unit(current, &access_unit_info))
                    {
                        state.finish_current_sample();
                    }
                    if state.current_access_unit_info.is_none() {
                        state.current_access_unit_info = Some(access_unit_info);
                    }
                    if let Some(parsed_poc) = access_unit_info.poc {
                        if state.current_sample_poc.is_none() {
                            state.current_sample_poc = Some(parsed_poc.poc);
                        }
                        state.prev_poc_lsb = parsed_poc.poc_lsb;
                        state.prev_poc_msb = parsed_poc.poc_msb;
                    }
                } else if h264_first_mb_in_slice(&nal.bytes, "h264")? == 0
                    && state.current_has_vcl
                {
                    state.finish_current_sample();
                }
                state.flush_pending_prefix_nals()?;
            }
            if is_vcl && state.current_sample_first_vcl_nal.is_none() {
                state.current_sample_first_vcl_nal = Some(nal.bytes.clone());
            }
            if is_vcl {
                state.append_sample_bytes(nal.bytes, nal_type == 5, true)?;
            } else {
                state.pending_prefix_nals.push(nal);
            }
        }
    }
    Ok(())
}

fn parse_h264_stage_access_unit_info(
    state: &H264StageState,
    nal: &[u8],
    spec: &str,
) -> Option<ParsedH264AccessUnitInfo> {
    let sps = state.sps_list.first()?;
    let pps = state.pps_list.first()?;
    let sps_info = parse_h264_sps(sps, spec).ok()?;
    let pps_info = parse_h264_pps(pps, spec).ok()?;
    parse_h264_access_unit_info(
        nal,
        &sps_info,
        &pps_info,
        state.prev_poc_lsb,
        state.prev_poc_msb,
        spec,
    )
}

fn finalize_h264_staged_track(
    path: &Path,
    mut state: H264StageState,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    if state.current_has_vcl {
        state.flush_pending_prefix_nals()?;
    }
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
    let pps_info = parse_h264_pps(&state.pps_list[0], spec)?;
    let (timescale, sample_duration) = if let (Some(time_scale), Some(num_units_in_tick)) = (
        sps_info.timing_time_scale,
        sps_info.timing_num_units_in_tick,
    ) {
        if time_scale == 0 || num_units_in_tick == 0 {
            default_raw_h264_sample_timing()
        } else {
            let first_field_pic_flag = state
                .sample_first_vcl_nals
                .first()
                .and_then(|nal| parse_h264_slice_poc(nal, &sps_info, &pps_info, 0, 0, spec))
                .map(|parsed| parsed.field_pic_flag)
                .unwrap_or(false);
            derive_raw_h264_sample_timing(
                time_scale,
                num_units_in_tick,
                sps_info.vui_pic_struct_present_flag,
                first_field_pic_flag,
                spec,
            )?
        }
    } else {
        default_raw_h264_sample_timing()
    };
    for sample in &mut state.samples {
        sample.duration = sample_duration;
    }
    state.trim_leading_non_sync_samples(spec)?;

    let single_sample = state.samples.len() == 1;
    let sample_timing = (!single_sample).then(|| {
        derive_h264_sample_timing_from_poc(
            &state.sample_first_vcl_nals,
            &state.samples,
            &sps_info,
            &pps_info,
            sample_duration,
            u64::from(sample_duration),
            spec,
        )
    });
    let mut source_edit_media_time = None;
    if let Some(Some(sample_timing)) = sample_timing {
        for (sample, composition_time_offset) in state
            .samples
            .iter_mut()
            .zip(sample_timing.composition_offsets.into_iter())
        {
            sample.composition_time_offset = composition_time_offset;
        }
        source_edit_media_time = sample_timing.source_edit_media_time;
    }

    let authored_sample_entry_box = build_h264_sample_entry_box(
        &sps_info,
        &state.sps_list,
        &state.pps_list,
        sps_info.color_info.is_some(),
    )?;
    let authored_media_duration = authored_h264_media_duration(
        state
            .samples
            .iter()
            .map(|sample| (sample.duration, sample.composition_time_offset)),
    )?;
    let sample_entry_box = if single_sample {
        authored_sample_entry_box
    } else {
        retune_carried_h264_sample_entry_box(
            &authored_sample_entry_box,
            timescale,
            Some(authored_media_duration),
            state
                .samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
            false,
            false,
        )?
    };
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
        source_edit_media_time,
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

    build_h264_sample_entry_box_from_avc_config(
        sps_info,
        avcc,
        FourCc::from_bytes(*b"avc1"),
        include_colr,
    )
}

fn build_h264_sample_entry_box_from_avc_config(
    sps_info: &H264SpsInfo,
    avcc: AVCDecoderConfiguration,
    sample_entry_type: FourCc,
    include_colr: bool,
) -> Result<Vec<u8>, MuxError> {
    let mut avc1 = VisualSampleEntry::default();
    avc1.set_box_type(sample_entry_type);
    avc1.sample_entry = SampleEntry {
        box_type: sample_entry_type,
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
    total_duration_override: Option<u64>,
    samples: I,
    include_pasp: bool,
    include_default_colr: bool,
) -> Result<Vec<u8>, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    if sample_entry_box.len() < 8 || &sample_entry_box[4..8] != b"avc1" {
        return Err(MuxError::UnsupportedTrackImport {
            spec: "h264".to_string(),
            message: "carried H.264 sample entry did not use the `avc1` sample entry type"
                .to_string(),
        });
    }

    let child_boxes = super::super::mp4::visual_sample_entry_immediate_children(sample_entry_box)?;
    let mut avcc_box = None::<Vec<u8>>;
    let mut preserved_pasp_box = None::<Vec<u8>>;
    let mut preserved_colr_box = None::<Vec<u8>>;
    let mut preserved_other_boxes = Vec::new();
    for child_box in child_boxes {
        let child_type = FourCc::from_bytes(
            child_box
                .get(4..8)
                .ok_or(MuxError::LayoutOverflow("carried H.264 child-box type"))?
                .try_into()
                .map_err(|_| MuxError::LayoutOverflow("carried H.264 child-box type"))?,
        );
        match child_type {
            value if value == FourCc::from_bytes(*b"avcC") => avcc_box = Some(child_box),
            value if value == FourCc::from_bytes(*b"pasp") => preserved_pasp_box = Some(child_box),
            value if value == FourCc::from_bytes(*b"colr") => preserved_colr_box = Some(child_box),
            value if value == FourCc::from_bytes(*b"btrt") => {}
            _ => preserved_other_boxes.push(child_box),
        }
    }

    let avcc_box = avcc_box.ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: "h264".to_string(),
        message: "carried H.264 sample entry did not contain an `avcC` decoder configuration box"
            .to_string(),
    })?;
    let btrt_box = super::super::mp4::encode_typed_box(
        &build_btrt_from_sample_sizes_with_total_duration(
            samples,
            timescale,
            total_duration_override,
        )
        .map_err(|error| match error {
            MuxError::LayoutOverflow(_) => error,
            _ => MuxError::LayoutOverflow("carried H.264 bitrate box"),
        })?,
        &[],
    )?;
    let pasp_box = if include_pasp {
        preserved_pasp_box.or(Some(super::super::mp4::encode_typed_box(
            &Pasp {
                h_spacing: 1,
                v_spacing: 1,
            },
            &[],
        )?))
    } else {
        None
    };
    let colr_box = match preserved_colr_box {
        Some(colr_box) => Some(colr_box),
        None if include_default_colr => Some(super::super::mp4::encode_typed_box(
            &Colr {
                colour_type: FourCc::from_bytes(*b"nclx"),
                colour_primaries: 1,
                transfer_characteristics: 1,
                matrix_coefficients: 1,
                full_range_flag: false,
                reserved: 0,
                profile: Vec::new(),
                unknown: Vec::new(),
            },
            &[],
        )?),
        None => None,
    };
    let mut rebuilt_children = Vec::with_capacity(
        2 + usize::from(pasp_box.is_some())
            + usize::from(colr_box.is_some())
            + preserved_other_boxes.len(),
    );
    rebuilt_children.push(avcc_box);
    if let Some(pasp_box) = pasp_box.as_ref() {
        rebuilt_children.push(pasp_box.clone());
    }
    if let Some(colr_box) = colr_box.as_ref() {
        rebuilt_children.push(colr_box.clone());
    }
    rebuilt_children.extend(preserved_other_boxes);
    rebuilt_children.push(btrt_box);
    super::super::mp4::replace_visual_sample_entry_immediate_children(
        sample_entry_box,
        &rebuilt_children,
    )
}

pub(super) fn authored_h264_media_duration<I>(samples: I) -> Result<u64, MuxError>
where
    I: IntoIterator<Item = (u32, i32)>,
{
    let mut decode_time = 0_u64;
    let mut max_presentation_end = 0_u64;
    for (duration, composition_time_offset) in samples {
        let presentation_end = i128::from(decode_time)
            .saturating_add(i128::from(composition_time_offset))
            .saturating_add(i128::from(duration));
        if presentation_end > 0 {
            max_presentation_end = max_presentation_end.max(
                u64::try_from(presentation_end)
                    .map_err(|_| MuxError::LayoutOverflow("carried H.264 media duration"))?,
            );
        }
        decode_time = decode_time
            .checked_add(u64::from(duration))
            .ok_or(MuxError::LayoutOverflow("carried H.264 media duration"))?;
    }
    Ok(max_presentation_end.max(decode_time))
}

const fn h264_profile_supports_config_extensions(profile: u8) -> bool {
    matches!(profile, 100 | 110 | 122 | 144)
}

struct H264SpsInfo {
    seq_parameter_set_id: u32,
    width: u16,
    height: u16,
    profile: u8,
    profile_compatibility: u8,
    level: u8,
    high_profile_fields_enabled: bool,
    separate_colour_plane_flag: bool,
    frame_mbs_only_flag: bool,
    log2_max_frame_num: u8,
    pic_order_cnt_type: u8,
    log2_max_pic_order_cnt_lsb: Option<u8>,
    chroma_format: u8,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
    timing_time_scale: Option<u32>,
    timing_num_units_in_tick: Option<u32>,
    vui_pic_struct_present_flag: bool,
    pixel_aspect_ratio: Option<H264PixelAspectRatio>,
    color_info: Option<H264ColorInfo>,
}

struct H264PpsInfo {
    pic_parameter_set_id: u32,
    seq_parameter_set_id: u32,
    bottom_field_pic_order_in_frame_present_flag: bool,
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
    bool,
    Option<H264PixelAspectRatio>,
    Option<H264ColorInfo>,
);

#[derive(Clone, Copy)]
struct ParsedH264Poc {
    poc_lsb: u32,
    poc_msb: i32,
    poc: i32,
    field_pic_flag: bool,
}

#[derive(Clone, Copy)]
struct ParsedH264AccessUnitInfo {
    first_mb_in_slice: u64,
    pic_parameter_set_id: u32,
    frame_num: u16,
    field_pic_flag: bool,
    bottom_field_flag: bool,
    nal_ref_idc: u8,
    idr_pic_flag: bool,
    idr_pic_id: Option<u32>,
    pic_order_cnt_lsb: Option<u32>,
    delta_pic_order_cnt_bottom: i32,
    poc: Option<ParsedH264Poc>,
}

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
    let seq_parameter_set_id = read_ue(&mut reader, spec)?;

    let mut chroma_format_idc = 1_u8;
    let mut bit_depth_luma_minus8 = 0_u8;
    let mut bit_depth_chroma_minus8 = 0_u8;
    let mut high_profile_fields_enabled = false;
    let mut separate_colour_plane_flag = false;
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
            separate_colour_plane_flag = read_bit(&mut reader, spec)?;
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

    let log2_max_frame_num = u8::try_from(
        read_ue(&mut reader, spec)?
            .checked_add(4)
            .ok_or(MuxError::LayoutOverflow("H.264 frame-num width"))?,
    )
    .map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "H.264 frame-num width does not fit in u8".to_string(),
    })?;
    let pic_order_cnt_type = read_ue(&mut reader, spec)?;
    let mut log2_max_pic_order_cnt_lsb = None;
    if pic_order_cnt_type == 0 {
        log2_max_pic_order_cnt_lsb = Some(
            u8::try_from(
                read_ue(&mut reader, spec)?
                    .checked_add(4)
                    .ok_or(MuxError::LayoutOverflow("H.264 POC-LSB width"))?,
            )
            .map_err(|_| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "H.264 POC-LSB width does not fit in u8".to_string(),
            })?,
        );
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
    let (
        timing_num_units_in_tick,
        timing_time_scale,
        vui_pic_struct_present_flag,
        pixel_aspect_ratio,
        color_info,
    ) = if vui_parameters_present_flag {
        parse_vui_timing(&mut reader, spec)?
    } else {
        (None, None, false, None, None)
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
        seq_parameter_set_id,
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
        separate_colour_plane_flag,
        frame_mbs_only_flag,
        log2_max_frame_num,
        pic_order_cnt_type: u8::try_from(pic_order_cnt_type).map_err(|_| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "H.264 pic_order_cnt_type does not fit in u8".to_string(),
            }
        })?,
        log2_max_pic_order_cnt_lsb,
        chroma_format: chroma_format_idc,
        bit_depth_luma_minus8,
        bit_depth_chroma_minus8,
        timing_time_scale,
        timing_num_units_in_tick,
        vui_pic_struct_present_flag,
        pixel_aspect_ratio,
        color_info,
    })
}

fn parse_h264_pps(nal: &[u8], spec: &str) -> Result<H264PpsInfo, MuxError> {
    if nal.len() < 2 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.264 PPS NAL is too short".to_string(),
        });
    }
    let rbsp = nal_to_rbsp(&nal[1..]);
    let mut reader = BitReader::new(Cursor::new(rbsp));
    let pic_parameter_set_id = read_ue(&mut reader, spec)?;
    let seq_parameter_set_id = read_ue(&mut reader, spec)?;
    let _entropy_coding_mode_flag = read_bit(&mut reader, spec)?;
    let bottom_field_pic_order_in_frame_present_flag = read_bit(&mut reader, spec)?;
    Ok(H264PpsInfo {
        pic_parameter_set_id,
        seq_parameter_set_id,
        bottom_field_pic_order_in_frame_present_flag,
    })
}

fn derive_raw_h264_sample_timing(
    time_scale: u32,
    num_units_in_tick: u32,
    vui_pic_struct_present_flag: bool,
    field_pic_flag: bool,
    spec: &str,
) -> Result<(u32, u32), MuxError> {
    let delta_tfi_divisor_idx = if !vui_pic_struct_present_flag {
        1_u32 + u32::from(!field_pic_flag)
    } else {
        2
    };
    let doubled_time_scale = time_scale.checked_mul(2);
    let (timescale, sample_duration) = if let Some(doubled_time_scale) = doubled_time_scale {
        let doubled_num_units_in_tick =
            num_units_in_tick
                .checked_mul(2)
                .ok_or(MuxError::LayoutOverflow(
                    "raw H.264 sample duration from SPS timing",
                ))?;
        (
            doubled_time_scale,
            doubled_num_units_in_tick
                .checked_mul(delta_tfi_divisor_idx)
                .ok_or(MuxError::LayoutOverflow(
                    "raw H.264 sample duration from SPS timing",
                ))?,
        )
    } else {
        (
            time_scale,
            num_units_in_tick.checked_mul(delta_tfi_divisor_idx).ok_or(
                MuxError::LayoutOverflow("raw H.264 sample duration from SPS timing"),
            )?,
        )
    };
    if timescale == 0 || sample_duration == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.264 SPS timing info produced an invalid zero cadence".to_string(),
        });
    }
    Ok((timescale, sample_duration))
}

fn default_raw_h264_sample_timing() -> (u32, u32) {
    (DEFAULT_RAW_H264_TIMESCALE, DEFAULT_RAW_H264_SAMPLE_DURATION)
}

struct H264DerivedSampleTiming {
    composition_offsets: Vec<i32>,
    source_edit_media_time: Option<u64>,
}

fn derive_h264_sample_timing_from_poc(
    first_vcl_nals: &[Vec<u8>],
    samples: &[StagedSample],
    sps_info: &H264SpsInfo,
    pps_info: &H264PpsInfo,
    sample_duration: u32,
    initial_presentation_time: u64,
    spec: &str,
) -> Option<H264DerivedSampleTiming> {
    if first_vcl_nals.len() != samples.len() {
        return None;
    }
    let mut pocs = Vec::with_capacity(first_vcl_nals.len());
    let mut prev_poc_lsb = 0_u32;
    let mut prev_poc_msb = 0_i32;

    for nal in first_vcl_nals {
        let parsed =
            parse_h264_slice_poc(nal, sps_info, pps_info, prev_poc_lsb, prev_poc_msb, spec)?;
        pocs.push(parsed.poc);
        prev_poc_lsb = parsed.poc_lsb;
        prev_poc_msb = parsed.poc_msb;
    }

    let poc_step = derive_h264_poc_step(&pocs).unwrap_or(1);
    let mut composition_offsets = Vec::with_capacity(samples.len());
    let initial_presentation_time = i64::try_from(initial_presentation_time).ok()?;
    let sample_duration = i64::from(sample_duration);
    let mut gop_start = 0usize;
    while gop_start < samples.len() {
        let gop_end = samples
            .iter()
            .enumerate()
            .skip(gop_start + 1)
            .find_map(|(index, sample)| sample.is_sync_sample.then_some(index))
            .unwrap_or(samples.len());
        let gop_base_poc = pocs[gop_start];
        let gop_display_ranks =
            derive_h264_gop_display_ranks(&pocs[gop_start..gop_end], gop_base_poc, poc_step)?;
        let gop_decode_start = i64::try_from(gop_start)
            .ok()?
            .checked_mul(sample_duration)?;
        for (local_index, display_rank) in gop_display_ranks.into_iter().enumerate() {
            let decode_time = gop_decode_start.checked_add(
                i64::try_from(local_index)
                    .ok()?
                    .checked_mul(sample_duration)?,
            )?;
            let presentation_time = gop_decode_start
                .checked_add(initial_presentation_time)?
                .checked_add(i64::from(display_rank).checked_mul(sample_duration)?)?;
            composition_offsets
                .push(i32::try_from(presentation_time.checked_sub(decode_time)?).ok()?);
        }
        gop_start = gop_end;
    }
    Some(H264DerivedSampleTiming {
        composition_offsets,
        source_edit_media_time: (initial_presentation_time > 0)
            .then_some(u64::try_from(initial_presentation_time).ok()?)
            .filter(|value| *value > 0),
    })
}

fn derive_h264_poc_step(pocs: &[i32]) -> Option<i32> {
    pocs.windows(2)
        .filter_map(|window| {
            let diff = (window[1] - window[0]).abs();
            (diff > 0).then_some(diff)
        })
        .min()
}

fn derive_h264_gop_display_ranks(
    gop_pocs: &[i32],
    gop_base_poc: i32,
    poc_step: i32,
) -> Option<Vec<i32>> {
    if gop_pocs.is_empty() || poc_step <= 0 {
        return Some(Vec::new());
    }
    let relative_pocs = gop_pocs
        .iter()
        .map(|poc| poc.checked_sub(gop_base_poc))
        .collect::<Option<Vec<_>>>()?;
    if relative_pocs
        .iter()
        .all(|relative_poc| *relative_poc >= 0 && *relative_poc % poc_step == 0)
    {
        return relative_pocs
            .into_iter()
            .map(|relative_poc| relative_poc.checked_div(poc_step))
            .collect();
    }

    let mut order = relative_pocs
        .iter()
        .copied()
        .enumerate()
        .collect::<Vec<_>>();
    order.sort_by_key(|(_, poc)| *poc);
    let mut display_ranks = vec![0_i32; gop_pocs.len()];
    for (rank, (index, _)) in order.into_iter().enumerate() {
        display_ranks[index] = i32::try_from(rank).ok()?;
    }
    Some(display_ranks)
}

fn parse_h264_slice_poc(
    nal: &[u8],
    sps_info: &H264SpsInfo,
    pps_info: &H264PpsInfo,
    prev_poc_lsb: u32,
    prev_poc_msb: i32,
    spec: &str,
) -> Option<ParsedH264Poc> {
    if nal.len() < 2
        || pps_info.seq_parameter_set_id != sps_info.seq_parameter_set_id
        || sps_info.pic_order_cnt_type != 0
    {
        return None;
    }
    let nal_type = nal[0] & 0x1F;
    if !is_h264_vcl_nal_type(nal_type) {
        return None;
    }

    let rbsp = nal_to_rbsp(&nal[1..]);
    let mut reader = BitReader::new(Cursor::new(rbsp));
    let _first_mb_in_slice = read_ue(&mut reader, spec).ok()?;
    let _slice_type = read_ue(&mut reader, spec).ok()?;
    let pic_parameter_set_id = read_ue(&mut reader, spec).ok()?;
    if pic_parameter_set_id != pps_info.pic_parameter_set_id {
        return None;
    }
    let _frame_num =
        read_bits_u16(&mut reader, usize::from(sps_info.log2_max_frame_num), spec).ok()?;
    if sps_info.separate_colour_plane_flag {
        let _colour_plane_id = read_bits_u8(&mut reader, 2, spec).ok()?;
    }
    let mut field_pic_flag = false;
    let mut bottom_field_flag = false;
    if !sps_info.frame_mbs_only_flag {
        field_pic_flag = read_bit(&mut reader, spec).ok()?;
        if field_pic_flag {
            bottom_field_flag = read_bit(&mut reader, spec).ok()?;
        }
    }
    if nal_type == 5 {
        let _idr_pic_id = read_ue(&mut reader, spec).ok()?;
    }
    let poc_lsb = read_bits_u32(
        &mut reader,
        usize::from(sps_info.log2_max_pic_order_cnt_lsb?),
        spec,
    )
    .ok()?;
    let delta_pic_order_cnt_bottom =
        if pps_info.bottom_field_pic_order_in_frame_present_flag && !field_pic_flag {
            read_se(&mut reader, spec).ok()?
        } else {
            0
        };

    let max_poc_lsb = 1_u32.checked_shl(u32::from(sps_info.log2_max_pic_order_cnt_lsb?))?;
    let poc_msb = if nal_type == 5 {
        0
    } else if poc_lsb < prev_poc_lsb && prev_poc_lsb - poc_lsb >= max_poc_lsb / 2 {
        prev_poc_msb.checked_add(i32::try_from(max_poc_lsb).ok()?)?
    } else if poc_lsb > prev_poc_lsb && poc_lsb - prev_poc_lsb > max_poc_lsb / 2 {
        prev_poc_msb.checked_sub(i32::try_from(max_poc_lsb).ok()?)?
    } else {
        prev_poc_msb
    };
    let top_field_order_cnt = poc_msb.checked_add(i32::try_from(poc_lsb).ok()?)?;
    let bottom_field_order_cnt = top_field_order_cnt.checked_add(delta_pic_order_cnt_bottom)?;
    let poc = if field_pic_flag {
        if bottom_field_flag {
            bottom_field_order_cnt
        } else {
            top_field_order_cnt
        }
    } else {
        top_field_order_cnt.min(bottom_field_order_cnt)
    };

    Some(ParsedH264Poc {
        poc_lsb,
        poc_msb,
        poc,
        field_pic_flag,
    })
}

fn parse_h264_access_unit_info(
    nal: &[u8],
    sps_info: &H264SpsInfo,
    pps_info: &H264PpsInfo,
    prev_poc_lsb: u32,
    prev_poc_msb: i32,
    spec: &str,
) -> Option<ParsedH264AccessUnitInfo> {
    if nal.len() < 2 || pps_info.seq_parameter_set_id != sps_info.seq_parameter_set_id {
        return None;
    }
    let nal_type = nal[0] & 0x1F;
    if !is_h264_vcl_nal_type(nal_type) {
        return None;
    }

    let rbsp = nal_to_rbsp(&nal[1..]);
    let mut reader = BitReader::new(Cursor::new(rbsp));
    let first_mb_in_slice = read_ue(&mut reader, spec).ok()?;
    let _slice_type = read_ue(&mut reader, spec).ok()?;
    let pic_parameter_set_id = read_ue(&mut reader, spec).ok()?;
    if pic_parameter_set_id != pps_info.pic_parameter_set_id {
        return None;
    }
    let frame_num =
        read_bits_u16(&mut reader, usize::from(sps_info.log2_max_frame_num), spec).ok()?;
    if sps_info.separate_colour_plane_flag {
        let _colour_plane_id = read_bits_u8(&mut reader, 2, spec).ok()?;
    }
    let nal_ref_idc = (nal[0] >> 5) & 0x3;
    let mut field_pic_flag = false;
    let mut bottom_field_flag = false;
    if !sps_info.frame_mbs_only_flag {
        field_pic_flag = read_bit(&mut reader, spec).ok()?;
        if field_pic_flag {
            bottom_field_flag = read_bit(&mut reader, spec).ok()?;
        }
    }
    let idr_pic_flag = nal_type == 5;
    let idr_pic_id = if idr_pic_flag {
        Some(read_ue(&mut reader, spec).ok()?)
    } else {
        None
    };

    let mut pic_order_cnt_lsb = None;
    let mut delta_pic_order_cnt_bottom = 0;
    let mut poc = None;
    if sps_info.pic_order_cnt_type == 0 {
        let parsed_poc = parse_h264_slice_poc(
            nal,
            sps_info,
            pps_info,
            prev_poc_lsb,
            prev_poc_msb,
            spec,
        )?;
        pic_order_cnt_lsb = Some(parsed_poc.poc_lsb);
        if pps_info.bottom_field_pic_order_in_frame_present_flag && !field_pic_flag {
            let rbsp = nal_to_rbsp(&nal[1..]);
            let mut reader = BitReader::new(Cursor::new(rbsp));
            let _first_mb_in_slice = read_ue(&mut reader, spec).ok()?;
            let _slice_type = read_ue(&mut reader, spec).ok()?;
            let _pic_parameter_set_id = read_ue(&mut reader, spec).ok()?;
            let _frame_num =
                read_bits_u16(&mut reader, usize::from(sps_info.log2_max_frame_num), spec).ok()?;
            if sps_info.separate_colour_plane_flag {
                let _colour_plane_id = read_bits_u8(&mut reader, 2, spec).ok()?;
            }
            if !sps_info.frame_mbs_only_flag {
                let field_pic_flag_value = read_bit(&mut reader, spec).ok()?;
                if field_pic_flag_value {
                    let _bottom_field_flag = read_bit(&mut reader, spec).ok()?;
                }
            }
            if idr_pic_flag {
                let _idr_pic_id = read_ue(&mut reader, spec).ok()?;
            }
            let _pic_order_cnt_lsb = read_bits_u32(
                &mut reader,
                usize::from(sps_info.log2_max_pic_order_cnt_lsb?),
                spec,
            )
            .ok()?;
            delta_pic_order_cnt_bottom = read_se(&mut reader, spec).ok()?;
        }
        poc = Some(parsed_poc);
    }

    Some(ParsedH264AccessUnitInfo {
        first_mb_in_slice: u64::from(first_mb_in_slice),
        pic_parameter_set_id,
        frame_num,
        field_pic_flag,
        bottom_field_flag,
        nal_ref_idc,
        idr_pic_flag,
        idr_pic_id,
        pic_order_cnt_lsb,
        delta_pic_order_cnt_bottom,
        poc,
    })
}

fn h264_starts_new_access_unit(
    current: &ParsedH264AccessUnitInfo,
    next: &ParsedH264AccessUnitInfo,
) -> bool {
    if next.first_mb_in_slice != 0 {
        return false;
    }
    current.frame_num != next.frame_num
        || current.pic_parameter_set_id != next.pic_parameter_set_id
        || current.field_pic_flag != next.field_pic_flag
        || (current.field_pic_flag
            && next.field_pic_flag
            && current.bottom_field_flag != next.bottom_field_flag)
        || ((current.nal_ref_idc == 0) != (next.nal_ref_idc == 0))
        || current.idr_pic_flag != next.idr_pic_flag
        || (current.idr_pic_flag && current.idr_pic_id != next.idr_pic_id)
        || (!current.field_pic_flag
            && !next.field_pic_flag
            && (current.pic_order_cnt_lsb != next.pic_order_cnt_lsb
                || current.delta_pic_order_cnt_bottom != next.delta_pic_order_cnt_bottom))
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
    let timing_info_present_flag = read_bit(reader, spec)?;
    let mut num_units_in_tick = None;
    let mut time_scale = None;
    if timing_info_present_flag {
        num_units_in_tick = Some(read_bits_u32(reader, 32, spec)?);
        time_scale = Some(read_bits_u32(reader, 32, spec)?);
        let _fixed_frame_rate_flag = read_bit(reader, spec)?;
    }
    let nal_hrd_parameters_present_flag = read_bit(reader, spec)?;
    if nal_hrd_parameters_present_flag {
        skip_hrd_parameters(reader, spec)?;
    }
    let vcl_hrd_parameters_present_flag = read_bit(reader, spec)?;
    if vcl_hrd_parameters_present_flag {
        skip_hrd_parameters(reader, spec)?;
    }
    if nal_hrd_parameters_present_flag || vcl_hrd_parameters_present_flag {
        let _low_delay_hrd_flag = read_bit(reader, spec)?;
    }
    let pic_struct_present_flag = read_bit(reader, spec)?;
    if read_bit(reader, spec)? {
        skip_bitstream_restrictions(reader, spec)?;
    }
    Ok((
        num_units_in_tick,
        time_scale,
        pic_struct_present_flag,
        pixel_aspect_ratio,
        color_info,
    ))
}

fn skip_hrd_parameters<R>(reader: &mut BitReader<R>, spec: &str) -> Result<(), MuxError>
where
    R: Read,
{
    let cpb_cnt_minus1 = read_ue(reader, spec)?;
    let _bit_rate_scale = read_bits_u8(reader, 4, spec)?;
    let _cpb_size_scale = read_bits_u8(reader, 4, spec)?;
    for _ in 0..=cpb_cnt_minus1 {
        let _bit_rate_value_minus1 = read_ue(reader, spec)?;
        let _cpb_size_value_minus1 = read_ue(reader, spec)?;
        let _cbr_flag = read_bit(reader, spec)?;
    }
    let _initial_cpb_removal_delay_length_minus1 = read_bits_u8(reader, 5, spec)?;
    let _cpb_removal_delay_length_minus1 = read_bits_u8(reader, 5, spec)?;
    let _dpb_output_delay_length_minus1 = read_bits_u8(reader, 5, spec)?;
    let _time_offset_length = read_bits_u8(reader, 5, spec)?;
    Ok(())
}

fn skip_bitstream_restrictions<R>(reader: &mut BitReader<R>, spec: &str) -> Result<(), MuxError>
where
    R: Read,
{
    let _motion_vectors_over_pic_boundaries_flag = read_bit(reader, spec)?;
    let _max_bytes_per_pic_denom = read_ue(reader, spec)?;
    let _max_bits_per_mb_denom = read_ue(reader, spec)?;
    let _log2_max_mv_length_horizontal = read_ue(reader, spec)?;
    let _log2_max_mv_length_vertical = read_ue(reader, spec)?;
    let _max_num_reorder_frames = read_ue(reader, spec)?;
    let _max_dec_frame_buffering = read_ue(reader, spec)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::mp4::visual_sample_entry_immediate_children;

    fn decode_hex(bytes: &str) -> Vec<u8> {
        assert_eq!(bytes.len() % 2, 0);
        bytes
            .as_bytes()
            .chunks_exact(2)
            .map(|chunk| {
                let text = std::str::from_utf8(chunk).unwrap();
                u8::from_str_radix(text, 16).unwrap()
            })
            .collect()
    }

    #[test]
    fn build_h264_sample_entry_from_avc_config_includes_default_colr_when_requested() {
        let avcc = AVCDecoderConfiguration {
            configuration_version: 1,
            profile: 100,
            profile_compatibility: 0,
            level: 13,
            length_size_minus_one: 3,
            num_of_sequence_parameter_sets: 1,
            sequence_parameter_sets: vec![AVCParameterSet {
                length: 24,
                nal_unit: decode_hex("6764000DAC34E505067E7840000019000005DAA3C50A4580"),
            }],
            num_of_picture_parameter_sets: 1,
            picture_parameter_sets: vec![AVCParameterSet {
                length: 5,
                nal_unit: decode_hex("68EEB2C8B0"),
            }],
            high_profile_fields_enabled: true,
            chroma_format: 1,
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            num_of_sequence_parameter_set_ext: 0,
            sequence_parameter_sets_ext: Vec::new(),
        };

        let (sample_entry_box, _, _) =
            build_h264_sample_entry_from_avc_config_with_options(&avcc, "test", true).unwrap();
        let child_boxes = visual_sample_entry_immediate_children(&sample_entry_box).unwrap();
        assert!(
            child_boxes
                .iter()
                .any(|child_box| child_box.get(4..8) == Some(&b"colr"[..]))
        );
    }

    #[test]
    fn default_raw_h264_sample_timing_uses_25_fps_cadence() {
        assert_eq!(default_raw_h264_sample_timing(), (25_000, 1_000));
    }
}
