use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
#[cfg(feature = "async")]
use std::pin::Pin;
#[cfg(feature = "async")]
use std::task::{Context, Poll};

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWriteExt, BufWriter, ReadBuf,
};

use crate::FourCc;
#[cfg(feature = "async")]
use crate::async_io::AsyncReadSeek;
use crate::bitio::BitReader;
use crate::boxes::AnyTypeBox;
use crate::boxes::etsi_ts_102_366::{Dac3, Dec3, Ec3Substream};
use crate::boxes::etsi_ts_103_190::Dac4;
use crate::boxes::iso14496_12::{
    AVCDecoderConfiguration, AVCParameterSet, AudioSampleEntry, Co64, Ctts, Elst,
    HEVCDecoderConfiguration, HEVCNalu, HEVCNaluArray, Hdlr, Mdhd, SampleEntry, Stco, Stsc, Stss,
    Stsz, Stts, TFHD_BASE_DATA_OFFSET_PRESENT, TFHD_DEFAULT_BASE_IS_MOOF,
    TFHD_DEFAULT_SAMPLE_DURATION_PRESENT, TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT,
    TFHD_DEFAULT_SAMPLE_SIZE_PRESENT, TRUN_DATA_OFFSET_PRESENT, TRUN_FIRST_SAMPLE_FLAGS_PRESENT,
    TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT,
    TRUN_SAMPLE_FLAGS_PRESENT, TRUN_SAMPLE_SIZE_PRESENT, Tfhd, Tkhd, Trex, Trun, VisualSampleEntry,
};
use crate::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    Esds,
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

use super::mp4::write_fragmented_mp4_mux;
#[cfg(feature = "async")]
use super::mp4::write_fragmented_mp4_mux_async;
#[cfg(feature = "async")]
use super::write_mp4_mux_async;
use super::{
    MuxDurationBoundaryKind, MuxError, MuxFileConfig, MuxInterleavePolicy, MuxMp4TrackSelector,
    MuxOutputLayout, MuxRawCodec, MuxRequest, MuxStagedMediaItem, MuxTrackConfig, MuxTrackKind,
    MuxTrackParameter, MuxTrackSpec, TrackCoordinationDirective,
    build_duration_chunk_sample_counts, build_duration_chunk_sample_counts_with_start_time,
    build_sync_aligned_segment_chunk_sample_counts, plan_staged_media_items_with_coordination,
    write_mp4_mux,
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
const ENCV: FourCc = FourCc::from_bytes(*b"encv");
const ENCA: FourCc = FourCc::from_bytes(*b"enca");
const AV01: FourCc = FourCc::from_bytes(*b"av01");
const VP08: FourCc = FourCc::from_bytes(*b"vp08");
const NON_KEY_SAMPLE_FLAGS: u32 = 0x0001_0000;
const VP09: FourCc = FourCc::from_bytes(*b"vp09");
const DVHE: FourCc = FourCc::from_bytes(*b"dvhe");
const DVH1: FourCc = FourCc::from_bytes(*b"dvh1");
const ALAC: FourCc = FourCc::from_bytes(*b"alac");
const DTSC: FourCc = FourCc::from_bytes(*b"dtsc");
const DTSE: FourCc = FourCc::from_bytes(*b"dtse");
const DTSH: FourCc = FourCc::from_bytes(*b"dtsh");
const DTSL: FourCc = FourCc::from_bytes(*b"dtsl");
const DTSM: FourCc = FourCc::from_bytes(*b"dtsm");
const DTSX: FourCc = FourCc::from_bytes(*b"dtsx");
const DDTS: FourCc = FourCc::from_bytes(*b"ddts");
const FLAC_ENTRY: FourCc = FourCc::from_bytes(*b"fLaC");
const OPUS_ENTRY: FourCc = FourCc::from_bytes(*b"Opus");
const IAMF_ENTRY: FourCc = FourCc::from_bytes(*b"iamf");
const MHA1: FourCc = FourCc::from_bytes(*b"mha1");
const MHM1: FourCc = FourCc::from_bytes(*b"mhm1");
const DDTS_EXTRA_DATA: [u8; 7] = [0xe4, 0x7c, 0x00, 0x04, 0x00, 0x0f, 0x00];

/// Opens the requested track specs, validates the narrowed mux request shape, and writes one
/// output MP4 file to `output_path`.
///
/// This task-level helper is the sync programmatic companion to the `mp4forge mux` CLI surface.
/// It accepts the same widened repeated-track grammar as the CLI, preserves the first MP4 input
/// as the authoritative merge source when every input is itself an MP4, and rejects unsupported
/// multi-video or duration-mode combinations explicitly.
pub fn mux_to_path<P>(request: &MuxRequest, output_path: P) -> Result<(), MuxError>
where
    P: AsRef<Path>,
{
    let prepared = prepare_request_sync(request, output_path.as_ref())?;
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
            &prepared.fragmented_edit_media_times,
            &prepared.plan,
        )?,
    }
    writer.flush()?;
    Ok(())
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
    let prepared = prepare_request_async(request, output_path.as_ref()).await?;
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
                &prepared.fragmented_edit_media_times,
                &prepared.plan,
            )
            .await?
        }
    }
    writer.flush().await?;
    Ok(())
}

struct PreparedMuxRequest {
    output_layout: MuxOutputLayout,
    file_config: MuxFileConfig,
    track_configs: Vec<MuxTrackConfig>,
    fragmented_single_sidx_reference: bool,
    fragmented_edit_media_times: Vec<Option<u64>>,
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
    TransformedAnnexB(TransformedAnnexBSourceSpec),
}

#[derive(Clone)]
struct TransformedAnnexBSourceSpec {
    path: PathBuf,
    segments: Vec<TransformedAnnexBSegment>,
    total_size: u64,
}

#[derive(Clone)]
struct TransformedAnnexBSegment {
    logical_offset: u64,
    data: TransformedAnnexBSegmentData,
}

#[derive(Clone)]
enum TransformedAnnexBSegmentData {
    Prefix([u8; 4]),
    FileRange { source_offset: u64, size: u32 },
}

impl TransformedAnnexBSegment {
    fn logical_size(&self) -> u64 {
        match &self.data {
            TransformedAnnexBSegmentData::Prefix(_) => 4,
            TransformedAnnexBSegmentData::FileRange { size, .. } => u64::from(*size),
        }
    }

    fn logical_end(&self) -> u64 {
        self.logical_offset + self.logical_size()
    }
}

fn find_transformed_segment_index(
    segments: &[TransformedAnnexBSegment],
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
            "invalid seek before start of transformed mux source",
        ));
    }
    u64::try_from(next).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid seek target for transformed mux source",
        )
    })
}

struct SyncMuxSource {
    inner: SyncMuxSourceInner,
}

enum SyncMuxSourceInner {
    File(File),
    TransformedAnnexB(TransformedSyncMuxSource),
}

struct TransformedSyncMuxSource {
    file: File,
    segments: Vec<TransformedAnnexBSegment>,
    total_size: u64,
    position: u64,
    file_position: Option<u64>,
}

impl SyncMuxSource {
    fn open(spec: &SourceSpec) -> Result<Self, MuxError> {
        let inner = match spec {
            SourceSpec::File(path) => SyncMuxSourceInner::File(File::open(path)?),
            SourceSpec::TransformedAnnexB(spec) => {
                SyncMuxSourceInner::TransformedAnnexB(TransformedSyncMuxSource {
                    file: File::open(&spec.path)?,
                    segments: spec.segments.clone(),
                    total_size: spec.total_size,
                    position: 0,
                    file_position: None,
                })
            }
        };
        Ok(Self { inner })
    }
}

impl TransformedSyncMuxSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.position >= self.total_size {
            return Ok(0);
        }

        let mut written = 0usize;
        while written < buf.len() && self.position < self.total_size {
            let Some(segment_index) = find_transformed_segment_index(&self.segments, self.position)
            else {
                break;
            };
            let segment = &self.segments[segment_index];
            let segment_offset =
                usize::try_from(self.position - segment.logical_offset).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "logical offset overflow")
                })?;
            match &segment.data {
                TransformedAnnexBSegmentData::Prefix(prefix) => {
                    let available = prefix.len().saturating_sub(segment_offset);
                    let to_copy = available.min(buf.len() - written);
                    buf[written..written + to_copy]
                        .copy_from_slice(&prefix[segment_offset..segment_offset + to_copy]);
                    written += to_copy;
                    self.position += u64::try_from(to_copy).unwrap();
                }
                TransformedAnnexBSegmentData::FileRange {
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
                            "truncated transformed mux source input",
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
            SyncMuxSourceInner::TransformedAnnexB(source) => source.read(buf),
        }
    }
}

impl Seek for SyncMuxSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match &mut self.inner {
            SyncMuxSourceInner::File(file) => file.seek(pos),
            SyncMuxSourceInner::TransformedAnnexB(source) => source.seek(pos),
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
    TransformedAnnexB(TransformedAsyncMuxSource),
}

#[cfg(feature = "async")]
struct TransformedAsyncMuxSource {
    file: TokioFile,
    segments: Vec<TransformedAnnexBSegment>,
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
            SourceSpec::TransformedAnnexB(spec) => {
                AsyncMuxSourceInner::TransformedAnnexB(TransformedAsyncMuxSource {
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
impl TransformedAsyncMuxSource {
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

        let Some(segment_index) = find_transformed_segment_index(&self.segments, self.position)
        else {
            return Poll::Ready(Ok(()));
        };
        let segment = &self.segments[segment_index];
        let segment_offset = usize::try_from(self.position - segment.logical_offset)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "logical offset overflow"))?;
        match &segment.data {
            TransformedAnnexBSegmentData::Prefix(prefix) => {
                let available = prefix.len().saturating_sub(segment_offset);
                let to_copy = available.min(buf.remaining());
                buf.put_slice(&prefix[segment_offset..segment_offset + to_copy]);
                self.position += u64::try_from(to_copy).unwrap();
                Poll::Ready(Ok(()))
            }
            TransformedAnnexBSegmentData::FileRange {
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
                                "truncated transformed mux source input",
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
            AsyncMuxSourceInner::TransformedAnnexB(source) => source.poll_read_internal(cx, buf),
        }
    }
}

#[cfg(feature = "async")]
impl AsyncSeek for AsyncMuxSource {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> io::Result<()> {
        match &mut self.inner {
            AsyncMuxSourceInner::File(file) => Pin::new(file).start_seek(position),
            AsyncMuxSourceInner::TransformedAnnexB(source) => source.start_seek(position),
        }
    }

    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        match &mut self.inner {
            AsyncMuxSourceInner::File(file) => Pin::new(file).poll_complete(cx),
            AsyncMuxSourceInner::TransformedAnnexB(source) => source.poll_complete(cx),
        }
    }
}

struct ImportedTrack {
    kind: MuxTrackKind,
    timescale: u32,
    language: [u8; 3],
    handler_name: String,
    width: u16,
    height: u16,
    sample_entry_box: Vec<u8>,
    source_edit_media_time: Option<u64>,
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

#[derive(Clone, Copy)]
struct StagedSample {
    data_offset: u64,
    data_size: u32,
    duration: u32,
    composition_time_offset: i32,
    is_sync_sample: bool,
}

#[derive(Clone)]
struct TrackCandidate {
    track_id: u32,
    kind: MuxTrackKind,
    timescale: u32,
    language: [u8; 3],
    handler_name: String,
    width: u16,
    height: u16,
    sample_entry_box: Vec<u8>,
    source_edit_media_time: Option<u64>,
    samples: Vec<CandidateSample>,
}

#[derive(Clone, Copy)]
struct CandidateSample {
    source_index: usize,
    data_offset: u64,
    data_size: u32,
    duration: u32,
    composition_time_offset: i32,
    is_sync_sample: bool,
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

    let all_mp4_inputs = request
        .tracks()
        .iter()
        .all(|track| matches!(track, MuxTrackSpec::Mp4 { .. }));
    let mut sources = SourceCatalog::default();
    let mut mp4_cache = BTreeMap::<PathBuf, Mp4SourceMetadata>::new();
    let mut imported_tracks = Vec::new();
    let mut authority_file_config = None::<MuxFileConfig>;

    for track in request.tracks() {
        match track {
            MuxTrackSpec::Raw {
                codec,
                path,
                parameters,
            } => {
                let spec = display_track_spec(track);
                imported_tracks.push(import_raw_track_sync(
                    path,
                    *codec,
                    parameters,
                    spec,
                    &mut sources,
                )?);
            }
            MuxTrackSpec::Mp4 { path, selector } => {
                let spec = display_track_spec(track);
                let metadata = load_mp4_source_sync(path, &mut mp4_cache, &mut sources)?;
                if all_mp4_inputs && authority_file_config.is_none() {
                    authority_file_config = Some(metadata.file_config.clone());
                }
                imported_tracks.push(select_mp4_track(metadata, *selector, spec)?);
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

    let all_mp4_inputs = request
        .tracks()
        .iter()
        .all(|track| matches!(track, MuxTrackSpec::Mp4 { .. }));
    let mut sources = SourceCatalog::default();
    let mut mp4_cache = BTreeMap::<PathBuf, Mp4SourceMetadata>::new();
    let mut imported_tracks = Vec::new();
    let mut authority_file_config = None::<MuxFileConfig>;

    for track in request.tracks() {
        match track {
            MuxTrackSpec::Raw {
                codec,
                path,
                parameters,
            } => {
                let spec = display_track_spec(track);
                imported_tracks.push(
                    import_raw_track_async(path, *codec, parameters, spec, &mut sources).await?,
                );
            }
            MuxTrackSpec::Mp4 { path, selector } => {
                let spec = display_track_spec(track);
                let metadata = load_mp4_source_async(path, &mut mp4_cache, &mut sources).await?;
                if all_mp4_inputs && authority_file_config.is_none() {
                    authority_file_config = Some(metadata.file_config.clone());
                }
                imported_tracks.push(select_mp4_track(metadata, *selector, spec)?);
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

    let movie_timescale = choose_movie_timescale(&imported_tracks, authority_file_config.as_ref())?;
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

    let mut staged_items = Vec::new();
    let mut track_configs = Vec::new();
    let mut fragmented_edit_media_times = Vec::new();
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
        .with_handler_name(imported_track.handler_name.clone());
        track_configs.push(config);
        fragmented_edit_media_times.push(imported_track.source_edit_media_time);
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
        fragmented_edit_media_times,
        plan,
        source_specs: sources.specs,
    })
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

    fn add_transformed_annex_b(
        &mut self,
        mut spec: TransformedAnnexBSourceSpec,
    ) -> Result<usize, MuxError> {
        spec.path = absolute_path(&spec.path)?;
        let index = self.specs.len();
        self.specs.push(SourceSpec::TransformedAnnexB(spec));
        Ok(index)
    }
}

struct Mp4SourceMetadata {
    file_config: MuxFileConfig,
    tracks: Vec<TrackCandidate>,
}

fn load_mp4_source_sync<'a>(
    path: &Path,
    cache: &'a mut BTreeMap<PathBuf, Mp4SourceMetadata>,
    sources: &mut SourceCatalog,
) -> Result<&'a Mp4SourceMetadata, MuxError> {
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
    cache: &'a mut BTreeMap<PathBuf, Mp4SourceMetadata>,
    sources: &mut SourceCatalog,
) -> Result<&'a Mp4SourceMetadata, MuxError> {
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

fn parse_mp4_source_sync<R>(
    path: &Path,
    source_index: usize,
    reader: &mut R,
) -> Result<Mp4SourceMetadata, MuxError>
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
    Ok(Mp4SourceMetadata {
        file_config,
        tracks,
    })
}

#[cfg(feature = "async")]
async fn parse_mp4_source_async<R>(
    path: &Path,
    source_index: usize,
    reader: &mut R,
) -> Result<Mp4SourceMetadata, MuxError>
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
    Ok(Mp4SourceMetadata {
        file_config,
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
    metadata: &Mp4SourceMetadata,
    selector: MuxMp4TrackSelector,
    spec: String,
) -> Result<ImportedTrack, MuxError> {
    let selected = match selector {
        MuxMp4TrackSelector::Video => metadata.tracks.iter().find(|track| track.kind.is_video()),
        MuxMp4TrackSelector::Audio { occurrence } => metadata
            .tracks
            .iter()
            .filter(|track| track.kind.is_audio())
            .nth(usize::try_from(occurrence.saturating_sub(1)).unwrap_or(usize::MAX)),
        MuxMp4TrackSelector::Text { occurrence } => metadata
            .tracks
            .iter()
            .filter(|track| track.kind.is_textual())
            .nth(usize::try_from(occurrence.saturating_sub(1)).unwrap_or(usize::MAX)),
        MuxMp4TrackSelector::TrackId { track_id } => metadata
            .tracks
            .iter()
            .find(|track| track.track_id == track_id),
    }
    .ok_or_else(|| MuxError::MissingTrackSelection { spec: spec.clone() })?;

    Ok(ImportedTrack {
        kind: selected.kind,
        timescale: selected.timescale,
        language: selected.language,
        handler_name: selected.handler_name.clone(),
        width: selected.width,
        height: selected.height,
        sample_entry_box: selected.sample_entry_box.clone(),
        source_edit_media_time: selected.source_edit_media_time,
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
        SUBT => MuxTrackKind::Subtitle,
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
    let sync_samples = expand_sync_samples(stss.as_ref(), sample_sizes.len(), path, tkhd.track_id)?;

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
            match kind {
                MuxTrackKind::Audio => "SoundHandler".to_string(),
                MuxTrackKind::Video => "VideoHandler".to_string(),
                MuxTrackKind::Text => "TextHandler".to_string(),
                MuxTrackKind::Subtitle => "SubtitleHandler".to_string(),
            }
        } else {
            hdlr.name
        },
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

fn import_raw_aac_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_adts_file_sync(path, &spec)?;
    let sample_entry_box = build_aac_sample_entry_box(
        parsed.audio_object_type,
        parsed.sampling_frequency_index,
        parsed.channel_configuration,
        parsed.sample_rate,
    )?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: "SoundHandler".to_string(),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
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
    let sample_entry_box = build_aac_sample_entry_box(
        parsed.audio_object_type,
        parsed.sampling_frequency_index,
        parsed.channel_configuration,
        parsed.sample_rate,
    )?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: "SoundHandler".to_string(),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_h264_sync(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let staged = stage_annex_b_h264_sync(path, &spec)?;
    let source_index = sources.add_transformed_annex_b(staged.transformed_source)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: staged.timescale,
        language: *b"und",
        handler_name: "VideoHandler".to_string(),
        width: staged.width,
        height: staged.height,
        sample_entry_box: staged.sample_entry_box,
        source_edit_media_time: None,
        samples: imported_samples_from_staged(staged.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_h264_async(
    path: &Path,
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let staged = stage_annex_b_h264_async(path, &spec).await?;
    let source_index = sources.add_transformed_annex_b(staged.transformed_source)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: staged.timescale,
        language: *b"und",
        handler_name: "VideoHandler".to_string(),
        width: staged.width,
        height: staged.height,
        sample_entry_box: staged.sample_entry_box,
        source_edit_media_time: None,
        samples: imported_samples_from_staged(staged.samples, source_index),
    })
}

fn import_raw_h265_sync(
    path: &Path,
    parameters: &[MuxTrackParameter],
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let staged = stage_annex_b_h265_sync(path, parameters, &spec)?;
    let source_index = sources.add_transformed_annex_b(staged.transformed_source)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: staged.timescale,
        language: *b"und",
        handler_name: "VideoHandler".to_string(),
        width: staged.width,
        height: staged.height,
        sample_entry_box: staged.sample_entry_box,
        source_edit_media_time: None,
        samples: imported_samples_from_staged(staged.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_h265_async(
    path: &Path,
    parameters: &[MuxTrackParameter],
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let staged = stage_annex_b_h265_async(path, parameters, &spec).await?;
    let source_index = sources.add_transformed_annex_b(staged.transformed_source)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale: staged.timescale,
        language: *b"und",
        handler_name: "VideoHandler".to_string(),
        width: staged.width,
        height: staged.height,
        sample_entry_box: staged.sample_entry_box,
        source_edit_media_time: None,
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
    let sample_entry_box = build_mp3_sample_entry_box(parsed.sample_rate, parsed.channel_count)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: "SoundHandler".to_string(),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
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
    let sample_entry_box = build_mp3_sample_entry_box(parsed.sample_rate, parsed.channel_count)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: "SoundHandler".to_string(),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
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
    let sample_entry_box = build_ac3_sample_entry_box(&parsed)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: "SoundHandler".to_string(),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
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
    let sample_entry_box = build_ac3_sample_entry_box(&parsed)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: "SoundHandler".to_string(),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
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
    let sample_entry_box = build_eac3_sample_entry_box(&parsed)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: "SoundHandler".to_string(),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
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
    let sample_entry_box = build_eac3_sample_entry_box(&parsed)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: "SoundHandler".to_string(),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn import_raw_ac4_sync(
    path: &Path,
    parameters: &[MuxTrackParameter],
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_ac4_file_sync(path, parameters, &spec)?;
    let sample_entry_box = build_ac4_sample_entry_box(&parsed)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: "SoundHandler".to_string(),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

#[cfg(feature = "async")]
async fn import_raw_ac4_async(
    path: &Path,
    parameters: &[MuxTrackParameter],
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let source_index = sources.add_file(path)?;
    let parsed = scan_ac4_file_async(path, parameters, &spec).await?;
    let sample_entry_box = build_ac4_sample_entry_box(&parsed)?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: parsed.sample_rate,
        language: *b"und",
        handler_name: "SoundHandler".to_string(),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
        samples: imported_samples_from_staged(parsed.samples, source_index),
    })
}

fn choose_movie_timescale(
    imported_tracks: &[ImportedTrack],
    authority_file_config: Option<&MuxFileConfig>,
) -> Result<u32, MuxError> {
    let mut common = 1_u32;
    for track in imported_tracks {
        common = lcm_u32(common, track.timescale)
            .ok_or(MuxError::LayoutOverflow("movie timescale selection"))?;
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
        return MuxFileConfig::new(movie_timescale);
    };

    let mut config = MuxFileConfig::new(movie_timescale)
        .with_major_brand(authority_file_config.major_brand())
        .with_minor_version(authority_file_config.minor_version());
    for brand in authority_file_config.compatible_brands() {
        config.add_compatible_brand(*brand);
    }
    config
}

fn validate_request_shape(request: &MuxRequest, output_path: &Path) -> Result<(), MuxError> {
    if request.tracks().is_empty() {
        return Err(MuxError::MissingTrackSpecs);
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
        .filter(|track| match track {
            MuxTrackSpec::Raw { codec, .. } => codec.is_video(),
            MuxTrackSpec::Mp4 {
                selector: MuxMp4TrackSelector::Video,
                ..
            } => true,
            _ => false,
        })
        .count();
    if video_count > 1 {
        return Err(MuxError::MultipleVideoTracks { count: video_count });
    }

    let output_absolute = absolute_path(output_path)?;
    for track in request.tracks() {
        let input_absolute = absolute_path(track.path())?;
        if input_absolute == output_absolute {
            return Err(MuxError::OutputPathConflict {
                output: output_absolute,
                input: input_absolute,
            });
        }
    }
    Ok(())
}

fn display_track_spec(track: &MuxTrackSpec) -> String {
    match track {
        MuxTrackSpec::Raw {
            codec,
            path,
            parameters,
        } => {
            let mut spec = format!("{}:{}", codec.prefix(), path.display());
            if !parameters.is_empty() {
                spec.push('#');
                spec.push_str(&format_track_parameters(parameters));
            }
            spec
        }
        MuxTrackSpec::Mp4 { path, selector } => {
            format!("{}#{}", path.display(), format_mp4_selector(*selector))
        }
    }
}

fn format_track_parameters(parameters: &[MuxTrackParameter]) -> String {
    parameters
        .iter()
        .map(|parameter| format!("{}={}", parameter.name(), parameter.value()))
        .collect::<Vec<_>>()
        .join(",")
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

fn import_raw_track_sync(
    path: &Path,
    codec: MuxRawCodec,
    parameters: &[MuxTrackParameter],
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    match codec {
        MuxRawCodec::H264 => {
            validate_no_raw_track_parameters(codec, parameters, &spec)?;
            import_raw_h264_sync(path, spec, sources)
        }
        MuxRawCodec::H265 => import_raw_h265_sync(path, parameters, spec, sources),
        MuxRawCodec::Av1 | MuxRawCodec::Vp8 | MuxRawCodec::Vp9 => {
            import_parameterized_raw_video_sync(path, codec, parameters, spec, sources)
        }
        MuxRawCodec::Aac => {
            validate_no_raw_track_parameters(codec, parameters, &spec)?;
            import_raw_aac_sync(path, spec, sources)
        }
        MuxRawCodec::Mp3 => {
            validate_no_raw_track_parameters(codec, parameters, &spec)?;
            import_raw_mp3_sync(path, spec, sources)
        }
        MuxRawCodec::Ac3 => {
            validate_no_raw_track_parameters(codec, parameters, &spec)?;
            import_raw_ac3_sync(path, spec, sources)
        }
        MuxRawCodec::Eac3 => {
            validate_no_raw_track_parameters(codec, parameters, &spec)?;
            import_raw_eac3_sync(path, spec, sources)
        }
        MuxRawCodec::Ac4 => import_raw_ac4_sync(path, parameters, spec, sources),
        MuxRawCodec::Alac
        | MuxRawCodec::Dtsc
        | MuxRawCodec::Dtse
        | MuxRawCodec::Dtsh
        | MuxRawCodec::Dtsl
        | MuxRawCodec::Dtsm
        | MuxRawCodec::Dtsx
        | MuxRawCodec::Flac
        | MuxRawCodec::Opus
        | MuxRawCodec::Iamf
        | MuxRawCodec::Mha1
        | MuxRawCodec::Mhm1 => {
            import_parameterized_raw_audio_sync(path, codec, parameters, spec, sources)
        }
    }
}

#[cfg(feature = "async")]
async fn import_raw_track_async(
    path: &Path,
    codec: MuxRawCodec,
    parameters: &[MuxTrackParameter],
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    match codec {
        MuxRawCodec::H264 => {
            validate_no_raw_track_parameters(codec, parameters, &spec)?;
            import_raw_h264_async(path, spec, sources).await
        }
        MuxRawCodec::H265 => import_raw_h265_async(path, parameters, spec, sources).await,
        MuxRawCodec::Av1 | MuxRawCodec::Vp8 | MuxRawCodec::Vp9 => {
            import_parameterized_raw_video_async(path, codec, parameters, spec, sources).await
        }
        MuxRawCodec::Aac => {
            validate_no_raw_track_parameters(codec, parameters, &spec)?;
            import_raw_aac_async(path, spec, sources).await
        }
        MuxRawCodec::Mp3 => {
            validate_no_raw_track_parameters(codec, parameters, &spec)?;
            import_raw_mp3_async(path, spec, sources).await
        }
        MuxRawCodec::Ac3 => {
            validate_no_raw_track_parameters(codec, parameters, &spec)?;
            import_raw_ac3_async(path, spec, sources).await
        }
        MuxRawCodec::Eac3 => {
            validate_no_raw_track_parameters(codec, parameters, &spec)?;
            import_raw_eac3_async(path, spec, sources).await
        }
        MuxRawCodec::Ac4 => import_raw_ac4_async(path, parameters, spec, sources).await,
        MuxRawCodec::Alac
        | MuxRawCodec::Dtsc
        | MuxRawCodec::Dtse
        | MuxRawCodec::Dtsh
        | MuxRawCodec::Dtsl
        | MuxRawCodec::Dtsm
        | MuxRawCodec::Dtsx
        | MuxRawCodec::Flac
        | MuxRawCodec::Opus
        | MuxRawCodec::Iamf
        | MuxRawCodec::Mha1
        | MuxRawCodec::Mhm1 => {
            import_parameterized_raw_audio_async(path, codec, parameters, spec, sources).await
        }
    }
}

fn validate_no_raw_track_parameters(
    codec: MuxRawCodec,
    parameters: &[MuxTrackParameter],
    spec: &str,
) -> Result<(), MuxError> {
    if parameters.is_empty() {
        return Ok(());
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!(
            "raw `{}` imports do not accept `#name=value` parameters yet",
            codec.prefix()
        ),
    })
}

fn collect_raw_track_parameters(
    parameters: &[MuxTrackParameter],
    spec: &str,
) -> Result<BTreeMap<String, String>, MuxError> {
    let mut collected = BTreeMap::new();
    for parameter in parameters {
        let name = parameter.name().to_string();
        if collected
            .insert(name.clone(), parameter.value().to_string())
            .is_some()
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("duplicate raw track parameter `{name}`"),
            });
        }
    }
    Ok(collected)
}

fn take_optional_raw_parameter(
    parameters: &mut BTreeMap<String, String>,
    name: &str,
) -> Option<String> {
    parameters.remove(name)
}

fn take_required_raw_parameter(
    parameters: &mut BTreeMap<String, String>,
    codec: MuxRawCodec,
    name: &str,
    spec: &str,
) -> Result<String, MuxError> {
    take_optional_raw_parameter(parameters, name).ok_or_else(|| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!(
            "raw `{}` imports require the `{name}` parameter",
            codec.prefix()
        ),
    })
}

fn take_optional_raw_u32_parameter(
    parameters: &mut BTreeMap<String, String>,
    codec: MuxRawCodec,
    name: &str,
    spec: &str,
) -> Result<Option<u32>, MuxError> {
    let Some(value) = take_optional_raw_parameter(parameters, name) else {
        return Ok(None);
    };
    Ok(Some(parse_raw_u32_parameter(codec, name, &value, spec)?))
}

fn take_required_raw_u32_parameter(
    parameters: &mut BTreeMap<String, String>,
    codec: MuxRawCodec,
    name: &str,
    spec: &str,
) -> Result<u32, MuxError> {
    let value = take_required_raw_parameter(parameters, codec, name, spec)?;
    parse_raw_u32_parameter(codec, name, &value, spec)
}

fn parse_raw_u32_parameter(
    codec: MuxRawCodec,
    name: &str,
    value: &str,
    spec: &str,
) -> Result<u32, MuxError> {
    let parsed = value
        .parse::<u32>()
        .map_err(|_| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "raw `{}` parameter `{name}` must be a non-negative integer, not `{value}`",
                codec.prefix()
            ),
        })?;
    if parsed == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "raw `{}` parameter `{name}` must be non-zero",
                codec.prefix()
            ),
        });
    }
    Ok(parsed)
}

fn parse_hex_parameter_bytes(
    codec: MuxRawCodec,
    name: &str,
    value: &str,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    if !value.len().is_multiple_of(2) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "raw `{}` parameter `{name}` must contain an even number of hexadecimal digits",
                codec.prefix()
            ),
        });
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let rendered = value.as_bytes();
    for index in (0..rendered.len()).step_by(2) {
        let pair = &value[index..index + 2];
        bytes.push(
            u8::from_str_radix(pair, 16).map_err(|_| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "raw `{}` parameter `{name}` contains invalid hexadecimal byte `{pair}`",
                    codec.prefix()
                ),
            })?,
        );
    }
    Ok(bytes)
}

fn build_generic_visual_sample_entry_box(
    sample_entry_type: FourCc,
    width: u16,
    height: u16,
) -> Result<Vec<u8>, MuxError> {
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
            depth: 0x0018,
            pre_defined3: -1,
            ..VisualSampleEntry::default()
        },
        &[],
    )
}

fn build_generic_audio_sample_entry_box(
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

fn build_parameterized_raw_audio_sample_entry_children(
    codec: MuxRawCodec,
    sample_rate: u32,
    sample_size: u16,
) -> Result<Vec<Vec<u8>>, MuxError> {
    if matches!(
        codec,
        MuxRawCodec::Dtsc
            | MuxRawCodec::Dtse
            | MuxRawCodec::Dtsh
            | MuxRawCodec::Dtsl
            | MuxRawCodec::Dtsm
            | MuxRawCodec::Dtsx
    ) {
        return Ok(vec![build_ddts_box(sample_rate, sample_size)?]);
    }
    Ok(Vec::new())
}

fn build_ddts_box(sample_rate: u32, sample_size: u16) -> Result<Vec<u8>, MuxError> {
    let pcm_sample_depth =
        u8::try_from(sample_size).map_err(|_| MuxError::LayoutOverflow("ddts pcm sample depth"))?;
    let mut payload = Vec::with_capacity(20);
    payload.extend_from_slice(&sample_rate.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.push(pcm_sample_depth);
    payload.extend_from_slice(&DDTS_EXTRA_DATA);
    super::mp4::encode_raw_box(DDTS, &payload)
}

fn parameterized_raw_video_sample_entry_type(codec: MuxRawCodec) -> FourCc {
    match codec {
        MuxRawCodec::Av1 => AV01,
        MuxRawCodec::Vp8 => VP08,
        MuxRawCodec::Vp9 => VP09,
        _ => unreachable!("only parameterized raw video codecs use this helper"),
    }
}

fn parameterized_raw_audio_sample_entry_type(codec: MuxRawCodec) -> FourCc {
    match codec {
        MuxRawCodec::Alac => ALAC,
        MuxRawCodec::Dtsc => DTSC,
        MuxRawCodec::Dtse => DTSE,
        MuxRawCodec::Dtsh => DTSH,
        MuxRawCodec::Dtsl => DTSL,
        MuxRawCodec::Dtsm => DTSM,
        MuxRawCodec::Dtsx => DTSX,
        MuxRawCodec::Flac => FLAC_ENTRY,
        MuxRawCodec::Opus => OPUS_ENTRY,
        MuxRawCodec::Iamf => IAMF_ENTRY,
        MuxRawCodec::Mha1 => MHA1,
        MuxRawCodec::Mhm1 => MHM1,
        _ => unreachable!("only parameterized raw audio codecs use this helper"),
    }
}

fn import_parameterized_raw_video_sync(
    path: &Path,
    codec: MuxRawCodec,
    parameters: &[MuxTrackParameter],
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let data_size = std::fs::metadata(path)?.len();
    import_parameterized_raw_video_from_file(path, data_size, codec, parameters, spec, sources)
}

#[cfg(feature = "async")]
async fn import_parameterized_raw_video_async(
    path: &Path,
    codec: MuxRawCodec,
    parameters: &[MuxTrackParameter],
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let data_size = tokio::fs::metadata(path).await?.len();
    import_parameterized_raw_video_from_file(path, data_size, codec, parameters, spec, sources)
}

fn import_parameterized_raw_video_from_file(
    path: &Path,
    data_size: u64,
    codec: MuxRawCodec,
    parameters: &[MuxTrackParameter],
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    if data_size == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.clone(),
            message: format!("raw `{}` input contained no sample bytes", codec.prefix()),
        });
    }

    let mut parameters = collect_raw_track_parameters(parameters, &spec)?;
    let width = u16::try_from(take_required_raw_u32_parameter(
        &mut parameters,
        codec,
        "width",
        &spec,
    )?)
    .map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.clone(),
        message: format!(
            "raw `{}` parameter `width` does not fit in u16",
            codec.prefix()
        ),
    })?;
    let height = u16::try_from(take_required_raw_u32_parameter(
        &mut parameters,
        codec,
        "height",
        &spec,
    )?)
    .map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.clone(),
        message: format!(
            "raw `{}` parameter `height` does not fit in u16",
            codec.prefix()
        ),
    })?;
    let timescale = take_optional_raw_u32_parameter(&mut parameters, codec, "timescale", &spec)?
        .unwrap_or(1_000);
    let sample_duration =
        take_optional_raw_u32_parameter(&mut parameters, codec, "sample_duration", &spec)?
            .unwrap_or(timescale);
    reject_unknown_raw_parameters(codec, &spec, &parameters)?;

    let source_index = sources.add_file(path)?;
    let sample_entry_box = build_generic_visual_sample_entry_box(
        parameterized_raw_video_sample_entry_type(codec),
        width,
        height,
    )?;
    let data_size = u32::try_from(data_size)
        .map_err(|_| MuxError::LayoutOverflow("parameterized raw video sample size"))?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Video,
        timescale,
        language: *b"und",
        handler_name: "VideoHandler".to_string(),
        width,
        height,
        sample_entry_box,
        source_edit_media_time: None,
        samples: vec![ImportedSample {
            source_index,
            data_offset: 0,
            data_size,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    })
}

fn import_parameterized_raw_audio_sync(
    path: &Path,
    codec: MuxRawCodec,
    parameters: &[MuxTrackParameter],
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let data_size = std::fs::metadata(path)?.len();
    import_parameterized_raw_audio_from_file(path, data_size, codec, parameters, spec, sources)
}

#[cfg(feature = "async")]
async fn import_parameterized_raw_audio_async(
    path: &Path,
    codec: MuxRawCodec,
    parameters: &[MuxTrackParameter],
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    let data_size = tokio::fs::metadata(path).await?.len();
    import_parameterized_raw_audio_from_file(path, data_size, codec, parameters, spec, sources)
}

fn import_parameterized_raw_audio_from_file(
    path: &Path,
    data_size: u64,
    codec: MuxRawCodec,
    parameters: &[MuxTrackParameter],
    spec: String,
    sources: &mut SourceCatalog,
) -> Result<ImportedTrack, MuxError> {
    if data_size == 0 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.clone(),
            message: format!("raw `{}` input contained no sample bytes", codec.prefix()),
        });
    }

    let mut parameters = collect_raw_track_parameters(parameters, &spec)?;
    let sample_rate =
        take_required_raw_u32_parameter(&mut parameters, codec, "sample_rate", &spec)?;
    let channel_count = u16::try_from(take_required_raw_u32_parameter(
        &mut parameters,
        codec,
        "channel_count",
        &spec,
    )?)
    .map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.clone(),
        message: format!(
            "raw `{}` parameter `channel_count` does not fit in u16",
            codec.prefix()
        ),
    })?;
    let sample_duration =
        take_optional_raw_u32_parameter(&mut parameters, codec, "sample_duration", &spec)?
            .unwrap_or(sample_rate);
    let sample_size =
        match take_optional_raw_u32_parameter(&mut parameters, codec, "sample_size", &spec)? {
            Some(value) => u16::try_from(value).map_err(|_| MuxError::UnsupportedTrackImport {
                spec: spec.clone(),
                message: format!(
                    "raw `{}` parameter `sample_size` does not fit in u16",
                    codec.prefix()
                ),
            })?,
            None => 16,
        };
    reject_unknown_raw_parameters(codec, &spec, &parameters)?;

    let source_index = sources.add_file(path)?;
    let sample_entry_children =
        build_parameterized_raw_audio_sample_entry_children(codec, sample_rate, sample_size)?;
    let sample_entry_box = build_generic_audio_sample_entry_box(
        parameterized_raw_audio_sample_entry_type(codec),
        sample_rate,
        channel_count,
        sample_size,
        &sample_entry_children,
    )?;
    let data_size = u32::try_from(data_size)
        .map_err(|_| MuxError::LayoutOverflow("parameterized raw audio sample size"))?;

    Ok(ImportedTrack {
        kind: MuxTrackKind::Audio,
        timescale: sample_rate,
        language: *b"und",
        handler_name: "SoundHandler".to_string(),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
        samples: vec![ImportedSample {
            source_index,
            data_offset: 0,
            data_size,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    })
}

fn reject_unknown_raw_parameters(
    codec: MuxRawCodec,
    spec: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<(), MuxError> {
    if let Some((name, _)) = parameters.iter().next() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "raw `{}` imports do not support the `{name}` parameter",
                codec.prefix()
            ),
        });
    }
    Ok(())
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
    sample_count: usize,
    path: &Path,
    track_id: u32,
) -> Result<Vec<bool>, MuxError> {
    let Some(stss) = stss else {
        return Ok(vec![true; sample_count]);
    };
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

struct ParsedAdtsTrack {
    audio_object_type: u8,
    sampling_frequency_index: u8,
    sample_rate: u32,
    channel_configuration: u16,
    samples: Vec<StagedSample>,
}

fn scan_adts_file_sync(path: &Path, spec: &str) -> Result<ParsedAdtsTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u8, u8, u32, u16)>;
    while offset < file_size {
        if file_size - offset < 7 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated ADTS header".to_string(),
            });
        }
        let mut header = [0_u8; 7];
        read_exact_at_sync(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated ADTS header",
        )?;
        if header[0] != 0xFF || header[1] & 0xF0 != 0xF0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing ADTS sync word at byte offset {offset}"),
            });
        }

        let protection_absent = header[1] & 0x01 != 0;
        let header_length = if protection_absent { 7 } else { 9 };
        if file_size - offset < u64::from(header_length as u32) {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated ADTS header".to_string(),
            });
        }
        let profile = ((header[2] >> 6) & 0x03) + 1;
        let sampling_frequency_index = (header[2] >> 2) & 0x0F;
        let channel_configuration = u16::from((header[2] & 0x01) << 2 | ((header[3] >> 6) & 0x03));
        let sample_rate = adts_sample_rate(sampling_frequency_index).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "unsupported ADTS sampling-frequency index {sampling_frequency_index}"
                ),
            }
        })?;
        let frame_length = usize::from(
            ((u16::from(header[3] & 0x03)) << 11)
                | (u16::from(header[4]) << 3)
                | u16::from(header[5] >> 5),
        );
        let raw_blocks = u32::from(header[6] & 0x03) + 1;
        if frame_length < header_length
            || offset
                .checked_add(u64::try_from(frame_length).unwrap_or(u64::MAX))
                .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated ADTS frame at byte offset {offset}"),
            });
        }

        let descriptor = (
            profile,
            sampling_frequency_index,
            sample_rate,
            channel_configuration,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "AAC frames changed profile, sample rate, or channel layout mid-stream"
                            .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }

        let payload_size = frame_length - header_length;
        samples.push(StagedSample {
            data_offset: offset + u64::from(header_length as u32),
            data_size: u32::try_from(payload_size)
                .map_err(|_| MuxError::LayoutOverflow("AAC frame size"))?,
            duration: 1024 * raw_blocks,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(
                u64::try_from(frame_length)
                    .map_err(|_| MuxError::LayoutOverflow("AAC frame size"))?,
            )
            .ok_or(MuxError::LayoutOverflow("AAC frame offset"))?;
    }
    let (audio_object_type, sampling_frequency_index, sample_rate, channel_configuration) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AAC input contained no ADTS frames".to_string(),
        })?;
    Ok(ParsedAdtsTrack {
        audio_object_type,
        sampling_frequency_index,
        sample_rate,
        channel_configuration,
        samples,
    })
}

#[cfg(feature = "async")]
async fn scan_adts_file_async(path: &Path, spec: &str) -> Result<ParsedAdtsTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u8, u8, u32, u16)>;
    while offset < file_size {
        if file_size - offset < 7 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated ADTS header".to_string(),
            });
        }
        let mut header = [0_u8; 7];
        read_exact_at_async(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated ADTS header",
        )
        .await?;
        if header[0] != 0xFF || header[1] & 0xF0 != 0xF0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing ADTS sync word at byte offset {offset}"),
            });
        }

        let protection_absent = header[1] & 0x01 != 0;
        let header_length = if protection_absent { 7 } else { 9 };
        if file_size - offset < u64::from(header_length as u32) {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated ADTS header".to_string(),
            });
        }
        let profile = ((header[2] >> 6) & 0x03) + 1;
        let sampling_frequency_index = (header[2] >> 2) & 0x0F;
        let channel_configuration = u16::from((header[2] & 0x01) << 2 | ((header[3] >> 6) & 0x03));
        let sample_rate = adts_sample_rate(sampling_frequency_index).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "unsupported ADTS sampling-frequency index {sampling_frequency_index}"
                ),
            }
        })?;
        let frame_length = usize::from(
            ((u16::from(header[3] & 0x03)) << 11)
                | (u16::from(header[4]) << 3)
                | u16::from(header[5] >> 5),
        );
        let raw_blocks = u32::from(header[6] & 0x03) + 1;
        if frame_length < header_length
            || offset
                .checked_add(u64::try_from(frame_length).unwrap_or(u64::MAX))
                .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated ADTS frame at byte offset {offset}"),
            });
        }

        let descriptor = (
            profile,
            sampling_frequency_index,
            sample_rate,
            channel_configuration,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "AAC frames changed profile, sample rate, or channel layout mid-stream"
                            .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }

        let payload_size = frame_length - header_length;
        samples.push(StagedSample {
            data_offset: offset + u64::from(header_length as u32),
            data_size: u32::try_from(payload_size)
                .map_err(|_| MuxError::LayoutOverflow("AAC frame size"))?,
            duration: 1024 * raw_blocks,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(
                u64::try_from(frame_length)
                    .map_err(|_| MuxError::LayoutOverflow("AAC frame size"))?,
            )
            .ok_or(MuxError::LayoutOverflow("AAC frame offset"))?;
    }

    let (audio_object_type, sampling_frequency_index, sample_rate, channel_configuration) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AAC input contained no ADTS frames".to_string(),
        })?;
    Ok(ParsedAdtsTrack {
        audio_object_type,
        sampling_frequency_index,
        sample_rate,
        channel_configuration,
        samples,
    })
}

fn build_aac_sample_entry_box(
    audio_object_type: u8,
    sampling_frequency_index: u8,
    channel_configuration: u16,
    sample_rate: u32,
) -> Result<Vec<u8>, MuxError> {
    let mut mp4a = AudioSampleEntry::default();
    mp4a.set_box_type(FourCc::from_bytes(*b"mp4a"));
    mp4a.sample_entry = SampleEntry {
        box_type: FourCc::from_bytes(*b"mp4a"),
        data_reference_index: 1,
    };
    mp4a.channel_count = channel_configuration;
    mp4a.sample_size = 16;
    mp4a.sample_rate = sample_rate << 16;

    super::mp4::encode_typed_box(
        &mp4a,
        &super::mp4::encode_typed_box(
            &aac_profile_esds(
                audio_object_type,
                sampling_frequency_index,
                channel_configuration,
            ),
            &[],
        )?,
    )
}

const fn adts_sample_rate(index: u8) -> Option<u32> {
    match index {
        0 => Some(96_000),
        1 => Some(88_200),
        2 => Some(64_000),
        3 => Some(48_000),
        4 => Some(44_100),
        5 => Some(32_000),
        6 => Some(24_000),
        7 => Some(22_050),
        8 => Some(16_000),
        9 => Some(12_000),
        10 => Some(11_025),
        11 => Some(8_000),
        12 => Some(7_350),
        _ => None,
    }
}

fn aac_profile_esds(
    audio_object_type: u8,
    sampling_frequency_index: u8,
    channel_configuration: u16,
) -> Esds {
    let audio_specific_config = build_aac_audio_specific_config(
        audio_object_type,
        sampling_frequency_index,
        channel_configuration,
    );
    let mut esds = Esds::default();
    esds.descriptors = vec![
        Descriptor {
            tag: DECODER_CONFIG_DESCRIPTOR_TAG,
            size: 13,
            decoder_config_descriptor: Some(DecoderConfigDescriptor {
                object_type_indication: 0x40,
                stream_type: 5,
                reserved: true,
                ..DecoderConfigDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_SPECIFIC_INFO_TAG,
            size: audio_specific_config.len() as u32,
            data: audio_specific_config,
            ..Descriptor::default()
        },
    ];
    esds
}

fn build_aac_audio_specific_config(
    audio_object_type: u8,
    sampling_frequency_index: u8,
    channel_configuration: u16,
) -> Vec<u8> {
    let config = ((u16::from(audio_object_type) & 0x1F) << 11)
        | ((u16::from(sampling_frequency_index) & 0x0F) << 7)
        | ((channel_configuration & 0x0F) << 3);
    vec![(config >> 8) as u8, (config & 0xFF) as u8]
}

fn mpeg_audio_esds(object_type_indication: u8) -> Esds {
    let mut esds = Esds::default();
    esds.descriptors = vec![Descriptor {
        tag: DECODER_CONFIG_DESCRIPTOR_TAG,
        size: 13,
        decoder_config_descriptor: Some(DecoderConfigDescriptor {
            object_type_indication,
            stream_type: 5,
            reserved: true,
            ..DecoderConfigDescriptor::default()
        }),
        ..Descriptor::default()
    }];
    esds
}

struct ParsedMp3Track {
    sample_rate: u32,
    channel_count: u16,
    samples: Vec<StagedSample>,
}

fn scan_mp3_file_sync(path: &Path, spec: &str) -> Result<ParsedMp3Track, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u32, u16, u32)>;
    while offset < file_size {
        if let Some(next_offset) = skip_id3v2_tag_sync(&mut file, file_size, offset, spec)? {
            offset = next_offset;
            continue;
        }
        if skip_trailing_id3v1_tag_offset(file_size, offset, &mut file)? {
            break;
        }
        if file_size - offset < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated MP3 frame header".to_string(),
            });
        }
        let mut header = [0_u8; 4];
        read_exact_at_sync(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated MP3 frame header",
        )?;
        if header[0] != 0xFF || header[1] & 0xE0 != 0xE0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing MP3 sync word at byte offset {offset}"),
            });
        }
        let version_id = (header[1] >> 3) & 0x03;
        if version_id == 0x01 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("reserved MP3 MPEG version at byte offset {offset}"),
            });
        }
        let layer = (header[1] >> 1) & 0x03;
        if layer != 0x01 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "the current raw MP3 mux importer only supports MPEG Layer III frames"
                    .to_string(),
            });
        }
        let bitrate_index = (header[2] >> 4) & 0x0F;
        if bitrate_index == 0 || bitrate_index == 0x0F {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported MP3 bitrate index {bitrate_index}"),
            });
        }
        let sample_rate_index = (header[2] >> 2) & 0x03;
        let sample_rate = mp3_sample_rate(version_id, sample_rate_index).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported MP3 sample-rate index {sample_rate_index}"),
            }
        })?;
        let bitrate_bps = mp3_bitrate_bps(version_id, bitrate_index).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported MP3 bitrate index {bitrate_index}"),
            }
        })?;
        let padding = u32::from((header[2] >> 1) & 0x01);
        let channel_count = if (header[3] >> 6) == 0x03 { 1 } else { 2 };
        let sample_duration = if version_id == 0x03 { 1152 } else { 576 };
        let frame_length = if version_id == 0x03 {
            ((144_u32 * bitrate_bps) / sample_rate).saturating_add(padding)
        } else {
            ((72_u32 * bitrate_bps) / sample_rate).saturating_add(padding)
        };
        if frame_length < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "MP3 frame length underflowed the header size".to_string(),
            });
        }
        let frame_length = usize::try_from(frame_length)
            .map_err(|_| MuxError::LayoutOverflow("MP3 frame length"))?;
        if offset
            .checked_add(u64::try_from(frame_length).unwrap_or(u64::MAX))
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated MP3 frame at byte offset {offset}"),
            });
        }
        let descriptor = (sample_rate, channel_count, sample_duration);
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "MP3 frames changed sample rate or channel layout mid-stream"
                        .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_length)
                .map_err(|_| MuxError::LayoutOverflow("MP3 frame size"))?,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(
                u64::try_from(frame_length)
                    .map_err(|_| MuxError::LayoutOverflow("MP3 frame length"))?,
            )
            .ok_or(MuxError::LayoutOverflow("MP3 frame offset"))?;
    }

    let (sample_rate, channel_count, _sample_duration) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MP3 input contained no MPEG audio frames".to_string(),
        })?;
    Ok(ParsedMp3Track {
        sample_rate,
        channel_count,
        samples,
    })
}

#[cfg(feature = "async")]
async fn scan_mp3_file_async(path: &Path, spec: &str) -> Result<ParsedMp3Track, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u32, u16, u32)>;
    while offset < file_size {
        if let Some(next_offset) = skip_id3v2_tag_async(&mut file, file_size, offset, spec).await? {
            offset = next_offset;
            continue;
        }
        if skip_trailing_id3v1_tag_offset_async(file_size, offset, &mut file).await? {
            break;
        }
        if file_size - offset < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated MP3 frame header".to_string(),
            });
        }
        let mut header = [0_u8; 4];
        read_exact_at_async(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated MP3 frame header",
        )
        .await?;
        if header[0] != 0xFF || header[1] & 0xE0 != 0xE0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing MP3 sync word at byte offset {offset}"),
            });
        }
        let version_id = (header[1] >> 3) & 0x03;
        if version_id == 0x01 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("reserved MP3 MPEG version at byte offset {offset}"),
            });
        }
        let layer = (header[1] >> 1) & 0x03;
        if layer != 0x01 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "the current raw MP3 mux importer only supports MPEG Layer III frames"
                    .to_string(),
            });
        }
        let bitrate_index = (header[2] >> 4) & 0x0F;
        if bitrate_index == 0 || bitrate_index == 0x0F {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported MP3 bitrate index {bitrate_index}"),
            });
        }
        let sample_rate_index = (header[2] >> 2) & 0x03;
        let sample_rate = mp3_sample_rate(version_id, sample_rate_index).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported MP3 sample-rate index {sample_rate_index}"),
            }
        })?;
        let bitrate_bps = mp3_bitrate_bps(version_id, bitrate_index).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported MP3 bitrate index {bitrate_index}"),
            }
        })?;
        let padding = u32::from((header[2] >> 1) & 0x01);
        let channel_count = if (header[3] >> 6) == 0x03 { 1 } else { 2 };
        let sample_duration = if version_id == 0x03 { 1152 } else { 576 };
        let frame_length = if version_id == 0x03 {
            ((144_u32 * bitrate_bps) / sample_rate).saturating_add(padding)
        } else {
            ((72_u32 * bitrate_bps) / sample_rate).saturating_add(padding)
        };
        if frame_length < 4 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "MP3 frame length underflowed the header size".to_string(),
            });
        }
        let frame_length = usize::try_from(frame_length)
            .map_err(|_| MuxError::LayoutOverflow("MP3 frame length"))?;
        if offset
            .checked_add(u64::try_from(frame_length).unwrap_or(u64::MAX))
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated MP3 frame at byte offset {offset}"),
            });
        }
        let descriptor = (sample_rate, channel_count, sample_duration);
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "MP3 frames changed sample rate or channel layout mid-stream"
                        .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_length)
                .map_err(|_| MuxError::LayoutOverflow("MP3 frame size"))?,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(
                u64::try_from(frame_length)
                    .map_err(|_| MuxError::LayoutOverflow("MP3 frame length"))?,
            )
            .ok_or(MuxError::LayoutOverflow("MP3 frame offset"))?;
    }

    let (sample_rate, channel_count, _sample_duration) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "MP3 input contained no MPEG audio frames".to_string(),
        })?;
    Ok(ParsedMp3Track {
        sample_rate,
        channel_count,
        samples,
    })
}

fn build_mp3_sample_entry_box(sample_rate: u32, channel_count: u16) -> Result<Vec<u8>, MuxError> {
    let mut mp4a = AudioSampleEntry::default();
    mp4a.set_box_type(FourCc::from_bytes(*b"mp4a"));
    mp4a.sample_entry = SampleEntry {
        box_type: FourCc::from_bytes(*b"mp4a"),
        data_reference_index: 1,
    };
    mp4a.channel_count = channel_count;
    mp4a.sample_size = 16;
    mp4a.sample_rate = sample_rate << 16;

    super::mp4::encode_typed_box(
        &mp4a,
        &super::mp4::encode_typed_box(&mpeg_audio_esds(0x6B), &[])?,
    )
}

fn skip_id3v2_tag(header: &[u8], spec: &str) -> Result<Option<usize>, MuxError> {
    if header.len() < 10 {
        return Ok(None);
    }
    if &header[..3] != b"ID3" {
        return Ok(None);
    }
    if header[6..10].iter().any(|byte| byte & 0x80 != 0) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "ID3v2 tag uses a non-synchsafe size field".to_string(),
        });
    }
    let tag_size = (usize::from(header[6]) << 21)
        | (usize::from(header[7]) << 14)
        | (usize::from(header[8]) << 7)
        | usize::from(header[9]);
    let footer_size = if header[5] & 0x10 != 0 { 10 } else { 0 };
    let total_size = 10_usize
        .checked_add(tag_size)
        .and_then(|size| size.checked_add(footer_size))
        .ok_or(MuxError::LayoutOverflow("ID3 tag size"))?;
    Ok(Some(total_size))
}

fn skip_id3v2_tag_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<Option<u64>, MuxError> {
    if file_size - offset < 10 {
        return Ok(None);
    }
    let mut header = [0_u8; 10];
    read_exact_at_sync(
        file,
        offset,
        &mut header,
        spec,
        "truncated ID3v2 tag ahead of MPEG audio frames",
    )?;
    skip_id3v2_tag(&header, spec)?
        .map(|size| {
            offset
                .checked_add(
                    u64::try_from(size).map_err(|_| MuxError::LayoutOverflow("ID3 tag size"))?,
                )
                .ok_or(MuxError::LayoutOverflow("ID3 tag offset"))
        })
        .transpose()
}

#[cfg(feature = "async")]
async fn skip_id3v2_tag_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<Option<u64>, MuxError> {
    if file_size - offset < 10 {
        return Ok(None);
    }
    let mut header = [0_u8; 10];
    read_exact_at_async(
        file,
        offset,
        &mut header,
        spec,
        "truncated ID3v2 tag ahead of MPEG audio frames",
    )
    .await?;
    skip_id3v2_tag(&header, spec)?
        .map(|size| {
            offset
                .checked_add(
                    u64::try_from(size).map_err(|_| MuxError::LayoutOverflow("ID3 tag size"))?,
                )
                .ok_or(MuxError::LayoutOverflow("ID3 tag offset"))
        })
        .transpose()
}

fn skip_trailing_id3v1_tag(header: &[u8]) -> bool {
    header.len() == 128 && &header[..3] == b"TAG"
}

fn skip_trailing_id3v1_tag_offset(
    file_size: u64,
    offset: u64,
    file: &mut File,
) -> Result<bool, MuxError> {
    if offset + 128 != file_size {
        return Ok(false);
    }
    let mut tag = [0_u8; 128];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut tag)?;
    Ok(skip_trailing_id3v1_tag(&tag))
}

#[cfg(feature = "async")]
async fn skip_trailing_id3v1_tag_offset_async(
    file_size: u64,
    offset: u64,
    file: &mut TokioFile,
) -> Result<bool, MuxError> {
    if offset + 128 != file_size {
        return Ok(false);
    }
    let mut tag = [0_u8; 128];
    file.seek(SeekFrom::Start(offset)).await?;
    file.read_exact(&mut tag).await?;
    Ok(skip_trailing_id3v1_tag(&tag))
}

const fn mp3_sample_rate(version_id: u8, sample_rate_index: u8) -> Option<u32> {
    let base = match sample_rate_index {
        0 => 44_100,
        1 => 48_000,
        2 => 32_000,
        _ => return None,
    };
    match version_id {
        0x03 => Some(base),
        0x02 => Some(base / 2),
        0x00 => Some(base / 4),
        _ => None,
    }
}

const fn mp3_bitrate_bps(version_id: u8, bitrate_index: u8) -> Option<u32> {
    let kbps = match version_id {
        0x03 => match bitrate_index {
            1 => 32,
            2 => 40,
            3 => 48,
            4 => 56,
            5 => 64,
            6 => 80,
            7 => 96,
            8 => 112,
            9 => 128,
            10 => 160,
            11 => 192,
            12 => 224,
            13 => 256,
            14 => 320,
            _ => return None,
        },
        0x02 | 0x00 => match bitrate_index {
            1 => 8,
            2 => 16,
            3 => 24,
            4 => 32,
            5 => 40,
            6 => 48,
            7 => 56,
            8 => 64,
            9 => 80,
            10 => 96,
            11 => 112,
            12 => 128,
            13 => 144,
            14 => 160,
            _ => return None,
        },
        _ => return None,
    };
    Some(kbps * 1_000)
}

fn read_exact_at_sync(
    file: &mut File,
    offset: u64,
    buf: &mut [u8],
    spec: &str,
    truncated_message: &'static str,
) -> Result<(), MuxError> {
    file.seek(SeekFrom::Start(offset))?;
    match file.read_exact(buf) {
        Ok(()) => Ok(()),
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
async fn read_exact_at_async(
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

struct IndexedAnnexBTrack {
    transformed_source: TransformedAnnexBSourceSpec,
    width: u16,
    height: u16,
    timescale: u32,
    sample_entry_box: Vec<u8>,
    samples: Vec<StagedSample>,
}

struct AnnexBNal {
    source_offset: u64,
    bytes: Vec<u8>,
}

struct ParsedH265Parameters {
    width: u16,
    height: u16,
    sample_entry_type: FourCc,
    timescale: u32,
    sample_duration: u32,
}

struct H265StageState {
    vps_list: Vec<Vec<u8>>,
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
    samples: Vec<StagedSample>,
    segments: Vec<TransformedAnnexBSegment>,
    current_sample_offset: Option<u64>,
    current_sample_size: u32,
    current_sync: bool,
    logical_size: u64,
}

impl H265StageState {
    fn new() -> Self {
        Self {
            vps_list: Vec::new(),
            sps_list: Vec::new(),
            pps_list: Vec::new(),
            samples: Vec::new(),
            segments: Vec::new(),
            current_sample_offset: None,
            current_sample_size: 0,
            current_sync: false,
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
        }
    }

    fn append_sample_nal(
        &mut self,
        source_offset: u64,
        source_size: u32,
        is_sync_sample: bool,
    ) -> Result<(), MuxError> {
        if self.current_sample_offset.is_none() {
            self.current_sample_offset = Some(self.logical_size);
        }
        let prefix = source_size.to_be_bytes();
        self.segments.push(TransformedAnnexBSegment {
            logical_offset: self.logical_size,
            data: TransformedAnnexBSegmentData::Prefix(prefix),
        });
        self.logical_size = self
            .logical_size
            .checked_add(4)
            .ok_or(MuxError::LayoutOverflow("raw H.265 transformed payload"))?;
        self.segments.push(TransformedAnnexBSegment {
            logical_offset: self.logical_size,
            data: TransformedAnnexBSegmentData::FileRange {
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
                        "raw H.265 transformed sample size",
                    ))?,
            )
            .ok_or(MuxError::LayoutOverflow("raw H.265 staged sample size"))?;
        self.logical_size = self
            .logical_size
            .checked_add(u64::from(source_size))
            .ok_or(MuxError::LayoutOverflow("raw H.265 transformed payload"))?;
        self.current_sync |= is_sync_sample;
        Ok(())
    }
}

fn parse_h265_raw_parameters(
    parameters: &[MuxTrackParameter],
    spec: &str,
) -> Result<ParsedH265Parameters, MuxError> {
    let mut parameters = collect_raw_track_parameters(parameters, spec)?;
    let width = u16::try_from(take_required_raw_u32_parameter(
        &mut parameters,
        MuxRawCodec::H265,
        "width",
        spec,
    )?)
    .map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "raw `h265` parameter `width` does not fit in u16".to_string(),
    })?;
    let height = u16::try_from(take_required_raw_u32_parameter(
        &mut parameters,
        MuxRawCodec::H265,
        "height",
        spec,
    )?)
    .map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "raw `h265` parameter `height` does not fit in u16".to_string(),
    })?;
    let sample_entry_type = take_optional_raw_parameter(&mut parameters, "sample_entry")
        .unwrap_or_else(|| "hvc1".into());
    let sample_entry_type = match sample_entry_type.as_str() {
        "hvc1" => FourCc::from_bytes(*b"hvc1"),
        "hev1" => FourCc::from_bytes(*b"hev1"),
        "dvh1" => DVH1,
        "dvhe" => DVHE,
        other => {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "raw `h265` parameter `sample_entry` must be `hvc1`, `hev1`, `dvh1`, or `dvhe`, not `{other}`"
                ),
            });
        }
    };
    let timescale =
        take_optional_raw_u32_parameter(&mut parameters, MuxRawCodec::H265, "timescale", spec)?
            .unwrap_or(0);
    let sample_duration = take_optional_raw_u32_parameter(
        &mut parameters,
        MuxRawCodec::H265,
        "sample_duration",
        spec,
    )?
    .unwrap_or(0);
    reject_unknown_raw_parameters(MuxRawCodec::H265, spec, &parameters)?;
    Ok(ParsedH265Parameters {
        width,
        height,
        sample_entry_type,
        timescale,
        sample_duration,
    })
}

fn stage_annex_b_h265_sync(
    path: &Path,
    parameters: &[MuxTrackParameter],
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    let parsed_parameters = parse_h265_raw_parameters(parameters, spec)?;
    let mut file = File::open(path)?;
    let mut scanner = AnnexBNalScanner::default();
    let mut state = H265StageState::new();
    let mut chunk = [0_u8; 16 * 1024];

    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        scanner.push(&chunk[..read], |nal| stage_h265_nal(&mut state, nal))?;
    }
    scanner.finish(|nal| stage_h265_nal(&mut state, nal))?;
    finalize_h265_staged_track(path, parsed_parameters, state, spec)
}

#[cfg(feature = "async")]
async fn stage_annex_b_h265_async(
    path: &Path,
    parameters: &[MuxTrackParameter],
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    let parsed_parameters = parse_h265_raw_parameters(parameters, spec)?;
    let mut file = TokioFile::open(path).await?;
    let mut scanner = AnnexBNalScanner::default();
    let mut state = H265StageState::new();
    let mut chunk = [0_u8; 16 * 1024];

    loop {
        let read = file.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        for nal in scanner.collect(&chunk[..read]) {
            stage_h265_nal(&mut state, nal)?;
        }
    }
    for nal in scanner.finish_collect() {
        stage_h265_nal(&mut state, nal)?;
    }
    finalize_h265_staged_track(path, parsed_parameters, state, spec)
}

fn stage_h265_nal(state: &mut H265StageState, nal: AnnexBNal) -> Result<(), MuxError> {
    if nal.bytes.len() < 2 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: "h265".to_string(),
            message: "H.265 NAL units must be at least two bytes long".to_string(),
        });
    }
    let nal_type = hevc_nal_type(&nal.bytes);
    match nal_type {
        32 => push_unique_nal(&mut state.vps_list, nal.bytes),
        33 => push_unique_nal(&mut state.sps_list, nal.bytes),
        34 => push_unique_nal(&mut state.pps_list, nal.bytes),
        35 => state.finish_current_sample(),
        _ => {
            let nal_len = u32::try_from(nal.bytes.len())
                .map_err(|_| MuxError::LayoutOverflow("H.265 NAL length"))?;
            state.append_sample_nal(nal.source_offset, nal_len, is_hevc_sync_nal_type(nal_type))?;
        }
    }
    Ok(())
}

fn finalize_h265_staged_track(
    path: &Path,
    parsed_parameters: ParsedH265Parameters,
    mut state: H265StageState,
    spec: &str,
) -> Result<IndexedAnnexBTrack, MuxError> {
    state.finish_current_sample();
    if state.vps_list.is_empty() || state.sps_list.is_empty() || state.pps_list.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.265 input must include VPS, SPS, and PPS NAL units".to_string(),
        });
    }
    if state.samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.265 input contained parameter sets but no media samples".to_string(),
        });
    }

    let timescale = if parsed_parameters.timescale != 0 {
        parsed_parameters.timescale
    } else if state.samples.len() == 1 {
        1
    } else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "multi-sample H.265 inputs currently require explicit `timescale` and `sample_duration` parameters"
                    .to_string(),
        });
    };
    let sample_duration = if parsed_parameters.sample_duration != 0 {
        parsed_parameters.sample_duration
    } else if state.samples.len() == 1 {
        1
    } else {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "multi-sample H.265 inputs currently require explicit `timescale` and `sample_duration` parameters"
                    .to_string(),
        });
    };
    for sample in &mut state.samples {
        sample.duration = sample_duration;
    }
    let sps_info = parse_h265_sps_configuration(&state.sps_list[0], spec)?;
    let sample_entry_box = build_h265_sample_entry_box(
        parsed_parameters.sample_entry_type,
        parsed_parameters.width,
        parsed_parameters.height,
        &sps_info,
        &state.vps_list,
        &state.sps_list,
        &state.pps_list,
    )?;

    Ok(IndexedAnnexBTrack {
        transformed_source: TransformedAnnexBSourceSpec {
            path: path.to_path_buf(),
            segments: state.segments,
            total_size: state.logical_size,
        },
        width: parsed_parameters.width,
        height: parsed_parameters.height,
        timescale,
        sample_entry_box,
        samples: state.samples,
    })
}

fn build_h265_sample_entry_box(
    sample_entry_type: FourCc,
    width: u16,
    height: u16,
    sps_info: &H265SpsInfo,
    vps_list: &[Vec<u8>],
    sps_list: &[Vec<u8>],
    pps_list: &[Vec<u8>],
) -> Result<Vec<u8>, MuxError> {
    let mut sample_entry = VisualSampleEntry::default();
    sample_entry.set_box_type(sample_entry_type);
    sample_entry.sample_entry = SampleEntry {
        box_type: sample_entry_type,
        data_reference_index: 1,
    };
    sample_entry.width = width;
    sample_entry.height = height;
    sample_entry.horizresolution = 72_u32 << 16;
    sample_entry.vertresolution = 72_u32 << 16;
    sample_entry.frame_count = 1;
    sample_entry.depth = 0x0018;
    sample_entry.pre_defined3 = -1;

    let nalu_arrays = [(&vps_list, 32_u8), (&sps_list, 33_u8), (&pps_list, 34_u8)]
        .into_iter()
        .map(|(group, nalu_type)| -> Result<HEVCNaluArray, MuxError> {
            Ok(HEVCNaluArray {
                completeness: true,
                reserved: false,
                nalu_type,
                num_nalus: u16::try_from(group.len())
                    .map_err(|_| MuxError::LayoutOverflow("HEVC NAL count"))?,
                nalus: group
                    .iter()
                    .map(|nal| -> Result<HEVCNalu, MuxError> {
                        Ok(HEVCNalu {
                            length: u16::try_from(nal.len())
                                .map_err(|_| MuxError::LayoutOverflow("HEVC NAL length"))?,
                            nal_unit: nal.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    super::mp4::encode_typed_box(
        &sample_entry,
        &super::mp4::encode_typed_box(
            &HEVCDecoderConfiguration {
                configuration_version: 1,
                general_profile_space: sps_info.general_profile_space,
                general_tier_flag: sps_info.general_tier_flag,
                general_profile_idc: sps_info.general_profile_idc,
                general_profile_compatibility: sps_info.general_profile_compatibility,
                general_constraint_indicator: sps_info.general_constraint_indicator,
                general_level_idc: sps_info.general_level_idc,
                min_spatial_segmentation_idc: 0,
                parallelism_type: 0,
                chroma_format_idc: sps_info.chroma_format_idc,
                bit_depth_luma_minus8: sps_info.bit_depth_luma_minus8,
                bit_depth_chroma_minus8: sps_info.bit_depth_chroma_minus8,
                avg_frame_rate: 0,
                constant_frame_rate: 0,
                num_temporal_layers: sps_info.num_temporal_layers,
                temporal_id_nested: sps_info.temporal_id_nested,
                length_size_minus_one: 3,
                num_of_nalu_arrays: u8::try_from(nalu_arrays.len())
                    .map_err(|_| MuxError::LayoutOverflow("HEVC NAL array count"))?,
                nalu_arrays,
            },
            &[],
        )?,
    )
}

fn push_unique_nal(existing: &mut Vec<Vec<u8>>, nal: Vec<u8>) {
    if !existing.iter().any(|entry| entry == &nal) {
        existing.push(nal);
    }
}

const fn hevc_nal_type(nal: &[u8]) -> u8 {
    (nal[0] >> 1) & 0x3F
}

const fn is_hevc_sync_nal_type(nal_type: u8) -> bool {
    matches!(nal_type, 16..=21)
}

struct H265SpsInfo {
    general_profile_space: u8,
    general_tier_flag: bool,
    general_profile_idc: u8,
    general_profile_compatibility: [bool; 32],
    general_constraint_indicator: [u8; 6],
    general_level_idc: u8,
    chroma_format_idc: u8,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
    num_temporal_layers: u8,
    temporal_id_nested: u8,
}

fn parse_h265_sps_configuration(nal: &[u8], spec: &str) -> Result<H265SpsInfo, MuxError> {
    if nal.len() < 3 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.265 SPS NAL is too short".to_string(),
        });
    }
    let rbsp = nal_to_rbsp(&nal[2..]);
    let mut reader = BitReader::new(Cursor::new(rbsp));
    let _sps_video_parameter_set_id = read_bits_u8_labeled(&mut reader, 4, spec, "H.265")?;
    let max_sub_layers_minus1 = read_bits_u8_labeled(&mut reader, 3, spec, "H.265")?;
    let temporal_id_nested = u8::from(read_bit_labeled(&mut reader, spec, "H.265")?);
    let general_profile_space = read_bits_u8_labeled(&mut reader, 2, spec, "H.265")?;
    let general_tier_flag = read_bit_labeled(&mut reader, spec, "H.265")?;
    let general_profile_idc = read_bits_u8_labeled(&mut reader, 5, spec, "H.265")?;
    let mut general_profile_compatibility = [false; 32];
    for entry in &mut general_profile_compatibility {
        *entry = read_bit_labeled(&mut reader, spec, "H.265")?;
    }
    let mut general_constraint_indicator = [0_u8; 6];
    for entry in &mut general_constraint_indicator {
        *entry = read_bits_u8_labeled(&mut reader, 8, spec, "H.265")?;
    }
    let general_level_idc = read_bits_u8_labeled(&mut reader, 8, spec, "H.265")?;

    let mut sub_layer_profile_present_flags =
        Vec::with_capacity(usize::from(max_sub_layers_minus1));
    let mut sub_layer_level_present_flags = Vec::with_capacity(usize::from(max_sub_layers_minus1));
    for _ in 0..max_sub_layers_minus1 {
        sub_layer_profile_present_flags.push(read_bit_labeled(&mut reader, spec, "H.265")?);
        sub_layer_level_present_flags.push(read_bit_labeled(&mut reader, spec, "H.265")?);
    }
    if max_sub_layers_minus1 > 0 {
        for _ in max_sub_layers_minus1..8 {
            skip_bits_labeled(&mut reader, 2, spec, "H.265")?;
        }
    }
    for (profile_present, level_present) in sub_layer_profile_present_flags
        .into_iter()
        .zip(sub_layer_level_present_flags)
    {
        if profile_present {
            skip_bits_labeled(&mut reader, 88, spec, "H.265")?;
        }
        if level_present {
            skip_bits_labeled(&mut reader, 8, spec, "H.265")?;
        }
    }

    let _sps_seq_parameter_set_id = read_ue_labeled(&mut reader, spec, "H.265")?;
    let chroma_format_idc =
        u8::try_from(read_ue_labeled(&mut reader, spec, "H.265")?).map_err(|_| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "H.265 chroma format does not fit in u8".to_string(),
            }
        })?;
    if chroma_format_idc == 3 {
        let _separate_colour_plane_flag = read_bit_labeled(&mut reader, spec, "H.265")?;
    }
    let _pic_width_in_luma_samples = read_ue_labeled(&mut reader, spec, "H.265")?;
    let _pic_height_in_luma_samples = read_ue_labeled(&mut reader, spec, "H.265")?;
    if read_bit_labeled(&mut reader, spec, "H.265")? {
        let _conf_win_left_offset = read_ue_labeled(&mut reader, spec, "H.265")?;
        let _conf_win_right_offset = read_ue_labeled(&mut reader, spec, "H.265")?;
        let _conf_win_top_offset = read_ue_labeled(&mut reader, spec, "H.265")?;
        let _conf_win_bottom_offset = read_ue_labeled(&mut reader, spec, "H.265")?;
    }
    let bit_depth_luma_minus8 = u8::try_from(read_ue_labeled(&mut reader, spec, "H.265")?)
        .map_err(|_| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.265 luma bit depth does not fit in u8".to_string(),
        })?;
    let bit_depth_chroma_minus8 = u8::try_from(read_ue_labeled(&mut reader, spec, "H.265")?)
        .map_err(|_| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "H.265 chroma bit depth does not fit in u8".to_string(),
        })?;

    Ok(H265SpsInfo {
        general_profile_space,
        general_tier_flag,
        general_profile_idc,
        general_profile_compatibility,
        general_constraint_indicator,
        general_level_idc,
        chroma_format_idc,
        bit_depth_luma_minus8,
        bit_depth_chroma_minus8,
        num_temporal_layers: max_sub_layers_minus1.saturating_add(1),
        temporal_id_nested,
    })
}

struct ParsedAc3Track {
    sample_rate: u32,
    channel_count: u16,
    fscod: u8,
    bsid: u8,
    bsmod: u8,
    acmod: u8,
    lfe_on: u8,
    bit_rate_code: u8,
    samples: Vec<StagedSample>,
}

fn scan_ac3_file_sync(path: &Path, spec: &str) -> Result<ParsedAc3Track, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u32, u16, u8, u8, u8, u8, u8)>;
    while offset < file_size {
        if file_size - offset < 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated AC-3 syncframe header".to_string(),
            });
        }
        let mut header = [0_u8; 8];
        read_exact_at_sync(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated AC-3 syncframe header",
        )?;
        if header[0] != 0x0B || header[1] != 0x77 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing AC-3 sync word at byte offset {offset}"),
            });
        }
        let fscod = header[4] >> 6;
        if fscod == 0x03 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "reserved AC-3 sample-rate code".to_string(),
            });
        }
        let frmsizecod = header[4] & 0x3F;
        let frame_size = ac3_frame_size_bytes(fscod, frmsizecod).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported AC-3 frame-size code {frmsizecod}"),
            }
        })?;
        let frame_size_u64 = u64::from(frame_size);
        if offset
            .checked_add(frame_size_u64)
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated AC-3 syncframe at byte offset {offset}"),
            });
        }
        let bsid = (header[5] >> 3) & 0x1F;
        let bsmod = header[5] & 0x07;
        let mut reader = BitReader::new(Cursor::new(&header[6..8]));
        let acmod = read_bits_u8_labeled(&mut reader, 3, spec, "AC-3")?;
        if acmod & 0x01 != 0 && acmod != 0x01 {
            skip_bits_labeled(&mut reader, 2, spec, "AC-3")?;
        }
        if acmod & 0x04 != 0 {
            skip_bits_labeled(&mut reader, 2, spec, "AC-3")?;
        }
        if acmod == 0x02 {
            skip_bits_labeled(&mut reader, 2, spec, "AC-3")?;
        }
        let lfe_on = u8::from(read_bit_labeled(&mut reader, spec, "AC-3")?);
        let sample_rate =
            ac3_sample_rate(fscod).ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported AC-3 sample-rate code {fscod}"),
            })?;
        let channel_count = ac3_channel_count(acmod, lfe_on != 0).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported AC-3 channel mode {acmod}"),
            }
        })?;
        let descriptor = (
            sample_rate,
            channel_count,
            bsid,
            bsmod,
            acmod,
            lfe_on,
            frmsizecod >> 1,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "AC-3 syncframes changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: frame_size,
            duration: 1536,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size_u64)
            .ok_or(MuxError::LayoutOverflow("AC-3 frame offset"))?;
    }

    let (sample_rate, channel_count, bsid, bsmod, acmod, lfe_on, bit_rate_code) = expected
        .ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-3 input contained no syncframes".to_string(),
        })?;
    Ok(ParsedAc3Track {
        sample_rate,
        channel_count,
        fscod: match sample_rate {
            48_000 => 0,
            44_100 => 1,
            32_000 => 2,
            _ => unreachable!(),
        },
        bsid,
        bsmod,
        acmod,
        lfe_on,
        bit_rate_code,
        samples,
    })
}

#[cfg(feature = "async")]
async fn scan_ac3_file_async(path: &Path, spec: &str) -> Result<ParsedAc3Track, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u32, u16, u8, u8, u8, u8, u8)>;
    while offset < file_size {
        if file_size - offset < 8 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated AC-3 syncframe header".to_string(),
            });
        }
        let mut header = [0_u8; 8];
        read_exact_at_async(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated AC-3 syncframe header",
        )
        .await?;
        if header[0] != 0x0B || header[1] != 0x77 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing AC-3 sync word at byte offset {offset}"),
            });
        }
        let fscod = header[4] >> 6;
        if fscod == 0x03 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "reserved AC-3 sample-rate code".to_string(),
            });
        }
        let frmsizecod = header[4] & 0x3F;
        let frame_size = ac3_frame_size_bytes(fscod, frmsizecod).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported AC-3 frame-size code {frmsizecod}"),
            }
        })?;
        let frame_size_u64 = u64::from(frame_size);
        if offset
            .checked_add(frame_size_u64)
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated AC-3 syncframe at byte offset {offset}"),
            });
        }
        let bsid = (header[5] >> 3) & 0x1F;
        let bsmod = header[5] & 0x07;
        let mut reader = BitReader::new(Cursor::new(&header[6..8]));
        let acmod = read_bits_u8_labeled(&mut reader, 3, spec, "AC-3")?;
        if acmod & 0x01 != 0 && acmod != 0x01 {
            skip_bits_labeled(&mut reader, 2, spec, "AC-3")?;
        }
        if acmod & 0x04 != 0 {
            skip_bits_labeled(&mut reader, 2, spec, "AC-3")?;
        }
        if acmod == 0x02 {
            skip_bits_labeled(&mut reader, 2, spec, "AC-3")?;
        }
        let lfe_on = u8::from(read_bit_labeled(&mut reader, spec, "AC-3")?);
        let sample_rate =
            ac3_sample_rate(fscod).ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported AC-3 sample-rate code {fscod}"),
            })?;
        let channel_count = ac3_channel_count(acmod, lfe_on != 0).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported AC-3 channel mode {acmod}"),
            }
        })?;
        let descriptor = (
            sample_rate,
            channel_count,
            bsid,
            bsmod,
            acmod,
            lfe_on,
            frmsizecod / 2,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "AC-3 syncframes changed decoder configuration mid-stream".to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }
        samples.push(StagedSample {
            data_offset: offset,
            data_size: frame_size,
            duration: 1536,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size_u64)
            .ok_or(MuxError::LayoutOverflow("AC-3 frame offset"))?;
    }

    let (sample_rate, channel_count, bsid, bsmod, acmod, lfe_on, bit_rate_code) = expected
        .ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-3 input contained no syncframes".to_string(),
        })?;
    Ok(ParsedAc3Track {
        sample_rate,
        channel_count,
        fscod: match sample_rate {
            48_000 => 0,
            44_100 => 1,
            32_000 => 2,
            _ => unreachable!(),
        },
        bsid,
        bsmod,
        acmod,
        lfe_on,
        bit_rate_code,
        samples,
    })
}

fn build_ac3_sample_entry_box(parsed: &ParsedAc3Track) -> Result<Vec<u8>, MuxError> {
    let mut sample_entry = AudioSampleEntry::default();
    sample_entry.set_box_type(FourCc::from_bytes(*b"ac-3"));
    sample_entry.sample_entry = SampleEntry {
        box_type: FourCc::from_bytes(*b"ac-3"),
        data_reference_index: 1,
    };
    sample_entry.channel_count = parsed.channel_count;
    sample_entry.sample_size = 16;
    sample_entry.sample_rate = parsed.sample_rate << 16;

    super::mp4::encode_typed_box(
        &sample_entry,
        &super::mp4::encode_typed_box(
            &Dac3 {
                fscod: parsed.fscod,
                bsid: parsed.bsid,
                bsmod: parsed.bsmod,
                acmod: parsed.acmod,
                lfe_on: parsed.lfe_on,
                bit_rate_code: parsed.bit_rate_code,
            },
            &[],
        )?,
    )
}

const fn ac3_sample_rate(fscod: u8) -> Option<u32> {
    match fscod {
        0 => Some(48_000),
        1 => Some(44_100),
        2 => Some(32_000),
        _ => None,
    }
}

fn ac3_frame_size_bytes(fscod: u8, frmsizecod: u8) -> Option<u32> {
    const AC3_FRAME_SIZE_WORDS: [[u16; 3]; 38] = [
        [96, 69, 64],
        [96, 70, 64],
        [120, 87, 80],
        [120, 88, 80],
        [144, 104, 96],
        [144, 105, 96],
        [168, 121, 112],
        [168, 122, 112],
        [192, 139, 128],
        [192, 140, 128],
        [240, 174, 160],
        [240, 175, 160],
        [288, 208, 192],
        [288, 209, 192],
        [336, 243, 224],
        [336, 244, 224],
        [384, 278, 256],
        [384, 279, 256],
        [480, 348, 320],
        [480, 349, 320],
        [576, 417, 384],
        [576, 418, 384],
        [672, 487, 448],
        [672, 488, 448],
        [768, 557, 512],
        [768, 558, 512],
        [960, 696, 640],
        [960, 697, 640],
        [1152, 835, 768],
        [1152, 836, 768],
        [1344, 975, 896],
        [1344, 976, 896],
        [1536, 1114, 1024],
        [1536, 1115, 1024],
        [1728, 1253, 1152],
        [1728, 1254, 1152],
        [1920, 1393, 1280],
        [1920, 1394, 1280],
    ];
    let frame_words = *AC3_FRAME_SIZE_WORDS.get(usize::from(frmsizecod))?;
    let sample_rate_index = match fscod {
        0 => 2,
        1 => 1,
        2 => 0,
        _ => return None,
    };
    Some(u32::from(frame_words[sample_rate_index]) * 2)
}

const fn ac3_channel_count(acmod: u8, lfe_on: bool) -> Option<u16> {
    let base = match acmod {
        0 => 2,
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 3,
        5 => 4,
        6 => 4,
        7 => 5,
        _ => return None,
    };
    Some(base + if lfe_on { 1 } else { 0 })
}

struct ParsedEac3Track {
    sample_rate: u32,
    channel_count: u16,
    fscod: u8,
    bsid: u8,
    bsmod: u8,
    acmod: u8,
    lfe_on: u8,
    data_rate: u16,
    samples: Vec<StagedSample>,
}

fn scan_eac3_file_sync(path: &Path, spec: &str) -> Result<ParsedEac3Track, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u32, u16, u8, u8, u8, u8)>;
    let mut data_rate = 0_u16;
    while offset < file_size {
        if file_size - offset < 6 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated E-AC-3 syncframe header".to_string(),
            });
        }
        let mut header = [0_u8; 6];
        read_exact_at_sync(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated E-AC-3 syncframe header",
        )?;
        if header[0] != 0x0B || header[1] != 0x77 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing E-AC-3 sync word at byte offset {offset}"),
            });
        }
        let mut reader = BitReader::new(Cursor::new(&header[2..]));
        let stream_type = read_bits_u8_labeled(&mut reader, 2, spec, "E-AC-3")?;
        if stream_type != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "the current raw E-AC-3 importer only supports independent substreams"
                    .to_string(),
            });
        }
        let _substream_id = read_bits_u8_labeled(&mut reader, 3, spec, "E-AC-3")?;
        let frame_size_words_minus_one = read_bits_u16_labeled(&mut reader, 11, spec, "E-AC-3")?;
        let frame_size = u64::from(frame_size_words_minus_one.saturating_add(1))
            .checked_mul(2)
            .ok_or(MuxError::LayoutOverflow("E-AC-3 frame size"))?;
        if offset
            .checked_add(frame_size)
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated E-AC-3 syncframe at byte offset {offset}"),
            });
        }
        let fscod = read_bits_u8_labeled(&mut reader, 2, spec, "E-AC-3")?;
        let (sample_rate, sample_duration) = if fscod == 0x03 {
            let fscod2 = read_bits_u8_labeled(&mut reader, 2, spec, "E-AC-3")?;
            let sample_rate = match fscod2 {
                0 => 24_000,
                1 => 22_050,
                2 => 16_000,
                _ => {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: format!("unsupported E-AC-3 half-rate code {fscod2}"),
                    });
                }
            };
            (sample_rate, 1536)
        } else {
            let numblkscod = read_bits_u8_labeled(&mut reader, 2, spec, "E-AC-3")?;
            let sample_rate =
                ac3_sample_rate(fscod).ok_or_else(|| MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!("unsupported E-AC-3 sample-rate code {fscod}"),
                })?;
            let sample_duration = match numblkscod {
                0 => 256,
                1 => 512,
                2 => 768,
                3 => 1536,
                _ => unreachable!(),
            };
            (sample_rate, sample_duration)
        };
        let acmod = read_bits_u8_labeled(&mut reader, 3, spec, "E-AC-3")?;
        let lfe_on = u8::from(read_bit_labeled(&mut reader, spec, "E-AC-3")?);
        let bsid = read_bits_u8_labeled(&mut reader, 5, spec, "E-AC-3")?;
        let channel_count = ac3_channel_count(acmod, lfe_on != 0).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported E-AC-3 channel mode {acmod}"),
            }
        })?;
        let descriptor = (sample_rate, channel_count, bsid, 0, acmod, lfe_on);
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "E-AC-3 syncframes changed decoder configuration mid-stream"
                        .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }
        data_rate = u16::try_from(
            ((frame_size * 8 * u64::from(sample_rate)) / u64::from(sample_duration))
                .div_ceil(1_000),
        )
        .map_err(|_| MuxError::LayoutOverflow("E-AC-3 data_rate"))?;
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_size)
                .map_err(|_| MuxError::LayoutOverflow("E-AC-3 frame size"))?,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size)
            .ok_or(MuxError::LayoutOverflow("E-AC-3 frame offset"))?;
    }

    let (sample_rate, channel_count, bsid, bsmod, acmod, lfe_on) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "E-AC-3 input contained no syncframes".to_string(),
        })?;
    Ok(ParsedEac3Track {
        sample_rate,
        channel_count,
        fscod: match sample_rate {
            48_000 => 0,
            44_100 => 1,
            32_000 => 2,
            _ => 3,
        },
        bsid,
        bsmod,
        acmod,
        lfe_on,
        data_rate,
        samples,
    })
}

#[cfg(feature = "async")]
async fn scan_eac3_file_async(path: &Path, spec: &str) -> Result<ParsedEac3Track, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u32, u16, u8, u8, u8, u8)>;
    let mut data_rate = 0_u16;
    while offset < file_size {
        if file_size - offset < 6 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated E-AC-3 syncframe header".to_string(),
            });
        }
        let mut header = [0_u8; 6];
        read_exact_at_async(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated E-AC-3 syncframe header",
        )
        .await?;
        if header[0] != 0x0B || header[1] != 0x77 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing E-AC-3 sync word at byte offset {offset}"),
            });
        }
        let mut reader = BitReader::new(Cursor::new(&header[2..]));
        let stream_type = read_bits_u8_labeled(&mut reader, 2, spec, "E-AC-3")?;
        if stream_type != 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "the current raw E-AC-3 importer only supports independent substreams"
                    .to_string(),
            });
        }
        let _substream_id = read_bits_u8_labeled(&mut reader, 3, spec, "E-AC-3")?;
        let frame_size_words_minus_one = read_bits_u16_labeled(&mut reader, 11, spec, "E-AC-3")?;
        let frame_size = u64::from(frame_size_words_minus_one.saturating_add(1))
            .checked_mul(2)
            .ok_or(MuxError::LayoutOverflow("E-AC-3 frame size"))?;
        if offset
            .checked_add(frame_size)
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated E-AC-3 syncframe at byte offset {offset}"),
            });
        }
        let fscod = read_bits_u8_labeled(&mut reader, 2, spec, "E-AC-3")?;
        let (sample_rate, sample_duration) = if fscod == 0x03 {
            let fscod2 = read_bits_u8_labeled(&mut reader, 2, spec, "E-AC-3")?;
            let sample_rate = match fscod2 {
                0 => 24_000,
                1 => 22_050,
                2 => 16_000,
                _ => {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: format!("unsupported E-AC-3 half-rate code {fscod2}"),
                    });
                }
            };
            (sample_rate, 1536)
        } else {
            let numblkscod = read_bits_u8_labeled(&mut reader, 2, spec, "E-AC-3")?;
            let sample_rate =
                ac3_sample_rate(fscod).ok_or_else(|| MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!("unsupported E-AC-3 sample-rate code {fscod}"),
                })?;
            let sample_duration = match numblkscod {
                0 => 256,
                1 => 512,
                2 => 768,
                3 => 1536,
                _ => unreachable!(),
            };
            (sample_rate, sample_duration)
        };
        let acmod = read_bits_u8_labeled(&mut reader, 3, spec, "E-AC-3")?;
        let lfe_on = u8::from(read_bit_labeled(&mut reader, spec, "E-AC-3")?);
        let bsid = read_bits_u8_labeled(&mut reader, 5, spec, "E-AC-3")?;
        let channel_count = ac3_channel_count(acmod, lfe_on != 0).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("unsupported E-AC-3 channel mode {acmod}"),
            }
        })?;
        let descriptor = (sample_rate, channel_count, bsid, 0, acmod, lfe_on);
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "E-AC-3 syncframes changed decoder configuration mid-stream"
                        .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }
        data_rate = u16::try_from(
            ((frame_size * 8 * u64::from(sample_rate)) / u64::from(sample_duration))
                .div_ceil(1_000),
        )
        .map_err(|_| MuxError::LayoutOverflow("E-AC-3 data_rate"))?;
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_size)
                .map_err(|_| MuxError::LayoutOverflow("E-AC-3 frame size"))?,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size)
            .ok_or(MuxError::LayoutOverflow("E-AC-3 frame offset"))?;
    }

    let (sample_rate, channel_count, bsid, bsmod, acmod, lfe_on) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "E-AC-3 input contained no syncframes".to_string(),
        })?;
    Ok(ParsedEac3Track {
        sample_rate,
        channel_count,
        fscod: match sample_rate {
            48_000 => 0,
            44_100 => 1,
            32_000 => 2,
            _ => 3,
        },
        bsid,
        bsmod,
        acmod,
        lfe_on,
        data_rate,
        samples,
    })
}

fn build_eac3_sample_entry_box(parsed: &ParsedEac3Track) -> Result<Vec<u8>, MuxError> {
    let mut sample_entry = AudioSampleEntry::default();
    sample_entry.set_box_type(FourCc::from_bytes(*b"ec-3"));
    sample_entry.sample_entry = SampleEntry {
        box_type: FourCc::from_bytes(*b"ec-3"),
        data_reference_index: 1,
    };
    sample_entry.channel_count = parsed.channel_count;
    sample_entry.sample_size = 16;
    sample_entry.sample_rate = parsed.sample_rate << 16;

    super::mp4::encode_typed_box(
        &sample_entry,
        &super::mp4::encode_typed_box(
            &Dec3 {
                data_rate: parsed.data_rate,
                num_ind_sub: 0,
                ec3_substreams: vec![Ec3Substream {
                    fscod: parsed.fscod,
                    bsid: parsed.bsid,
                    asvc: 0,
                    bsmod: parsed.bsmod,
                    acmod: parsed.acmod,
                    lfe_on: parsed.lfe_on,
                    num_dep_sub: 0,
                    chan_loc: 0,
                }],
                reserved: Vec::new(),
            },
            &[],
        )?,
    )
}

struct ParsedAc4Track {
    sample_rate: u32,
    channel_count: u16,
    dac4_data: Vec<u8>,
    samples: Vec<StagedSample>,
}

fn scan_ac4_file_sync(
    path: &Path,
    parameters: &[MuxTrackParameter],
    spec: &str,
) -> Result<ParsedAc4Track, MuxError> {
    let mut parameters = collect_raw_track_parameters(parameters, spec)?;
    let sample_rate =
        take_required_raw_u32_parameter(&mut parameters, MuxRawCodec::Ac4, "sample_rate", spec)?;
    let channel_count = u16::try_from(take_required_raw_u32_parameter(
        &mut parameters,
        MuxRawCodec::Ac4,
        "channel_count",
        spec,
    )?)
    .map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "raw `ac4` parameter `channel_count` does not fit in u16".to_string(),
    })?;
    let sample_duration = take_required_raw_u32_parameter(
        &mut parameters,
        MuxRawCodec::Ac4,
        "sample_duration",
        spec,
    )?;
    let dac4_data = match take_optional_raw_parameter(&mut parameters, "dac4") {
        Some(value) => parse_hex_parameter_bytes(MuxRawCodec::Ac4, "dac4", &value, spec)?,
        None => Vec::new(),
    };
    reject_unknown_raw_parameters(MuxRawCodec::Ac4, spec, &parameters)?;

    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    while offset < file_size {
        let frame_size = read_ac4_frame_size_sync(&mut file, file_size, offset, spec)?;
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_size)
                .map_err(|_| MuxError::LayoutOverflow("AC-4 frame size"))?,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size)
            .ok_or(MuxError::LayoutOverflow("AC-4 frame offset"))?;
    }
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-4 input contained no syncframes".to_string(),
        });
    }
    Ok(ParsedAc4Track {
        sample_rate,
        channel_count,
        dac4_data,
        samples,
    })
}

#[cfg(feature = "async")]
async fn scan_ac4_file_async(
    path: &Path,
    parameters: &[MuxTrackParameter],
    spec: &str,
) -> Result<ParsedAc4Track, MuxError> {
    let mut parameters = collect_raw_track_parameters(parameters, spec)?;
    let sample_rate =
        take_required_raw_u32_parameter(&mut parameters, MuxRawCodec::Ac4, "sample_rate", spec)?;
    let channel_count = u16::try_from(take_required_raw_u32_parameter(
        &mut parameters,
        MuxRawCodec::Ac4,
        "channel_count",
        spec,
    )?)
    .map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "raw `ac4` parameter `channel_count` does not fit in u16".to_string(),
    })?;
    let sample_duration = take_required_raw_u32_parameter(
        &mut parameters,
        MuxRawCodec::Ac4,
        "sample_duration",
        spec,
    )?;
    let dac4_data = match take_optional_raw_parameter(&mut parameters, "dac4") {
        Some(value) => parse_hex_parameter_bytes(MuxRawCodec::Ac4, "dac4", &value, spec)?,
        None => Vec::new(),
    };
    reject_unknown_raw_parameters(MuxRawCodec::Ac4, spec, &parameters)?;

    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    while offset < file_size {
        let frame_size = read_ac4_frame_size_async(&mut file, file_size, offset, spec).await?;
        samples.push(StagedSample {
            data_offset: offset,
            data_size: u32::try_from(frame_size)
                .map_err(|_| MuxError::LayoutOverflow("AC-4 frame size"))?,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size)
            .ok_or(MuxError::LayoutOverflow("AC-4 frame offset"))?;
    }
    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-4 input contained no syncframes".to_string(),
        });
    }
    Ok(ParsedAc4Track {
        sample_rate,
        channel_count,
        dac4_data,
        samples,
    })
}

fn read_ac4_frame_size_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<u64, MuxError> {
    let mut header = [0_u8; 7];
    if file_size - offset < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated AC-4 syncframe header".to_string(),
        });
    }
    read_exact_at_sync(
        file,
        offset,
        &mut header[..4],
        spec,
        "truncated AC-4 syncframe header",
    )?;
    parse_ac4_frame_size(&header, file_size, offset, spec)
}

#[cfg(feature = "async")]
async fn read_ac4_frame_size_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<u64, MuxError> {
    let mut header = [0_u8; 7];
    if file_size - offset < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "truncated AC-4 syncframe header".to_string(),
        });
    }
    read_exact_at_async(
        file,
        offset,
        &mut header[..4],
        spec,
        "truncated AC-4 syncframe header",
    )
    .await?;
    if u16::from_be_bytes([header[2], header[3]]) == 0xFFFF {
        if file_size - offset < 7 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated extended AC-4 syncframe header".to_string(),
            });
        }
        read_exact_at_async(
            file,
            offset,
            &mut header,
            spec,
            "truncated extended AC-4 syncframe header",
        )
        .await?;
    }
    parse_ac4_frame_size(&header, file_size, offset, spec)
}

fn parse_ac4_frame_size(
    header: &[u8; 7],
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<u64, MuxError> {
    let syncword = u16::from_be_bytes([header[0], header[1]]);
    if syncword != 0xAC40 && syncword != 0xAC41 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("missing AC-4 sync word at byte offset {offset}"),
        });
    }
    let size_code = u16::from_be_bytes([header[2], header[3]]);
    let (header_size, frame_payload_size) = if size_code == 0xFFFF {
        (
            7_u64,
            u64::from(header[4]) << 16 | u64::from(header[5]) << 8 | u64::from(header[6]),
        )
    } else {
        (4_u64, u64::from(size_code))
    };
    let mut frame_size = header_size
        .checked_add(frame_payload_size)
        .ok_or(MuxError::LayoutOverflow("AC-4 frame size"))?;
    if frame_size <= header_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AC-4 syncframes must carry payload bytes".to_string(),
        });
    }
    if offset
        .checked_add(frame_size)
        .is_none_or(|end| end > file_size)
    {
        if size_code != 0xFFFF {
            let alternate_frame_size = u64::from(size_code)
                .checked_add(2)
                .ok_or(MuxError::LayoutOverflow("AC-4 alternate frame size"))?;
            if alternate_frame_size > header_size
                && offset
                    .checked_add(alternate_frame_size)
                    .is_some_and(|end| end <= file_size)
            {
                frame_size = alternate_frame_size;
            } else {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!("truncated AC-4 syncframe at byte offset {offset}"),
                });
            }
        } else {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated AC-4 syncframe at byte offset {offset}"),
            });
        }
    }
    Ok(frame_size)
}

fn build_ac4_sample_entry_box(parsed: &ParsedAc4Track) -> Result<Vec<u8>, MuxError> {
    let mut sample_entry = AudioSampleEntry::default();
    sample_entry.set_box_type(FourCc::from_bytes(*b"ac-4"));
    sample_entry.sample_entry = SampleEntry {
        box_type: FourCc::from_bytes(*b"ac-4"),
        data_reference_index: 1,
    };
    sample_entry.channel_count = parsed.channel_count;
    sample_entry.sample_size = 16;
    sample_entry.sample_rate = parsed.sample_rate << 16;

    super::mp4::encode_typed_box(
        &sample_entry,
        &super::mp4::encode_typed_box(
            &Dac4 {
                data: parsed.dac4_data.clone(),
            },
            &[],
        )?,
    )
}

struct H264StageState {
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
    samples: Vec<StagedSample>,
    segments: Vec<TransformedAnnexBSegment>,
    current_sample_offset: Option<u64>,
    current_sample_size: u32,
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
                is_sync_sample: true,
            });
            self.current_sample_size = 0;
        }
    }

    fn append_sample_nal(&mut self, source_offset: u64, source_size: u32) -> Result<(), MuxError> {
        if self.current_sample_offset.is_none() {
            self.current_sample_offset = Some(self.logical_size);
        }
        let prefix = source_size.to_be_bytes();
        self.segments.push(TransformedAnnexBSegment {
            logical_offset: self.logical_size,
            data: TransformedAnnexBSegmentData::Prefix(prefix),
        });
        self.logical_size = self
            .logical_size
            .checked_add(4)
            .ok_or(MuxError::LayoutOverflow("raw H.264 transformed payload"))?;
        self.segments.push(TransformedAnnexBSegment {
            logical_offset: self.logical_size,
            data: TransformedAnnexBSegmentData::FileRange {
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
        Ok(())
    }
}

#[derive(Default)]
struct AnnexBNalScanner {
    buffer: Vec<u8>,
    buffer_start_offset: u64,
    next_input_offset: u64,
}

impl AnnexBNalScanner {
    fn push<F>(&mut self, chunk: &[u8], mut on_nal: F) -> Result<(), MuxError>
    where
        F: FnMut(AnnexBNal) -> Result<(), MuxError>,
    {
        for nal in self.collect(chunk) {
            on_nal(nal)?;
        }
        Ok(())
    }

    fn finish<F>(&mut self, mut on_nal: F) -> Result<(), MuxError>
    where
        F: FnMut(AnnexBNal) -> Result<(), MuxError>,
    {
        for nal in self.finish_collect() {
            on_nal(nal)?;
        }
        Ok(())
    }

    fn collect(&mut self, chunk: &[u8]) -> Vec<AnnexBNal> {
        if self.buffer.is_empty() {
            self.buffer_start_offset = self.next_input_offset;
        }
        self.buffer.extend_from_slice(chunk);
        self.next_input_offset = self
            .next_input_offset
            .saturating_add(u64::try_from(chunk.len()).unwrap());
        self.drain_available()
    }

    fn finish_collect(&mut self) -> Vec<AnnexBNal> {
        let mut nals = self.drain_available();
        if let Some((start, start_len)) = find_annex_b_start_code(&self.buffer) {
            let data_start = start + start_len;
            if data_start < self.buffer.len() {
                let mut data_end = self.buffer.len();
                while data_end > data_start && self.buffer[data_end - 1] == 0 {
                    data_end -= 1;
                }
                if data_end > data_start {
                    nals.push(AnnexBNal {
                        source_offset: self.buffer_start_offset
                            + u64::try_from(data_start).unwrap(),
                        bytes: self.buffer[data_start..data_end].to_vec(),
                    });
                }
            }
        }
        self.buffer.clear();
        nals
    }

    fn drain_available(&mut self) -> Vec<AnnexBNal> {
        let mut nals = Vec::new();
        loop {
            let Some((first_start, first_len)) = find_annex_b_start_code(&self.buffer) else {
                if self.buffer.len() > 3 {
                    let retain_from = self.buffer.len() - 3;
                    self.buffer.drain(..retain_from);
                    self.buffer_start_offset += u64::try_from(retain_from).unwrap();
                }
                break;
            };
            if first_start > 0 {
                self.buffer.drain(..first_start);
                self.buffer_start_offset += u64::try_from(first_start).unwrap();
                continue;
            }
            let Some((next_start, _)) = find_annex_b_start_code(&self.buffer[first_len..])
                .map(|(start, len)| (start + first_len, len))
            else {
                break;
            };
            let data_start = first_len;
            let mut data_end = next_start;
            while data_end > data_start && self.buffer[data_end - 1] == 0 {
                data_end -= 1;
            }
            if data_end > data_start {
                nals.push(AnnexBNal {
                    source_offset: self.buffer_start_offset + u64::try_from(data_start).unwrap(),
                    bytes: self.buffer[data_start..data_end].to_vec(),
                });
            }
            self.buffer.drain(..next_start);
            self.buffer_start_offset += u64::try_from(next_start).unwrap();
        }
        nals
    }
}

fn find_annex_b_start_code(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0usize;
    while index + 2 < bytes.len() {
        if index + 3 < bytes.len() && bytes[index..].starts_with(&[0, 0, 0, 1]) {
            return Some((index, 4));
        }
        if bytes[index..].starts_with(&[0, 0, 1]) {
            return Some((index, 3));
        }
        index += 1;
    }
    None
}

fn stage_annex_b_h264_sync(path: &Path, spec: &str) -> Result<IndexedAnnexBTrack, MuxError> {
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

#[cfg(feature = "async")]
async fn stage_annex_b_h264_async(path: &Path, spec: &str) -> Result<IndexedAnnexBTrack, MuxError> {
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
            let nal_len = u32::try_from(nal.bytes.len())
                .map_err(|_| MuxError::LayoutOverflow("H.264 NAL length"))?;
            state.append_sample_nal(nal.source_offset, nal_len)?;
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
        _ if state.samples.len() == 1 => (1, 1),
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
        build_h264_sample_entry_box(&sps_info, &state.sps_list, &state.pps_list)?;
    Ok(IndexedAnnexBTrack {
        transformed_source: TransformedAnnexBSourceSpec {
            path: path.to_path_buf(),
            segments: state.segments,
            total_size: state.logical_size,
        },
        width: sps_info.width,
        height: sps_info.height,
        timescale,
        sample_entry_box,
        samples: state.samples,
    })
}

fn build_h264_sample_entry_box(
    sps_info: &H264SpsInfo,
    sequence_parameter_sets: &[Vec<u8>],
    picture_parameter_sets: &[Vec<u8>],
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
    super::mp4::encode_typed_box(&avc1, &super::mp4::encode_typed_box(&avcc, &[])?)
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
    let (timing_num_units_in_tick, timing_time_scale) = if vui_parameters_present_flag {
        parse_vui_timing(&mut reader, spec)?
    } else {
        (None, None)
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
    })
}

fn nal_to_rbsp(nal: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(nal.len());
    let mut zero_count = 0_u8;
    for &byte in nal {
        if zero_count == 2 && byte == 0x03 {
            zero_count = 0;
            continue;
        }
        rbsp.push(byte);
        if byte == 0 {
            zero_count = zero_count.saturating_add(1);
        } else {
            zero_count = 0;
        }
    }
    rbsp
}

fn parse_vui_timing<R>(
    reader: &mut BitReader<R>,
    spec: &str,
) -> Result<(Option<u32>, Option<u32>), MuxError>
where
    R: Read,
{
    if read_bit(reader, spec)? {
        let aspect_ratio_idc = read_bits_u8(reader, 8, spec)?;
        if aspect_ratio_idc == 255 {
            let _sar_width = read_bits_u16(reader, 16, spec)?;
            let _sar_height = read_bits_u16(reader, 16, spec)?;
        }
    }
    if read_bit(reader, spec)? {
        let _overscan_appropriate_flag = read_bit(reader, spec)?;
    }
    if read_bit(reader, spec)? {
        let _video_format = read_bits_u8(reader, 3, spec)?;
        let _video_full_range_flag = read_bit(reader, spec)?;
        if read_bit(reader, spec)? {
            let _colour_primaries = read_bits_u8(reader, 8, spec)?;
            let _transfer_characteristics = read_bits_u8(reader, 8, spec)?;
            let _matrix_coefficients = read_bits_u8(reader, 8, spec)?;
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
        return Ok((Some(num_units_in_tick), Some(time_scale)));
    }
    Ok((None, None))
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

fn skip_bits_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<(), MuxError>
where
    R: Read,
{
    let _ = reader
        .read_bits(width)
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })?;
    Ok(())
}

fn read_bit_labeled<R>(reader: &mut BitReader<R>, spec: &str, label: &str) -> Result<bool, MuxError>
where
    R: Read,
{
    reader
        .read_bit()
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })
}

fn read_bits_u8_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u8, MuxError>
where
    R: Read,
{
    let bits = reader
        .read_bits(width)
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })?;
    let mut value = 0_u16;
    for byte in bits {
        value = (value << 8) | u16::from(byte);
    }
    u8::try_from(value).map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("{label} bitfield does not fit in u8"),
    })
}

fn read_bits_u16_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u16, MuxError>
where
    R: Read,
{
    let bits = reader
        .read_bits(width)
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })?;
    let mut value = 0_u32;
    for byte in bits {
        value = (value << 8) | u32::from(byte);
    }
    u16::try_from(value).map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("{label} bitfield does not fit in u16"),
    })
}

fn read_bits_u32_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u32, MuxError>
where
    R: Read,
{
    let bits = reader
        .read_bits(width)
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })?;
    let mut value = 0_u64;
    for byte in bits {
        value = (value << 8) | u64::from(byte);
    }
    u32::try_from(value).map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("{label} bitfield does not fit in u32"),
    })
}

fn read_ue_labeled<R>(reader: &mut BitReader<R>, spec: &str, label: &str) -> Result<u32, MuxError>
where
    R: Read,
{
    let mut leading_zero_bits = 0_u32;
    while !read_bit_labeled(reader, spec, label)? {
        leading_zero_bits = leading_zero_bits
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("Exp-Golomb prefix"))?;
        if leading_zero_bits > 31 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("{label} Exp-Golomb value is too large"),
            });
        }
    }
    if leading_zero_bits == 0 {
        return Ok(0);
    }
    let suffix = read_bits_u32_labeled(reader, leading_zero_bits as usize, spec, label)?;
    Ok((1_u32 << leading_zero_bits) - 1 + suffix)
}

fn read_se_labeled<R>(reader: &mut BitReader<R>, spec: &str, label: &str) -> Result<i32, MuxError>
where
    R: Read,
{
    let code_num = read_ue_labeled(reader, spec, label)?;
    let magnitude =
        i32::try_from(code_num.div_ceil(2)).map_err(|_| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("{label} signed Exp-Golomb value is too large"),
        })?;
    if code_num % 2 == 0 {
        Ok(-magnitude)
    } else {
        Ok(magnitude)
    }
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
