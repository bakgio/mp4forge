//! Feature-gated mux planning, real MP4 container assembly, and sample-reader helpers.
//!
//! The additive `mux` feature exposes two layers:
//! - low-level staged media-item planning plus payload-copy helpers
//! - higher-level real MP4 mux helpers that assemble `ftyp`, `moov`, and `mdat`
//!
//! Internally, both layers build on one mux event graph that carries stream descriptions, ordered
//! sample events, and boundary events. The task-level sample-reader helpers live under
//! [`crate::mux::sample_reader`], while the real file-backed mux surface builds actual MP4
//! container output on top of the same internal event flow.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::FourCc;
#[cfg(feature = "async")]
use crate::async_io::{AsyncReadForward, AsyncReadSeek, AsyncWrite, AsyncWriteForward};
use crate::codec::CodecError;
use crate::header::HeaderError;
use crate::queue::{OrderedWorkQueue, QueueWorkItem};
use crate::writer::WriterError;

mod coordination;
mod demux;
pub(crate) mod event;
mod import;
mod mp4;
/// Feature-gated planned sample-reader helpers built on mux plans.
#[cfg_attr(docsrs, doc(cfg(feature = "mux")))]
pub mod sample_reader;

use coordination::MuxCoordinationPlan;
pub(crate) use coordination::{
    MuxDurationBoundaryKind, TrackCoordinationDirective, build_capped_duration_chunk_sample_counts,
    build_duration_chunk_sample_counts, build_duration_chunk_sample_counts_with_start_time,
    build_sync_aligned_segment_chunk_sample_counts,
    rebalance_small_multi_audio_chunk_sample_counts,
};
pub(crate) use event::{MuxEventCursor, MuxEventGraph, MuxSampleEvent};
pub use import::mux_into_path;
#[cfg(feature = "async")]
pub use import::mux_into_path_async;
pub use import::mux_to_path;
#[cfg(feature = "async")]
pub use import::mux_to_path_async;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MuxRawCodec {
    /// AV1 elementary input.
    Av1,
    /// MPEG-4 Part 2 elementary input.
    Mp4v,
    /// H.263 elementary input.
    H263,
    /// H.264 or AVC elementary input.
    H264,
    /// H.265 or HEVC elementary input.
    H265,
    /// H.266 or VVC elementary input.
    Vvc,
    /// VP8 elementary input.
    Vp8,
    /// VP9 elementary input.
    Vp9,
    /// VP10 elementary input.
    Vp10,
    /// AAC input.
    Aac,
    /// AAC LATM input.
    Latm,
    /// MP3 input.
    Mp3,
    /// AC-3 input.
    Ac3,
    /// E-AC-3 input.
    Eac3,
    /// AC-4 input.
    Ac4,
    /// AMR narrowband input.
    Amr,
    /// AMR wideband input.
    AmrWb,
    /// QCP-wrapped voice input carrying QCELP, EVRC, or SMV frames.
    Qcp,
    /// JPEG still-image input.
    Jpeg,
    /// PNG still-image input.
    Png,
    /// WAVE or PCM input.
    Pcm,
    /// DTS core input.
    Dts,
    /// Dolby TrueHD input.
    Truehd,
    /// ALAC input.
    Alac,
    /// FLAC input.
    Flac,
    /// IAMF elementary input.
    Iamf,
    /// MPEG-H AudioMux input.
    MpegH,
    /// Opus input.
    Opus,
    /// Vorbis input.
    Vorbis,
    /// Speex input.
    Speex,
    /// Theora input.
    Theora,
}

impl MuxRawCodec {
    pub const fn prefix(&self) -> &'static str {
        match self {
            Self::Av1 => "av1",
            Self::Mp4v => "mp4v",
            Self::H263 => "h263",
            Self::H264 => "h264",
            Self::H265 => "h265",
            Self::Vvc => "vvc",
            Self::Vp8 => "vp8",
            Self::Vp9 => "vp9",
            Self::Vp10 => "vp10",
            Self::Aac => "aac",
            Self::Latm => "latm",
            Self::Mp3 => "mp3",
            Self::Ac3 => "ac3",
            Self::Eac3 => "ec3",
            Self::Ac4 => "ac4",
            Self::Amr => "amr",
            Self::AmrWb => "amr-wb",
            Self::Qcp => "qcp",
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Pcm => "pcm",
            Self::Dts => "dts",
            Self::Truehd => "truehd",
            Self::Alac => "alac",
            Self::Flac => "flac",
            Self::Iamf => "iamf",
            Self::MpegH => "mhas",
            Self::Opus => "opus",
            Self::Vorbis => "vorbis",
            Self::Speex => "speex",
            Self::Theora => "theora",
        }
    }
}

/// One MP4-side track selector accepted by widened `mux` track specs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MuxMp4TrackSelector {
    /// Select the first video track from one MP4 source.
    Video,
    /// Select one audio track occurrence from one MP4 source.
    ///
    /// The occurrence index is one-based in the public surface, so `1` means the first audio
    /// track in file order and `2` means the second.
    Audio { occurrence: u32 },
    /// Select one text-track occurrence from one MP4 source.
    ///
    /// The occurrence index is one-based in the public surface.
    Text { occurrence: u32 },
    /// Select one specific track identifier from one MP4 source.
    TrackId { track_id: u32 },
}

/// One validated public track specification for the mux task surface.
///
/// The current path-first `mux` grammar uses one repeated track-spec model for both CLI and
/// library callers:
/// - path-only imports: `PATH`
/// - path plus selector: `PATH#video`, `PATH#audio`, `PATH#audio:N`, `PATH#text`,
///   `PATH#text:N`, `PATH#track:ID`
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MuxTrackSpec {
    /// Import one input path, optionally selecting one track when the source is containerized.
    Path {
        /// The filesystem path to import.
        path: PathBuf,
        /// The optional public selector to resolve inside that source.
        selector: Option<MuxMp4TrackSelector>,
    },
}

impl MuxTrackSpec {
    /// Creates one path-first track specification from `path`.
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self::Path {
            path: path.into(),
            selector: None,
        }
    }

    /// Creates one path-first track specification from `path` and `selector`.
    pub fn selected(path: impl Into<PathBuf>, selector: MuxMp4TrackSelector) -> Self {
        Self::Path {
            path: path.into(),
            selector: Some(selector),
        }
    }

    /// Creates one compatibility selected track specification from `path` and `selector`.
    pub fn mp4(path: impl Into<PathBuf>, selector: MuxMp4TrackSelector) -> Self {
        Self::selected(path, selector)
    }

    /// Returns the filesystem path referenced by this track specification.
    pub fn input_path(&self) -> &Path {
        match self {
            Self::Path { path, .. } => path.as_path(),
        }
    }
}

impl FromStr for MuxTrackSpec {
    type Err = MuxError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            return Err(MuxError::InvalidTrackSpec {
                spec: value.to_string(),
                message: "missing input path".to_string(),
            });
        }

        if let Some((path, selector_text)) = value.rsplit_once('#') {
            if path.is_empty() {
                return Err(MuxError::InvalidTrackSpec {
                    spec: value.to_string(),
                    message: "missing input path before `#`".to_string(),
                });
            }
            let selector = parse_mp4_track_selector(value, selector_text)?;
            return Ok(Self::Path {
                path: PathBuf::from(path),
                selector: Some(selector),
            });
        }

        Ok(Self::path(value))
    }
}

fn parse_mp4_track_selector(spec: &str, selector: &str) -> Result<MuxMp4TrackSelector, MuxError> {
    if selector.is_empty() {
        return Err(MuxError::InvalidTrackSpec {
            spec: spec.to_string(),
            message:
                "expected one selector after `#`, such as `video`, `audio`, `text`, or `track:ID`"
                    .to_string(),
        });
    }
    if selector.contains('=') || selector.contains(',') {
        return Err(MuxError::InvalidTrackSpec {
            spec: spec.to_string(),
            message: "public mux track specs only allow selector suffixes such as `#video`, `#audio`, `#text`, or `#track:ID`; raw `#name=value` parameters are no longer accepted".to_string(),
        });
    }
    if selector == "video" {
        return Ok(MuxMp4TrackSelector::Video);
    }
    if selector == "audio" {
        return Ok(MuxMp4TrackSelector::Audio { occurrence: 1 });
    }
    if selector == "text" {
        return Ok(MuxMp4TrackSelector::Text { occurrence: 1 });
    }
    if let Some(index) = selector.strip_prefix("audio:") {
        let occurrence = index
            .parse::<u32>()
            .map_err(|_| MuxError::InvalidTrackSpec {
                spec: spec.to_string(),
                message: format!("invalid audio occurrence `{index}`"),
            })?;
        if occurrence == 0 {
            return Err(MuxError::InvalidTrackSpec {
                spec: spec.to_string(),
                message: "audio occurrences are one-based; `audio:0` is invalid".to_string(),
            });
        }
        return Ok(MuxMp4TrackSelector::Audio { occurrence });
    }
    if let Some(index) = selector.strip_prefix("text:") {
        let occurrence = index
            .parse::<u32>()
            .map_err(|_| MuxError::InvalidTrackSpec {
                spec: spec.to_string(),
                message: format!("invalid text occurrence `{index}`"),
            })?;
        if occurrence == 0 {
            return Err(MuxError::InvalidTrackSpec {
                spec: spec.to_string(),
                message: "text occurrences are one-based; `text:0` is invalid".to_string(),
            });
        }
        return Ok(MuxMp4TrackSelector::Text { occurrence });
    }
    if let Some(track_id) = selector.strip_prefix("track:") {
        let track_id = track_id
            .parse::<u32>()
            .map_err(|_| MuxError::InvalidTrackSpec {
                spec: spec.to_string(),
                message: format!("invalid track id `{track_id}`"),
            })?;
        if track_id == 0 {
            return Err(MuxError::InvalidTrackSpec {
                spec: spec.to_string(),
                message: "track ids are one-based; `track:0` is invalid".to_string(),
            });
        }
        return Ok(MuxMp4TrackSelector::TrackId { track_id });
    }

    Err(MuxError::InvalidTrackSpec {
        spec: spec.to_string(),
        message: format!(
            "unsupported MP4 track selector `{selector}`; expected `video`, `audio`, `audio:N`, `text`, `text:N`, or `track:ID`"
        ),
    })
}

/// Duration-boundary mode for the narrowed public mux surface.
///
/// The current `mp4forge` mux follow-on keeps the public duration surface intentionally narrow:
/// callers may request exactly one boundary mode, and today those duration-boundary modes are
/// limited to single-track jobs when the current one-file MP4 output can model them correctly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MuxDurationMode {
    /// Coordinate track chunks around one target segment duration in seconds.
    Segment { seconds: f64 },
    /// Coordinate track chunks around one target fragment duration in seconds.
    Fragment { seconds: f64 },
}

impl MuxDurationMode {
    /// Returns the public mode label used by diagnostics and CLI help.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Segment { .. } => "segment_duration",
            Self::Fragment { .. } => "fragment_duration",
        }
    }

    /// Returns the requested duration in seconds.
    pub const fn seconds(&self) -> f64 {
        match self {
            Self::Segment { seconds } | Self::Fragment { seconds } => *seconds,
        }
    }
}

/// Container layout used by the public mux request surface.
///
/// The default `mp4forge` mux behavior remains one flat `ftyp + moov + mdat` file. Fragmented
/// output is additive and explicit so callers do not accidentally change container structure just
/// by supplying one duration mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum MuxOutputLayout {
    /// Write one flat self-contained MP4 with `ftyp`, `moov`, and `mdat`.
    #[default]
    Flat,
    /// Write one fragmented MP4 with `sidx` plus one or more `moof`/`mdat` pairs.
    Fragmented,
}

impl MuxOutputLayout {
    /// Returns the public layout label used by CLI parsing and diagnostics.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Fragmented => "fragmented",
        }
    }
}

/// Destination mode used by the public mux request surface.
///
/// The force-new mode writes one newly created output file to a caller-supplied path. The
/// destination-path mode follows an update-or-create model: if the destination already exists as
/// an MP4, its tracks are preserved and additional tracks are imported into it; otherwise the same
/// path is treated as the newly created output file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum MuxDestinationMode {
    /// Write one newly created output file supplied separately to the file-backed helpers.
    #[default]
    CreateNew,
    /// Preserve one destination MP4 when it already exists, or create it at the same path.
    UpdateOrCreateDestination,
}

impl MuxDestinationMode {
    /// Returns the public destination-mode label used by CLI parsing and diagnostics.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::CreateNew => "create-new",
            Self::UpdateOrCreateDestination => "update-or-create-destination",
        }
    }
}

/// One high-level mux request aligned with the public CLI surface.
///
/// The narrowed public `mux` surface now centers on repeated [`MuxTrackSpec`] values, one
/// caller-supplied destination path, one explicit output layout, and at most one
/// duration-boundary mode.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MuxRequest {
    tracks: Vec<MuxTrackSpec>,
    output_layout: MuxOutputLayout,
    destination_mode: MuxDestinationMode,
    duration_mode: Option<MuxDurationMode>,
}

impl MuxRequest {
    /// Creates one mux request from repeated public track specs.
    pub fn new(tracks: Vec<MuxTrackSpec>) -> Self {
        Self {
            tracks,
            output_layout: MuxOutputLayout::Flat,
            destination_mode: MuxDestinationMode::CreateNew,
            duration_mode: None,
        }
    }

    /// Returns the public track specs carried by this request.
    pub fn tracks(&self) -> &[MuxTrackSpec] {
        &self.tracks
    }

    /// Returns the explicit container layout requested by the caller.
    pub const fn output_layout(&self) -> MuxOutputLayout {
        self.output_layout
    }

    /// Returns the destination mode requested by the caller.
    pub const fn destination_mode(&self) -> MuxDestinationMode {
        self.destination_mode
    }

    /// Returns the configured public duration-boundary mode, if any.
    pub const fn duration_mode(&self) -> Option<MuxDurationMode> {
        self.duration_mode
    }

    /// Returns a copy of this request with one explicit container layout configured.
    pub const fn with_output_layout(mut self, output_layout: MuxOutputLayout) -> Self {
        self.output_layout = output_layout;
        self
    }

    /// Returns a copy of this request with one explicit destination mode configured.
    pub const fn with_destination_mode(mut self, destination_mode: MuxDestinationMode) -> Self {
        self.destination_mode = destination_mode;
        self
    }

    /// Returns a copy of this request with one public duration-boundary mode configured.
    pub const fn with_duration_mode(mut self, duration_mode: MuxDurationMode) -> Self {
        self.duration_mode = Some(duration_mode);
        self
    }
}

/// Interleave policy used when ordering staged media items into one output payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum MuxInterleavePolicy {
    /// Orders staged items by normalized decode time while keeping ties stable by track and
    /// source-offset order.
    #[default]
    DecodeTime,
}

/// One staged media item that a later mux step can schedule into one output payload.
///
/// The current foundation expects `decode_time` to already be normalized onto one interleave
/// timeline across every staged source involved in the plan. Future phases can widen the staging
/// model with richer timeline normalization once full container assembly lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MuxStagedMediaItem {
    source_index: usize,
    track_id: u32,
    decode_time: u64,
    composition_time_offset: i32,
    duration: u32,
    data_offset: u64,
    data_size: u32,
    is_sync_sample: bool,
}

impl MuxStagedMediaItem {
    /// Creates one staged media item for a later mux payload plan.
    pub const fn new(
        source_index: usize,
        track_id: u32,
        decode_time: u64,
        duration: u32,
        data_offset: u64,
        data_size: u32,
    ) -> Self {
        Self {
            source_index,
            track_id,
            decode_time,
            composition_time_offset: 0,
            duration,
            data_offset,
            data_size,
            is_sync_sample: false,
        }
    }

    /// Returns the staged source slot this item will read from during payload copy.
    pub const fn source_index(&self) -> usize {
        self.source_index
    }

    /// Returns the destination track identifier for this item.
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// Returns the normalized decode time used by the current interleave planner.
    pub const fn decode_time(&self) -> u64 {
        self.decode_time
    }

    /// Returns the composition offset carried with this item.
    pub const fn composition_time_offset(&self) -> i32 {
        self.composition_time_offset
    }

    /// Returns this item's decode duration on the staged mux timeline.
    pub const fn duration(&self) -> u32 {
        self.duration
    }

    /// Returns the source byte offset for this item's sample payload.
    pub const fn data_offset(&self) -> u64 {
        self.data_offset
    }

    /// Returns the number of bytes to copy for this item's sample payload.
    pub const fn data_size(&self) -> u32 {
        self.data_size
    }

    /// Returns whether the staged item is marked as a sync sample.
    pub const fn is_sync_sample(&self) -> bool {
        self.is_sync_sample
    }

    /// Returns a copy of this item with a non-zero composition offset.
    pub const fn with_composition_time_offset(mut self, composition_time_offset: i32) -> Self {
        self.composition_time_offset = composition_time_offset;
        self
    }

    /// Returns a copy of this item with an explicit sync-sample marker.
    pub const fn with_sync_sample(mut self, is_sync_sample: bool) -> Self {
        self.is_sync_sample = is_sync_sample;
        self
    }
}

/// One planned media item with its final output payload placement.
///
/// This is the current mux-side boundary surface for future higher-level work: one item carries
/// the sample order, the source byte range, the decode interval, and the output payload span
/// without exposing the crate-private queue internals directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MuxPlannedMediaItem {
    staged: MuxStagedMediaItem,
    output_offset: u64,
}

impl MuxPlannedMediaItem {
    /// Returns the original staged media item.
    pub const fn staged(&self) -> &MuxStagedMediaItem {
        &self.staged
    }

    /// Returns the byte offset this item occupies in the final payload order.
    pub const fn output_offset(&self) -> u64 {
        self.output_offset
    }

    /// Returns the first byte offset after this item's payload in the final output order.
    pub const fn output_end_offset(&self) -> u64 {
        self.output_offset + self.staged.data_size as u64
    }

    /// Returns the decode end time of this item on the planned mux timeline.
    pub const fn decode_end_time(&self) -> u64 {
        self.staged.decode_time + self.staged.duration as u64
    }
}

/// Aggregate per-track timing and item-count information for a mux plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MuxTrackPlan {
    track_id: u32,
    item_count: u32,
    first_decode_time: u64,
    end_decode_time: u64,
}

impl MuxTrackPlan {
    /// Returns the track identifier summarized by this plan entry.
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// Returns the number of staged items scheduled for this track.
    pub const fn item_count(&self) -> u32 {
        self.item_count
    }

    /// Returns the earliest decode time assigned to this track in the current plan.
    pub const fn first_decode_time(&self) -> u64 {
        self.first_decode_time
    }

    /// Returns the decode end time of the last staged item scheduled for this track.
    pub const fn end_decode_time(&self) -> u64 {
        self.end_decode_time
    }
}

/// Planned mux payload order and per-track timing summaries.
///
/// The stable task-level plan view intentionally mirrors the internal mux event graph. Callers
/// continue to consume planned items and per-track summaries, while the crate-private event graph
/// drives the current payload-copy, chunk coordination, and planned sample-reader helpers
/// underneath.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MuxPlan {
    interleave_policy: MuxInterleavePolicy,
    planned_items: Vec<MuxPlannedMediaItem>,
    track_plans: Vec<MuxTrackPlan>,
    total_payload_size: u64,
    coordination: MuxCoordinationPlan,
    event_graph: MuxEventGraph,
}

impl MuxPlan {
    /// Returns the interleave policy used when building this plan.
    pub const fn interleave_policy(&self) -> MuxInterleavePolicy {
        self.interleave_policy
    }

    /// Returns the staged items in final payload order.
    ///
    /// This slice is the stable task-level view of the current mux event graph. Callers that need
    /// sample-order timing or payload spans should build on these planned items instead of
    /// depending on the crate-private event graph directly.
    pub fn planned_items(&self) -> &[MuxPlannedMediaItem] {
        &self.planned_items
    }

    /// Returns the per-track summaries collected during planning.
    pub fn track_plans(&self) -> &[MuxTrackPlan] {
        &self.track_plans
    }

    /// Returns the total number of bytes the planned payload copy will emit.
    pub const fn total_payload_size(&self) -> u64 {
        self.total_payload_size
    }

    pub(crate) fn chunk_sample_counts(&self, track_id: u32) -> Result<&[u32], MuxError> {
        self.coordination.chunk_sample_counts(track_id)
    }

    pub(crate) fn event_graph(&self) -> &MuxEventGraph {
        &self.event_graph
    }
}

/// File-level MP4 mux configuration for the real container-writing surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MuxFileConfig {
    movie_timescale: u32,
    major_brand: FourCc,
    minor_version: u32,
    compatible_brands: Vec<FourCc>,
    auto_flat_profile: bool,
}

impl MuxFileConfig {
    /// Creates one MP4 mux configuration with the supplied movie timescale.
    ///
    /// The default brand layout is `isom` plus `mp42` compatibility.
    pub fn new(movie_timescale: u32) -> Self {
        Self {
            movie_timescale,
            major_brand: FourCc::from_bytes(*b"isom"),
            minor_version: 0,
            compatible_brands: vec![FourCc::from_bytes(*b"isom"), FourCc::from_bytes(*b"mp42")],
            auto_flat_profile: false,
        }
    }

    /// Returns the movie timescale used for `mvhd` and `tkhd` durations.
    pub const fn movie_timescale(&self) -> u32 {
        self.movie_timescale
    }

    /// Returns the file's major brand.
    pub const fn major_brand(&self) -> FourCc {
        self.major_brand
    }

    /// Returns the file's minor version.
    pub const fn minor_version(&self) -> u32 {
        self.minor_version
    }

    /// Returns the compatible brands written into `ftyp`.
    pub fn compatible_brands(&self) -> &[FourCc] {
        &self.compatible_brands
    }

    /// Returns a copy of this configuration with a different major brand.
    pub const fn with_major_brand(mut self, major_brand: FourCc) -> Self {
        self.major_brand = major_brand;
        self
    }

    /// Returns a copy of this configuration with a different minor version.
    pub const fn with_minor_version(mut self, minor_version: u32) -> Self {
        self.minor_version = minor_version;
        self
    }

    /// Adds `brand` to the compatibility list if it is not already present.
    pub fn add_compatible_brand(&mut self, brand: FourCc) {
        if !self.compatible_brands.contains(&brand) {
            self.compatible_brands.push(brand);
        }
    }

    /// Returns a copy of this configuration with one extra compatible brand.
    pub fn with_compatible_brand(mut self, brand: FourCc) -> Self {
        self.add_compatible_brand(brand);
        self
    }

    pub(crate) const fn auto_flat_profile(&self) -> bool {
        self.auto_flat_profile
    }

    pub(crate) const fn with_auto_flat_profile(mut self, auto_flat_profile: bool) -> Self {
        self.auto_flat_profile = auto_flat_profile;
        self
    }
}

/// Track kind used by the real MP4 mux surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MuxTrackKind {
    /// Sound track with `smhd`, `soun`, and non-zero default track volume.
    Audio,
    /// Visual track with `vmhd`, `vide`, width, and height metadata.
    Video,
    /// Timed text track with `nmhd`, `text`, and zero default track volume.
    Text,
    /// Timed subtitle track with `sthd`, `subt`, and zero default track volume.
    Subtitle,
}

impl MuxTrackKind {
    /// Returns whether this track kind is audio.
    pub const fn is_audio(self) -> bool {
        matches!(self, Self::Audio)
    }

    /// Returns whether this track kind is video.
    pub const fn is_video(self) -> bool {
        matches!(self, Self::Video)
    }

    /// Returns whether this track kind is one of the timed-text families.
    pub const fn is_textual(self) -> bool {
        matches!(self, Self::Text | Self::Subtitle)
    }
}

/// Per-track configuration for the real MP4 mux surface.
///
/// The current real muxer expects one fully encoded sample-entry box per track. That keeps the
/// public API codec-agnostic while still letting callers build container output with the crate's
/// existing typed box models or with retained encoded sample-entry bytes from elsewhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MuxTrackConfig {
    track_id: u32,
    kind: MuxTrackKind,
    timescale: u32,
    language: [u8; 3],
    handler_name: String,
    track_width: u16,
    track_height: u16,
    volume: i16,
    edit_media_time: Option<u64>,
    sample_roll_distance: Option<i16>,
    sample_entry_box: Vec<u8>,
    sync_sample_table_mode: SyncSampleTableMode,
    stsc_run_encoding_mode: StscRunEncodingMode,
    flat_timing_override: Option<FlatTimingOverride>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SyncSampleTableMode {
    Auto,
    ForceEmpty,
    ForceAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StscRunEncodingMode {
    CollapseIdentical,
    PreserveTerminalBoundary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FlatTimingOverride {
    pub(crate) sample_durations: Vec<u32>,
    pub(crate) media_duration: u64,
    pub(crate) presentation_duration: u64,
}

impl MuxTrackConfig {
    /// Creates one audio-track configuration with a full encoded sample-entry box.
    pub fn new_audio(track_id: u32, timescale: u32, sample_entry_box: Vec<u8>) -> Self {
        Self {
            track_id,
            kind: MuxTrackKind::Audio,
            timescale,
            language: *b"und",
            handler_name: "SoundHandler".to_string(),
            track_width: 0,
            track_height: 0,
            volume: 0x0100,
            edit_media_time: None,
            sample_roll_distance: None,
            sample_entry_box,
            sync_sample_table_mode: SyncSampleTableMode::Auto,
            stsc_run_encoding_mode: StscRunEncodingMode::CollapseIdentical,
            flat_timing_override: None,
        }
    }

    /// Creates one video-track configuration with a full encoded sample-entry box.
    pub fn new_video(
        track_id: u32,
        timescale: u32,
        width: u16,
        height: u16,
        sample_entry_box: Vec<u8>,
    ) -> Self {
        Self {
            track_id,
            kind: MuxTrackKind::Video,
            timescale,
            language: *b"und",
            handler_name: "VideoHandler".to_string(),
            track_width: width,
            track_height: height,
            volume: 0,
            edit_media_time: None,
            sample_roll_distance: None,
            sample_entry_box,
            sync_sample_table_mode: SyncSampleTableMode::Auto,
            stsc_run_encoding_mode: StscRunEncodingMode::CollapseIdentical,
            flat_timing_override: None,
        }
    }

    /// Creates one timed-text track configuration with a full encoded sample-entry box.
    pub fn new_text(
        track_id: u32,
        timescale: u32,
        width: u16,
        height: u16,
        sample_entry_box: Vec<u8>,
    ) -> Self {
        Self {
            track_id,
            kind: MuxTrackKind::Text,
            timescale,
            language: *b"und",
            handler_name: "TextHandler".to_string(),
            track_width: width,
            track_height: height,
            volume: 0,
            edit_media_time: None,
            sample_roll_distance: None,
            sample_entry_box,
            sync_sample_table_mode: SyncSampleTableMode::Auto,
            stsc_run_encoding_mode: StscRunEncodingMode::CollapseIdentical,
            flat_timing_override: None,
        }
    }

    /// Creates one timed-subtitle track configuration with a full encoded sample-entry box.
    pub fn new_subtitle(
        track_id: u32,
        timescale: u32,
        width: u16,
        height: u16,
        sample_entry_box: Vec<u8>,
    ) -> Self {
        Self {
            track_id,
            kind: MuxTrackKind::Subtitle,
            timescale,
            language: *b"und",
            handler_name: "SubtitleHandler".to_string(),
            track_width: width,
            track_height: height,
            volume: 0,
            edit_media_time: None,
            sample_roll_distance: None,
            sample_entry_box,
            sync_sample_table_mode: SyncSampleTableMode::Auto,
            stsc_run_encoding_mode: StscRunEncodingMode::CollapseIdentical,
            flat_timing_override: None,
        }
    }

    /// Returns the track identifier.
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// Returns the configured track kind.
    pub const fn kind(&self) -> MuxTrackKind {
        self.kind
    }

    /// Returns the media timescale used by this track's `mdhd` and sample tables.
    pub const fn timescale(&self) -> u32 {
        self.timescale
    }

    /// Returns the three-letter ISO-639-2 language code carried by this track.
    pub const fn language(&self) -> [u8; 3] {
        self.language
    }

    /// Returns the handler name written into `hdlr`.
    pub fn handler_name(&self) -> &str {
        &self.handler_name
    }

    /// Returns the width recorded in `tkhd` for this track.
    pub const fn track_width(&self) -> u16 {
        self.track_width
    }

    /// Returns the height recorded in `tkhd` for this track.
    pub const fn track_height(&self) -> u16 {
        self.track_height
    }

    /// Returns the fixed-point 8.8 track volume written into `tkhd`.
    pub const fn volume(&self) -> i16 {
        self.volume
    }

    /// Returns the optional media-time trim that should be written into one edit list.
    pub const fn edit_media_time(&self) -> Option<u64> {
        self.edit_media_time
    }

    pub(crate) const fn sample_roll_distance(&self) -> Option<i16> {
        self.sample_roll_distance
    }

    /// Returns the full encoded sample-entry box written under `stsd`.
    pub fn sample_entry_box(&self) -> &[u8] {
        &self.sample_entry_box
    }

    /// Returns a copy of this configuration with a different language code.
    pub const fn with_language(mut self, language: [u8; 3]) -> Self {
        self.language = language;
        self
    }

    /// Returns a copy of this configuration with a different `hdlr` name.
    pub fn with_handler_name(mut self, handler_name: impl Into<String>) -> Self {
        self.handler_name = handler_name.into();
        self
    }

    /// Returns a copy of this configuration with a different fixed-point 8.8 track volume.
    pub const fn with_volume(mut self, volume: i16) -> Self {
        self.volume = volume;
        self
    }

    /// Returns a copy of this configuration with one edit-list media-time trim.
    pub const fn with_edit_media_time(mut self, edit_media_time: u64) -> Self {
        self.edit_media_time = Some(edit_media_time);
        self
    }

    pub(crate) const fn with_sample_roll_distance(mut self, sample_roll_distance: i16) -> Self {
        self.sample_roll_distance = Some(sample_roll_distance);
        self
    }

    pub(crate) const fn with_sync_sample_table_mode(
        mut self,
        sync_sample_table_mode: SyncSampleTableMode,
    ) -> Self {
        self.sync_sample_table_mode = sync_sample_table_mode;
        self
    }

    pub(crate) const fn stsc_run_encoding_mode(&self) -> StscRunEncodingMode {
        self.stsc_run_encoding_mode
    }

    pub(crate) const fn with_stsc_run_encoding_mode(
        mut self,
        stsc_run_encoding_mode: StscRunEncodingMode,
    ) -> Self {
        self.stsc_run_encoding_mode = stsc_run_encoding_mode;
        self
    }

    pub(crate) fn flat_timing_override(&self) -> Option<&FlatTimingOverride> {
        self.flat_timing_override.as_ref()
    }

    pub(crate) fn with_flat_timing_override(
        mut self,
        flat_timing_override: FlatTimingOverride,
    ) -> Self {
        self.flat_timing_override = Some(flat_timing_override);
        self
    }
}

/// Errors returned by the additive mux foundation helpers.
#[derive(Debug)]
pub enum MuxError {
    /// One public mux track spec did not match the fixed supported grammar.
    InvalidTrackSpec { spec: String, message: String },
    /// The current mux request selected more than one video track.
    MultipleVideoTracks { count: usize },
    /// The current mux request did not carry any tracks.
    MissingTrackSpecs,
    /// One requested MP4 track selector did not resolve to a matching track.
    MissingTrackSelection { spec: String },
    /// One track import was recognized but is not supported by the current mux follow-on.
    UnsupportedTrackImport { spec: String, message: String },
    /// One duration-boundary mode conflicts with the current request shape or requested value.
    InvalidDurationMode { mode: &'static str, message: String },
    /// One explicit mux output layout conflicts with the current request shape.
    InvalidOutputLayout {
        layout: &'static str,
        message: String,
    },
    /// One explicit destination mode conflicts with the current request shape.
    InvalidDestinationMode { mode: &'static str, message: String },
    /// The output path conflicts with one of the supplied input paths.
    OutputPathConflict { output: PathBuf, input: PathBuf },
    /// One track timeline could not be normalized onto the selected movie timescale exactly.
    IncompatibleTrackTiming {
        track_id: u32,
        track_timescale: u32,
        movie_timescale: u32,
        value: i64,
    },
    /// One chunk or segment coordination plan was internally inconsistent.
    InvalidChunkPlan { track_id: u32, message: String },
    /// The planned payload would overflow a 64-bit output offset or size.
    PayloadSizeOverflow,
    /// One planned item referenced a staged source index that was not provided by the caller.
    MissingSourceIndex {
        source_index: usize,
        source_count: usize,
    },
    /// A progressive source would need to seek backward to satisfy the staged plan.
    NonMonotonicSourceOffset {
        source_index: usize,
        previous_offset: u64,
        next_offset: u64,
    },
    /// A progressive source ended before it reached the requested staged offset.
    IncompleteAdvance {
        source_index: usize,
        expected_offset: u64,
        actual_offset: u64,
    },
    /// A source did not produce the number of bytes described by the plan.
    IncompleteCopy {
        source_index: usize,
        expected_size: u64,
        actual_size: u64,
    },
    /// The real mux surface requires a non-zero movie timescale.
    InvalidMovieTimescale,
    /// One real mux track configuration used a zero or otherwise incompatible media timescale.
    InvalidTrackTimescale { track_id: u32 },
    /// One real mux track language code was not a valid three-letter ISO-639-2 code.
    InvalidTrackLanguage { track_id: u32, language: String },
    /// More than one track configuration used the same track identifier.
    DuplicateTrackId { track_id: u32 },
    /// The plan referenced a track that was not configured for the real mux surface.
    MissingTrackId { track_id: u32 },
    /// One configured track had no planned samples.
    TrackHasNoSamples { track_id: u32 },
    /// One track regressed in decode ordering inside the mux event graph.
    NonMonotonicTrackDecodeTime {
        track_id: u32,
        previous_decode_time: u64,
        next_decode_time: u64,
    },
    /// One configured sample-entry box was not a single valid encoded box.
    InvalidSampleEntryBox { track_id: u32, message: String },
    /// The real mux layout overflowed one container field.
    LayoutOverflow(&'static str),
    /// A typed box payload could not be encoded.
    Codec(CodecError),
    /// A container box could not be written or finalized.
    Writer(WriterError),
    /// A box header could not be parsed or encoded.
    Header(HeaderError),
    /// One typed extract helper failed while importing a track.
    Extract(crate::extract::ExtractError),
    /// One typed probe helper failed while importing a track.
    Probe(crate::probe::ProbeError),
    /// An I/O error occurred while reading staged payloads or writing output bytes.
    Io(io::Error),
}

impl fmt::Display for MuxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTrackSpec { spec, message } => {
                write!(f, "invalid mux track spec `{spec}`: {message}")
            }
            Self::MultipleVideoTracks { count } => write!(
                f,
                "the current mux surface supports at most one video track per job, but {count} were requested"
            ),
            Self::MissingTrackSpecs => {
                write!(
                    f,
                    "the current mux surface requires at least one `--track` input"
                )
            }
            Self::MissingTrackSelection { spec } => {
                write!(
                    f,
                    "mux track spec `{spec}` did not resolve to a matching input track"
                )
            }
            Self::UnsupportedTrackImport { spec, message } => {
                write!(f, "mux track spec `{spec}` is not supported: {message}")
            }
            Self::InvalidDurationMode { mode, message } => {
                write!(f, "invalid mux {mode}: {message}")
            }
            Self::InvalidOutputLayout { layout, message } => {
                write!(f, "invalid mux layout `{layout}`: {message}")
            }
            Self::InvalidDestinationMode { mode, message } => {
                write!(f, "invalid mux destination mode `{mode}`: {message}")
            }
            Self::OutputPathConflict { output, input } => write!(
                f,
                "output path `{}` conflicts with input `{}`",
                output.display(),
                input.display()
            ),
            Self::IncompatibleTrackTiming {
                track_id,
                track_timescale,
                movie_timescale,
                value,
            } => write!(
                f,
                "track {track_id} timing value {value} from timescale {track_timescale} cannot be normalized exactly onto movie timescale {movie_timescale}"
            ),
            Self::InvalidChunkPlan { track_id, message } => {
                write!(
                    f,
                    "track {track_id} produced an invalid chunk plan: {message}"
                )
            }
            Self::PayloadSizeOverflow => {
                write!(f, "planned mux payload size overflowed the supported range")
            }
            Self::MissingSourceIndex {
                source_index,
                source_count,
            } => write!(
                f,
                "mux plan referenced source index {source_index}, but only {source_count} sources were provided"
            ),
            Self::NonMonotonicSourceOffset {
                source_index,
                previous_offset,
                next_offset,
            } => write!(
                f,
                "source index {source_index} would need to move backward from offset {previous_offset} to {next_offset}"
            ),
            Self::IncompleteAdvance {
                source_index,
                expected_offset,
                actual_offset,
            } => write!(
                f,
                "source index {source_index} ended while advancing to offset {expected_offset}; only reached {actual_offset}"
            ),
            Self::IncompleteCopy {
                source_index,
                expected_size,
                actual_size,
            } => write!(
                f,
                "source index {source_index} produced {actual_size} bytes, expected {expected_size}"
            ),
            Self::InvalidMovieTimescale => {
                write!(f, "real mux output requires a non-zero movie timescale")
            }
            Self::InvalidTrackTimescale { track_id } => {
                write!(
                    f,
                    "track {track_id} uses an invalid or incompatible media timescale for the planned mux timeline"
                )
            }
            Self::InvalidTrackLanguage { track_id, language } => write!(
                f,
                "track {track_id} uses invalid language code `{language}`; expected three ASCII letters"
            ),
            Self::DuplicateTrackId { track_id } => {
                write!(f, "duplicate mux track id {track_id}")
            }
            Self::MissingTrackId { track_id } => {
                write!(
                    f,
                    "mux plan referenced track id {track_id}, but no matching track configuration was provided"
                )
            }
            Self::TrackHasNoSamples { track_id } => {
                write!(f, "mux track {track_id} has no planned samples")
            }
            Self::NonMonotonicTrackDecodeTime {
                track_id,
                previous_decode_time,
                next_decode_time,
            } => write!(
                f,
                "track {track_id} regressed in decode order from {previous_decode_time} to {next_decode_time}"
            ),
            Self::InvalidSampleEntryBox { track_id, message } => write!(
                f,
                "track {track_id} provided an invalid sample-entry box: {message}"
            ),
            Self::LayoutOverflow(field) => write!(
                f,
                "real mux layout overflowed the supported range while building {field}"
            ),
            Self::Codec(error) => error.fmt(f),
            Self::Writer(error) => error.fmt(f),
            Self::Header(error) => error.fmt(f),
            Self::Extract(error) => error.fmt(f),
            Self::Probe(error) => error.fmt(f),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl Error for MuxError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            Self::Writer(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::Extract(error) => Some(error),
            Self::Probe(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for MuxError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<CodecError> for MuxError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl From<WriterError> for MuxError {
    fn from(error: WriterError) -> Self {
        Self::Writer(error)
    }
}

impl From<HeaderError> for MuxError {
    fn from(error: HeaderError) -> Self {
        Self::Header(error)
    }
}

impl From<crate::extract::ExtractError> for MuxError {
    fn from(error: crate::extract::ExtractError) -> Self {
        Self::Extract(error)
    }
}

impl From<crate::probe::ProbeError> for MuxError {
    fn from(error: crate::probe::ProbeError) -> Self {
        Self::Probe(error)
    }
}

/// Plans one output payload order from staged media items using the selected interleave policy.
pub fn plan_staged_media_items(
    items: Vec<MuxStagedMediaItem>,
    interleave_policy: MuxInterleavePolicy,
) -> Result<MuxPlan, MuxError> {
    plan_staged_media_items_with_coordination(items, interleave_policy, Vec::new())
}

pub(crate) fn plan_staged_media_items_with_coordination(
    items: Vec<MuxStagedMediaItem>,
    interleave_policy: MuxInterleavePolicy,
    coordination_directives: Vec<TrackCoordinationDirective>,
) -> Result<MuxPlan, MuxError> {
    let mut queue_items = items
        .into_iter()
        .map(MuxQueueItem::from_staged)
        .collect::<Vec<_>>();

    match interleave_policy {
        MuxInterleavePolicy::DecodeTime => {
            // Keep equal decode-time items stable by track, source, and byte offset before the
            // queue layer applies the decode-time ordering key.
            queue_items.sort_by_key(|item| {
                (
                    item.staged.track_id,
                    item.staged.source_index,
                    item.staged.data_offset,
                )
            });
        }
    }

    let queue = OrderedWorkQueue::new(queue_items);
    let mut items_by_track = BTreeMap::<u32, Vec<MuxStagedMediaItem>>::new();
    let mut track_state = BTreeMap::<u32, MuxTrackPlanState>::new();

    for item in queue.iter() {
        let end_decode_time = item
            .staged
            .decode_time
            .checked_add(u64::from(item.staged.duration))
            .ok_or(MuxError::PayloadSizeOverflow)?;
        items_by_track
            .entry(item.staged.track_id)
            .or_default()
            .push(item.staged);
        track_state
            .entry(item.staged.track_id)
            .and_modify(|state| {
                state.item_count += 1;
                state.end_decode_time = state.end_decode_time.max(end_decode_time);
                state.first_decode_time = state.first_decode_time.min(item.staged.decode_time);
            })
            .or_insert(MuxTrackPlanState {
                item_count: 1,
                first_decode_time: item.staged.decode_time,
                end_decode_time,
            });
    }

    let track_plans = track_state
        .into_iter()
        .map(|(track_id, state)| MuxTrackPlan {
            track_id,
            item_count: state.item_count,
            first_decode_time: state.first_decode_time,
            end_decode_time: state.end_decode_time,
        })
        .collect::<Vec<_>>();

    let coordination =
        MuxCoordinationPlan::from_track_plans(&track_plans, coordination_directives)?;
    let (planned_items, total_payload_size) =
        build_planned_items_from_tracks(&items_by_track, &coordination, interleave_policy)?;
    let event_graph = MuxEventGraph::from_plan(
        &planned_items,
        &track_plans,
        total_payload_size,
        &coordination,
    );

    Ok(MuxPlan {
        interleave_policy,
        planned_items,
        track_plans,
        total_payload_size,
        coordination,
        event_graph,
    })
}

/// Writes one real MP4 file to `writer` from staged seekable `sources`, `plan`, and track
/// metadata.
///
/// This higher-level mux surface assembles `ftyp`, `moov`, and `mdat` around the staged sample
/// order produced by [`plan_staged_media_items`]. The lower-level payload-copy helpers remain
/// available for callers that only need interleaved raw payload output.
pub fn write_mp4_mux<R, W>(
    sources: &mut [R],
    writer: &mut W,
    file_config: &MuxFileConfig,
    track_configs: &[MuxTrackConfig],
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    R: Read + Seek,
    W: Write,
{
    mp4::write_mp4_mux(sources, writer, file_config, track_configs, plan)
}

/// Opens staged source files and writes one real MP4 file to `output_path`.
pub fn write_mp4_mux_to_path<P, Q>(
    source_paths: &[P],
    output_path: Q,
    file_config: &MuxFileConfig,
    track_configs: &[MuxTrackConfig],
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    mp4::write_mp4_mux_to_path(source_paths, output_path, file_config, track_configs, plan)
}

/// Writes one real MP4 file through the additive Tokio-based async mux surface.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(all(feature = "mux", feature = "async"))))]
pub async fn write_mp4_mux_async<R, W>(
    sources: &mut [R],
    writer: &mut W,
    file_config: &MuxFileConfig,
    track_configs: &[MuxTrackConfig],
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    R: AsyncReadSeek,
    W: AsyncWrite + Unpin,
{
    mp4::write_mp4_mux_async(sources, writer, file_config, track_configs, plan).await
}

/// Opens staged source files asynchronously and writes one real MP4 file to `output_path`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(all(feature = "mux", feature = "async"))))]
pub async fn write_mp4_mux_to_path_async<P, Q>(
    source_paths: &[P],
    output_path: Q,
    file_config: &MuxFileConfig,
    track_configs: &[MuxTrackConfig],
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    mp4::write_mp4_mux_to_path_async(source_paths, output_path, file_config, track_configs, plan)
        .await
}

/// Copies the payload bytes described by `plan` from the staged seekable `sources` into
/// `writer`.
pub fn copy_planned_payloads<R, W>(
    sources: &mut [R],
    writer: &mut W,
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    R: Read + Seek,
    W: Write,
{
    let mut cursor = plan.event_graph.cursor();
    while let Some(sample) = cursor.next_sample() {
        let staged = sample.planned_item().staged();
        let Some(source) = sources.get_mut(staged.source_index()) else {
            return Err(MuxError::MissingSourceIndex {
                source_index: staged.source_index(),
                source_count: sources.len(),
            });
        };

        source.seek(SeekFrom::Start(staged.data_offset()))?;
        let mut limited = source.take(u64::from(staged.data_size()));
        let copied = io::copy(&mut limited, writer)?;
        if copied != u64::from(staged.data_size()) {
            return Err(MuxError::IncompleteCopy {
                source_index: staged.source_index(),
                expected_size: u64::from(staged.data_size()),
                actual_size: copied,
            });
        }
    }

    Ok(())
}

/// Copies the payload bytes described by `plan` from staged non-seekable `sources` into `writer`.
///
/// This progressive path keeps one forward-only read cursor per source. It supports plans whose
/// staged items consume each source in monotonic byte-offset order, and it reports a structured
/// error when a caller asks it to seek backward implicitly.
pub fn copy_planned_payloads_progressive<R, W>(
    sources: &mut [R],
    writer: &mut W,
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    R: Read,
    W: Write,
{
    let mut source_offsets = vec![0_u64; sources.len()];
    let mut cursor = plan.event_graph.cursor();
    while let Some(sample) = cursor.next_sample() {
        let staged = sample.planned_item().staged();
        let Some(source) = sources.get_mut(staged.source_index()) else {
            return Err(MuxError::MissingSourceIndex {
                source_index: staged.source_index(),
                source_count: sources.len(),
            });
        };

        let source_offset = source_offsets.get_mut(staged.source_index()).unwrap();
        advance_progressive_source(
            source,
            staged.source_index(),
            source_offset,
            staged.data_offset(),
        )?;
        copy_progressive_payload(
            source,
            writer,
            staged.source_index(),
            source_offset,
            u64::from(staged.data_size()),
        )?;
    }

    Ok(())
}

/// Opens staged source files and copies the payload bytes described by `plan` into `output_path`.
pub fn copy_planned_payloads_to_path<P, Q>(
    source_paths: &[P],
    output_path: Q,
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let mut sources = source_paths
        .iter()
        .map(File::open)
        .collect::<Result<Vec<_>, _>>()?;
    let mut writer = File::create(output_path)?;
    copy_planned_payloads(&mut sources, &mut writer, plan)
}

/// Copies the payload bytes described by `plan` from the staged seekable async `sources` into
/// `writer`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(all(feature = "mux", feature = "async"))))]
pub async fn copy_planned_payloads_async<R, W>(
    sources: &mut [R],
    writer: &mut W,
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    R: AsyncReadSeek,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut cursor = plan.event_graph.cursor();
    while let Some(sample) = cursor.next_sample() {
        let staged = sample.planned_item().staged();
        let Some(source) = sources.get_mut(staged.source_index()) else {
            return Err(MuxError::MissingSourceIndex {
                source_index: staged.source_index(),
                source_count: sources.len(),
            });
        };

        source.seek(SeekFrom::Start(staged.data_offset())).await?;
        let mut remaining = u64::from(staged.data_size());
        let mut copied = 0_u64;
        while remaining > 0 {
            let chunk_len = remaining.min(buffer.len() as u64) as usize;
            let read = source.read(&mut buffer[..chunk_len]).await?;
            if read == 0 {
                break;
            }
            writer.write_all(&buffer[..read]).await?;
            copied += read as u64;
            remaining -= read as u64;
        }

        if copied != u64::from(staged.data_size()) {
            return Err(MuxError::IncompleteCopy {
                source_index: staged.source_index(),
                expected_size: u64::from(staged.data_size()),
                actual_size: copied,
            });
        }
    }

    writer.flush().await?;
    Ok(())
}

/// Copies the payload bytes described by `plan` from staged non-seekable async `sources` into
/// `writer`.
///
/// Like [`copy_planned_payloads_progressive`], this path supports only plans whose staged items
/// consume each source in monotonic byte-offset order.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(all(feature = "mux", feature = "async"))))]
pub async fn copy_planned_payloads_async_progressive<R, W>(
    sources: &mut [R],
    writer: &mut W,
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    R: AsyncReadForward,
    W: AsyncWriteForward,
{
    let mut source_offsets = vec![0_u64; sources.len()];
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut cursor = plan.event_graph.cursor();
    while let Some(sample) = cursor.next_sample() {
        let staged = sample.planned_item().staged();
        let Some(source) = sources.get_mut(staged.source_index()) else {
            return Err(MuxError::MissingSourceIndex {
                source_index: staged.source_index(),
                source_count: sources.len(),
            });
        };

        let source_offset = source_offsets.get_mut(staged.source_index()).unwrap();
        advance_progressive_source_async(
            source,
            staged.source_index(),
            source_offset,
            staged.data_offset(),
            &mut buffer,
        )
        .await?;
        copy_progressive_payload_async(
            source,
            writer,
            staged.source_index(),
            source_offset,
            u64::from(staged.data_size()),
            &mut buffer,
        )
        .await?;
    }

    writer.flush().await?;
    Ok(())
}

/// Opens staged source files asynchronously and copies the payload bytes described by `plan` into
/// `output_path`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(all(feature = "mux", feature = "async"))))]
pub async fn copy_planned_payloads_to_path_async<P, Q>(
    source_paths: &[P],
    output_path: Q,
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let mut sources = Vec::with_capacity(source_paths.len());
    for path in source_paths {
        sources.push(TokioFile::open(path).await?);
    }
    let mut writer = TokioFile::create(output_path).await?;
    copy_planned_payloads_async(&mut sources, &mut writer, plan).await
}

struct MuxQueueItem {
    staged: MuxStagedMediaItem,
}

impl MuxQueueItem {
    fn from_staged(staged: MuxStagedMediaItem) -> Self {
        Self { staged }
    }
}

impl QueueWorkItem for MuxQueueItem {
    fn queue_order_key(&self) -> u64 {
        self.staged.decode_time
    }
}

struct MuxTrackPlanState {
    item_count: u32,
    first_decode_time: u64,
    end_decode_time: u64,
}

#[derive(Clone, Copy)]
struct PlannedChunk {
    order_key: PlannedChunkOrderKey,
    track_id: u32,
    start_index: usize,
    end_index: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PlannedChunkOrderKey {
    decode_time: u64,
    track_id: u32,
    source_index: usize,
    data_offset: u64,
}

fn build_planned_items_from_tracks(
    items_by_track: &BTreeMap<u32, Vec<MuxStagedMediaItem>>,
    coordination: &MuxCoordinationPlan,
    interleave_policy: MuxInterleavePolicy,
) -> Result<(Vec<MuxPlannedMediaItem>, u64), MuxError> {
    let mut chunks = Vec::new();
    let total_sample_count = items_by_track.values().map(Vec::len).sum();
    for (&track_id, items) in items_by_track {
        let chunk_sample_counts = coordination.chunk_sample_counts(track_id)?;
        let mut start_index = 0_usize;
        for &samples_per_chunk in chunk_sample_counts {
            let chunk_len = usize::try_from(samples_per_chunk)
                .map_err(|_| MuxError::LayoutOverflow("chunk sample-count conversion"))?;
            let end_index = start_index
                .checked_add(chunk_len)
                .ok_or(MuxError::LayoutOverflow("chunk sample indexing"))?;
            let first_sample =
                items
                    .get(start_index)
                    .ok_or_else(|| MuxError::InvalidChunkPlan {
                        track_id,
                        message: "chunk boundaries ran past the staged sample count".to_string(),
                    })?;
            chunks.push(PlannedChunk {
                order_key: PlannedChunkOrderKey {
                    decode_time: first_sample.decode_time(),
                    track_id,
                    source_index: first_sample.source_index(),
                    data_offset: first_sample.data_offset(),
                },
                track_id,
                start_index,
                end_index,
            });
            start_index = end_index;
        }
        if start_index != items.len() {
            return Err(MuxError::InvalidChunkPlan {
                track_id,
                message: "chunk boundaries did not cover every staged sample".to_string(),
            });
        }
    }

    match interleave_policy {
        MuxInterleavePolicy::DecodeTime => {
            chunks.sort_by_key(|chunk| chunk.order_key);
        }
    }

    let mut planned_items = Vec::with_capacity(total_sample_count);
    let mut total_payload_size = 0_u64;
    for chunk in chunks {
        let items = items_by_track
            .get(&chunk.track_id)
            .ok_or(MuxError::MissingTrackId {
                track_id: chunk.track_id,
            })?;
        for staged in &items[chunk.start_index..chunk.end_index] {
            planned_items.push(MuxPlannedMediaItem {
                staged: *staged,
                output_offset: total_payload_size,
            });
            total_payload_size = total_payload_size
                .checked_add(u64::from(staged.data_size()))
                .ok_or(MuxError::PayloadSizeOverflow)?;
        }
    }

    Ok((planned_items, total_payload_size))
}

fn advance_progressive_source<R>(
    source: &mut R,
    source_index: usize,
    current_offset: &mut u64,
    target_offset: u64,
) -> Result<(), MuxError>
where
    R: Read,
{
    if target_offset < *current_offset {
        return Err(MuxError::NonMonotonicSourceOffset {
            source_index,
            previous_offset: *current_offset,
            next_offset: target_offset,
        });
    }

    let mut remaining = target_offset - *current_offset;
    let mut buffer = [0_u8; 16 * 1024];
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len() as u64) as usize;
        let read = source.read(&mut buffer[..chunk_len])?;
        if read == 0 {
            return Err(MuxError::IncompleteAdvance {
                source_index,
                expected_offset: target_offset,
                actual_offset: *current_offset,
            });
        }
        *current_offset += read as u64;
        remaining -= read as u64;
    }

    Ok(())
}

fn copy_progressive_payload<R, W>(
    source: &mut R,
    writer: &mut W,
    source_index: usize,
    current_offset: &mut u64,
    size: u64,
) -> Result<(), MuxError>
where
    R: Read,
    W: Write,
{
    let mut remaining = size;
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len() as u64) as usize;
        let read = source.read(&mut buffer[..chunk_len])?;
        if read == 0 {
            return Err(MuxError::IncompleteCopy {
                source_index,
                expected_size: size,
                actual_size: copied,
            });
        }
        writer.write_all(&buffer[..read])?;
        *current_offset += read as u64;
        copied += read as u64;
        remaining -= read as u64;
    }

    Ok(())
}

#[cfg(feature = "async")]
async fn advance_progressive_source_async<R>(
    source: &mut R,
    source_index: usize,
    current_offset: &mut u64,
    target_offset: u64,
    buffer: &mut [u8],
) -> Result<(), MuxError>
where
    R: AsyncReadForward,
{
    if target_offset < *current_offset {
        return Err(MuxError::NonMonotonicSourceOffset {
            source_index,
            previous_offset: *current_offset,
            next_offset: target_offset,
        });
    }

    let mut remaining = target_offset - *current_offset;
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len() as u64) as usize;
        let read = source.read(&mut buffer[..chunk_len]).await?;
        if read == 0 {
            return Err(MuxError::IncompleteAdvance {
                source_index,
                expected_offset: target_offset,
                actual_offset: *current_offset,
            });
        }
        *current_offset += read as u64;
        remaining -= read as u64;
    }

    Ok(())
}

#[cfg(feature = "async")]
async fn copy_progressive_payload_async<R, W>(
    source: &mut R,
    writer: &mut W,
    source_index: usize,
    current_offset: &mut u64,
    size: u64,
    buffer: &mut [u8],
) -> Result<(), MuxError>
where
    R: AsyncReadForward,
    W: AsyncWriteForward,
{
    let mut remaining = size;
    let mut copied = 0_u64;
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len() as u64) as usize;
        let read = source.read(&mut buffer[..chunk_len]).await?;
        if read == 0 {
            return Err(MuxError::IncompleteCopy {
                source_index,
                expected_size: size,
                actual_size: copied,
            });
        }
        writer.write_all(&buffer[..read]).await?;
        *current_offset += read as u64;
        copied += read as u64;
        remaining -= read as u64;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinated_chunk_plans_keep_multi_sample_chunks_contiguous_in_output_order() {
        let plan = plan_staged_media_items_with_coordination(
            vec![
                MuxStagedMediaItem::new(0, 1, 0, 10, 0, 4),
                MuxStagedMediaItem::new(0, 1, 10, 10, 4, 4),
                MuxStagedMediaItem::new(1, 2, 0, 10, 0, 3),
                MuxStagedMediaItem::new(1, 2, 10, 10, 3, 3),
            ],
            MuxInterleavePolicy::DecodeTime,
            vec![
                TrackCoordinationDirective::new(1, vec![2]),
                TrackCoordinationDirective::new(2, vec![2]),
            ],
        )
        .unwrap();

        let planned = plan.planned_items();
        assert_eq!(planned.len(), 4);
        assert_eq!(planned[0].staged().track_id(), 1);
        assert_eq!(planned[1].staged().track_id(), 1);
        assert_eq!(planned[2].staged().track_id(), 2);
        assert_eq!(planned[3].staged().track_id(), 2);
        assert_eq!(planned[0].output_offset(), 0);
        assert_eq!(planned[1].output_offset(), 4);
        assert_eq!(planned[2].output_offset(), 8);
        assert_eq!(planned[3].output_offset(), 11);
    }
}
