use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
#[cfg(feature = "async")]
use std::pin::Pin;
#[cfg(feature = "async")]
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWriteExt, BufWriter, ReadBuf,
};

use crate::FourCc;
#[cfg(feature = "async")]
use crate::async_io::AsyncReadSeek;
use crate::boxes::iso14496_12::{
    AudioSampleEntry, Btrt, Co64, Ctts, Elst, GenericMediaSampleEntry, Hdlr, Mdhd, SampleEntry,
    Stco, Stsc, Stss, Stsz, Stts, TFHD_BASE_DATA_OFFSET_PRESENT, TFHD_DEFAULT_BASE_IS_MOOF,
    TFHD_DEFAULT_SAMPLE_DURATION_PRESENT, TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT,
    TFHD_DEFAULT_SAMPLE_SIZE_PRESENT, TRUN_DATA_OFFSET_PRESENT, TRUN_FIRST_SAMPLE_FLAGS_PRESENT,
    TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT,
    TRUN_SAMPLE_FLAGS_PRESENT, TRUN_SAMPLE_SIZE_PRESENT, Tfhd, Tkhd, Trex, Trun, VisualSampleEntry,
};
use crate::codec::{CodecBox, ImmutableBox};
use crate::extract::{
    ExtractedBox, extract_box, extract_box_as, extract_box_bytes, extract_box_with_payload,
};
#[cfg(feature = "async")]
use crate::extract::{
    extract_box_as_async, extract_box_async, extract_box_bytes_async,
    extract_box_with_payload_async,
};
use crate::header::BoxInfo as HeaderInfo;
use crate::walk::BoxPath;

use super::demux::{
    DetectedContainerPathKind, DetectedPathTrackKind, detect_caf_track_kind_sync,
    detect_id3_wrapped_audio_from_prefix, detect_ogg_track_kind_sync,
    detect_path_track_kind_from_prefix, id3v2_size_from_prefix, scan_ac3_file_sync,
    scan_ac4_file_sync, scan_adts_file_sync, scan_amr_file_sync, scan_amr_wb_file_sync,
    scan_av1_file_sync, scan_avi_source_sync, scan_caf_alac_file_sync, scan_dts_file_sync,
    scan_eac3_file_sync, scan_flac_file_sync, scan_h263_file_sync, scan_iamf_file_sync,
    scan_jpeg_file_sync, scan_latm_file_sync, scan_mhas_file_sync, scan_mp3_file_sync,
    scan_mp4v_file_sync, scan_ogg_flac_file_sync, scan_ogg_opus_file_sync,
    scan_ogg_speex_file_sync, scan_ogg_theora_file_sync, scan_ogg_vorbis_file_sync,
    scan_pcm_file_sync, scan_png_file_sync, scan_program_stream_sync, scan_qcp_file_sync,
    scan_transport_stream_sync, scan_truehd_file_sync, scan_vobsub_source_sync, scan_vp8_file_sync,
    scan_vp9_file_sync, scan_vp10_file_sync, stage_annex_b_h264_sync, stage_annex_b_h265_sync,
    stage_annex_b_vvc_sync,
};
#[cfg(feature = "async")]
use super::demux::{
    detect_caf_track_kind_async, detect_ogg_track_kind_async, scan_ac3_file_async,
    scan_ac4_file_async, scan_adts_file_async, scan_amr_file_async, scan_amr_wb_file_async,
    scan_av1_file_async, scan_avi_source_async, scan_caf_alac_file_async, scan_dts_file_async,
    scan_eac3_file_async, scan_flac_file_async, scan_h263_file_async, scan_iamf_file_async,
    scan_jpeg_file_async, scan_latm_file_async, scan_mhas_file_async, scan_mp3_file_async,
    scan_mp4v_file_async, scan_ogg_flac_file_async, scan_ogg_opus_file_async,
    scan_ogg_speex_file_async, scan_ogg_theora_file_async, scan_ogg_vorbis_file_async,
    scan_pcm_file_async, scan_png_file_async, scan_program_stream_async, scan_qcp_file_async,
    scan_transport_stream_async, scan_truehd_file_async, scan_vobsub_source_async,
    scan_vp8_file_async, scan_vp9_file_async, scan_vp10_file_async, stage_annex_b_h264_async,
    stage_annex_b_h265_async, stage_annex_b_vvc_async,
};
use super::mp4::write_fragmented_mp4_mux;
#[cfg(feature = "async")]
use super::mp4::write_fragmented_mp4_mux_async;
#[cfg(feature = "async")]
use super::write_mp4_mux_async;
use super::{
    FlatTimingOverride, MuxDestinationMode, MuxDurationBoundaryKind, MuxError, MuxFileConfig,
    MuxInterleavePolicy, MuxMp4TrackSelector, MuxOutputLayout, MuxRawCodec, MuxRequest,
    MuxStagedMediaItem, MuxTrackConfig, MuxTrackKind, MuxTrackSpec, StscRunEncodingMode,
    SyncSampleTableMode, TrackCoordinationDirective, build_capped_duration_chunk_sample_counts,
    build_duration_chunk_sample_counts, build_duration_chunk_sample_counts_with_start_time,
    build_sync_aligned_segment_chunk_sample_counts, plan_staged_media_items_with_coordination,
    rebalance_small_multi_audio_chunk_sample_counts, write_mp4_mux,
};

const MOOV: FourCc = FourCc::from_bytes(*b"moov");
const TRAK: FourCc = FourCc::from_bytes(*b"trak");
const TKHD: FourCc = FourCc::from_bytes(*b"tkhd");
const EDTS: FourCc = FourCc::from_bytes(*b"edts");
const ELST: FourCc = FourCc::from_bytes(*b"elst");
const MDIA: FourCc = FourCc::from_bytes(*b"mdia");
const MDHD: FourCc = FourCc::from_bytes(*b"mdhd");
const HDLR: FourCc = FourCc::from_bytes(*b"hdlr");
const MINF: FourCc = FourCc::from_bytes(*b"minf");
const STBL: FourCc = FourCc::from_bytes(*b"stbl");
const STSD: FourCc = FourCc::from_bytes(*b"stsd");
const STTS: FourCc = FourCc::from_bytes(*b"stts");
const CTTS: FourCc = FourCc::from_bytes(*b"ctts");
const STSC: FourCc = FourCc::from_bytes(*b"stsc");
const STSZ: FourCc = FourCc::from_bytes(*b"stsz");
const STCO: FourCc = FourCc::from_bytes(*b"stco");
const CO64: FourCc = FourCc::from_bytes(*b"co64");
const STSS: FourCc = FourCc::from_bytes(*b"stss");
const MVEX: FourCc = FourCc::from_bytes(*b"mvex");
const TREX: FourCc = FourCc::from_bytes(*b"trex");
const MOOF: FourCc = FourCc::from_bytes(*b"moof");
const TRAF: FourCc = FourCc::from_bytes(*b"traf");
const TFHD: FourCc = FourCc::from_bytes(*b"tfhd");
const TRUN: FourCc = FourCc::from_bytes(*b"trun");
const VIDE: FourCc = FourCc::from_bytes(*b"vide");
const SOUN: FourCc = FourCc::from_bytes(*b"soun");
const TEXT: FourCc = FourCc::from_bytes(*b"text");
const SUBT: FourCc = FourCc::from_bytes(*b"subt");
const SUBP: FourCc = FourCc::from_bytes(*b"subp");
const ENCV: FourCc = FourCc::from_bytes(*b"encv");
const ENCA: FourCc = FourCc::from_bytes(*b"enca");
const NON_KEY_SAMPLE_FLAGS: u32 = 0x0001_0000;
const AUTO_FLAT_INTERLEAVE_MILLISECONDS: u64 = 500;
/// Opens the requested track specs, validates the narrowed mux request shape, and writes one newly
/// created output MP4 file to `output_path`.
///
/// This task-level helper is the sync programmatic companion to the explicit `--out PATH` mux CLI
/// surface. It always treats `output_path` as a newly created destination and rejects unsupported
/// multi-video or duration-mode combinations explicitly.
pub fn mux_to_path<P>(request: &MuxRequest, output_path: P) -> Result<(), MuxError>
where
    P: AsRef<Path>,
{
    let request = request
        .clone()
        .with_destination_mode(MuxDestinationMode::CreateNew);
    mux_to_path_inner(&request, output_path.as_ref())
}

/// Opens the requested track specs, preserves an existing MP4 destination when present, and
/// otherwise creates one new output MP4 at `destination_path`.
///
/// When `destination_path` already exists and probes as MP4, this helper preserves that file's
/// current tracks and imports the requested tracks into it. When the path does not exist or does
/// not probe as MP4, the same path is treated as the newly created destination file.
pub fn mux_into_path<P>(request: &MuxRequest, destination_path: P) -> Result<(), MuxError>
where
    P: AsRef<Path>,
{
    let request = request
        .clone()
        .with_destination_mode(MuxDestinationMode::UpdateOrCreateDestination);
    mux_into_path_inner(&request, destination_path.as_ref())
}

fn mux_to_path_inner(request: &MuxRequest, output_path: &Path) -> Result<(), MuxError> {
    let prepared = prepare_request_sync(request, output_path)?;
    let mut sources = prepared
        .source_specs
        .iter()
        .map(SyncMuxSource::open)
        .collect::<Result<Vec<_>, _>>()?;
    let mut writer = File::create(output_path)?;
    match prepared.output_layout {
        MuxOutputLayout::Flat => write_mp4_mux(
            &mut sources,
            &mut writer,
            &prepared.file_config,
            &prepared.track_configs,
            &prepared.plan,
        )?,
        MuxOutputLayout::Fragmented => write_fragmented_mp4_mux(
            &mut sources,
            &mut writer,
            &prepared.file_config,
            &prepared.track_configs,
            prepared.fragmented_single_sidx_reference,
            &prepared.plan,
        )?,
    }
    writer.flush()?;
    Ok(())
}

fn mux_into_path_inner(request: &MuxRequest, destination_path: &Path) -> Result<(), MuxError> {
    if should_preserve_destination_mp4(destination_path) {
        let amended_request = build_destination_preserving_request(request, destination_path)?;
        let temp_path = create_update_temp_path(destination_path, request.destination_mode())?;
        let write_result = mux_to_path_inner(&amended_request, &temp_path);
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temp_path);
            return Err(error);
        }
        replace_output_path(&temp_path, destination_path)?;
        return Ok(());
    }
    let create_new_request = request
        .clone()
        .with_destination_mode(MuxDestinationMode::CreateNew);
    mux_to_path_inner(&create_new_request, destination_path)
}

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(all(feature = "mux", feature = "async"))))]
/// Async companion to [`mux_to_path`] that keeps the file-backed mux path on the crate's additive
/// Tokio-based async surface.
///
/// The request validation and supported public behavior match the sync helper exactly; only the
/// file-backed I/O path differs.
pub async fn mux_to_path_async<P>(request: &MuxRequest, output_path: P) -> Result<(), MuxError>
where
    P: AsRef<Path>,
{
    let request = request
        .clone()
        .with_destination_mode(MuxDestinationMode::CreateNew);
    mux_to_path_async_inner(&request, output_path.as_ref()).await
}

/// Async companion to [`mux_into_path`] on the file-backed Tokio surface.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(all(feature = "mux", feature = "async"))))]
pub async fn mux_into_path_async<P>(
    request: &MuxRequest,
    destination_path: P,
) -> Result<(), MuxError>
where
    P: AsRef<Path>,
{
    let request = request
        .clone()
        .with_destination_mode(MuxDestinationMode::UpdateOrCreateDestination);
    mux_into_path_async_inner(&request, destination_path.as_ref()).await
}

#[cfg(feature = "async")]
async fn mux_to_path_async_inner(request: &MuxRequest, output_path: &Path) -> Result<(), MuxError> {
    let prepared = prepare_request_async(request, output_path).await?;
    let mut sources = Vec::with_capacity(prepared.source_specs.len());
    for spec in &prepared.source_specs {
        sources.push(AsyncMuxSource::open(spec).await?);
    }
    let output = TokioFile::create(output_path).await?;
    let mut writer = BufWriter::new(output);
    match prepared.output_layout {
        MuxOutputLayout::Flat => {
            write_mp4_mux_async(
                &mut sources,
                &mut writer,
                &prepared.file_config,
                &prepared.track_configs,
                &prepared.plan,
            )
            .await?
        }
        MuxOutputLayout::Fragmented => {
            write_fragmented_mp4_mux_async(
                &mut sources,
                &mut writer,
                &prepared.file_config,
                &prepared.track_configs,
                prepared.fragmented_single_sidx_reference,
                &prepared.plan,
            )
            .await?
        }
    }
    writer.flush().await?;
    Ok(())
}

#[cfg(feature = "async")]
async fn mux_into_path_async_inner(
    request: &MuxRequest,
    destination_path: &Path,
) -> Result<(), MuxError> {
    if should_preserve_destination_mp4(destination_path) {
        let amended_request = build_destination_preserving_request(request, destination_path)?;
        let temp_path = create_update_temp_path(destination_path, request.destination_mode())?;
        let write_result = mux_to_path_async_inner(&amended_request, &temp_path).await;
        if let Err(error) = write_result {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(error);
        }
        replace_output_path_async(&temp_path, destination_path).await?;
        return Ok(());
    }
    let create_new_request = request
        .clone()
        .with_destination_mode(MuxDestinationMode::CreateNew);
    mux_to_path_async_inner(&create_new_request, destination_path).await
}

struct PreparedMuxRequest {
    output_layout: MuxOutputLayout,
    file_config: MuxFileConfig,
    track_configs: Vec<MuxTrackConfig>,
    fragmented_single_sidx_reference: bool,
    plan: super::MuxPlan,
    source_specs: Vec<SourceSpec>,
}

struct FragmentRunContext<'a> {
    path: &'a Path,
    source_index: usize,
    track_id: u32,
    moof_offset: u64,
    trex: Option<&'a Trex>,
}

#[derive(Clone)]
enum SourceSpec {
    File(PathBuf),
    Segmented(SegmentedMuxSourceSpec),
}

#[derive(Clone)]
pub(in crate::mux) struct SegmentedMuxSourceSpec {
    pub(in crate::mux) path: PathBuf,
    pub(in crate::mux) segments: Vec<SegmentedMuxSourceSegment>,
    pub(in crate::mux) total_size: u64,
}

#[derive(Clone)]
pub(in crate::mux) struct SegmentedMuxSourceSegment {
    pub(in crate::mux) logical_offset: u64,
    pub(in crate::mux) data: SegmentedMuxSourceSegmentData,
}

#[derive(Clone)]
pub(in crate::mux) enum SegmentedMuxSourceSegmentData {
    Prefix([u8; 4]),
    Bytes(Vec<u8>),
    FileRange { source_offset: u64, size: u32 },
}

impl SegmentedMuxSourceSegment {
    fn logical_size(&self) -> u64 {
        match &self.data {
            SegmentedMuxSourceSegmentData::Prefix(_) => 4,
            SegmentedMuxSourceSegmentData::Bytes(bytes) => u64::try_from(bytes.len()).unwrap(),
            SegmentedMuxSourceSegmentData::FileRange { size, .. } => u64::from(*size),
        }
    }

    fn logical_end(&self) -> u64 {
        self.logical_offset + self.logical_size()
    }
}

fn find_segmented_source_segment_index(
    segments: &[SegmentedMuxSourceSegment],
    position: u64,
) -> Option<usize> {
    segments
        .binary_search_by(|segment| {
            if segment.logical_end() <= position {
                std::cmp::Ordering::Less
            } else if segment.logical_offset > position {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .ok()
}

fn seek_mux_source_position(position: u64, end: u64, target: SeekFrom) -> io::Result<u64> {
    let next = match target {
        SeekFrom::Start(offset) => i128::from(offset),
        SeekFrom::Current(delta) => i128::from(position) + i128::from(delta),
        SeekFrom::End(delta) => i128::from(end) + i128::from(delta),
    };
    if next < 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid seek before start of segmented mux source",
        ));
    }
    u64::try_from(next).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid seek target for segmented mux source",
        )
    })
}

struct SyncMuxSource {
    inner: SyncMuxSourceInner,
}

enum SyncMuxSourceInner {
    File(File),
    Segmented(SegmentedSyncMuxSource),
}

struct SegmentedSyncMuxSource {
    file: File,
    segments: Vec<SegmentedMuxSourceSegment>,
    total_size: u64,
    position: u64,
    file_position: Option<u64>,
}

impl SyncMuxSource {
    fn open(spec: &SourceSpec) -> Result<Self, MuxError> {
        let inner = match spec {
            SourceSpec::File(path) => SyncMuxSourceInner::File(File::open(path)?),
            SourceSpec::Segmented(spec) => SyncMuxSourceInner::Segmented(SegmentedSyncMuxSource {
                file: File::open(&spec.path)?,
                segments: spec.segments.clone(),
                total_size: spec.total_size,
                position: 0,
                file_position: None,
            }),
        };
        Ok(Self { inner })
    }
}

impl SegmentedSyncMuxSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.position >= self.total_size {
            return Ok(0);
        }

        let mut written = 0usize;
        while written < buf.len() && self.position < self.total_size {
            let Some(segment_index) =
                find_segmented_source_segment_index(&self.segments, self.position)
            else {
                break;
            };
            let segment = &self.segments[segment_index];
            let segment_offset =
                usize::try_from(self.position - segment.logical_offset).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "logical offset overflow")
                })?;
            match &segment.data {
                SegmentedMuxSourceSegmentData::Prefix(prefix) => {
                    let available = prefix.len().saturating_sub(segment_offset);
                    let to_copy = available.min(buf.len() - written);
                    buf[written..written + to_copy]
                        .copy_from_slice(&prefix[segment_offset..segment_offset + to_copy]);
                    written += to_copy;
                    self.position += u64::try_from(to_copy).unwrap();
                }
                SegmentedMuxSourceSegmentData::Bytes(bytes) => {
                    let available = bytes.len().saturating_sub(segment_offset);
                    let to_copy = available.min(buf.len() - written);
                    buf[written..written + to_copy]
                        .copy_from_slice(&bytes[segment_offset..segment_offset + to_copy]);
                    written += to_copy;
                    self.position += u64::try_from(to_copy).unwrap();
                }
                SegmentedMuxSourceSegmentData::FileRange {
                    source_offset,
                    size,
                } => {
                    let available =
                        usize::try_from(u64::from(*size) - u64::try_from(segment_offset).unwrap())
                            .map_err(|_| {
                                io::Error::new(io::ErrorKind::InvalidData, "segment size overflow")
                            })?;
                    let to_read = available.min(buf.len() - written);
                    let file_offset = source_offset + u64::try_from(segment_offset).unwrap();
                    if self.file_position != Some(file_offset) {
                        self.file.seek(SeekFrom::Start(file_offset))?;
                        self.file_position = Some(file_offset);
                    }
                    let read = self.file.read(&mut buf[written..written + to_read])?;
                    if read == 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated segmented mux source input",
                        ));
                    }
                    written += read;
                    self.position += u64::try_from(read).unwrap();
                    self.file_position = Some(file_offset + u64::try_from(read).unwrap());
                }
            }
        }
        Ok(written)
    }

    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        self.position = seek_mux_source_position(self.position, self.total_size, target)?;
        Ok(self.position)
    }
}

impl Read for SyncMuxSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match &mut self.inner {
            SyncMuxSourceInner::File(file) => file.read(buf),
            SyncMuxSourceInner::Segmented(source) => source.read(buf),
        }
    }
}

impl Seek for SyncMuxSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match &mut self.inner {
            SyncMuxSourceInner::File(file) => file.seek(pos),
            SyncMuxSourceInner::Segmented(source) => source.seek(pos),
        }
    }
}

#[cfg(feature = "async")]
struct AsyncMuxSource {
    inner: AsyncMuxSourceInner,
}

#[cfg(feature = "async")]
enum AsyncMuxSourceInner {
    File(TokioFile),
    Segmented(SegmentedAsyncMuxSource),
}

#[cfg(feature = "async")]
struct SegmentedAsyncMuxSource {
    file: TokioFile,
    segments: Vec<SegmentedMuxSourceSegment>,
    total_size: u64,
    position: u64,
    file_position: Option<u64>,
    pending_file_seek: Option<u64>,
}

#[cfg(feature = "async")]
impl AsyncMuxSource {
    async fn open(spec: &SourceSpec) -> Result<Self, MuxError> {
        let inner = match spec {
            SourceSpec::File(path) => AsyncMuxSourceInner::File(TokioFile::open(path).await?),
            SourceSpec::Segmented(spec) => {
                AsyncMuxSourceInner::Segmented(SegmentedAsyncMuxSource {
                    file: TokioFile::open(&spec.path).await?,
                    segments: spec.segments.clone(),
                    total_size: spec.total_size,
                    position: 0,
                    file_position: None,
                    pending_file_seek: None,
                })
            }
        };
        Ok(Self { inner })
    }
}

#[cfg(feature = "async")]
impl SegmentedAsyncMuxSource {
    fn start_seek(&mut self, target: SeekFrom) -> io::Result<()> {
        self.position = seek_mux_source_position(self.position, self.total_size, target)?;
        Ok(())
    }

    fn poll_complete(&mut self, _cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Poll::Ready(Ok(self.position))
    }

    fn poll_read_internal(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 || self.position >= self.total_size {
            return Poll::Ready(Ok(()));
        }

        let Some(segment_index) =
            find_segmented_source_segment_index(&self.segments, self.position)
        else {
            return Poll::Ready(Ok(()));
        };
        let segment = &self.segments[segment_index];
        let segment_offset = usize::try_from(self.position - segment.logical_offset)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "logical offset overflow"))?;
        match &segment.data {
            SegmentedMuxSourceSegmentData::Prefix(prefix) => {
                let available = prefix.len().saturating_sub(segment_offset);
                let to_copy = available.min(buf.remaining());
                buf.put_slice(&prefix[segment_offset..segment_offset + to_copy]);
                self.position += u64::try_from(to_copy).unwrap();
                Poll::Ready(Ok(()))
            }
            SegmentedMuxSourceSegmentData::Bytes(bytes) => {
                let available = bytes.len().saturating_sub(segment_offset);
                let to_copy = available.min(buf.remaining());
                buf.put_slice(&bytes[segment_offset..segment_offset + to_copy]);
                self.position += u64::try_from(to_copy).unwrap();
                Poll::Ready(Ok(()))
            }
            SegmentedMuxSourceSegmentData::FileRange {
                source_offset,
                size,
            } => {
                let available =
                    usize::try_from(u64::from(*size) - u64::try_from(segment_offset).unwrap())
                        .map_err(|_| {
                            io::Error::new(io::ErrorKind::InvalidData, "segment size overflow")
                        })?;
                let to_read = available.min(buf.remaining()).min(8192);
                let file_offset = source_offset + u64::try_from(segment_offset).unwrap();
                if self.file_position != Some(file_offset) {
                    if self.pending_file_seek.is_none() {
                        Pin::new(&mut self.file).start_seek(SeekFrom::Start(file_offset))?;
                        self.pending_file_seek = Some(file_offset);
                    }
                    match Pin::new(&mut self.file).poll_complete(cx) {
                        Poll::Ready(Ok(position)) => {
                            self.pending_file_seek = None;
                            self.file_position = Some(position);
                        }
                        Poll::Ready(Err(error)) => {
                            self.pending_file_seek = None;
                            return Poll::Ready(Err(error));
                        }
                        Poll::Pending => return Poll::Pending,
                    }
                }

                let mut scratch = [0_u8; 8192];
                let mut temp = ReadBuf::new(&mut scratch[..to_read]);
                match Pin::new(&mut self.file).poll_read(cx, &mut temp) {
                    Poll::Ready(Ok(())) => {
                        let read = temp.filled().len();
                        if read == 0 {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "truncated segmented mux source input",
                            )));
                        }
                        buf.put_slice(temp.filled());
                        self.position += u64::try_from(read).unwrap();
                        self.file_position = Some(file_offset + u64::try_from(read).unwrap());
                        Poll::Ready(Ok(()))
                    }
                    Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}

#[cfg(feature = "async")]
impl AsyncRead for AsyncMuxSource {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut self.inner {
            AsyncMuxSourceInner::File(file) => Pin::new(file).poll_read(cx, buf),
            AsyncMuxSourceInner::Segmented(source) => source.poll_read_internal(cx, buf),
        }
    }
}

#[cfg(feature = "async")]
impl AsyncSeek for AsyncMuxSource {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> io::Result<()> {
        match &mut self.inner {
            AsyncMuxSourceInner::File(file) => Pin::new(file).start_seek(position),
            AsyncMuxSourceInner::Segmented(source) => source.start_seek(position),
        }
    }

    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        match &mut self.inner {
            AsyncMuxSourceInner::File(file) => Pin::new(file).poll_complete(cx),
            AsyncMuxSourceInner::Segmented(source) => source.poll_complete(cx),
        }
    }
}

struct ImportedTrack {
    kind: MuxTrackKind,
    timescale: u32,
    language: [u8; 3],
    handler_name: String,
    mux_policy: ImportedTrackMuxPolicy,
    width: u16,
    height: u16,
    sample_entry_box: Vec<u8>,
    source_edit_media_time: Option<u64>,
    sample_roll_distance: Option<i16>,
    samples: Vec<ImportedSample>,
}

#[derive(Clone, Copy)]
struct ImportedSample {
    source_index: usize,
    data_offset: u64,
    data_size: u32,
    duration: u32,
    composition_time_offset: i32,
    is_sync_sample: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mux) enum FlatTimingOverrideKind {
    None,
    IamfSequencePresentation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mux) struct ImportedTrackMuxPolicy {
    sync_sample_table_mode: SyncSampleTableMode,
    stsc_run_encoding_mode: StscRunEncodingMode,
    flat_timing_override_kind: FlatTimingOverrideKind,
}

impl ImportedTrackMuxPolicy {
    const DEFAULT: Self = Self {
        sync_sample_table_mode: SyncSampleTableMode::Auto,
        stsc_run_encoding_mode: StscRunEncodingMode::CollapseIdentical,
        flat_timing_override_kind: FlatTimingOverrideKind::None,
    };
}

#[derive(Clone, Copy)]
pub(in crate::mux) struct StagedSample {
    pub(in crate::mux) data_offset: u64,
    pub(in crate::mux) data_size: u32,
    pub(in crate::mux) duration: u32,
    pub(in crate::mux) composition_time_offset: i32,
    pub(in crate::mux) is_sync_sample: bool,
}

#[derive(Clone)]
pub(in crate::mux) struct TrackCandidate {
    pub(in crate::mux) track_id: u32,
    pub(in crate::mux) kind: MuxTrackKind,
    pub(in crate::mux) timescale: u32,
    pub(in crate::mux) language: [u8; 3],
    pub(in crate::mux) handler_name: String,
    pub(in crate::mux) mux_policy: ImportedTrackMuxPolicy,
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) source_edit_media_time: Option<u64>,
    pub(in crate::mux) samples: Vec<CandidateSample>,
}

#[derive(Clone, Copy)]
pub(in crate::mux) struct CandidateSample {
    pub(in crate::mux) source_index: usize,
    pub(in crate::mux) data_offset: u64,
    pub(in crate::mux) data_size: u32,
    pub(in crate::mux) duration: u32,
    pub(in crate::mux) composition_time_offset: i32,
    pub(in crate::mux) is_sync_sample: bool,
}

pub(in crate::mux) struct CompositeTrackCandidate {
    pub(in crate::mux) track: TrackCandidate,
    pub(in crate::mux) source_spec: SegmentedMuxSourceSpec,
}

fn assign_candidate_source_index(track: &mut TrackCandidate, source_index: usize) {
    for sample in &mut track.samples {
        sample.source_index = source_index;
    }
}

fn imported_samples_from_staged(
    staged_samples: Vec<StagedSample>,
    source_index: usize,
) -> Vec<ImportedSample> {
    staged_samples
        .into_iter()
        .map(|sample| ImportedSample {
            source_index,
            data_offset: sample.data_offset,
            data_size: sample.data_size,
            duration: sample.duration,
            composition_time_offset: sample.composition_time_offset,
            is_sync_sample: sample.is_sync_sample,
        })
        .collect()
}

fn prepare_request_sync(
    request: &MuxRequest,
    output_path: &Path,
) -> Result<PreparedMuxRequest, MuxError> {
    validate_request_shape(request, output_path)?;

    let mut path_kinds = Vec::with_capacity(request.tracks().len());
    let mut all_mp4_inputs = true;
    for track in request.tracks() {
        let kind = match track {
            MuxTrackSpec::Path { path, .. } => detect_path_track_kind_sync(path)?,
        };
        if !matches!(kind, DetectedPathTrackKind::Mp4) {
            all_mp4_inputs = false;
        }
        path_kinds.push(kind);
    }
    let mut sources = SourceCatalog::default();
    let mut mp4_cache = BTreeMap::<PathBuf, PathSourceMetadata>::new();
    let mut avi_cache = BTreeMap::<PathBuf, ContainerSourceMetadata>::new();
    let mut program_stream_cache = BTreeMap::<PathBuf, ContainerSourceMetadata>::new();
    let mut transport_stream_cache = BTreeMap::<PathBuf, ContainerSourceMetadata>::new();
    let mut vobsub_cache = BTreeMap::<PathBuf, ContainerSourceMetadata>::new();
    let mut imported_tracks = Vec::new();
    let mut authority_file_config = None::<MuxFileConfig>;

    for (track, path_kind) in request.tracks().iter().zip(path_kinds.into_iter()) {
        let MuxTrackSpec::Path { path, selector } = track;
        let spec = display_track_spec(track);
        let selector = *selector;
        match path_kind {
            DetectedPathTrackKind::Mp4 => {
                let metadata = load_mp4_source_sync(path.as_path(), &mut mp4_cache, &mut sources)?;
                if all_mp4_inputs && authority_file_config.is_none() {
                    authority_file_config = metadata.file_config.clone();
                }
                let mut selected = select_container_tracks(&metadata.tracks, selector, spec)?;
                imported_tracks.append(&mut selected);
            }
            DetectedPathTrackKind::Container(DetectedContainerPathKind::Avi) => {
                let metadata = load_avi_source_sync(path.as_path(), &mut avi_cache, &mut sources)?;
                let mut selected = select_container_tracks(&metadata.tracks, selector, spec)?;
                imported_tracks.append(&mut selected);
            }
            DetectedPathTrackKind::Container(DetectedContainerPathKind::ProgramStream) => {
                let metadata = load_program_stream_source_sync(
                    path.as_path(),
                    &mut program_stream_cache,
                    &mut sources,
                )?;
                let mut selected = select_container_tracks(&metadata.tracks, selector, spec)?;
                imported_tracks.append(&mut selected);
            }
            DetectedPathTrackKind::Container(DetectedContainerPathKind::TransportStream) => {
                let metadata = load_transport_stream_source_sync(
                    path.as_path(),
                    &mut transport_stream_cache,
                    &mut sources,
                )?;
                let mut selected = select_container_tracks(&metadata.tracks, selector, spec)?;
                imported_tracks.append(&mut selected);
            }
            DetectedPathTrackKind::Container(DetectedContainerPathKind::VobSub) => {
                let metadata =
                    load_vobsub_source_sync(path.as_path(), &mut vobsub_cache, &mut sources)?;
                let mut selected = select_container_tracks(&metadata.tracks, selector, spec)?;
                imported_tracks.append(&mut selected);
            }
            DetectedPathTrackKind::Raw(_)
            | DetectedPathTrackKind::Mp4ImportOnly(_)
            | DetectedPathTrackKind::Unknown => {
                if let Some(selector) = selector {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec,
                        message: format!(
                            "selector `{}` only applies to containerized sources",
                            format_mp4_selector(selector)
                        ),
                    });
                }
                imported_tracks.push(import_detected_path_raw_sync(
                    path.as_path(),
                    &spec,
                    &mut sources,
                )?);
            }
        }
    }

    finish_prepared_request(
        request,
        output_path,
        imported_tracks,
        sources,
        authority_file_config,
    )
}

#[cfg(feature = "async")]
async fn prepare_request_async(
    request: &MuxRequest,
    output_path: &Path,
) -> Result<PreparedMuxRequest, MuxError> {
    validate_request_shape(request, output_path)?;

    let mut path_kinds = Vec::with_capacity(request.tracks().len());
    let mut all_mp4_inputs = true;
    for track in request.tracks() {
        let kind = match track {
            MuxTrackSpec::Path { path, .. } => detect_path_track_kind_async(path).await?,
        };
        if !matches!(kind, DetectedPathTrackKind::Mp4) {
            all_mp4_inputs = false;
        }
        path_kinds.push(kind);
    }
    let mut sources = SourceCatalog::default();
    let mut mp4_cache = BTreeMap::<PathBuf, PathSourceMetadata>::new();
    let mut avi_cache = BTreeMap::<PathBuf, ContainerSourceMetadata>::new();
    let mut program_stream_cache = BTreeMap::<PathBuf, ContainerSourceMetadata>::new();
    let mut transport_stream_cache = BTreeMap::<PathBuf, ContainerSourceMetadata>::new();
    let mut vobsub_cache = BTreeMap::<PathBuf, ContainerSourceMetadata>::new();
    let mut imported_tracks = Vec::new();
    let mut authority_file_config = None::<MuxFileConfig>;

    for (track, path_kind) in request.tracks().iter().zip(path_kinds.into_iter()) {
        let MuxTrackSpec::Path { path, selector } = track;
        let spec = display_track_spec(track);
        let selector = *selector;
        match path_kind {
            DetectedPathTrackKind::Mp4 => {
                let metadata =
                    load_mp4_source_async(path.as_path(), &mut mp4_cache, &mut sources).await?;
                if all_mp4_inputs && authority_file_config.is_none() {
                    authority_file_config = metadata.file_config.clone();
                }
                let mut selected = select_container_tracks(&metadata.tracks, selector, spec)?;
                imported_tracks.append(&mut selected);
            }
            DetectedPathTrackKind::Container(DetectedContainerPathKind::Avi) => {
                let metadata =
                    load_avi_source_async(path.as_path(), &mut avi_cache, &mut sources).await?;
                let mut selected = select_container_tracks(&metadata.tracks, selector, spec)?;
                imported_tracks.append(&mut selected);
            }
            DetectedPathTrackKind::Container(DetectedContainerPathKind::ProgramStream) => {
                let metadata = load_program_stream_source_async(
                    path.as_path(),
                    &mut program_stream_cache,
                    &mut sources,
                )
                .await?;
                let mut selected = select_container_tracks(&metadata.tracks, selector, spec)?;
                imported_tracks.append(&mut selected);
            }
            DetectedPathTrackKind::Container(DetectedContainerPathKind::TransportStream) => {
                let metadata = load_transport_stream_source_async(
                    path.as_path(),
                    &mut transport_stream_cache,
                    &mut sources,
                )
                .await?;
                let mut selected = select_container_tracks(&metadata.tracks, selector, spec)?;
                imported_tracks.append(&mut selected);
            }
            DetectedPathTrackKind::Container(DetectedContainerPathKind::VobSub) => {
                let metadata =
                    load_vobsub_source_async(path.as_path(), &mut vobsub_cache, &mut sources)
                        .await?;
                let mut selected = select_container_tracks(&metadata.tracks, selector, spec)?;
                imported_tracks.append(&mut selected);
            }
            DetectedPathTrackKind::Raw(_)
            | DetectedPathTrackKind::Mp4ImportOnly(_)
            | DetectedPathTrackKind::Unknown => {
                if let Some(selector) = selector {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec,
                        message: format!(
                            "selector `{}` only applies to containerized sources",
                            format_mp4_selector(selector)
                        ),
                    });
                }
                imported_tracks.push(
                    import_detected_path_raw_async(path.as_path(), &spec, &mut sources).await?,
                );
            }
        }
    }

    finish_prepared_request(
        request,
        output_path,
        imported_tracks,
        sources,
        authority_file_config,
    )
}

fn finish_prepared_request(
    request: &MuxRequest,
    _output_path: &Path,
    imported_tracks: Vec<ImportedTrack>,
    sources: SourceCatalog,
    authority_file_config: Option<MuxFileConfig>,
) -> Result<PreparedMuxRequest, MuxError> {
    let video_count = imported_tracks
        .iter()
        .filter(|track| track.kind == MuxTrackKind::Video)
        .count();
    if video_count > 1 {
        return Err(MuxError::MultipleVideoTracks { count: video_count });
    }

    let movie_timescale = choose_movie_timescale(
        &imported_tracks,
        authority_file_config.as_ref(),
        request.output_layout(),
    )?;
    let file_config = choose_file_config(movie_timescale, authority_file_config.as_ref());
    let duration_boundary_kind = request
        .duration_mode()
        .map(|duration_mode| match duration_mode {
            super::MuxDurationMode::Segment { .. } => MuxDurationBoundaryKind::Segment,
            super::MuxDurationMode::Fragment { .. } => MuxDurationBoundaryKind::Fragment,
        });
    let fragmented_single_sidx_reference = matches!(
        request.duration_mode(),
        Some(super::MuxDurationMode::Fragment { .. })
    );

    let duration_target = if let Some(duration_mode) = request.duration_mode() {
        if request.tracks().len() != 1 {
            return Err(MuxError::InvalidDurationMode {
                mode: duration_mode.label(),
                message: "the current one-file mux follow-on only supports duration-boundary modes for single-track jobs".to_string(),
            });
        }
        let seconds = duration_mode.seconds();
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(MuxError::InvalidDurationMode {
                mode: duration_mode.label(),
                message: "duration must be a finite value greater than zero".to_string(),
            });
        }
        let ticks = (seconds * f64::from(movie_timescale)).round();
        if ticks < 1.0 {
            return Err(MuxError::InvalidDurationMode {
                mode: duration_mode.label(),
                message: "duration is too small for the selected movie timescale".to_string(),
            });
        }
        Some(ticks as u64)
    } else {
        None
    };
    let auto_flat_interleave_target = if duration_target.is_none()
        && request.output_layout() == MuxOutputLayout::Flat
        && file_config.auto_flat_profile()
    {
        Some(auto_flat_interleave_target_ticks(movie_timescale))
    } else {
        None
    };
    let audio_track_count = imported_tracks
        .iter()
        .filter(|track| track.kind.is_audio())
        .count();

    let mut staged_items = Vec::new();
    let mut track_configs = Vec::new();
    let mut coordination_directives = Vec::new();
    for (index, imported_track) in imported_tracks.iter().enumerate() {
        let track_id = u32::try_from(index + 1)
            .map_err(|_| MuxError::LayoutOverflow("track identifier assignment"))?;
        let mut decode_time = 0_u64;
        if let (Some(target_ticks), Some(duration_boundary_kind)) =
            (duration_target, duration_boundary_kind)
        {
            let normalized_sample_durations = imported_track
                .samples
                .iter()
                .map(|sample| {
                    scale_track_time_to_movie(
                        track_id,
                        i64::from(sample.duration),
                        imported_track.timescale,
                        movie_timescale,
                    )
                    .map(|duration| duration as u32)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if !normalized_sample_durations.is_empty() {
                let chunk_sample_counts = if imported_track.kind.is_video() {
                    let start_time_ticks = imported_track
                        .source_edit_media_time
                        .map(|media_time| {
                            scale_track_time_to_movie(
                                track_id,
                                i64::try_from(media_time).map_err(|_| {
                                    MuxError::LayoutOverflow("segment start-time normalization")
                                })?,
                                imported_track.timescale,
                                movie_timescale,
                            )
                            .map(|normalized| -normalized)
                        })
                        .transpose()?
                        .unwrap_or(0);
                    let segment_samples = imported_track
                        .samples
                        .iter()
                        .zip(normalized_sample_durations.iter().copied())
                        .map(|(sample, duration_ticks)| {
                            let composition_offset_ticks = scale_track_time_to_movie(
                                track_id,
                                i64::from(sample.composition_time_offset),
                                imported_track.timescale,
                                movie_timescale,
                            )?;
                            Ok((
                                duration_ticks,
                                composition_offset_ticks,
                                sample.is_sync_sample,
                            ))
                        })
                        .collect::<Result<Vec<_>, MuxError>>()?;
                    build_sync_aligned_segment_chunk_sample_counts(
                        track_id,
                        segment_samples,
                        target_ticks,
                        start_time_ticks,
                    )?
                } else if duration_boundary_kind == MuxDurationBoundaryKind::Segment {
                    let start_time_ticks = imported_track
                        .source_edit_media_time
                        .map(|media_time| {
                            scale_track_time_to_movie(
                                track_id,
                                i64::try_from(media_time).map_err(|_| {
                                    MuxError::LayoutOverflow("segment start-time normalization")
                                })?,
                                imported_track.timescale,
                                movie_timescale,
                            )
                            .map(|normalized| -normalized)
                        })
                        .transpose()?
                        .unwrap_or(0);
                    build_duration_chunk_sample_counts_with_start_time(
                        track_id,
                        normalized_sample_durations,
                        target_ticks,
                        start_time_ticks,
                    )?
                } else {
                    build_duration_chunk_sample_counts(
                        track_id,
                        normalized_sample_durations,
                        target_ticks,
                    )?
                };
                coordination_directives.push(
                    TrackCoordinationDirective::new(track_id, chunk_sample_counts)
                        .with_duration_boundaries(duration_boundary_kind),
                );
            }
        } else if let Some(target_ticks) = auto_flat_interleave_target {
            if imported_track.kind.is_audio() {
                let normalized_sample_durations = imported_track
                    .samples
                    .iter()
                    .map(|sample| {
                        scale_track_time_to_movie(
                            track_id,
                            i64::from(sample.duration),
                            imported_track.timescale,
                            movie_timescale,
                        )
                        .map(|duration| duration as u32)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if !normalized_sample_durations.is_empty() {
                    let mut chunk_sample_counts = build_capped_duration_chunk_sample_counts(
                        track_id,
                        normalized_sample_durations,
                        target_ticks,
                    )?;
                    if audio_track_count > 1 {
                        rebalance_small_multi_audio_chunk_sample_counts(&mut chunk_sample_counts);
                    }
                    coordination_directives.push(TrackCoordinationDirective::new(
                        track_id,
                        chunk_sample_counts,
                    ));
                }
            } else if imported_track.kind == MuxTrackKind::Subtitle
                && imported_track.sample_entry_box.get(4..8) == Some(b"mp4s".as_slice())
                && !imported_track.samples.is_empty()
            {
                coordination_directives.push(TrackCoordinationDirective::new(
                    track_id,
                    vec![1; imported_track.samples.len()],
                ));
            } else if imported_track.kind.is_video() && !imported_track.samples.is_empty() {
                coordination_directives.push(TrackCoordinationDirective::new(
                    track_id,
                    vec![
                        u32::try_from(imported_track.samples.len())
                            .map_err(|_| MuxError::LayoutOverflow("flat video chunk count"))?,
                    ],
                ));
            }
        }

        for sample in &imported_track.samples {
            let duration = scale_track_time_to_movie(
                track_id,
                i64::from(sample.duration),
                imported_track.timescale,
                movie_timescale,
            )? as u32;
            let composition_time_offset = scale_track_time_to_movie(
                track_id,
                i64::from(sample.composition_time_offset),
                imported_track.timescale,
                movie_timescale,
            )? as i32;
            staged_items.push(
                MuxStagedMediaItem::new(
                    sample.source_index,
                    track_id,
                    decode_time,
                    duration,
                    sample.data_offset,
                    sample.data_size,
                )
                .with_composition_time_offset(composition_time_offset)
                .with_sync_sample(sample.is_sync_sample),
            );
            decode_time = decode_time
                .checked_add(u64::from(duration))
                .ok_or(MuxError::LayoutOverflow("track decode timeline"))?;
        }

        let config = match imported_track.kind {
            MuxTrackKind::Audio => MuxTrackConfig::new_audio(
                track_id,
                imported_track.timescale,
                imported_track.sample_entry_box.clone(),
            ),
            MuxTrackKind::Video => MuxTrackConfig::new_video(
                track_id,
                imported_track.timescale,
                imported_track.width,
                imported_track.height,
                imported_track.sample_entry_box.clone(),
            ),
            MuxTrackKind::Text => MuxTrackConfig::new_text(
                track_id,
                imported_track.timescale,
                imported_track.width,
                imported_track.height,
                imported_track.sample_entry_box.clone(),
            ),
            MuxTrackKind::Subtitle => MuxTrackConfig::new_subtitle(
                track_id,
                imported_track.timescale,
                imported_track.width,
                imported_track.height,
                imported_track.sample_entry_box.clone(),
            ),
        }
        .with_language(imported_track.language)
        .with_handler_name(imported_track.handler_name.clone())
        .with_sync_sample_table_mode(sync_sample_table_mode_for_imported_track(imported_track))
        .with_stsc_run_encoding_mode(stsc_run_encoding_mode_for_imported_track(imported_track));
        let config = if let Some(edit_media_time) = imported_track.source_edit_media_time {
            config.with_edit_media_time(edit_media_time)
        } else {
            config
        };
        let config = if let Some(sample_roll_distance) = imported_track.sample_roll_distance {
            config.with_sample_roll_distance(sample_roll_distance)
        } else {
            config
        };
        let config = if let Some(flat_timing_override) =
            flat_timing_override_for_imported_track(imported_track)
        {
            config.with_flat_timing_override(flat_timing_override)
        } else {
            config
        };
        track_configs.push(config);
    }

    let plan = plan_staged_media_items_with_coordination(
        staged_items,
        MuxInterleavePolicy::DecodeTime,
        coordination_directives,
    )?;
    Ok(PreparedMuxRequest {
        output_layout: request.output_layout(),
        file_config,
        track_configs,
        fragmented_single_sidx_reference,
        plan,
        source_specs: sources.specs,
    })
}

fn auto_flat_interleave_target_ticks(movie_timescale: u32) -> u64 {
    u64::from(movie_timescale)
        .saturating_mul(AUTO_FLAT_INTERLEAVE_MILLISECONDS)
        .div_ceil(1_000)
        .max(1)
}

#[derive(Default)]
struct SourceCatalog {
    specs: Vec<SourceSpec>,
    files: BTreeMap<PathBuf, usize>,
}

impl SourceCatalog {
    fn add_file(&mut self, path: &Path) -> Result<usize, MuxError> {
        let absolute = absolute_path(path)?;
        if let Some(existing) = self.files.get(&absolute) {
            return Ok(*existing);
        }
        let index = self.specs.len();
        self.specs.push(SourceSpec::File(absolute.clone()));
        self.files.insert(absolute, index);
        Ok(index)
    }

    fn add_segmented(&mut self, mut spec: SegmentedMuxSourceSpec) -> Result<usize, MuxError> {
        spec.path = absolute_path(&spec.path)?;
        let index = self.specs.len();
        self.specs.push(SourceSpec::Segmented(spec));
        Ok(index)
    }
}

struct PathSourceMetadata {
    file_config: Option<MuxFileConfig>,
    tracks: Vec<TrackCandidate>,
}

struct ContainerSourceMetadata {
    tracks: Vec<TrackCandidate>,
}

fn materialize_composite_tracks(
    sources: &mut SourceCatalog,
    composite_tracks: Vec<CompositeTrackCandidate>,
) -> Result<ContainerSourceMetadata, MuxError> {
    let mut tracks = Vec::with_capacity(composite_tracks.len());
    for composite in composite_tracks {
        let source_index = sources.add_segmented(composite.source_spec)?;
        let mut track = composite.track;
        assign_candidate_source_index(&mut track, source_index);
        tracks.push(track);
    }
    Ok(ContainerSourceMetadata { tracks })
}

fn load_mp4_source_sync<'a>(
    path: &Path,
    cache: &'a mut BTreeMap<PathBuf, PathSourceMetadata>,
    sources: &mut SourceCatalog,
) -> Result<&'a PathSourceMetadata, MuxError> {
    let absolute = absolute_path(path)?;
    if !cache.contains_key(&absolute) {
        let source_index = sources.add_file(&absolute)?;
        let mut reader = File::open(&absolute)?;
        cache.insert(
            absolute.clone(),
            parse_mp4_source_sync(&absolute, source_index, &mut reader)?,
        );
    }
    Ok(cache.get(&absolute).unwrap())
}

#[cfg(feature = "async")]
async fn load_mp4_source_async<'a>(
    path: &Path,
    cache: &'a mut BTreeMap<PathBuf, PathSourceMetadata>,
    sources: &mut SourceCatalog,
) -> Result<&'a PathSourceMetadata, MuxError> {
    let absolute = absolute_path(path)?;
    if !cache.contains_key(&absolute) {
        let source_index = sources.add_file(&absolute)?;
        let mut reader = TokioFile::open(&absolute).await?;
        cache.insert(
            absolute.clone(),
            parse_mp4_source_async(&absolute, source_index, &mut reader).await?,
        );
    }
    Ok(cache.get(&absolute).unwrap())
}

fn load_avi_source_sync<'a>(
    path: &Path,
    cache: &'a mut BTreeMap<PathBuf, ContainerSourceMetadata>,
    sources: &mut SourceCatalog,
) -> Result<&'a ContainerSourceMetadata, MuxError> {
    let absolute = absolute_path(path)?;
    if !cache.contains_key(&absolute) {
        let source_index = sources.add_file(&absolute)?;
        let scanned =
            scan_avi_source_sync(&absolute, &absolute.display().to_string(), source_index)?;
        let mut tracks = scanned.tracks;
        if !scanned.composite_tracks.is_empty() {
            tracks.extend(materialize_composite_tracks(sources, scanned.composite_tracks)?.tracks);
        }
        cache.insert(absolute.clone(), ContainerSourceMetadata { tracks });
    }
    Ok(cache.get(&absolute).unwrap())
}

fn load_program_stream_source_sync<'a>(
    path: &Path,
    cache: &'a mut BTreeMap<PathBuf, ContainerSourceMetadata>,
    sources: &mut SourceCatalog,
) -> Result<&'a ContainerSourceMetadata, MuxError> {
    let absolute = absolute_path(path)?;
    if !cache.contains_key(&absolute) {
        cache.insert(
            absolute.clone(),
            materialize_composite_tracks(
                sources,
                scan_program_stream_sync(&absolute, &absolute.display().to_string())?,
            )?,
        );
    }
    Ok(cache.get(&absolute).unwrap())
}

fn load_transport_stream_source_sync<'a>(
    path: &Path,
    cache: &'a mut BTreeMap<PathBuf, ContainerSourceMetadata>,
    sources: &mut SourceCatalog,
) -> Result<&'a ContainerSourceMetadata, MuxError> {
    let absolute = absolute_path(path)?;
    if !cache.contains_key(&absolute) {
        cache.insert(
            absolute.clone(),
            materialize_composite_tracks(
                sources,
                scan_transport_stream_sync(&absolute, &absolute.display().to_string())?,
            )?,
        );
    }
    Ok(cache.get(&absolute).unwrap())
}

fn load_vobsub_source_sync<'a>(
    path: &Path,
    cache: &'a mut BTreeMap<PathBuf, ContainerSourceMetadata>,
    sources: &mut SourceCatalog,
) -> Result<&'a ContainerSourceMetadata, MuxError> {
    let absolute = absolute_path(path)?;
    if !cache.contains_key(&absolute) {
        cache.insert(
            absolute.clone(),
            materialize_composite_tracks(
                sources,
                scan_vobsub_source_sync(&absolute, &absolute.display().to_string())?,
            )?,
        );
    }
    Ok(cache.get(&absolute).unwrap())
}

#[cfg(feature = "async")]
async fn load_avi_source_async<'a>(
    path: &Path,
    cache: &'a mut BTreeMap<PathBuf, ContainerSourceMetadata>,
    sources: &mut SourceCatalog,
) -> Result<&'a ContainerSourceMetadata, MuxError> {
    let absolute = absolute_path(path)?;
    if !cache.contains_key(&absolute) {
        let source_index = sources.add_file(&absolute)?;
        let scanned =
            scan_avi_source_async(&absolute, &absolute.display().to_string(), source_index).await?;
        let mut tracks = scanned.tracks;
        if !scanned.composite_tracks.is_empty() {
            tracks.extend(materialize_composite_tracks(sources, scanned.composite_tracks)?.tracks);
        }
        cache.insert(absolute.clone(), ContainerSourceMetadata { tracks });
    }
    Ok(cache.get(&absolute).unwrap())
}

#[cfg(feature = "async")]
async fn load_vobsub_source_async<'a>(
    path: &Path,
    cache: &'a mut BTreeMap<PathBuf, ContainerSourceMetadata>,
    sources: &mut SourceCatalog,
) -> Result<&'a ContainerSourceMetadata, MuxError> {
    let absolute = absolute_path(path)?;
    if !cache.contains_key(&absolute) {
        cache.insert(
            absolute.clone(),
            materialize_composite_tracks(
                sources,
                scan_vobsub_source_async(&absolute, &absolute.display().to_string()).await?,
            )?,
        );
    }
    Ok(cache.get(&absolute).unwrap())
}

#[cfg(feature = "async")]
async fn load_program_stream_source_async<'a>(
    path: &Path,
    cache: &'a mut BTreeMap<PathBuf, ContainerSourceMetadata>,
    sources: &mut SourceCatalog,
) -> Result<&'a ContainerSourceMetadata, MuxError> {
    let absolute = absolute_path(path)?;
    if !cache.contains_key(&absolute) {
        cache.insert(
            absolute.clone(),
            materialize_composite_tracks(
                sources,
                scan_program_stream_async(&absolute, &absolute.display().to_string()).await?,
            )?,
        );
    }
    Ok(cache.get(&absolute).unwrap())
}

#[cfg(feature = "async")]
async fn load_transport_stream_source_async<'a>(
    path: &Path,
    cache: &'a mut BTreeMap<PathBuf, ContainerSourceMetadata>,
    sources: &mut SourceCatalog,
) -> Result<&'a ContainerSourceMetadata, MuxError> {
    let absolute = absolute_path(path)?;
    if !cache.contains_key(&absolute) {
        cache.insert(
            absolute.clone(),
            materialize_composite_tracks(
                sources,
                scan_transport_stream_async(&absolute, &absolute.display().to_string()).await?,
            )?,
        );
    }
    Ok(cache.get(&absolute).unwrap())
}

fn parse_mp4_source_sync<R>(
    path: &Path,
    source_index: usize,
    reader: &mut R,
) -> Result<PathSourceMetadata, MuxError>
where
    R: Read + Seek,
{
    let file_config = probe_file_config_sync(reader)?;
    let track_infos = extract_box(reader, None, BoxPath::from([MOOV, TRAK]))?;
    let mut tracks = Vec::new();
    for trak_info in track_infos {
        if let Some(track) = parse_track_candidate_sync(path, source_index, reader, &trak_info)? {
            tracks.push(track);
        }
    }
    populate_empty_fragmented_track_samples_sync(path, source_index, reader, &mut tracks)?;
    Ok(PathSourceMetadata {
        file_config: Some(file_config),
        tracks,
    })
}

#[cfg(feature = "async")]
async fn parse_mp4_source_async<R>(
    path: &Path,
    source_index: usize,
    reader: &mut R,
) -> Result<PathSourceMetadata, MuxError>
where
    R: AsyncReadSeek,
{
    let file_config = probe_file_config_async(reader).await?;
    let track_infos = extract_box_async(reader, None, BoxPath::from([MOOV, TRAK])).await?;
    let mut tracks = Vec::new();
    for trak_info in track_infos {
        if let Some(track) =
            parse_track_candidate_async(path, source_index, reader, &trak_info).await?
        {
            tracks.push(track);
        }
    }
    populate_empty_fragmented_track_samples_async(path, source_index, reader, &mut tracks).await?;
    Ok(PathSourceMetadata {
        file_config: Some(file_config),
        tracks,
    })
}

fn populate_empty_fragmented_track_samples_sync<R>(
    path: &Path,
    source_index: usize,
    reader: &mut R,
    tracks: &mut [TrackCandidate],
) -> Result<(), MuxError>
where
    R: Read + Seek,
{
    if tracks.iter().all(|track| !track.samples.is_empty()) {
        return Ok(());
    }

    let moof_infos = extract_box(reader, None, BoxPath::from([MOOF]))?;
    if moof_infos.is_empty() {
        return Ok(());
    }
    let trex_by_track_id =
        extract_box_as::<_, Trex>(reader, None, BoxPath::from([MOOV, MVEX, TREX]))?
            .into_iter()
            .map(|trex| (trex.track_id, trex))
            .collect::<BTreeMap<_, _>>();

    for track in tracks.iter_mut().filter(|track| track.samples.is_empty()) {
        let samples = collect_fragment_candidate_samples_sync(
            path,
            source_index,
            reader,
            track.track_id,
            &moof_infos,
            trex_by_track_id.get(&track.track_id),
        )?;
        if !samples.is_empty() {
            track.samples = samples;
        }
    }
    Ok(())
}

#[cfg(feature = "async")]
async fn populate_empty_fragmented_track_samples_async<R>(
    path: &Path,
    source_index: usize,
    reader: &mut R,
    tracks: &mut [TrackCandidate],
) -> Result<(), MuxError>
where
    R: AsyncReadSeek,
{
    if tracks.iter().all(|track| !track.samples.is_empty()) {
        return Ok(());
    }

    let moof_infos = extract_box_async(reader, None, BoxPath::from([MOOF])).await?;
    if moof_infos.is_empty() {
        return Ok(());
    }
    let trex_by_track_id =
        extract_box_as_async::<_, Trex>(reader, None, BoxPath::from([MOOV, MVEX, TREX]))
            .await?
            .into_iter()
            .map(|trex| (trex.track_id, trex))
            .collect::<BTreeMap<_, _>>();

    for track in tracks.iter_mut().filter(|track| track.samples.is_empty()) {
        let samples = collect_fragment_candidate_samples_async(
            path,
            source_index,
            reader,
            track.track_id,
            &moof_infos,
            trex_by_track_id.get(&track.track_id),
        )
        .await?;
        if !samples.is_empty() {
            track.samples = samples;
        }
    }
    Ok(())
}

fn collect_fragment_candidate_samples_sync<R>(
    path: &Path,
    source_index: usize,
    reader: &mut R,
    track_id: u32,
    moof_infos: &[HeaderInfo],
    trex: Option<&Trex>,
) -> Result<Vec<CandidateSample>, MuxError>
where
    R: Read + Seek,
{
    let mut samples = Vec::new();
    for moof_info in moof_infos {
        let traf_infos = extract_box(reader, Some(moof_info), BoxPath::from([TRAF]))?;
        for traf_info in traf_infos {
            let tfhd = extract_required_single_as_sync::<_, Tfhd>(
                reader,
                &traf_info,
                BoxPath::from([TFHD]),
                "tfhd",
            )?;
            if tfhd.track_id != track_id {
                continue;
            }
            let truns = extract_box_as::<_, Trun>(reader, Some(&traf_info), BoxPath::from([TRUN]))?;
            let trun_infos = extract_box(reader, Some(&traf_info), BoxPath::from([TRUN]))?;
            let context = FragmentRunContext {
                path,
                source_index,
                track_id,
                moof_offset: moof_info.offset(),
                trex,
            };
            collect_fragment_candidate_samples_from_runs(
                &context,
                &tfhd,
                &truns,
                &trun_infos,
                &mut samples,
            )?;
        }
    }
    Ok(samples)
}

#[cfg(feature = "async")]
async fn collect_fragment_candidate_samples_async<R>(
    path: &Path,
    source_index: usize,
    reader: &mut R,
    track_id: u32,
    moof_infos: &[HeaderInfo],
    trex: Option<&Trex>,
) -> Result<Vec<CandidateSample>, MuxError>
where
    R: AsyncReadSeek,
{
    let mut samples = Vec::new();
    for moof_info in moof_infos {
        let traf_infos = extract_box_async(reader, Some(moof_info), BoxPath::from([TRAF])).await?;
        for traf_info in traf_infos {
            let tfhd = extract_required_single_as_async::<_, Tfhd>(
                reader,
                &traf_info,
                BoxPath::from([TFHD]),
                "tfhd",
            )
            .await?;
            if tfhd.track_id != track_id {
                continue;
            }
            let truns =
                extract_box_as_async::<_, Trun>(reader, Some(&traf_info), BoxPath::from([TRUN]))
                    .await?;
            let trun_infos =
                extract_box_async(reader, Some(&traf_info), BoxPath::from([TRUN])).await?;
            let context = FragmentRunContext {
                path,
                source_index,
                track_id,
                moof_offset: moof_info.offset(),
                trex,
            };
            collect_fragment_candidate_samples_from_runs(
                &context,
                &tfhd,
                &truns,
                &trun_infos,
                &mut samples,
            )?;
        }
    }
    Ok(samples)
}

fn collect_fragment_candidate_samples_from_runs(
    context: &FragmentRunContext<'_>,
    tfhd: &Tfhd,
    truns: &[Trun],
    trun_infos: &[HeaderInfo],
    output: &mut Vec<CandidateSample>,
) -> Result<(), MuxError> {
    let path = context.path;
    let track_id = context.track_id;
    if truns.len() != trun_infos.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!("track {track_id} exposes misaligned fragmented run metadata"),
        });
    }

    let base_data_offset = if tfhd.flags() & TFHD_BASE_DATA_OFFSET_PRESENT != 0 {
        tfhd.base_data_offset
    } else {
        context.moof_offset
    };
    let mut next_offset = None::<u64>;

    for (trun, trun_info) in truns.iter().zip(trun_infos.iter()) {
        let sample_count = usize::try_from(trun.sample_count).map_err(|_| {
            MuxError::UnsupportedTrackImport {
                spec: path.display().to_string(),
                message: format!(
                    "track {track_id} exposes a fragmented run whose sample count does not fit in usize"
                ),
            }
        })?;
        validate_fragment_trun_layout(path, track_id, trun, trun_info, sample_count)?;

        let mut current_offset = if trun.flags() & TRUN_DATA_OFFSET_PRESENT != 0 {
            let absolute = i128::from(base_data_offset) + i128::from(trun.data_offset);
            if absolute < 0 || absolute > i128::from(u64::MAX) {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: path.display().to_string(),
                    message: format!(
                        "track {track_id} computed an invalid fragmented data offset at trun {}",
                        trun_info.offset()
                    ),
                });
            }
            absolute as u64
        } else if let Some(next_offset) = next_offset {
            next_offset
        } else if tfhd.flags() & TFHD_DEFAULT_BASE_IS_MOOF != 0 {
            context.moof_offset
        } else {
            base_data_offset
        };

        for sample_index in 0..sample_count {
            let sample_size = effective_fragment_sample_size(
                path,
                track_id,
                tfhd,
                context.trex,
                trun,
                trun_info,
                sample_index,
            )?;
            let sample_duration = effective_fragment_sample_duration(
                path,
                track_id,
                tfhd,
                context.trex,
                trun,
                trun_info,
                sample_index,
            )?;
            let sample_flags =
                effective_fragment_sample_flags(tfhd, context.trex, trun, sample_index)
                    .unwrap_or(0);
            let composition_time_offset = if trun.flags()
                & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT
                != 0
            {
                let offset = trun.sample_composition_time_offset(sample_index);
                i32::try_from(offset).map_err(|_| MuxError::UnsupportedTrackImport {
                        spec: path.display().to_string(),
                        message: format!(
                            "track {track_id} fragmented run at {} exposes composition offset {} that does not fit in i32",
                            trun_info.offset(),
                            offset
                        ),
                    })?
            } else {
                0
            };

            output.push(CandidateSample {
                source_index: context.source_index,
                data_offset: current_offset,
                data_size: sample_size,
                duration: sample_duration,
                composition_time_offset,
                is_sync_sample: sample_flags & NON_KEY_SAMPLE_FLAGS == 0,
            });
            current_offset = current_offset
                .checked_add(u64::from(sample_size))
                .ok_or(MuxError::LayoutOverflow("fragmented sample offset"))?;
        }
        next_offset = Some(current_offset);
    }

    Ok(())
}

fn validate_fragment_trun_layout(
    path: &Path,
    track_id: u32,
    trun: &Trun,
    trun_info: &HeaderInfo,
    sample_count: usize,
) -> Result<(), MuxError> {
    let per_sample_fields_present = trun.flags()
        & (TRUN_SAMPLE_DURATION_PRESENT
            | TRUN_SAMPLE_SIZE_PRESENT
            | TRUN_SAMPLE_FLAGS_PRESENT
            | TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT)
        != 0;
    if per_sample_fields_present && trun.entries.len() != sample_count {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {track_id} fragmented run at {} declares {} samples but carries {} entries",
                trun_info.offset(),
                trun.sample_count,
                trun.entries.len()
            ),
        });
    }
    if !per_sample_fields_present && !trun.entries.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {track_id} fragmented run at {} carries unexpected inline sample entries",
                trun_info.offset()
            ),
        });
    }
    Ok(())
}

fn effective_fragment_sample_size(
    path: &Path,
    track_id: u32,
    tfhd: &Tfhd,
    trex: Option<&Trex>,
    trun: &Trun,
    trun_info: &HeaderInfo,
    sample_index: usize,
) -> Result<u32, MuxError> {
    if trun.flags() & TRUN_SAMPLE_SIZE_PRESENT != 0 {
        return trun
            .entries
            .get(sample_index)
            .map(|entry| entry.sample_size)
            .ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: path.display().to_string(),
                message: format!(
                    "track {track_id} fragmented run at {} is missing sample size entry {}",
                    trun_info.offset(),
                    sample_index + 1
                ),
            });
    }
    if tfhd.flags() & TFHD_DEFAULT_SAMPLE_SIZE_PRESENT != 0 {
        return Ok(tfhd.default_sample_size);
    }
    if let Some(trex) = trex {
        return Ok(trex.default_sample_size);
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: path.display().to_string(),
        message: format!(
            "track {track_id} requires fragmented sample-size defaults from tfhd or trex"
        ),
    })
}

fn effective_fragment_sample_duration(
    path: &Path,
    track_id: u32,
    tfhd: &Tfhd,
    trex: Option<&Trex>,
    trun: &Trun,
    trun_info: &HeaderInfo,
    sample_index: usize,
) -> Result<u32, MuxError> {
    if trun.flags() & TRUN_SAMPLE_DURATION_PRESENT != 0 {
        return trun
            .entries
            .get(sample_index)
            .map(|entry| entry.sample_duration)
            .ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: path.display().to_string(),
                message: format!(
                    "track {track_id} fragmented run at {} is missing sample duration entry {}",
                    trun_info.offset(),
                    sample_index + 1
                ),
            });
    }
    if tfhd.flags() & TFHD_DEFAULT_SAMPLE_DURATION_PRESENT != 0 {
        return Ok(tfhd.default_sample_duration);
    }
    if let Some(trex) = trex {
        return Ok(trex.default_sample_duration);
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: path.display().to_string(),
        message: format!(
            "track {track_id} requires fragmented sample-duration defaults from tfhd or trex"
        ),
    })
}

fn effective_fragment_sample_flags(
    tfhd: &Tfhd,
    trex: Option<&Trex>,
    trun: &Trun,
    sample_index: usize,
) -> Option<u32> {
    if trun.flags() & TRUN_SAMPLE_FLAGS_PRESENT != 0 {
        return trun
            .entries
            .get(sample_index)
            .map(|entry| entry.sample_flags);
    }
    if sample_index == 0 && trun.flags() & TRUN_FIRST_SAMPLE_FLAGS_PRESENT != 0 {
        return Some(trun.first_sample_flags);
    }
    if tfhd.flags() & TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT != 0 {
        return Some(tfhd.default_sample_flags);
    }
    trex.map(|trex| trex.default_sample_flags)
}

fn select_mp4_track(
    tracks: &[TrackCandidate],
    selector: MuxMp4TrackSelector,
    spec: String,
) -> Result<ImportedTrack, MuxError> {
    let selected = match selector {
        MuxMp4TrackSelector::Video => tracks.iter().find(|track| track.kind.is_video()),
        MuxMp4TrackSelector::Audio { occurrence } => tracks
            .iter()
            .filter(|track| track.kind.is_audio())
            .nth(usize::try_from(occurrence.saturating_sub(1)).unwrap_or(usize::MAX)),
        MuxMp4TrackSelector::Text { occurrence } => tracks
            .iter()
            .filter(|track| track.kind.is_textual())
            .nth(usize::try_from(occurrence.saturating_sub(1)).unwrap_or(usize::MAX)),
        MuxMp4TrackSelector::TrackId { track_id } => {
            tracks.iter().find(|track| track.track_id == track_id)
        }
    }
    .ok_or_else(|| MuxError::MissingTrackSelection { spec: spec.clone() })?;

    Ok(ImportedTrack {
        kind: selected.kind,
        timescale: selected.timescale,
        language: selected.language,
        handler_name: selected.handler_name.clone(),
        mux_policy: selected.mux_policy,
        width: selected.width,
        height: selected.height,
        sample_entry_box: selected.sample_entry_box.clone(),
        source_edit_media_time: selected.source_edit_media_time,
        sample_roll_distance: None,
        samples: selected
            .samples
            .iter()
            .map(|sample| ImportedSample {
                source_index: 0,
                data_offset: sample.data_offset,
                data_size: sample.data_size,
                duration: sample.duration,
                composition_time_offset: sample.composition_time_offset,
                is_sync_sample: sample.is_sync_sample,
            })
            .collect(),
    }
    .with_source_index_from_candidate(selected))
}

fn select_container_tracks(
    tracks: &[TrackCandidate],
    selector: Option<MuxMp4TrackSelector>,
    spec: String,
) -> Result<Vec<ImportedTrack>, MuxError> {
    match selector {
        Some(selector) => Ok(vec![select_mp4_track(tracks, selector, spec)?]),
        None => {
            let selected = tracks
                .iter()
                .filter(|track| {
                    matches!(
                        track.kind,
                        MuxTrackKind::Video
                            | MuxTrackKind::Audio
                            | MuxTrackKind::Text
                            | MuxTrackKind::Subtitle
                    )
                })
                .map(|track| ImportedTrack {
                    kind: track.kind,
                    timescale: track.timescale,
                    language: track.language,
                    handler_name: track.handler_name.clone(),
                    mux_policy: track.mux_policy,
                    width: track.width,
                    height: track.height,
                    sample_entry_box: track.sample_entry_box.clone(),
                    source_edit_media_time: track.source_edit_media_time,
                    sample_roll_distance: None,
                    samples: track
                        .samples
                        .iter()
                        .map(|sample| ImportedSample {
                            source_index: sample.source_index,
                            data_offset: sample.data_offset,
                            data_size: sample.data_size,
                            duration: sample.duration,
                            composition_time_offset: sample.composition_time_offset,
                            is_sync_sample: sample.is_sync_sample,
                        })
                        .collect(),
                })
                .collect::<Vec<_>>();
            if selected.is_empty() {
                return Err(MuxError::MissingTrackSelection { spec });
            }
            Ok(selected)
        }
    }
}

trait ImportedTrackExt {
    fn with_source_index_from_candidate(self, candidate: &TrackCandidate) -> Self;
}

impl ImportedTrackExt for ImportedTrack {
    fn with_source_index_from_candidate(mut self, candidate: &TrackCandidate) -> Self {
        for (sample, source) in self.samples.iter_mut().zip(candidate.samples.iter()) {
            sample.source_index = source.source_index;
        }
        self
    }
}

fn parse_track_candidate_sync<R>(
    path: &Path,
    source_index: usize,
    reader: &mut R,
    trak_info: &HeaderInfo,
) -> Result<Option<TrackCandidate>, MuxError>
where
    R: Read + Seek,
{
    let tkhd = extract_required_single_as_sync::<_, Tkhd>(
        reader,
        trak_info,
        BoxPath::from([TKHD]),
        "tkhd",
    )?;
    let mdhd = extract_required_single_as_sync::<_, Mdhd>(
        reader,
        trak_info,
        BoxPath::from([MDIA, MDHD]),
        "mdhd",
    )?;
    let hdlr = extract_required_single_as_sync::<_, Hdlr>(
        reader,
        trak_info,
        BoxPath::from([MDIA, HDLR]),
        "hdlr",
    )?;
    let stsd_info = extract_required_single_info_sync(
        reader,
        trak_info,
        BoxPath::from([MDIA, MINF, STBL, STSD]),
        "stsd",
    )?;
    let stsd = extract_required_single_as_sync::<_, crate::boxes::iso14496_12::Stsd>(
        reader,
        trak_info,
        BoxPath::from([MDIA, MINF, STBL, STSD]),
        "stsd",
    )?;
    if stsd.entry_count != 1 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {} uses {} sample descriptions; the current mux import expects exactly one",
                tkhd.track_id, stsd.entry_count
            ),
        });
    }
    let sample_entries =
        extract_box_with_payload(reader, Some(&stsd_info), BoxPath::from([FourCc::ANY]))?;
    let [sample_entry] = sample_entries.as_slice() else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {} does not expose exactly one sample-entry payload",
                tkhd.track_id
            ),
        });
    };
    let sample_entry_bytes =
        extract_box_bytes(reader, Some(&stsd_info), BoxPath::from([FourCc::ANY]))?;
    let [sample_entry_box] = sample_entry_bytes.as_slice() else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {} does not expose exactly one encoded sample-entry box",
                tkhd.track_id
            ),
        });
    };
    parse_track_candidate_from_components(
        path,
        source_index,
        tkhd,
        mdhd,
        hdlr,
        sample_entry,
        sample_entry_box.clone(),
        extract_required_single_as_sync::<_, Stts>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, STTS]),
            "stts",
        )?,
        extract_optional_single_as_sync::<_, Ctts>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, CTTS]),
        )?,
        extract_optional_single_as_sync::<_, Elst>(reader, trak_info, BoxPath::from([EDTS, ELST]))?,
        extract_required_single_as_sync::<_, Stsc>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, STSC]),
            "stsc",
        )?,
        extract_required_single_as_sync::<_, Stsz>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, STSZ]),
            "stsz",
        )?,
        extract_optional_single_as_sync::<_, Stco>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, STCO]),
        )?,
        extract_optional_single_as_sync::<_, Co64>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, CO64]),
        )?,
        extract_optional_single_as_sync::<_, Stss>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, STSS]),
        )?,
    )
}

#[cfg(feature = "async")]
async fn parse_track_candidate_async<R>(
    path: &Path,
    source_index: usize,
    reader: &mut R,
    trak_info: &HeaderInfo,
) -> Result<Option<TrackCandidate>, MuxError>
where
    R: AsyncReadSeek,
{
    let tkhd = extract_required_single_as_async::<_, Tkhd>(
        reader,
        trak_info,
        BoxPath::from([TKHD]),
        "tkhd",
    )
    .await?;
    let mdhd = extract_required_single_as_async::<_, Mdhd>(
        reader,
        trak_info,
        BoxPath::from([MDIA, MDHD]),
        "mdhd",
    )
    .await?;
    let hdlr = extract_required_single_as_async::<_, Hdlr>(
        reader,
        trak_info,
        BoxPath::from([MDIA, HDLR]),
        "hdlr",
    )
    .await?;
    let stsd_info = extract_required_single_info_async(
        reader,
        trak_info,
        BoxPath::from([MDIA, MINF, STBL, STSD]),
        "stsd",
    )
    .await?;
    let stsd = extract_required_single_as_async::<_, crate::boxes::iso14496_12::Stsd>(
        reader,
        trak_info,
        BoxPath::from([MDIA, MINF, STBL, STSD]),
        "stsd",
    )
    .await?;
    if stsd.entry_count != 1 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {} uses {} sample descriptions; the current mux import expects exactly one",
                tkhd.track_id, stsd.entry_count
            ),
        });
    }
    let sample_entries =
        extract_box_with_payload_async(reader, Some(&stsd_info), BoxPath::from([FourCc::ANY]))
            .await?;
    let [sample_entry] = sample_entries.as_slice() else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {} does not expose exactly one sample-entry payload",
                tkhd.track_id
            ),
        });
    };
    let sample_entry_bytes =
        extract_box_bytes_async(reader, Some(&stsd_info), BoxPath::from([FourCc::ANY])).await?;
    let [sample_entry_box] = sample_entry_bytes.as_slice() else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {} does not expose exactly one encoded sample-entry box",
                tkhd.track_id
            ),
        });
    };
    parse_track_candidate_from_components(
        path,
        source_index,
        tkhd,
        mdhd,
        hdlr,
        sample_entry,
        sample_entry_box.clone(),
        extract_required_single_as_async::<_, Stts>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, STTS]),
            "stts",
        )
        .await?,
        extract_optional_single_as_async::<_, Ctts>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, CTTS]),
        )
        .await?,
        extract_optional_single_as_async::<_, Elst>(reader, trak_info, BoxPath::from([EDTS, ELST]))
            .await?,
        extract_required_single_as_async::<_, Stsc>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, STSC]),
            "stsc",
        )
        .await?,
        extract_required_single_as_async::<_, Stsz>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, STSZ]),
            "stsz",
        )
        .await?,
        extract_optional_single_as_async::<_, Stco>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, STCO]),
        )
        .await?,
        extract_optional_single_as_async::<_, Co64>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, CO64]),
        )
        .await?,
        extract_optional_single_as_async::<_, Stss>(
            reader,
            trak_info,
            BoxPath::from([MDIA, MINF, STBL, STSS]),
        )
        .await?,
    )
}

#[allow(clippy::too_many_arguments)]
fn parse_track_candidate_from_components(
    path: &Path,
    source_index: usize,
    tkhd: Tkhd,
    mdhd: Mdhd,
    hdlr: Hdlr,
    sample_entry: &ExtractedBox,
    sample_entry_box: Vec<u8>,
    stts: Stts,
    ctts: Option<Ctts>,
    elst: Option<Elst>,
    stsc: Stsc,
    stsz: Stsz,
    stco: Option<Stco>,
    co64: Option<Co64>,
    stss: Option<Stss>,
) -> Result<Option<TrackCandidate>, MuxError> {
    let kind = match hdlr.handler_type {
        VIDE => MuxTrackKind::Video,
        SOUN => MuxTrackKind::Audio,
        TEXT => MuxTrackKind::Text,
        SUBT | SUBP => MuxTrackKind::Subtitle,
        _ => return Ok(None),
    };
    let sample_entry_type = sample_entry.info.box_type();
    if matches!(sample_entry_type, ENCV | ENCA) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {} uses protected sample entry `{sample_entry_type}`; decrypt before muxing",
                tkhd.track_id
            ),
        });
    }

    let (width, height) = match kind {
        MuxTrackKind::Audio => (0, 0),
        MuxTrackKind::Video | MuxTrackKind::Text | MuxTrackKind::Subtitle => (
            fixed_16_16_to_u16(tkhd.width),
            fixed_16_16_to_u16(tkhd.height),
        ),
    };

    let sample_sizes = expand_sample_sizes(&stsz, path, tkhd.track_id)?;
    let sample_durations = expand_sample_durations(&stts, sample_sizes.len(), path, tkhd.track_id)?;
    let composition_offsets =
        expand_composition_offsets(ctts.as_ref(), sample_sizes.len(), path, tkhd.track_id)?;
    let chunk_offsets = select_chunk_offsets(stco.as_ref(), co64.as_ref(), path, tkhd.track_id)?;
    let sample_offsets =
        expand_sample_offsets(&stsc, &sample_sizes, &chunk_offsets, path, tkhd.track_id)?;
    let sync_samples = expand_sync_samples(
        stss.as_ref(),
        sample_entry_type,
        sample_sizes.len(),
        path,
        tkhd.track_id,
    )?;

    let language = decode_mdhd_language(mdhd.language);
    let mut samples = Vec::with_capacity(sample_sizes.len());
    for index in 0..sample_sizes.len() {
        samples.push(CandidateSample {
            source_index,
            data_offset: sample_offsets[index],
            data_size: sample_sizes[index],
            duration: sample_durations[index],
            composition_time_offset: composition_offsets[index],
            is_sync_sample: sync_samples[index],
        });
    }

    Ok(Some(TrackCandidate {
        track_id: tkhd.track_id,
        kind,
        timescale: mdhd.timescale,
        language,
        handler_name: if hdlr.name.is_empty() {
            default_handler_name_for_kind(kind).to_string()
        } else {
            hdlr.name
        },
        mux_policy: ImportedTrackMuxPolicy::DEFAULT,
        width,
        height,
        sample_entry_box,
        source_edit_media_time: elst
            .as_ref()
            .filter(|table| table.entry_count != 0)
            .and_then(|table| {
                let media_time = table.media_time(0);
                (media_time > 0).then_some(media_time as u64)
            }),
        samples,
    }))
}

fn fixed_16_16_to_u16(value: u32) -> u16 {
    u16::try_from(value >> 16).unwrap_or(u16::MAX)
}

const fn default_handler_name_for_kind(kind: MuxTrackKind) -> &'static str {
    match kind {
        MuxTrackKind::Audio => "SoundHandler",
        MuxTrackKind::Video => "VideoHandler",
        MuxTrackKind::Text => "TextHandler",
        MuxTrackKind::Subtitle => "SubtitleHandler",
    }
}

pub(in crate::mux) fn direct_ingest_handler_name(codec_label: &str) -> String {
    let kind = match codec_label {
        "h263" | "h264" | "h265" | "vvc" | "av1" | "vp8" | "vp9" | "mp4v" | "ogg-theora"
        | "jpeg" | "png" => MuxTrackKind::Video,
        "vobsub" => MuxTrackKind::Subtitle,
        _ => MuxTrackKind::Audio,
    };
    default_handler_name_for_kind(kind).to_string()
}

pub(in crate::mux) fn direct_ingest_mux_policy(
    codec_label: &str,
    kind: MuxTrackKind,
) -> ImportedTrackMuxPolicy {
    let mut policy = ImportedTrackMuxPolicy::DEFAULT;
    if kind.is_audio() || codec_label == "vobsub" {
        policy.stsc_run_encoding_mode = StscRunEncodingMode::PreserveTerminalBoundary;
    }
    match codec_label {
        "vp8" | "iamf" => {
            policy.sync_sample_table_mode = SyncSampleTableMode::ForceEmpty;
        }
        "mhas" => {
            policy.sync_sample_table_mode = SyncSampleTableMode::ForceAll;
        }
        _ => {}
    }
    if codec_label == "iamf" {
        policy.flat_timing_override_kind = FlatTimingOverrideKind::IamfSequencePresentation;
    }
    policy
}

pub(in crate::mux) fn with_force_empty_sync_sample_table(
    mut policy: ImportedTrackMuxPolicy,
) -> ImportedTrackMuxPolicy {
    policy.sync_sample_table_mode = SyncSampleTableMode::ForceEmpty;
    policy
}

fn flat_timing_override_for_imported_track(
    imported_track: &ImportedTrack,
) -> Option<FlatTimingOverride> {
    if imported_track.mux_policy.flat_timing_override_kind
        != FlatTimingOverrideKind::IamfSequencePresentation
        || imported_track.samples.is_empty()
    {
        return None;
    }

    let mut sample_durations = Vec::with_capacity(imported_track.samples.len());
    if imported_track.samples.len() > 1 {
        sample_durations.resize(imported_track.samples.len() - 1, 1);
    }
    sample_durations.push(u32::MAX);

    let media_duration = u64::from(u32::MAX)
        .checked_add(u64::try_from(imported_track.samples.len().saturating_sub(1)).ok()?)?;
    Some(FlatTimingOverride {
        sample_durations,
        media_duration,
        presentation_duration: media_duration,
    })
}

fn sync_sample_table_mode_for_imported_track(
    imported_track: &ImportedTrack,
) -> SyncSampleTableMode {
    imported_track.mux_policy.sync_sample_table_mode
}

fn stsc_run_encoding_mode_for_imported_track(
    imported_track: &ImportedTrack,
) -> StscRunEncodingMode {
    imported_track.mux_policy.stsc_run_encoding_mode
}

fn import_raw_aac_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_adts_file_sync(path, &spec)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("aac"),
        mux_policy: direct_ingest_mux_policy("aac", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_aac_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_adts_file_async(path, &spec).await?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("aac"),
        mux_policy: direct_ingest_mux_policy("aac", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_latm_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let parsed = scan_latm_file_sync(path, &spec)?;
    let source_index = sources.add_segmented(parsed.segmented_source)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("latm"),
        mux_policy: direct_ingest_mux_policy("latm", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_latm_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let parsed = scan_latm_file_async(path, &spec).await?;
    let source_index = sources.add_segmented(parsed.segmented_source)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("latm"),
        mux_policy: direct_ingest_mux_policy("latm", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_h263_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_h263_file_sync(path, &spec)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: parsed.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("h263"),
        mux_policy: direct_ingest_mux_policy("h263", MuxTrackKind::Video),
        width: parsed.width,
        height: parsed.height,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_mp4v_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_mp4v_file_sync(path, &spec)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: parsed.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("mp4v"),
        mux_policy: direct_ingest_mux_policy("mp4v", MuxTrackKind::Video),
        width: parsed.width,
        height: parsed.height,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_h264_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let staged = stage_annex_b_h264_sync(path, &spec)?;
    let source_index = sources.add_segmented(staged.segmented_source)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: staged.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("h264"),
        mux_policy: direct_ingest_mux_policy("h264", MuxTrackKind::Video),
        width: staged.track_width,
        height: staged.track_height,
        sample_entry_box: staged.sample_entry_box,
        source_edit_media_time: staged.source_edit_media_time,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(staged.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_h263_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_h263_file_async(path, &spec).await?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: parsed.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("h263"),
        mux_policy: direct_ingest_mux_policy("h263", MuxTrackKind::Video),
        width: parsed.width,
        height: parsed.height,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_mp4v_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_mp4v_file_async(path, &spec).await?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: parsed.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("mp4v"),
        mux_policy: direct_ingest_mux_policy("mp4v", MuxTrackKind::Video),
        width: parsed.width,
        height: parsed.height,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_h264_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let staged = stage_annex_b_h264_async(path, &spec).await?;
    let source_index = sources.add_segmented(staged.segmented_source)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: staged.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("h264"),
        mux_policy: direct_ingest_mux_policy("h264", MuxTrackKind::Video),
        width: staged.track_width,
        height: staged.track_height,
        sample_entry_box: staged.sample_entry_box,
        source_edit_media_time: staged.source_edit_media_time,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(staged.samples, source_index),
    })
}

fn import_raw_h265_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let staged = stage_annex_b_h265_sync(path, &spec)?;
    let source_index = sources.add_segmented(staged.segmented_source)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: staged.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("h265"),
        mux_policy: direct_ingest_mux_policy("h265", MuxTrackKind::Video),
        width: staged.track_width,
        height: staged.track_height,
        sample_entry_box: staged.sample_entry_box,
        source_edit_media_time: staged.source_edit_media_time,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(staged.samples, source_index),
    })
}

fn import_raw_vvc_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let staged = stage_annex_b_vvc_sync(path, &spec)?;
    let source_index = sources.add_segmented(staged.segmented_source)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: staged.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("vvc"),
        mux_policy: direct_ingest_mux_policy("vvc", MuxTrackKind::Video),
        width: staged.track_width,
        height: staged.track_height,
        sample_entry_box: staged.sample_entry_box,
        source_edit_media_time: staged.source_edit_media_time,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(staged.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_h265_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let staged = stage_annex_b_h265_async(path, &spec).await?;
    let source_index = sources.add_segmented(staged.segmented_source)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: staged.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("h265"),
        mux_policy: direct_ingest_mux_policy("h265", MuxTrackKind::Video),
        width: staged.track_width,
        height: staged.track_height,
        sample_entry_box: staged.sample_entry_box,
        source_edit_media_time: staged.source_edit_media_time,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(staged.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_vvc_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let staged = stage_annex_b_vvc_async(path, &spec).await?;
    let source_index = sources.add_segmented(staged.segmented_source)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: staged.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("vvc"),
        mux_policy: direct_ingest_mux_policy("vvc", MuxTrackKind::Video),
        width: staged.track_width,
        height: staged.track_height,
        sample_entry_box: staged.sample_entry_box,
        source_edit_media_time: staged.source_edit_media_time,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(staged.samples, source_index),
    })
}

fn import_raw_mp3_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_mp3_file_sync(path, &spec)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("mp3"),
        mux_policy: direct_ingest_mux_policy("mp3", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_mp3_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_mp3_file_async(path, &spec).await?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("mp3"),
        mux_policy: direct_ingest_mux_policy("mp3", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_ac3_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_ac3_file_sync(path, &spec)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ac3"),
        mux_policy: direct_ingest_mux_policy("ac3", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_ac3_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_ac3_file_async(path, &spec).await?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ac3"),
        mux_policy: direct_ingest_mux_policy("ac3", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_eac3_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_eac3_file_sync(path, &spec)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ec3"),
        mux_policy: direct_ingest_mux_policy("ec3", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_eac3_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_eac3_file_async(path, &spec).await?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ec3"),
        mux_policy: direct_ingest_mux_policy("ec3", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_ac4_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_ac4_file_sync(path, &spec)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.media_time_scale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ac4"),
        mux_policy: direct_ingest_mux_policy("ac4", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_amr_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_amr_file_sync(path, &spec)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name(parsed.handler_label),
        mux_policy: direct_ingest_mux_policy(parsed.handler_label, MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_amr_wb_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_amr_wb_file_sync(path, &spec)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name(parsed.handler_label),
        mux_policy: direct_ingest_mux_policy(parsed.handler_label, MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_qcp_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_qcp_file_sync(path, &spec)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name(parsed.handler_label),
        mux_policy: direct_ingest_mux_policy(parsed.handler_label, MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_jpeg_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_jpeg_file_sync(path, &spec)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: 1_000,
        language: *b"und",
        handler_name: direct_ingest_handler_name("jpeg"),
        mux_policy: direct_ingest_mux_policy("jpeg", MuxTrackKind::Video),
        width: parsed.width,
        height: parsed.height,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: vec![ImportedSample {
            source_index,
            data_offset: 0,
            data_size: parsed.data_size,
            duration: 1_000,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    })
}

fn import_raw_png_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_png_file_sync(path, &spec)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: 1_000,
        language: *b"und",
        handler_name: direct_ingest_handler_name("png"),
        mux_policy: direct_ingest_mux_policy("png", MuxTrackKind::Video),
        width: parsed.width,
        height: parsed.height,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: vec![ImportedSample {
            source_index,
            data_offset: 0,
            data_size: parsed.data_size,
            duration: 1_000,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    })
}

fn import_raw_dts_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_dts_file_sync(path, &spec)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.media_timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("dts"),
        mux_policy: direct_ingest_mux_policy("dts", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_truehd_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_truehd_file_sync(path, &spec)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("truehd"),
        mux_policy: direct_ingest_mux_policy("truehd", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_wave_pcm_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_pcm_file_sync(path, &spec)?;
    let sample_rate = parsed.sample_rate;
    let samples = imported_pcm_samples(
        source_index,
        parsed.data_offset,
        parsed.frame_size,
        parsed.frame_count,
    )?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("pcm"),
        mux_policy: direct_ingest_mux_policy("pcm", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples,
    })
}

#[cfg(feature = "async")]
async fn import_raw_ac4_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_ac4_file_async(path, &spec).await?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.media_time_scale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ac4"),
        mux_policy: direct_ingest_mux_policy("ac4", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_amr_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_amr_file_async(path, &spec).await?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name(parsed.handler_label),
        mux_policy: direct_ingest_mux_policy(parsed.handler_label, MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_amr_wb_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_amr_wb_file_async(path, &spec).await?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name(parsed.handler_label),
        mux_policy: direct_ingest_mux_policy(parsed.handler_label, MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_qcp_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_qcp_file_async(path, &spec).await?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name(parsed.handler_label),
        mux_policy: direct_ingest_mux_policy(parsed.handler_label, MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_jpeg_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_jpeg_file_async(path, &spec).await?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: 1_000,
        language: *b"und",
        handler_name: direct_ingest_handler_name("jpeg"),
        mux_policy: direct_ingest_mux_policy("jpeg", MuxTrackKind::Video),
        width: parsed.width,
        height: parsed.height,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: vec![ImportedSample {
            source_index,
            data_offset: 0,
            data_size: parsed.data_size,
            duration: 1_000,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    })
}

#[cfg(feature = "async")]
async fn import_raw_png_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_png_file_async(path, &spec).await?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: 1_000,
        language: *b"und",
        handler_name: direct_ingest_handler_name("png"),
        mux_policy: direct_ingest_mux_policy("png", MuxTrackKind::Video),
        width: parsed.width,
        height: parsed.height,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: vec![ImportedSample {
            source_index,
            data_offset: 0,
            data_size: parsed.data_size,
            duration: 1_000,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    })
}

#[cfg(feature = "async")]
async fn import_raw_truehd_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_truehd_file_async(path, &spec).await?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("truehd"),
        mux_policy: direct_ingest_mux_policy("truehd", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_wave_pcm_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_pcm_file_async(path, &spec).await?;
    let sample_rate = parsed.sample_rate;
    let samples = imported_pcm_samples(
        source_index,
        parsed.data_offset,
        parsed.frame_size,
        parsed.frame_count,
    )?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("pcm"),
        mux_policy: direct_ingest_mux_policy("pcm", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples,
    })
}

fn imported_pcm_samples(
    source_index: usize,
    data_offset: u64,
    frame_size: u32,
    frame_count: u32,
) -> Result<Vec<ImportedSample>, MuxError> {
    let mut data_offset = data_offset;
    let mut samples = Vec::with_capacity(
        usize::try_from(frame_count).map_err(|_| MuxError::LayoutOverflow("PCM frame count"))?,
    );
    for _ in 0..frame_count {
        samples.push(ImportedSample {
            source_index,
            data_offset,
            data_size: frame_size,
            duration: 1,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        data_offset = data_offset
            .checked_add(u64::from(frame_size))
            .ok_or(MuxError::LayoutOverflow("PCM frame offset"))?;
    }
    Ok(samples)
}

#[cfg(feature = "async")]
async fn import_raw_dts_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_dts_file_async(path, &spec).await?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.media_timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("dts"),
        mux_policy: direct_ingest_mux_policy("dts", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_flac_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    if path_starts_with_sync(path, b"OggS")? {
        return import_ogg_flac_sync(path, spec, sources);
    }
    let source_index = sources.add_file(path)?;
    let parsed = scan_flac_file_sync(path, &spec)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("flac"),
        mux_policy: direct_ingest_mux_policy("flac", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_flac_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    if path_starts_with_async(path, b"OggS").await? {
        return import_ogg_flac_async(path, spec, sources).await;
    }
    let source_index = sources.add_file(path)?;
    let parsed = scan_flac_file_async(path, &spec).await?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("flac"),
        mux_policy: direct_ingest_mux_policy("flac", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_mhas_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_mhas_file_sync(path, &spec)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("mhas"),
        mux_policy: direct_ingest_mux_policy("mhas", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_mhas_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_mhas_file_async(path, &spec).await?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("mhas"),
        mux_policy: direct_ingest_mux_policy("mhas", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_iamf_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_iamf_file_sync(path, &spec)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("iamf"),
        mux_policy: direct_ingest_mux_policy("iamf", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_iamf_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_iamf_file_async(path, &spec).await?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("iamf"),
        mux_policy: direct_ingest_mux_policy("iamf", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_ogg_flac_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let parsed = scan_ogg_flac_file_sync(path, &spec)?;
    let source_index = sources.add_segmented(parsed.segmented_source)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ogg-flac"),
        mux_policy: direct_ingest_mux_policy("ogg-flac", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_ogg_flac_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let parsed = scan_ogg_flac_file_async(path, &spec).await?;
    let source_index = sources.add_segmented(parsed.segmented_source)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ogg-flac"),
        mux_policy: direct_ingest_mux_policy("ogg-flac", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_ogg_opus_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let parsed = scan_ogg_opus_file_sync(path, &spec)?;
    let source_index = sources.add_segmented(parsed.segmented_source)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: 48_000,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ogg-opus"),
        mux_policy: direct_ingest_mux_policy("ogg-opus", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: parsed.edit_media_time,
        sample_roll_distance: parsed.sample_roll_distance,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_ogg_vorbis_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let parsed = scan_ogg_vorbis_file_sync(path, &spec)?;
    let source_index = sources.add_segmented(parsed.segmented_source)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ogg-vorbis"),
        mux_policy: direct_ingest_mux_policy("ogg-vorbis", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_ogg_speex_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let parsed = scan_ogg_speex_file_sync(path, &spec)?;
    let source_index = sources.add_segmented(parsed.segmented_source)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ogg-speex"),
        mux_policy: direct_ingest_mux_policy("ogg-speex", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_ogg_theora_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let parsed = scan_ogg_theora_file_sync(path, &spec)?;
    let source_index = sources.add_segmented(parsed.segmented_source)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: parsed.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ogg-theora"),
        mux_policy: direct_ingest_mux_policy("ogg-theora", MuxTrackKind::Video),
        width: parsed.width,
        height: parsed.height,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_ogg_opus_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let parsed = scan_ogg_opus_file_async(path, &spec).await?;
    let source_index = sources.add_segmented(parsed.segmented_source)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: 48_000,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ogg-opus"),
        mux_policy: direct_ingest_mux_policy("ogg-opus", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: parsed.edit_media_time,
        sample_roll_distance: parsed.sample_roll_distance,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_caf_alac_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_caf_alac_file_sync(path, &spec)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("caf-alac"),
        mux_policy: direct_ingest_mux_policy("caf-alac", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_ogg_vorbis_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let parsed = scan_ogg_vorbis_file_async(path, &spec).await?;
    let source_index = sources.add_segmented(parsed.segmented_source)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ogg-vorbis"),
        mux_policy: direct_ingest_mux_policy("ogg-vorbis", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_ogg_speex_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let parsed = scan_ogg_speex_file_async(path, &spec).await?;
    let source_index = sources.add_segmented(parsed.segmented_source)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ogg-speex"),
        mux_policy: direct_ingest_mux_policy("ogg-speex", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_ogg_theora_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let parsed = scan_ogg_theora_file_async(path, &spec).await?;
    let source_index = sources.add_segmented(parsed.segmented_source)?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: parsed.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("ogg-theora"),
        mux_policy: direct_ingest_mux_policy("ogg-theora", MuxTrackKind::Video),
        width: parsed.width,
        height: parsed.height,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_caf_alac_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_caf_alac_file_async(path, &spec).await?;
    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("caf-alac"),
        mux_policy: direct_ingest_mux_policy("caf-alac", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn choose_movie_timescale(
    imported_tracks: &[ImportedTrack],
    authority_file_config: Option<&MuxFileConfig>,
    output_layout: MuxOutputLayout,
) -> Result<u32, MuxError> {
    let mut common = 1_u32;
    for track in imported_tracks {
        common = lcm_u32(common, track.timescale)
            .ok_or(MuxError::LayoutOverflow("movie timescale selection"))?;
    }

    if matches!(output_layout, MuxOutputLayout::Fragmented) {
        return Ok(common.max(1));
    }

    let Some(authority_file_config) = authority_file_config else {
        return Ok(common.max(1));
    };

    let preferred = authority_file_config.movie_timescale();
    if preferred != 0
        && imported_tracks
            .iter()
            .all(|track| track_times_fit_movie_timescale(track, preferred))
    {
        return Ok(preferred);
    }
    Ok(common.max(1))
}

fn choose_file_config(
    movie_timescale: u32,
    authority_file_config: Option<&MuxFileConfig>,
) -> MuxFileConfig {
    let Some(authority_file_config) = authority_file_config else {
        return MuxFileConfig::new(movie_timescale).with_auto_flat_profile(true);
    };

    let mut config = MuxFileConfig::new(movie_timescale)
        .with_major_brand(authority_file_config.major_brand())
        .with_minor_version(authority_file_config.minor_version())
        .with_auto_flat_profile(false);
    for brand in authority_file_config.compatible_brands() {
        config.add_compatible_brand(*brand);
    }
    config
}

fn validate_request_shape(request: &MuxRequest, output_path: &Path) -> Result<(), MuxError> {
    if request.tracks().is_empty() {
        return Err(MuxError::MissingTrackSpecs);
    }
    if matches!(
        request.destination_mode(),
        MuxDestinationMode::UpdateOrCreateDestination
    ) {
        if !matches!(request.output_layout(), MuxOutputLayout::Flat) {
            return Err(MuxError::InvalidDestinationMode {
                mode: request.destination_mode().label(),
                message: "the current destination-path mux mode only supports flat output; use `--out PATH` for create-new fragmented output".to_string(),
            });
        }
        let output_absolute = absolute_path(output_path)?;
        for track in request.tracks() {
            let input_absolute = absolute_path(track.input_path())?;
            if input_absolute == output_absolute {
                return Err(MuxError::InvalidDestinationMode {
                    mode: request.destination_mode().label(),
                    message: "destination-path mux mode does not accept the destination file as an explicit input track".to_string(),
                });
            }
        }
    }
    match (request.output_layout(), request.duration_mode()) {
        (MuxOutputLayout::Flat, Some(duration_mode)) => {
            return Err(MuxError::InvalidOutputLayout {
                layout: request.output_layout().label(),
                message: format!(
                    "flat output does not support `--{}`; use `--layout fragmented` instead",
                    duration_mode.label()
                ),
            });
        }
        (MuxOutputLayout::Fragmented, None) => {
            return Err(MuxError::InvalidOutputLayout {
                layout: request.output_layout().label(),
                message: "fragmented output requires exactly one of `--segment_duration` or `--fragment_duration`".to_string(),
            });
        }
        (MuxOutputLayout::Fragmented, Some(_)) if request.tracks().len() != 1 => {
            return Err(MuxError::InvalidOutputLayout {
                layout: request.output_layout().label(),
                message: "the current fragmented mux follow-on only supports single-track jobs"
                    .to_string(),
            });
        }
        _ => {}
    }
    let video_count = request
        .tracks()
        .iter()
        .filter(|track| {
            matches!(
                track,
                MuxTrackSpec::Path {
                    selector: Some(MuxMp4TrackSelector::Video),
                    ..
                }
            )
        })
        .count();
    if video_count > 1 {
        return Err(MuxError::MultipleVideoTracks { count: video_count });
    }

    let output_absolute = absolute_path(output_path)?;
    for track in request.tracks() {
        let input_absolute = absolute_path(track.input_path())?;
        if input_absolute == output_absolute {
            return Err(MuxError::OutputPathConflict {
                output: output_absolute,
                input: input_absolute,
            });
        }
    }
    Ok(())
}

fn build_destination_preserving_request(
    request: &MuxRequest,
    destination_path: &Path,
) -> Result<MuxRequest, MuxError> {
    if !matches!(
        request.destination_mode(),
        MuxDestinationMode::UpdateOrCreateDestination
    ) {
        return Err(MuxError::InvalidDestinationMode {
            mode: request.destination_mode().label(),
            message: "request did not opt into the destination-path mux mode".to_string(),
        });
    }
    let mut tracks = Vec::with_capacity(request.tracks().len() + 1);
    tracks.push(MuxTrackSpec::path(destination_path.to_path_buf()));
    tracks.extend(request.tracks().iter().cloned());
    let mut amended = MuxRequest::new(tracks)
        .with_output_layout(request.output_layout())
        .with_destination_mode(MuxDestinationMode::CreateNew);
    if let Some(duration_mode) = request.duration_mode() {
        amended = amended.with_duration_mode(duration_mode);
    }
    Ok(amended)
}

fn should_preserve_destination_mp4(destination_path: &Path) -> bool {
    is_mp4_like_path(destination_path)
}

fn create_update_temp_path(
    output_path: &Path,
    mode: MuxDestinationMode,
) -> Result<PathBuf, MuxError> {
    let parent = output_path
        .parent()
        .ok_or_else(|| MuxError::InvalidDestinationMode {
            mode: mode.label(),
            message: format!(
                "cannot derive a temporary rewrite path for `{}`",
                output_path.display()
            ),
        })?;
    let file_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| MuxError::InvalidDestinationMode {
            mode: mode.label(),
            message: format!(
                "cannot derive a temporary rewrite path for `{}`",
                output_path.display()
            ),
        })?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| MuxError::InvalidDestinationMode {
            mode: mode.label(),
            message: "system clock is earlier than the Unix epoch".to_string(),
        })?
        .as_nanos();
    Ok(parent.join(format!("{file_name}.mp4forge-rewrite-{stamp}.tmp")))
}

fn replace_output_path(temp_path: &Path, output_path: &Path) -> Result<(), MuxError> {
    let backup_path = temp_path.with_extension("backup");
    if backup_path.exists() {
        std::fs::remove_file(&backup_path)?;
    }
    std::fs::rename(output_path, &backup_path)?;
    match std::fs::rename(temp_path, output_path) {
        Ok(()) => {
            let _ = std::fs::remove_file(&backup_path);
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::rename(&backup_path, output_path);
            Err(MuxError::Io(error))
        }
    }
}

#[cfg(feature = "async")]
async fn replace_output_path_async(temp_path: &Path, output_path: &Path) -> Result<(), MuxError> {
    let backup_path = temp_path.with_extension("backup");
    if tokio::fs::try_exists(&backup_path).await? {
        tokio::fs::remove_file(&backup_path).await?;
    }
    tokio::fs::rename(output_path, &backup_path).await?;
    match tokio::fs::rename(temp_path, output_path).await {
        Ok(()) => {
            let _ = tokio::fs::remove_file(&backup_path).await;
            Ok(())
        }
        Err(error) => {
            let _ = tokio::fs::rename(&backup_path, output_path).await;
            Err(MuxError::Io(error))
        }
    }
}

fn display_track_spec(track: &MuxTrackSpec) -> String {
    match track {
        MuxTrackSpec::Path { path, selector } => match selector {
            Some(selector) => format!("{}#{}", path.display(), format_mp4_selector(*selector)),
            None => path.display().to_string(),
        },
    }
}

fn format_mp4_selector(selector: MuxMp4TrackSelector) -> String {
    match selector {
        MuxMp4TrackSelector::Video => "video".to_string(),
        MuxMp4TrackSelector::Audio { occurrence: 1 } => "audio".to_string(),
        MuxMp4TrackSelector::Audio { occurrence } => format!("audio:{occurrence}"),
        MuxMp4TrackSelector::Text { occurrence: 1 } => "text".to_string(),
        MuxMp4TrackSelector::Text { occurrence } => format!("text:{occurrence}"),
        MuxMp4TrackSelector::TrackId { track_id } => format!("track:{track_id}"),
    }
}

fn detect_path_track_kind_sync(path: &Path) -> Result<DetectedPathTrackKind, MuxError> {
    let mut file = File::open(path)?;
    let mut prefix = [0_u8; 512];
    let read = file.read(&mut prefix)?;
    let prefix = &prefix[..read];
    if prefix.starts_with(b"OggS") {
        file.seek(SeekFrom::Start(0))?;
        return detect_ogg_track_kind_sync(&mut file);
    }
    if prefix.starts_with(b"caff") {
        file.seek(SeekFrom::Start(0))?;
        return detect_caf_track_kind_sync(&mut file);
    }
    if let Some(kind) = detect_id3_wrapped_audio_sync(&mut file, prefix)? {
        return Ok(kind);
    }
    if let Some(kind) = detect_vobsub_track_kind_sync(path, prefix)? {
        return Ok(kind);
    }
    Ok(detect_path_track_kind_from_prefix(prefix))
}

fn is_mp4_like_path(path: &Path) -> bool {
    matches!(
        detect_path_track_kind_sync(path),
        Ok(DetectedPathTrackKind::Mp4)
    )
}

#[cfg(feature = "async")]
async fn detect_path_track_kind_async(path: &Path) -> Result<DetectedPathTrackKind, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let mut prefix = [0_u8; 512];
    let read = file.read(&mut prefix).await?;
    let prefix = &prefix[..read];
    if prefix.starts_with(b"OggS") {
        file.seek(SeekFrom::Start(0)).await?;
        return detect_ogg_track_kind_async(&mut file).await;
    }
    if prefix.starts_with(b"caff") {
        file.seek(SeekFrom::Start(0)).await?;
        return detect_caf_track_kind_async(&mut file).await;
    }
    if let Some(kind) = detect_id3_wrapped_audio_async(&mut file, prefix).await? {
        return Ok(kind);
    }
    if let Some(kind) = detect_vobsub_track_kind_async(path, prefix).await? {
        return Ok(kind);
    }
    Ok(detect_path_track_kind_from_prefix(prefix))
}

fn detect_vobsub_track_kind_sync(
    path: &Path,
    prefix: &[u8],
) -> Result<Option<DetectedPathTrackKind>, MuxError> {
    if detect_path_track_kind_from_prefix(prefix)
        == DetectedPathTrackKind::Container(DetectedContainerPathKind::VobSub)
    {
        return Ok(Some(DetectedPathTrackKind::Container(
            DetectedContainerPathKind::VobSub,
        )));
    }
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return Ok(None);
    };
    if extension.eq_ignore_ascii_case("sub") {
        let idx_path = path.with_extension("idx");
        if idx_path.is_file() && path_starts_with_sync(&idx_path, b"# VobSub")? {
            return Ok(Some(DetectedPathTrackKind::Container(
                DetectedContainerPathKind::VobSub,
            )));
        }
    }
    Ok(None)
}

#[cfg(feature = "async")]
async fn detect_vobsub_track_kind_async(
    path: &Path,
    prefix: &[u8],
) -> Result<Option<DetectedPathTrackKind>, MuxError> {
    if detect_path_track_kind_from_prefix(prefix)
        == DetectedPathTrackKind::Container(DetectedContainerPathKind::VobSub)
    {
        return Ok(Some(DetectedPathTrackKind::Container(
            DetectedContainerPathKind::VobSub,
        )));
    }
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return Ok(None);
    };
    if extension.eq_ignore_ascii_case("sub") {
        let idx_path = path.with_extension("idx");
        if idx_path.is_file() && path_starts_with_async(&idx_path, b"# VobSub").await? {
            return Ok(Some(DetectedPathTrackKind::Container(
                DetectedContainerPathKind::VobSub,
            )));
        }
    }
    Ok(None)
}

fn detect_id3_wrapped_audio_sync(
    file: &mut File,
    prefix: &[u8],
) -> Result<Option<DetectedPathTrackKind>, MuxError> {
    let Some(id3_offset) = id3v2_size_from_prefix(prefix) else {
        return Ok(None);
    };
    if let Some(kind) = detect_id3_wrapped_audio_from_prefix(prefix, id3_offset) {
        return Ok(Some(kind));
    }
    let mut header = [0_u8; 7];
    file.seek(SeekFrom::Start(
        u64::try_from(id3_offset).map_err(|_| MuxError::LayoutOverflow("ID3v2 size"))?,
    ))?;
    let read = file.read(&mut header)?;
    Ok(detect_id3_wrapped_audio_from_prefix(&header[..read], 0))
}

#[cfg(feature = "async")]
async fn detect_id3_wrapped_audio_async(
    file: &mut TokioFile,
    prefix: &[u8],
) -> Result<Option<DetectedPathTrackKind>, MuxError> {
    let Some(id3_offset) = id3v2_size_from_prefix(prefix) else {
        return Ok(None);
    };
    if let Some(kind) = detect_id3_wrapped_audio_from_prefix(prefix, id3_offset) {
        return Ok(Some(kind));
    }
    file.seek(SeekFrom::Start(
        u64::try_from(id3_offset).map_err(|_| MuxError::LayoutOverflow("ID3v2 size"))?,
    ))
    .await?;
    let mut header = [0_u8; 7];
    let read = file.read(&mut header).await?;
    Ok(detect_id3_wrapped_audio_from_prefix(&header[..read], 0))
}

fn path_starts_with_sync(path: &Path, signature: &[u8]) -> Result<bool, MuxError> {
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

fn import_detected_path_raw_sync(
    path: &Path,
    spec: &str,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    match detect_path_track_kind_sync(path)? {
        DetectedPathTrackKind::Raw(codec) => import_detected_raw_codec_sync(path, codec, spec, sources),
        DetectedPathTrackKind::Container(DetectedContainerPathKind::Avi) => {
            Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "detected an AVI container on the raw-import path unexpectedly".to_string(),
            })
        }
        DetectedPathTrackKind::Container(DetectedContainerPathKind::ProgramStream) => {
            Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message:
                    "detected an MPEG program stream on the raw-import path unexpectedly"
                        .to_string(),
            })
        }
        DetectedPathTrackKind::Container(DetectedContainerPathKind::TransportStream) => {
            Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message:
                    "detected an MPEG transport stream on the raw-import path unexpectedly"
                        .to_string(),
            })
        }
        DetectedPathTrackKind::Container(DetectedContainerPathKind::VobSub) => {
            Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "detected a VobSub source on the raw-import path unexpectedly"
                    .to_string(),
            })
        }
        DetectedPathTrackKind::Mp4ImportOnly(kind) => Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "path-only mux import for `{kind}` is not supported; import this family from an MP4 source with `#audio` or `#track:ID` instead"
            ),
        }),
        DetectedPathTrackKind::Mp4 => Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "detected an MP4-style source on the raw-import path unexpectedly".to_string(),
        }),
        DetectedPathTrackKind::Unknown => Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "path-only mux input is not currently recognized as MP4, VobSub, supported AVI audio or MPEG-4 Part 2 video, supported MPEG-PS MPEG audio, AC-3, or MPEG-4 Part 2/H.264/H.265/VVC video, supported MPEG-TS MPEG audio, AC-3, E-AC-3, MPEG-4 Part 2, H.264, H.265, VVC, DVB subtitle, or DVB teletext video or subtitle carriage, JPEG still images, PNG still images, WAVE/AIFF/AIFC PCM, AAC ADTS, AAC LATM, MP3, AC-3, E-AC-3, AC-4, AMR, AMR-WB, QCP voice audio, DTS core audio, leading-sync MHAS MPEG-H, FLAC, IAMF, H.263 elementary video, MPEG-4 Part 2 elementary video, H.264 Annex B, H.265 Annex B, IVF-backed AV1/VP8/VP9/VP10, Ogg FLAC, Ogg Opus, Ogg Vorbis, Ogg Speex, Ogg Theora, or CAF ALAC".to_string(),
        }),
    }
}

#[cfg(feature = "async")]
async fn import_detected_path_raw_async(
    path: &Path,
    spec: &str,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    match detect_path_track_kind_async(path).await? {
        DetectedPathTrackKind::Raw(codec) => {
            import_detected_raw_codec_async(path, codec, spec, sources).await
        }
        DetectedPathTrackKind::Container(DetectedContainerPathKind::Avi) => {
            Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "detected an AVI container on the raw-import path unexpectedly".to_string(),
            })
        }
        DetectedPathTrackKind::Container(DetectedContainerPathKind::ProgramStream) => {
            Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message:
                    "detected an MPEG program stream on the raw-import path unexpectedly"
                        .to_string(),
            })
        }
        DetectedPathTrackKind::Container(DetectedContainerPathKind::TransportStream) => {
            Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message:
                    "detected an MPEG transport stream on the raw-import path unexpectedly"
                        .to_string(),
            })
        }
        DetectedPathTrackKind::Container(DetectedContainerPathKind::VobSub) => {
            Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "detected a VobSub source on the raw-import path unexpectedly"
                    .to_string(),
            })
        }
        DetectedPathTrackKind::Mp4ImportOnly(kind) => Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "path-only mux import for `{kind}` is not supported; import this family from an MP4 source with `#audio` or `#track:ID` instead"
            ),
        }),
        DetectedPathTrackKind::Mp4 => Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "detected an MP4-style source on the raw-import path unexpectedly".to_string(),
        }),
        DetectedPathTrackKind::Unknown => Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "path-only mux input is not currently recognized as MP4, VobSub, supported AVI audio or MPEG-4 Part 2 video, supported MPEG-PS MPEG audio, AC-3, or MPEG-4 Part 2/H.264/H.265/VVC video, supported MPEG-TS MPEG audio, AC-3, E-AC-3, MPEG-4 Part 2, H.264, H.265, VVC, DVB subtitle, or DVB teletext video or subtitle carriage, JPEG still images, PNG still images, WAVE/AIFF/AIFC PCM, AAC ADTS, AAC LATM, MP3, AC-3, E-AC-3, AC-4, AMR, AMR-WB, DTS core audio, leading-sync MHAS MPEG-H, FLAC, IAMF, H.263 elementary video, MPEG-4 Part 2 elementary video, H.264 Annex B, H.265 Annex B, IVF-backed AV1/VP8/VP9/VP10, Ogg FLAC, Ogg Opus, Ogg Vorbis, Ogg Speex, Ogg Theora, or CAF ALAC".to_string(),
        }),
    }
}

fn import_detected_raw_codec_sync(
    path: &Path,
    codec: MuxRawCodec,
    spec: &str,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    import_raw_track_sync(path, codec, spec.to_string(), sources)
}

#[cfg(feature = "async")]
async fn import_detected_raw_codec_async(
    path: &Path,
    codec: MuxRawCodec,
    spec: &str,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    import_raw_track_async(path, codec, spec.to_string(), sources).await
}

fn import_raw_track_sync(
    path: &Path,
    codec: MuxRawCodec,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    match codec {
        MuxRawCodec::Mp4v => import_raw_mp4v_sync(path, spec, sources),
        MuxRawCodec::H263 => import_raw_h263_sync(path, spec, sources),
        MuxRawCodec::H264 => import_raw_h264_sync(path, spec, sources),
        MuxRawCodec::H265 => import_raw_h265_sync(path, spec, sources),
        MuxRawCodec::Vvc => import_raw_vvc_sync(path, spec, sources),
        MuxRawCodec::Av1 | MuxRawCodec::Vp8 | MuxRawCodec::Vp9 | MuxRawCodec::Vp10 => {
            import_ivf_video_sync(path, codec, spec, sources)
        }
        MuxRawCodec::Aac => import_raw_aac_sync(path, spec, sources),
        MuxRawCodec::Latm => import_raw_latm_sync(path, spec, sources),
        MuxRawCodec::Mp3 => import_raw_mp3_sync(path, spec, sources),
        MuxRawCodec::Ac3 => import_raw_ac3_sync(path, spec, sources),
        MuxRawCodec::Eac3 => import_raw_eac3_sync(path, spec, sources),
        MuxRawCodec::Ac4 => import_raw_ac4_sync(path, spec, sources),
        MuxRawCodec::Amr => import_raw_amr_sync(path, spec, sources),
        MuxRawCodec::AmrWb => import_raw_amr_wb_sync(path, spec, sources),
        MuxRawCodec::Qcp => import_raw_qcp_sync(path, spec, sources),
        MuxRawCodec::Jpeg => import_raw_jpeg_sync(path, spec, sources),
        MuxRawCodec::Png => import_raw_png_sync(path, spec, sources),
        MuxRawCodec::Pcm => import_wave_pcm_sync(path, spec, sources),
        MuxRawCodec::Dts => import_raw_dts_sync(path, spec, sources),
        MuxRawCodec::Truehd => import_raw_truehd_sync(path, spec, sources),
        MuxRawCodec::Alac => import_caf_alac_sync(path, spec, sources),
        MuxRawCodec::Flac => import_raw_flac_sync(path, spec, sources),
        MuxRawCodec::Iamf => import_raw_iamf_sync(path, spec, sources),
        MuxRawCodec::MpegH => import_raw_mhas_sync(path, spec, sources),
        MuxRawCodec::Opus => import_ogg_opus_sync(path, spec, sources),
        MuxRawCodec::Vorbis => import_ogg_vorbis_sync(path, spec, sources),
        MuxRawCodec::Speex => import_ogg_speex_sync(path, spec, sources),
        MuxRawCodec::Theora => import_ogg_theora_sync(path, spec, sources),
    }
}

#[cfg(feature = "async")]
async fn import_raw_track_async(
    path: &Path,
    codec: MuxRawCodec,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    match codec {
        MuxRawCodec::Mp4v => import_raw_mp4v_async(path, spec, sources).await,
        MuxRawCodec::H263 => import_raw_h263_async(path, spec, sources).await,
        MuxRawCodec::H264 => import_raw_h264_async(path, spec, sources).await,
        MuxRawCodec::H265 => import_raw_h265_async(path, spec, sources).await,
        MuxRawCodec::Vvc => import_raw_vvc_async(path, spec, sources).await,
        MuxRawCodec::Av1 | MuxRawCodec::Vp8 | MuxRawCodec::Vp9 | MuxRawCodec::Vp10 => {
            import_ivf_video_async(path, codec, spec, sources).await
        }
        MuxRawCodec::Aac => import_raw_aac_async(path, spec, sources).await,
        MuxRawCodec::Latm => import_raw_latm_async(path, spec, sources).await,
        MuxRawCodec::Mp3 => import_raw_mp3_async(path, spec, sources).await,
        MuxRawCodec::Ac3 => import_raw_ac3_async(path, spec, sources).await,
        MuxRawCodec::Eac3 => import_raw_eac3_async(path, spec, sources).await,
        MuxRawCodec::Ac4 => import_raw_ac4_async(path, spec, sources).await,
        MuxRawCodec::Amr => import_raw_amr_async(path, spec, sources).await,
        MuxRawCodec::AmrWb => import_raw_amr_wb_async(path, spec, sources).await,
        MuxRawCodec::Qcp => import_raw_qcp_async(path, spec, sources).await,
        MuxRawCodec::Jpeg => import_raw_jpeg_async(path, spec, sources).await,
        MuxRawCodec::Png => import_raw_png_async(path, spec, sources).await,
        MuxRawCodec::Pcm => import_wave_pcm_async(path, spec, sources).await,
        MuxRawCodec::Dts => import_raw_dts_async(path, spec, sources).await,
        MuxRawCodec::Truehd => import_raw_truehd_async(path, spec, sources).await,
        MuxRawCodec::Alac => import_caf_alac_async(path, spec, sources).await,
        MuxRawCodec::Flac => import_raw_flac_async(path, spec, sources).await,
        MuxRawCodec::Iamf => import_raw_iamf_async(path, spec, sources).await,
        MuxRawCodec::MpegH => import_raw_mhas_async(path, spec, sources).await,
        MuxRawCodec::Opus => import_ogg_opus_async(path, spec, sources).await,
        MuxRawCodec::Vorbis => import_ogg_vorbis_async(path, spec, sources).await,
        MuxRawCodec::Speex => import_ogg_speex_async(path, spec, sources).await,
        MuxRawCodec::Theora => import_ogg_theora_async(path, spec, sources).await,
    }
}

pub(in crate::mux) fn build_visual_sample_entry_box(
    sample_entry_type: FourCc,
    width: u16,
    height: u16,
    child_boxes: &[Vec<u8>],
) -> Result<Vec<u8>, MuxError> {
    build_visual_sample_entry_box_with_compressor_name(
        sample_entry_type,
        width,
        height,
        &[],
        child_boxes,
    )
}

pub(in crate::mux) fn build_visual_sample_entry_box_with_compressor_name(
    sample_entry_type: FourCc,
    width: u16,
    height: u16,
    compressor_name: &[u8],
    child_boxes: &[Vec<u8>],
) -> Result<Vec<u8>, MuxError> {
    let mut compressorname = [0_u8; 32];
    let visible_len = compressor_name.len().min(31);
    compressorname[0] =
        u8::try_from(visible_len).map_err(|_| MuxError::LayoutOverflow("compressor name"))?;
    compressorname[1..1 + visible_len].copy_from_slice(&compressor_name[..visible_len]);
    super::mp4::encode_typed_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: sample_entry_type,
                data_reference_index: 1,
            },
            width,
            height,
            horizresolution: 72_u32 << 16,
            vertresolution: 72_u32 << 16,
            frame_count: 1,
            compressorname,
            depth: 0x0018,
            pre_defined3: -1,
            ..VisualSampleEntry::default()
        },
        &child_boxes.concat(),
    )
}

pub(in crate::mux) fn build_generic_audio_sample_entry_box(
    sample_entry_type: FourCc,
    sample_rate: u32,
    channel_count: u16,
    sample_size: u16,
    child_boxes: &[Vec<u8>],
) -> Result<Vec<u8>, MuxError> {
    super::mp4::encode_typed_box(
        &AudioSampleEntry {
            sample_entry: SampleEntry {
                box_type: sample_entry_type,
                data_reference_index: 1,
            },
            channel_count,
            sample_size,
            sample_rate: sample_rate << 16,
            ..AudioSampleEntry::default()
        },
        &child_boxes.concat(),
    )
}

pub(in crate::mux) fn build_generic_media_sample_entry_box(
    sample_entry_type: FourCc,
    child_boxes: &[Vec<u8>],
) -> Result<Vec<u8>, MuxError> {
    super::mp4::encode_typed_box(
        &GenericMediaSampleEntry {
            sample_entry: SampleEntry {
                box_type: sample_entry_type,
                data_reference_index: 1,
            },
        },
        &child_boxes.concat(),
    )
}

pub(in crate::mux) fn build_btrt_from_sample_sizes<I>(
    samples: I,
    timescale: u32,
) -> Result<Btrt, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    if timescale == 0 {
        return Ok(Btrt::default());
    }

    let mut saw_sample = false;
    let mut buffer_size_db = 0_u32;
    let mut total_payload_bytes = 0_u64;
    let mut total_duration = 0_u64;
    let mut max_window_payload_bytes = 0_u64;
    let mut current_window_payload_bytes = 0_u64;
    let mut window_start_decode_time = 0_u64;
    let mut sample_decode_time = 0_u64;
    for (data_size, duration) in samples {
        saw_sample = true;
        buffer_size_db = buffer_size_db.max(data_size);
        total_payload_bytes = total_payload_bytes
            .checked_add(u64::from(data_size))
            .ok_or(MuxError::LayoutOverflow("audio total payload bytes"))?;
        total_duration = total_duration
            .checked_add(u64::from(duration))
            .ok_or(MuxError::LayoutOverflow("audio total duration"))?;
        current_window_payload_bytes = current_window_payload_bytes
            .checked_add(u64::from(data_size))
            .ok_or(MuxError::LayoutOverflow("audio bitrate window payload"))?;
        if sample_decode_time > window_start_decode_time.saturating_add(u64::from(timescale)) {
            max_window_payload_bytes = max_window_payload_bytes.max(current_window_payload_bytes);
            window_start_decode_time = sample_decode_time;
            current_window_payload_bytes = 0;
        }
        sample_decode_time = sample_decode_time
            .checked_add(u64::from(duration))
            .ok_or(MuxError::LayoutOverflow("audio decode time"))?;
    }
    if !saw_sample || total_duration == 0 {
        return Ok(Btrt::default());
    }

    let avg_bitrate = total_payload_bytes
        .checked_mul(8)
        .and_then(|bits| bits.checked_mul(u64::from(timescale)))
        .ok_or(MuxError::LayoutOverflow("audio average bitrate"))?
        / total_duration;
    let avg_bitrate = avg_bitrate & !7;

    let max_bitrate = if max_window_payload_bytes == 0 {
        avg_bitrate
    } else {
        max_window_payload_bytes
            .checked_mul(8)
            .ok_or(MuxError::LayoutOverflow("audio maximum bitrate"))?
    };

    Ok(Btrt {
        buffer_size_db,
        max_bitrate: u32::try_from(max_bitrate)
            .map_err(|_| MuxError::LayoutOverflow("audio maximum bitrate"))?,
        avg_bitrate: u32::try_from(avg_bitrate)
            .map_err(|_| MuxError::LayoutOverflow("audio average bitrate"))?,
    })
}

fn import_ivf_video_sync(
    path: &Path,
    codec: MuxRawCodec,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = match codec {
        MuxRawCodec::Av1 => scan_av1_file_sync(path, &spec)?,
        MuxRawCodec::Vp8 => scan_vp8_file_sync(path, &spec)?,
        MuxRawCodec::Vp9 => scan_vp9_file_sync(path, &spec)?,
        MuxRawCodec::Vp10 => scan_vp10_file_sync(path, &spec)?,
        _ => unreachable!("only IVF-backed codecs use this import helper"),
    };
    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: parsed.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name(match codec {
            MuxRawCodec::Av1 => "av1",
            MuxRawCodec::Vp8 => "vp8",
            MuxRawCodec::Vp9 => "vp9",
            MuxRawCodec::Vp10 => "vp10",
            _ => unreachable!("only IVF-backed codecs use this import helper"),
        }),
        mux_policy: direct_ingest_mux_policy(
            match codec {
                MuxRawCodec::Av1 => "av1",
                MuxRawCodec::Vp8 => "vp8",
                MuxRawCodec::Vp9 => "vp9",
                MuxRawCodec::Vp10 => "vp10",
                _ => unreachable!("only IVF-backed codecs use this import helper"),
            },
            MuxTrackKind::Video,
        ),
        width: parsed.width,
        height: parsed.height,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_ivf_video_async(
    path: &Path,
    codec: MuxRawCodec,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = match codec {
        MuxRawCodec::Av1 => scan_av1_file_async(path, &spec).await?,
        MuxRawCodec::Vp8 => scan_vp8_file_async(path, &spec).await?,
        MuxRawCodec::Vp9 => scan_vp9_file_async(path, &spec).await?,
        MuxRawCodec::Vp10 => scan_vp10_file_async(path, &spec).await?,
        _ => unreachable!("only IVF-backed codecs use this import helper"),
    };
    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: parsed.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name(match codec {
            MuxRawCodec::Av1 => "av1",
            MuxRawCodec::Vp8 => "vp8",
            MuxRawCodec::Vp9 => "vp9",
            MuxRawCodec::Vp10 => "vp10",
            _ => unreachable!("only IVF-backed codecs use this import helper"),
        }),
        mux_policy: direct_ingest_mux_policy(
            match codec {
                MuxRawCodec::Av1 => "av1",
                MuxRawCodec::Vp8 => "vp8",
                MuxRawCodec::Vp9 => "vp9",
                MuxRawCodec::Vp10 => "vp10",
                _ => unreachable!("only IVF-backed codecs use this import helper"),
            },
            MuxTrackKind::Video,
        ),
        width: parsed.width,
        height: parsed.height,
        sample_entry_box: parsed.sample_entry_box,
        source_edit_media_time: None,
        sample_roll_distance: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[derive(Clone, Copy)]
pub(in crate::mux) struct SourceFileSpan {
    pub(in crate::mux) source_offset: u64,
    pub(in crate::mux) size: u32,
}

pub(in crate::mux) fn read_exact_at_sync(
    file: &mut File,
    offset: u64,
    buf: &mut [u8],
    spec: &str,
    truncated_message: &'static str,
) -> Result<(), MuxError> {
    file.seek(SeekFrom::Start(offset))?;
    match file.read_exact(buf) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: truncated_message.to_string(),
            })
        }
        Err(error) => Err(MuxError::Io(error)),
    }
}

pub(in crate::mux) fn read_spans_sync(
    file: &mut File,
    spans: &[SourceFileSpan],
    total_size: u32,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let mut bytes = Vec::with_capacity(
        usize::try_from(total_size)
            .map_err(|_| MuxError::LayoutOverflow("packet byte capacity"))?,
    );
    for span in spans {
        let mut chunk = vec![0_u8; usize::try_from(span.size).unwrap()];
        read_exact_at_sync(
            file,
            span.source_offset,
            &mut chunk,
            spec,
            truncated_message,
        )?;
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn absolute_path(path: &Path) -> Result<PathBuf, MuxError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()?.join(path))
}

fn extract_required_single_as_sync<R, T>(
    reader: &mut R,
    parent: &HeaderInfo,
    path: BoxPath,
    name: &'static str,
) -> Result<T, MuxError>
where
    R: Read + Seek,
    T: CodecBox + Clone + 'static,
{
    let boxes = extract_box_as::<_, T>(reader, Some(parent), path)?;
    let [value] = boxes.as_slice() else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: name.to_string(),
            message: format!("expected exactly one {name} box but found {}", boxes.len()),
        });
    };
    Ok(value.clone())
}

fn extract_optional_single_as_sync<R, T>(
    reader: &mut R,
    parent: &HeaderInfo,
    path: BoxPath,
) -> Result<Option<T>, MuxError>
where
    R: Read + Seek,
    T: CodecBox + Clone + 'static,
{
    let boxes = extract_box_as::<_, T>(reader, Some(parent), path)?;
    match boxes.len() {
        0 => Ok(None),
        1 => Ok(Some(boxes[0].clone())),
        _ => Err(MuxError::UnsupportedTrackImport {
            spec: "track".to_string(),
            message: "expected at most one optional box".to_string(),
        }),
    }
}

fn extract_required_single_info_sync<R>(
    reader: &mut R,
    parent: &HeaderInfo,
    path: BoxPath,
    name: &'static str,
) -> Result<HeaderInfo, MuxError>
where
    R: Read + Seek,
{
    let infos = extract_box(reader, Some(parent), path)?;
    let [info] = infos.as_slice() else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: name.to_string(),
            message: format!("expected exactly one {name} box but found {}", infos.len()),
        });
    };
    Ok(*info)
}

#[cfg(feature = "async")]
async fn extract_required_single_as_async<R, T>(
    reader: &mut R,
    parent: &HeaderInfo,
    path: BoxPath,
    name: &'static str,
) -> Result<T, MuxError>
where
    R: AsyncReadSeek,
    T: CodecBox + Clone + 'static,
{
    let boxes = extract_box_as_async::<_, T>(reader, Some(parent), path).await?;
    let [value] = boxes.as_slice() else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: name.to_string(),
            message: format!("expected exactly one {name} box but found {}", boxes.len()),
        });
    };
    Ok(value.clone())
}

#[cfg(feature = "async")]
async fn extract_optional_single_as_async<R, T>(
    reader: &mut R,
    parent: &HeaderInfo,
    path: BoxPath,
) -> Result<Option<T>, MuxError>
where
    R: AsyncReadSeek,
    T: CodecBox + Clone + 'static,
{
    let boxes = extract_box_as_async::<_, T>(reader, Some(parent), path).await?;
    match boxes.len() {
        0 => Ok(None),
        1 => Ok(Some(boxes[0].clone())),
        _ => Err(MuxError::UnsupportedTrackImport {
            spec: "track".to_string(),
            message: "expected at most one optional box".to_string(),
        }),
    }
}

#[cfg(feature = "async")]
async fn extract_required_single_info_async<R>(
    reader: &mut R,
    parent: &HeaderInfo,
    path: BoxPath,
    name: &'static str,
) -> Result<HeaderInfo, MuxError>
where
    R: AsyncReadSeek,
{
    let infos = extract_box_async(reader, Some(parent), path).await?;
    let [info] = infos.as_slice() else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: name.to_string(),
            message: format!("expected exactly one {name} box but found {}", infos.len()),
        });
    };
    Ok(*info)
}

fn expand_sample_sizes(stsz: &Stsz, path: &Path, track_id: u32) -> Result<Vec<u32>, MuxError> {
    if stsz.sample_size != 0 {
        return Ok(vec![stsz.sample_size; stsz.sample_count as usize]);
    }
    if stsz.entry_size.len() != stsz.sample_count as usize {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {track_id} has stsz sample_count {} but {} explicit entry sizes",
                stsz.sample_count,
                stsz.entry_size.len()
            ),
        });
    }
    stsz.entry_size
        .iter()
        .map(|size| {
            u32::try_from(*size).map_err(|_| MuxError::UnsupportedTrackImport {
                spec: path.display().to_string(),
                message: format!("track {track_id} has a sample size that does not fit in u32"),
            })
        })
        .collect()
}

fn expand_sample_durations(
    stts: &Stts,
    sample_count: usize,
    path: &Path,
    track_id: u32,
) -> Result<Vec<u32>, MuxError> {
    let mut durations = Vec::with_capacity(sample_count);
    for entry in &stts.entries {
        for _ in 0..entry.sample_count {
            durations.push(entry.sample_delta);
        }
    }
    if durations.len() != sample_count {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {track_id} resolves {} durations from stts but has {sample_count} samples",
                durations.len()
            ),
        });
    }
    Ok(durations)
}

fn expand_composition_offsets(
    ctts: Option<&Ctts>,
    sample_count: usize,
    path: &Path,
    track_id: u32,
) -> Result<Vec<i32>, MuxError> {
    let Some(ctts) = ctts else {
        return Ok(vec![0; sample_count]);
    };
    let mut offsets = Vec::with_capacity(sample_count);
    for (entry_index, entry) in ctts.entries.iter().enumerate() {
        for _ in 0..entry.sample_count {
            offsets.push(i32::try_from(ctts.sample_offset(entry_index)).map_err(|_| {
                MuxError::UnsupportedTrackImport {
                    spec: path.display().to_string(),
                    message: format!("track {track_id} uses a composition offset outside i32"),
                }
            })?);
        }
    }
    if offsets.len() != sample_count {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {track_id} resolves {} composition offsets but has {sample_count} samples",
                offsets.len()
            ),
        });
    }
    Ok(offsets)
}

fn select_chunk_offsets(
    stco: Option<&Stco>,
    co64: Option<&Co64>,
    path: &Path,
    track_id: u32,
) -> Result<Vec<u64>, MuxError> {
    match (stco, co64) {
        (Some(_), Some(_)) => Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!("track {track_id} carries both stco and co64"),
        }),
        (Some(stco), None) => Ok(stco.chunk_offset.clone()),
        (None, Some(co64)) => Ok(co64.chunk_offset.clone()),
        (None, None) => Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!("track {track_id} is missing stco/co64 chunk offsets"),
        }),
    }
}

fn expand_sample_offsets(
    stsc: &Stsc,
    sample_sizes: &[u32],
    chunk_offsets: &[u64],
    path: &Path,
    track_id: u32,
) -> Result<Vec<u64>, MuxError> {
    if stsc.entries.is_empty() {
        if sample_sizes.is_empty() && chunk_offsets.is_empty() {
            return Ok(Vec::new());
        }
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!("track {track_id} has no stsc entries"),
        });
    }

    let mut mappings = Vec::with_capacity(chunk_offsets.len());
    for (index, entry) in stsc.entries.iter().enumerate() {
        if entry.first_chunk == 0 || entry.sample_description_index != 1 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: path.display().to_string(),
                message: format!(
                    "track {track_id} uses unsupported stsc entry first_chunk={} sample_description_index={}",
                    entry.first_chunk, entry.sample_description_index
                ),
            });
        }
        let next_first_chunk = stsc
            .entries
            .get(index + 1)
            .map(|next| next.first_chunk)
            .unwrap_or(
                u32::try_from(chunk_offsets.len())
                    .map_err(|_| MuxError::LayoutOverflow("chunk count"))?
                    .saturating_add(1),
            );
        if next_first_chunk <= entry.first_chunk {
            return Err(MuxError::UnsupportedTrackImport {
                spec: path.display().to_string(),
                message: format!("track {track_id} has descending stsc first_chunk values"),
            });
        }
        for _ in entry.first_chunk..next_first_chunk {
            mappings.push(entry.samples_per_chunk);
        }
    }
    if mappings.len() != chunk_offsets.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {track_id} resolved {} chunk mappings for {} chunk offsets",
                mappings.len(),
                chunk_offsets.len()
            ),
        });
    }

    let mut sample_offsets = Vec::with_capacity(sample_sizes.len());
    let mut sample_index = 0_usize;
    for (chunk_offset, samples_per_chunk) in chunk_offsets.iter().zip(mappings) {
        let mut running_offset = *chunk_offset;
        for _ in 0..samples_per_chunk {
            let Some(sample_size) = sample_sizes.get(sample_index).copied() else {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: path.display().to_string(),
                    message: format!(
                        "track {track_id} resolved more chunk samples than stsz entries"
                    ),
                });
            };
            sample_offsets.push(running_offset);
            running_offset = running_offset
                .checked_add(u64::from(sample_size))
                .ok_or(MuxError::LayoutOverflow("sample offset"))?;
            sample_index += 1;
        }
    }
    if sample_index != sample_sizes.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: path.display().to_string(),
            message: format!(
                "track {track_id} resolved {sample_index} sample offsets for {} sample sizes",
                sample_sizes.len()
            ),
        });
    }
    Ok(sample_offsets)
}

fn expand_sync_samples(
    stss: Option<&Stss>,
    sample_entry_type: FourCc,
    sample_count: usize,
    path: &Path,
    track_id: u32,
) -> Result<Vec<bool>, MuxError> {
    let Some(stss) = stss else {
        return Ok(vec![true; sample_count]);
    };
    if stss.entry_count == 0
        && matches!(
            sample_entry_type,
            value if value == FourCc::from_bytes(*b"vp08")
                || value == FourCc::from_bytes(*b"vp09")
        )
    {
        return Ok(vec![true; sample_count]);
    }
    let mut sync = vec![false; sample_count];
    for sample_number in &stss.sample_number {
        let index = usize::try_from(sample_number.saturating_sub(1)).map_err(|_| {
            MuxError::UnsupportedTrackImport {
                spec: path.display().to_string(),
                message: format!(
                    "track {track_id} exposes an stss entry that does not fit in usize"
                ),
            }
        })?;
        let Some(entry) = sync.get_mut(index) else {
            return Err(MuxError::UnsupportedTrackImport {
                spec: path.display().to_string(),
                message: format!(
                    "track {track_id} exposes an stss sample number outside its sample count"
                ),
            });
        };
        *entry = true;
    }
    Ok(sync)
}

fn decode_mdhd_language(encoded: [u8; 3]) -> [u8; 3] {
    let mut decoded = [b'u', b'n', b'd'];
    for (index, value) in encoded.into_iter().enumerate() {
        decoded[index] = if (1..=26).contains(&value) {
            value + b'`'
        } else {
            b"und"[index]
        };
    }
    decoded
}

fn scale_track_time_to_movie(
    track_id: u32,
    value: i64,
    track_timescale: u32,
    movie_timescale: u32,
) -> Result<i64, MuxError> {
    if track_timescale == 0 || movie_timescale == 0 {
        return Err(MuxError::InvalidTrackTimescale { track_id });
    }
    let sign = value.signum();
    let magnitude = value.unsigned_abs();
    let scaled = magnitude
        .checked_mul(u64::from(movie_timescale))
        .ok_or(MuxError::LayoutOverflow("track time normalization"))?;
    if scaled % u64::from(track_timescale) != 0 {
        return Err(MuxError::IncompatibleTrackTiming {
            track_id,
            track_timescale,
            movie_timescale,
            value,
        });
    }
    let normalized = scaled / u64::from(track_timescale);
    i64::try_from(normalized)
        .map_err(|_| MuxError::LayoutOverflow("track time normalization"))
        .map(|normalized| normalized * sign)
}

fn track_times_fit_movie_timescale(track: &ImportedTrack, movie_timescale: u32) -> bool {
    if track.timescale == 0 || movie_timescale == 0 {
        return false;
    }
    track.samples.iter().all(|sample| {
        can_scale_track_time_to_movie(i64::from(sample.duration), track.timescale, movie_timescale)
            && can_scale_track_time_to_movie(
                i64::from(sample.composition_time_offset),
                track.timescale,
                movie_timescale,
            )
    })
}

fn can_scale_track_time_to_movie(value: i64, track_timescale: u32, movie_timescale: u32) -> bool {
    let magnitude = value.unsigned_abs();
    magnitude
        .checked_mul(u64::from(movie_timescale))
        .is_some_and(|scaled| scaled % u64::from(track_timescale) == 0)
}

fn lcm_u32(left: u32, right: u32) -> Option<u32> {
    let gcd = gcd_u32(left, right);
    left.checked_div(gcd)?.checked_mul(right)
}

const fn gcd_u32(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let next = left % right;
        left = right;
        right = next;
    }
    left
}

fn probe_file_config_sync<R>(reader: &mut R) -> Result<MuxFileConfig, MuxError>
where
    R: Read + Seek,
{
    use crate::probe::probe_with_options;
    let summary = probe_with_options(reader, crate::probe::ProbeOptions::lightweight())?;
    let mut config = MuxFileConfig::new(summary.timescale.max(1))
        .with_major_brand(summary.major_brand)
        .with_minor_version(summary.minor_version);
    for brand in summary.compatible_brands {
        config.add_compatible_brand(brand);
    }
    Ok(config)
}

#[cfg(feature = "async")]
async fn probe_file_config_async<R>(reader: &mut R) -> Result<MuxFileConfig, MuxError>
where
    R: AsyncReadSeek,
{
    use crate::probe::probe_with_options_async;
    let summary =
        probe_with_options_async(reader, crate::probe::ProbeOptions::lightweight()).await?;
    let mut config = MuxFileConfig::new(summary.timescale.max(1))
        .with_major_brand(summary.major_brand)
        .with_minor_version(summary.minor_version);
    for brand in summary.compatible_brands {
        config.add_compatible_brand(brand);
    }
    Ok(config)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn read_exact_at_async(
    file: &mut TokioFile,
    offset: u64,
    buf: &mut [u8],
    spec: &str,
    truncated_message: &'static str,
) -> Result<(), MuxError> {
    file.seek(SeekFrom::Start(offset)).await?;
    match file.read_exact(buf).await {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: truncated_message.to_string(),
            })
        }
        Err(error) => Err(MuxError::Io(error)),
    }
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn read_spans_async(
    file: &mut TokioFile,
    spans: &[SourceFileSpan],
    total_size: u32,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let mut bytes = Vec::with_capacity(
        usize::try_from(total_size)
            .map_err(|_| MuxError::LayoutOverflow("packet byte capacity"))?,
    );
    for span in spans {
        let mut chunk = vec![0_u8; usize::try_from(span.size).unwrap()];
        read_exact_at_async(
            file,
            span.source_offset,
            &mut chunk,
            spec,
            truncated_message,
        )
        .await?;
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
