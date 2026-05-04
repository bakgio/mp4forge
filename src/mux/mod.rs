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
pub(crate) mod event;
mod import;
mod mp4;
/// Feature-gated planned sample-reader helpers built on mux plans.
#[cfg_attr(docsrs, doc(cfg(feature = "mux")))]
pub mod sample_reader;

use coordination::MuxCoordinationPlan;
pub(crate) use coordination::{
    MuxDurationBoundaryKind, TrackCoordinationDirective, build_duration_chunk_sample_counts,
    build_duration_chunk_sample_counts_with_start_time,
    build_sync_aligned_segment_chunk_sample_counts,
};
pub(crate) use event::{MuxEventCursor, MuxEventGraph, MuxSampleEvent};
pub use import::mux_to_path;
#[cfg(feature = "async")]
pub use import::mux_to_path_async;

/// One named parameter carried inside a widened `mux` track specification.
///
/// Raw track forms may carry optional `name=value` pairs after `#`, separated by commas. The
/// parser preserves those pairs in order so later codec-specific importers can validate or consume
/// them without widening the top-level CLI surface.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MuxTrackParameter {
    name: String,
    value: String,
}

impl MuxTrackParameter {
    /// Creates one raw track parameter with the provided `name` and `value`.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }

    /// Returns the parameter name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the parameter value.
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// One codec-family prefix accepted by widened raw mux track specs.
///
/// The additive `mux` surface now uses explicit codec prefixes instead of the older generic
/// `video:` and `audio:` aliases as its authoritative public model. Self-describing families such
/// as H.264, H.265, AAC, MP3, AC-3, E-AC-3, and AC-4 parse their native framing directly, while
/// broader raw families accept explicit `#key=value` layout parameters when the source bytes are
/// not self-describing enough to derive one safe MP4 sample-entry shape automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MuxRawCodec {
    /// AV1 elementary input.
    Av1,
    /// H.264 or AVC elementary input.
    H264,
    /// H.265 or HEVC elementary input.
    H265,
    /// VP8 elementary input.
    Vp8,
    /// VP9 elementary input.
    Vp9,
    /// AAC input.
    Aac,
    /// MP3 input.
    Mp3,
    /// AC-3 input.
    Ac3,
    /// E-AC-3 input.
    Eac3,
    /// AC-4 input.
    Ac4,
    /// ALAC input.
    Alac,
    /// DTS Core input.
    Dtsc,
    /// DTS Express input.
    Dtse,
    /// DTS-HD High Resolution input.
    Dtsh,
    /// DTS-HD Master Audio input.
    Dtsl,
    /// DTS-HD MA or LBR extension input.
    Dtsm,
    /// DTS:X input.
    Dtsx,
    /// FLAC input.
    Flac,
    /// Opus input.
    Opus,
    /// IAMF input.
    Iamf,
    /// MPEG-H `mha1` input.
    Mha1,
    /// MPEG-H `mhm1` input.
    Mhm1,
}

impl MuxRawCodec {
    /// Returns the canonical CLI prefix for this raw codec.
    pub const fn prefix(&self) -> &'static str {
        match self {
            Self::Av1 => "av1",
            Self::H264 => "h264",
            Self::H265 => "h265",
            Self::Vp8 => "vp8",
            Self::Vp9 => "vp9",
            Self::Aac => "aac",
            Self::Mp3 => "mp3",
            Self::Ac3 => "ac3",
            Self::Eac3 => "ec3",
            Self::Ac4 => "ac4",
            Self::Alac => "alac",
            Self::Dtsc => "dtsc",
            Self::Dtse => "dtse",
            Self::Dtsh => "dtsh",
            Self::Dtsl => "dtsl",
            Self::Dtsm => "dtsm",
            Self::Dtsx => "dtsx",
            Self::Flac => "flac",
            Self::Opus => "opus",
            Self::Iamf => "iamf",
            Self::Mha1 => "mha1",
            Self::Mhm1 => "mhm1",
        }
    }

    /// Returns whether this raw codec family is video.
    pub const fn is_video(&self) -> bool {
        matches!(
            self,
            Self::Av1 | Self::H264 | Self::H265 | Self::Vp8 | Self::Vp9
        )
    }

    /// Returns whether this raw codec family is audio.
    pub const fn is_audio(&self) -> bool {
        !self.is_video()
    }

    fn from_prefix(prefix: &str) -> Option<Self> {
        match prefix {
            "av1" => Some(Self::Av1),
            "h264" | "video" => Some(Self::H264),
            "h265" => Some(Self::H265),
            "vp8" => Some(Self::Vp8),
            "vp9" => Some(Self::Vp9),
            "aac" | "audio" => Some(Self::Aac),
            "mp3" => Some(Self::Mp3),
            "ac3" => Some(Self::Ac3),
            "ec3" => Some(Self::Eac3),
            "ac4" => Some(Self::Ac4),
            "alac" => Some(Self::Alac),
            "dtsc" => Some(Self::Dtsc),
            "dtse" => Some(Self::Dtse),
            "dtsh" => Some(Self::Dtsh),
            "dtsl" => Some(Self::Dtsl),
            "dtsm" => Some(Self::Dtsm),
            "dtsx" => Some(Self::Dtsx),
            "flac" => Some(Self::Flac),
            "opus" => Some(Self::Opus),
            "iamf" => Some(Self::Iamf),
            "mha1" => Some(Self::Mha1),
            "mhm1" => Some(Self::Mhm1),
            _ => None,
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
/// The widened `mux` grammar now uses one repeated track-spec model for both CLI and library
/// callers:
/// - raw imports: `<codec>:PATH[#key=value[,key=value...]]`
/// - MP4 selectors: `PATH.mp4#video`, `PATH.mp4#audio`, `PATH.mp4#audio:N`, `PATH.mp4#text`,
///   `PATH.mp4#text:N`, `PATH.mp4#track:ID`
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MuxTrackSpec {
    /// Import one typed raw input from `path`.
    Raw {
        /// The raw codec family chosen by the public prefix.
        codec: MuxRawCodec,
        /// The filesystem path to import.
        path: PathBuf,
        /// Optional typed parameters carried inside the track spec.
        parameters: Vec<MuxTrackParameter>,
    },
    /// Select one track from an MP4 source file.
    Mp4 {
        /// The MP4 source path.
        path: PathBuf,
        /// The public selector to resolve inside that MP4 source.
        selector: MuxMp4TrackSelector,
    },
}

impl MuxTrackSpec {
    /// Creates one raw track specification from `codec` and `path`.
    pub fn raw(codec: MuxRawCodec, path: impl Into<PathBuf>) -> Self {
        Self::Raw {
            codec,
            path: path.into(),
            parameters: Vec::new(),
        }
    }

    /// Creates one MP4 track specification from `path` and `selector`.
    pub fn mp4(path: impl Into<PathBuf>, selector: MuxMp4TrackSelector) -> Self {
        Self::Mp4 {
            path: path.into(),
            selector,
        }
    }

    /// Returns the filesystem path referenced by this track specification.
    pub fn path(&self) -> &Path {
        match self {
            Self::Raw { path, .. } | Self::Mp4 { path, .. } => path.as_path(),
        }
    }
}

impl FromStr for MuxTrackSpec {
    type Err = MuxError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some((prefix, remainder)) = value.split_once(':')
            && let Some(codec) = MuxRawCodec::from_prefix(prefix)
        {
            let (path, parameters) = if let Some((path, parameter_text)) = remainder.split_once('#')
            {
                (path, parse_track_parameters(value, parameter_text)?)
            } else {
                (remainder, Vec::new())
            };
            if path.is_empty() {
                return Err(MuxError::InvalidTrackSpec {
                    spec: value.to_string(),
                    message: format!("missing input path after `{prefix}:`"),
                });
            }
            return Ok(Self::Raw {
                codec,
                path: PathBuf::from(path),
                parameters,
            });
        }

        let Some((path, selector)) = value.rsplit_once('#') else {
            return Err(MuxError::InvalidTrackSpec {
                spec: value.to_string(),
                message: "expected `<codec>:PATH[#key=value[,key=value...]]` or `PATH.mp4#video`, `PATH.mp4#audio`, `PATH.mp4#audio:N`, `PATH.mp4#text`, `PATH.mp4#text:N`, or `PATH.mp4#track:ID`".to_string(),
            });
        };
        if path.is_empty() {
            return Err(MuxError::InvalidTrackSpec {
                spec: value.to_string(),
                message: "missing MP4 input path before `#`".to_string(),
            });
        }
        let selector = parse_mp4_track_selector(value, selector)?;
        Ok(Self::Mp4 {
            path: PathBuf::from(path),
            selector,
        })
    }
}

fn parse_track_parameters(
    spec: &str,
    parameter_text: &str,
) -> Result<Vec<MuxTrackParameter>, MuxError> {
    if parameter_text.is_empty() {
        return Err(MuxError::InvalidTrackSpec {
            spec: spec.to_string(),
            message: "expected at least one `name=value` parameter after `#`".to_string(),
        });
    }
    let mut parameters = Vec::new();
    for part in parameter_text.split(',') {
        let Some((name, value)) = part.split_once('=') else {
            return Err(MuxError::InvalidTrackSpec {
                spec: spec.to_string(),
                message: format!("invalid track parameter `{part}`; expected `name=value`"),
            });
        };
        if name.is_empty() || value.is_empty() {
            return Err(MuxError::InvalidTrackSpec {
                spec: spec.to_string(),
                message: format!(
                    "invalid track parameter `{part}`; expected non-empty `name=value`"
                ),
            });
        }
        if parameters
            .iter()
            .any(|parameter: &MuxTrackParameter| parameter.name == name)
        {
            return Err(MuxError::InvalidTrackSpec {
                spec: spec.to_string(),
                message: format!("duplicate track parameter `{name}`"),
            });
        }
        parameters.push(MuxTrackParameter::new(name, value));
    }
    Ok(parameters)
}

fn parse_mp4_track_selector(spec: &str, selector: &str) -> Result<MuxMp4TrackSelector, MuxError> {
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

/// One high-level mux request aligned with the public CLI surface.
///
/// The narrowed public `mux` surface now centers on repeated [`MuxTrackSpec`] values, one output
/// path supplied separately to the file-backed helpers, one explicit output layout, and at most
/// one duration-boundary mode.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MuxRequest {
    tracks: Vec<MuxTrackSpec>,
    output_layout: MuxOutputLayout,
    duration_mode: Option<MuxDurationMode>,
}

impl MuxRequest {
    /// Creates one mux request from repeated public track specs.
    pub fn new(tracks: Vec<MuxTrackSpec>) -> Self {
        Self {
            tracks,
            output_layout: MuxOutputLayout::Flat,
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

    /// Returns the configured public duration-boundary mode, if any.
    pub const fn duration_mode(&self) -> Option<MuxDurationMode> {
        self.duration_mode
    }

    /// Returns a copy of this request with one explicit container layout configured.
    pub const fn with_output_layout(mut self, output_layout: MuxOutputLayout) -> Self {
        self.output_layout = output_layout;
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
    sample_entry_box: Vec<u8>,
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
            sample_entry_box,
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
            sample_entry_box,
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
            sample_entry_box,
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
            sample_entry_box,
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
    let mut planned_items = Vec::with_capacity(queue.iter().len());
    let mut track_state = BTreeMap::<u32, MuxTrackPlanState>::new();
    let mut total_payload_size = 0_u64;

    for item in queue.iter() {
        planned_items.push(MuxPlannedMediaItem {
            staged: item.staged,
            output_offset: total_payload_size,
        });

        total_payload_size = total_payload_size
            .checked_add(u64::from(item.staged.data_size))
            .ok_or(MuxError::PayloadSizeOverflow)?;

        let end_decode_time = item
            .staged
            .decode_time
            .checked_add(u64::from(item.staged.duration))
            .ok_or(MuxError::PayloadSizeOverflow)?;
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
