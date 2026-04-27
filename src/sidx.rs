//! Top-level `sidx` helpers for fragmented MP4 files.
//!
//! These helpers keep `mp4forge`'s existing box extraction and rewrite surfaces unchanged while
//! exposing the file-level defaults needed to analyze fragmented files, build typed update plans,
//! and apply those plans without disturbing unrelated bytes.

use std::error::Error;
use std::fmt;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use crate::FourCc;
#[cfg(feature = "async")]
use crate::async_io::{AsyncReadSeek, AsyncWriteSeek};
use crate::boxes::iso14496_12::{
    Mdhd, Sidx, SidxReference, TFHD_DEFAULT_SAMPLE_DURATION_PRESENT,
    TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT, Tfdt, Tfhd, Tkhd,
    Trex, Trun,
};
use crate::codec::{CodecBox, CodecError, ImmutableBox, MutableBox, marshal, unmarshal};
use crate::extract::{ExtractError, extract_box_as, extract_boxes};
use crate::header::{BoxInfo, HeaderError, LARGE_HEADER_SIZE, SMALL_HEADER_SIZE};
use crate::walk::BoxPath;
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

const MOOV: FourCc = FourCc::from_bytes(*b"moov");
const MOOF: FourCc = FourCc::from_bytes(*b"moof");
const MVEX: FourCc = FourCc::from_bytes(*b"mvex");
const MDAT: FourCc = FourCc::from_bytes(*b"mdat");
const STYP: FourCc = FourCc::from_bytes(*b"styp");
const SIDX: FourCc = FourCc::from_bytes(*b"sidx");
const TRAK: FourCc = FourCc::from_bytes(*b"trak");
const TREX: FourCc = FourCc::from_bytes(*b"trex");
const TKHD: FourCc = FourCc::from_bytes(*b"tkhd");
const MDIA: FourCc = FourCc::from_bytes(*b"mdia");
const MDHD: FourCc = FourCc::from_bytes(*b"mdhd");
const HDLR: FourCc = FourCc::from_bytes(*b"hdlr");
const TRAF: FourCc = FourCc::from_bytes(*b"traf");
const TFHD: FourCc = FourCc::from_bytes(*b"tfhd");
const TFDT: FourCc = FourCc::from_bytes(*b"tfdt");
const TRUN: FourCc = FourCc::from_bytes(*b"trun");
const EMSG: FourCc = FourCc::from_bytes(*b"emsg");
const VIDE: FourCc = FourCc::from_bytes(*b"vide");
const SOUN: FourCc = FourCc::from_bytes(*b"soun");

#[derive(Clone)]
struct InitTrackInfo {
    track_id: u32,
    handler_type: Option<FourCc>,
    timescale: u32,
}

struct InitAnalysis {
    timing_track: SidxTimingTrackInfo,
    trex: Trex,
}

#[derive(Clone)]
struct SegmentAccumulator {
    first_box: BoxInfo,
    moofs: Vec<BoxInfo>,
    size: u64,
    segment_sidx_count: usize,
}

#[derive(Clone)]
struct ExistingTopLevelSidxInternal {
    public: ExistingTopLevelSidx,
}

/// File-level analysis data needed to derive the default inputs for a top-level `sidx` refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopLevelSidxUpdateAnalysis {
    /// Track chosen for timing calculations.
    pub timing_track: SidxTimingTrackInfo,
    /// Derived media-segment inputs that feed `sidx` entry construction.
    pub segments: Vec<SidxMediaSegment>,
    /// Existing top-level `sidx` placement data plus the first insertion position for a new box.
    pub placement: TopLevelSidxPlacement,
}

/// Timing-track metadata derived from the fragmented init segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidxTimingTrackInfo {
    /// Track identifier selected for timing calculations.
    pub track_id: u32,
    /// Handler type from the selected track's `hdlr`, when present.
    pub handler_type: Option<FourCc>,
    /// Timescale from the selected track's `mdhd`.
    pub timescale: u32,
}

/// One grouped media-segment input used to build a top-level `sidx` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidxMediaSegment {
    /// First top-level box covered by the grouped media segment.
    pub first_box: BoxInfo,
    /// File offset of the first covered `moof`.
    pub first_moof_offset: u64,
    /// Number of covered `moof` boxes in the grouped media segment.
    pub moof_count: usize,
    /// Number of covered fragments that contributed timing from the selected track.
    pub timing_fragment_count: usize,
    /// Presentation time for the grouped segment in the selected track timescale.
    pub presentation_time: u64,
    /// Base decode time for the grouped segment in the selected track timescale.
    pub base_decode_time: u64,
    /// Total duration contributed by the selected track across the grouped segment.
    pub duration: u64,
    /// Total serialized size of the grouped media segment in bytes.
    pub size: u64,
}

/// Placement data for replacing or inserting a top-level `sidx`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopLevelSidxPlacement {
    /// First top-level media box before which a new `sidx` should be inserted.
    pub insertion_box: BoxInfo,
    /// Existing file-level `sidx` boxes in top-level order.
    pub existing_top_level_sidxs: Vec<ExistingTopLevelSidx>,
}

/// Existing file-level `sidx` metadata reused by the refresh planner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExistingTopLevelSidx {
    /// Header metadata for the serialized top-level `sidx`.
    pub info: BoxInfo,
    /// Absolute anchor point used to derive indexed segment start offsets.
    pub anchor_point: u64,
    /// Absolute start offset for each indexed media segment.
    pub segment_starts: Vec<u64>,
}

/// Planning options for the deterministic top-level `sidx` refresh builder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TopLevelSidxPlanOptions {
    /// Whether the planner should build an insertion plan when no top-level `sidx` exists yet.
    pub add_if_not_exists: bool,
    /// Whether the planned version 1 `sidx` should preserve the first segment's non-zero earliest
    /// presentation time.
    pub non_zero_ept: bool,
}

/// Deterministic top-level `sidx` refresh plan built from analyzed file data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopLevelSidxPlan {
    /// Track selected for timing calculations in the planned `sidx`.
    pub timing_track: SidxTimingTrackInfo,
    /// Concrete version 1 `sidx` payload to write.
    pub sidx: Sidx,
    /// Whether the plan inserts a new top-level `sidx` or replaces one existing box.
    pub action: TopLevelSidxPlanAction,
    /// Top-level box before which the planned `sidx` should be written.
    pub insertion_box: BoxInfo,
    /// Expected serialized size of the planned `sidx` box, including its header.
    pub encoded_box_size: u64,
    /// Planned `sidx` entry coverage in file order.
    pub entries: Vec<TopLevelSidxEntryPlan>,
}

/// Top-level write action selected for a deterministic `sidx` refresh plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TopLevelSidxPlanAction {
    /// Insert a new top-level `sidx` before [`TopLevelSidxPlan::insertion_box`].
    Insert,
    /// Replace one existing top-level `sidx` while keeping it before the covered media run.
    Replace {
        /// Existing top-level `sidx` selected as the replacement target.
        existing: ExistingTopLevelSidx,
    },
}

/// Planned coverage for one `sidx` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopLevelSidxEntryPlan {
    /// One-based entry index inside the planned `sidx`.
    pub entry_index: u32,
    /// Absolute start offset of the grouped media run in the original file.
    pub start_offset: u64,
    /// Absolute end offset of the grouped media run in the original file.
    pub end_offset: u64,
    /// File-level grouped media-segment data that feeds this `sidx` entry.
    pub segment: SidxMediaSegment,
    /// Planned target-size value.
    pub target_size: u32,
    /// Planned `SubSegmentDuration` value.
    pub subsegment_duration: u32,
}

/// Final top-level `sidx` box produced by a rewrite helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedTopLevelSidx {
    /// Header metadata for the rewritten top-level `sidx`.
    pub info: BoxInfo,
    /// Rewritten typed `sidx` payload with final offsets and entry sizes.
    pub sidx: Sidx,
}

/// Errors raised while deriving top-level `sidx` update defaults.
#[derive(Debug)]
pub enum SidxAnalysisError {
    Io(io::Error),
    Header(HeaderError),
    Codec(CodecError),
    Extract(ExtractError),
    /// The file does not expose a fragmented layout that can carry a top-level `sidx`.
    NotFragmented,
    /// The file is missing the root `moov` box needed to resolve track defaults.
    MissingMovieBox,
    /// The fragmented init segment does not expose the `mvex` defaults required for planning.
    MissingMovieExtendsBox,
    /// No track boxes were available inside the fragmented init segment.
    MissingTracks,
    /// No grouped media segments were available for analysis.
    MissingMediaSegments,
    /// A required child box was missing from a parsed container.
    MissingRequiredChild {
        /// Parent box type that should have contained the child.
        parent_box_type: FourCc,
        /// Absolute offset of the parent box.
        parent_offset: u64,
        /// Missing child box type.
        child_box_type: FourCc,
    },
    /// A container carried multiple matches for a child box that should be unique.
    UnexpectedChildCount {
        /// Parent box type that contained the duplicate children.
        parent_box_type: FourCc,
        /// Absolute offset of the parent box.
        parent_offset: u64,
        /// Child box type that appeared more than once.
        child_box_type: FourCc,
        /// Number of matched child boxes.
        count: usize,
    },
    /// No matching `trex` defaults were available for the selected timing track.
    MissingTrackExtends {
        /// Track identifier that needed a matching `trex`.
        track_id: u32,
    },
    /// A grouped media segment did not contain any `moof` boxes.
    SegmentWithoutMovieFragment {
        /// One-based segment index.
        segment_index: usize,
        /// Absolute start offset of the segment.
        segment_offset: u64,
    },
    /// A grouped media segment did not carry any fragments for the selected timing track.
    MissingTimingTrackFragments {
        /// One-based segment index.
        segment_index: usize,
        /// Timing-track identifier.
        track_id: u32,
    },
    /// A matching track fragment was missing its required `tfdt`.
    MissingTrackFragmentDecodeTime {
        /// One-based segment index.
        segment_index: usize,
        /// Absolute `moof` offset that carried the incomplete fragment.
        moof_offset: u64,
        /// Timing-track identifier.
        track_id: u32,
    },
    /// A typed `trun` payload declared a sample count that does not match its decoded entries.
    TrunSampleCountMismatch {
        /// Absolute `moof` offset that carried the invalid `trun`.
        moof_offset: u64,
        /// Timing-track identifier.
        track_id: u32,
        /// Declared `sample_count` from the `trun`.
        declared: u32,
        /// Decoded number of entry records.
        actual: usize,
    },
    /// The existing top-level `sidx` layout uses chained entries that this helper does not
    /// model.
    UnsupportedTopLevelSidxIndirectEntry {
        /// Absolute offset of the unsupported top-level `sidx`.
        sidx_offset: u64,
        /// One-based entry index inside that `sidx`.
        entry_index: usize,
    },
    /// The typed `sidx` payload declared an entry count that does not match the decoded list.
    SidxEntryCountMismatch {
        /// Absolute offset of the invalid top-level `sidx`.
        sidx_offset: u64,
        /// Declared `entry_count` from the `sidx`.
        declared: u16,
        /// Decoded number of entries.
        actual: usize,
    },
    /// Derived arithmetic overflowed the helper's supported range.
    NumericOverflow {
        /// Human-readable name of the derived field that overflowed.
        field_name: &'static str,
    },
}

/// Errors raised while building a deterministic top-level `sidx` refresh plan.
#[derive(Debug)]
pub enum SidxPlanError {
    Analysis(SidxAnalysisError),
    Codec(CodecError),
    /// More than one file-level top-level `sidx` would need coordinated replacement.
    UnsupportedTopLevelSidxCount {
        /// Number of file-level top-level `sidx` boxes discovered during analysis.
        count: usize,
    },
    /// The existing replacement target does not cover the same media start as the derived plan.
    UnsupportedReplacementPlacement {
        /// Absolute offset of the existing replacement target.
        existing_offset: u64,
        /// Absolute offset of the first covered media box.
        media_start_offset: u64,
    },
    /// The planned entry count does not fit in the typed `Sidx` model.
    TooManyEntries {
        /// Number of grouped media runs that would become `sidx` entries.
        count: usize,
    },
    /// One grouped run's serialized size exceeds the supported `sidx` field width.
    SegmentSizeOverflow {
        /// One-based grouped segment index.
        segment_index: usize,
        /// Actual grouped segment size in bytes.
        size: u64,
    },
    /// One grouped run's duration exceeds the supported `sidx` field width.
    SegmentDurationOverflow {
        /// One-based grouped segment index.
        segment_index: usize,
        /// Actual grouped segment duration in track timescale units.
        duration: u64,
    },
    /// The derived grouped-run end offset overflowed the helper's supported range.
    EntryEndOffsetOverflow {
        /// One-based grouped segment index.
        segment_index: usize,
        /// Absolute grouped-run start offset.
        start_offset: u64,
        /// Grouped-run size in bytes.
        size: u64,
    },
    /// The serialized `sidx` box size overflowed the helper's supported range.
    EncodedBoxSizeOverflow,
}

/// Errors raised while applying a deterministic top-level `sidx` rewrite plan.
#[derive(Debug)]
pub enum SidxRewriteError {
    Io(io::Error),
    Header(HeaderError),
    Codec(CodecError),
    /// The supplied plan did not contain any grouped entries to write.
    EmptyPlanEntries,
    /// The typed `sidx` payload and grouped entry coverage did not agree on entry counts.
    PlannedEntryCountMismatch {
        /// Declared `entry_count` stored in the typed `sidx`.
        declared: u16,
        /// Number of typed `sidx` entries in the plan payload.
        sidx_entries: usize,
        /// Number of grouped entry spans in the plan coverage.
        plan_entries: usize,
    },
    /// The supplied plan carried an unsupported `sidx` version.
    UnsupportedSidxVersion {
        /// Unsupported full-box version.
        version: u8,
    },
    /// A planned root box no longer matched the input bytes at the expected offset.
    PlannedBoxMismatch {
        /// Box type recorded in the plan.
        expected_type: FourCc,
        /// Absolute offset recorded in the plan.
        expected_offset: u64,
        /// Total box size recorded in the plan.
        expected_size: u64,
        /// Box type read from the input while validating the plan.
        actual_type: FourCc,
        /// Absolute offset read from the input while validating the plan.
        actual_offset: u64,
        /// Total box size read from the input while validating the plan.
        actual_size: u64,
    },
    /// The rewritten `sidx` box would extend past the first covered media span.
    InvalidRewrittenLayout {
        /// Absolute end offset of the rewritten `sidx`.
        sidx_end_offset: u64,
        /// Absolute start offset of the first covered media span after rewriting.
        first_segment_start_offset: u64,
    },
    /// One grouped entry span became invalid after rewriting.
    InvalidEntrySpan {
        /// One-based entry index inside the rewritten `sidx`.
        entry_index: u32,
        /// Rewritten grouped span start offset.
        start_offset: u64,
        /// Rewritten grouped span end offset.
        end_offset: u64,
    },
    /// One rewritten entry span overflowed the supported `target_size` field width.
    TargetSizeOverflow {
        /// One-based entry index inside the rewritten `sidx`.
        entry_index: u32,
        /// Rewritten grouped span size in bytes.
        size: u64,
    },
    /// The rewrite helper could not copy the requested byte range in full.
    IncompleteCopy {
        /// Number of bytes that should have been copied.
        expected_size: u64,
        /// Number of bytes that were actually copied.
        actual_size: u64,
    },
    /// Numeric overflow occurred while deriving the rewritten box layout.
    NumericOverflow {
        /// Derived field whose value overflowed.
        field_name: &'static str,
    },
    /// The serialized rewritten `sidx` box size overflowed the helper's supported range.
    EncodedBoxSizeOverflow,
}

impl fmt::Display for SidxAnalysisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Header(error) => write!(f, "{error}"),
            Self::Codec(error) => write!(f, "{error}"),
            Self::Extract(error) => write!(f, "{error}"),
            Self::NotFragmented => f.write_str("input file is not fragmented"),
            Self::MissingMovieBox => f.write_str("input file does not have a moov box"),
            Self::MissingMovieExtendsBox => {
                f.write_str("fragmented init segment does not have a moov/mvex layout")
            }
            Self::MissingTracks => {
                f.write_str("fragmented init segment does not contain any tracks")
            }
            Self::MissingMediaSegments => {
                f.write_str("input file does not have any media segments")
            }
            Self::MissingRequiredChild {
                parent_box_type,
                parent_offset,
                child_box_type,
            } => write!(
                f,
                "missing required {child_box_type} child inside {parent_box_type} at offset {parent_offset}"
            ),
            Self::UnexpectedChildCount {
                parent_box_type,
                parent_offset,
                child_box_type,
                count,
            } => write!(
                f,
                "expected one {child_box_type} child inside {parent_box_type} at offset {parent_offset}, found {count}"
            ),
            Self::MissingTrackExtends { track_id } => {
                write!(f, "no trex box found for track {track_id}")
            }
            Self::SegmentWithoutMovieFragment {
                segment_index,
                segment_offset,
            } => write!(
                f,
                "segment {segment_index} at offset {segment_offset} does not contain a moof box"
            ),
            Self::MissingTimingTrackFragments {
                segment_index,
                track_id,
            } => write!(
                f,
                "segment {segment_index} does not contain fragments for timing track {track_id}"
            ),
            Self::MissingTrackFragmentDecodeTime {
                segment_index,
                moof_offset,
                track_id,
            } => write!(
                f,
                "segment {segment_index} moof at offset {moof_offset} is missing tfdt for track {track_id}"
            ),
            Self::TrunSampleCountMismatch {
                moof_offset,
                track_id,
                declared,
                actual,
            } => write!(
                f,
                "moof at offset {moof_offset} has a trun sample count mismatch for track {track_id}: declared {declared}, decoded {actual}"
            ),
            Self::UnsupportedTopLevelSidxIndirectEntry {
                sidx_offset,
                entry_index,
            } => write!(
                f,
                "top-level sidx at offset {sidx_offset} uses unsupported type 1 at entry {entry_index}"
            ),
            Self::SidxEntryCountMismatch {
                sidx_offset,
                declared,
                actual,
            } => write!(
                f,
                "top-level sidx at offset {sidx_offset} declared {declared} entries but decoded {actual}"
            ),
            Self::NumericOverflow { field_name } => {
                write!(f, "numeric overflow while deriving {field_name}")
            }
        }
    }
}

impl fmt::Display for SidxPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Analysis(error) => write!(f, "{error}"),
            Self::Codec(error) => write!(f, "{error}"),
            Self::UnsupportedTopLevelSidxCount { count } => write!(
                f,
                "unsupported top-level sidx topology: expected at most one file-level top-level sidx, found {count}"
            ),
            Self::UnsupportedReplacementPlacement {
                existing_offset,
                media_start_offset,
            } => write!(
                f,
                "unsupported top-level sidx replacement layout: existing sidx at offset {existing_offset} does not align with first media start {media_start_offset}"
            ),
            Self::TooManyEntries { count } => {
                write!(f, "planned sidx entry count {count} does not fit in u16")
            }
            Self::SegmentSizeOverflow {
                segment_index,
                size,
            } => write!(
                f,
                "segment {segment_index} size {size} does not fit in the 31-bit target-size field"
            ),
            Self::SegmentDurationOverflow {
                segment_index,
                duration,
            } => write!(
                f,
                "segment {segment_index} duration {duration} does not fit in the 32-bit subsegment-duration field"
            ),
            Self::EntryEndOffsetOverflow {
                segment_index,
                start_offset,
                size,
            } => write!(
                f,
                "segment {segment_index} end offset overflowed while adding size {size} to start {start_offset}"
            ),
            Self::EncodedBoxSizeOverflow => {
                f.write_str("encoded sidx box size overflowed the supported range")
            }
        }
    }
}

impl fmt::Display for SidxRewriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Header(error) => write!(f, "{error}"),
            Self::Codec(error) => write!(f, "{error}"),
            Self::EmptyPlanEntries => {
                f.write_str("planned top-level sidx rewrite does not contain any entries")
            }
            Self::PlannedEntryCountMismatch {
                declared,
                sidx_entries,
                plan_entries,
            } => write!(
                f,
                "planned top-level sidx entry counts disagree: declared {declared}, typed payload {sidx_entries}, grouped spans {plan_entries}"
            ),
            Self::UnsupportedSidxVersion { version } => {
                write!(
                    f,
                    "planned top-level sidx uses unsupported version {version}"
                )
            }
            Self::PlannedBoxMismatch {
                expected_type,
                expected_offset,
                expected_size,
                actual_type,
                actual_offset,
                actual_size,
            } => write!(
                f,
                "planned box mismatch at offset {expected_offset}: expected {expected_type} size {expected_size}, found {actual_type} at offset {actual_offset} size {actual_size}"
            ),
            Self::InvalidRewrittenLayout {
                sidx_end_offset,
                first_segment_start_offset,
            } => write!(
                f,
                "rewritten top-level sidx would end at {sidx_end_offset} after the first covered segment starts at {first_segment_start_offset}"
            ),
            Self::InvalidEntrySpan {
                entry_index,
                start_offset,
                end_offset,
            } => write!(
                f,
                "rewritten entry {entry_index} ended at {end_offset} before its start {start_offset}"
            ),
            Self::TargetSizeOverflow { entry_index, size } => write!(
                f,
                "rewritten entry {entry_index} size {size} does not fit in the 31-bit target-size field"
            ),
            Self::IncompleteCopy {
                expected_size,
                actual_size,
            } => write!(
                f,
                "failed to copy rewrite bytes: expected {expected_size} bytes, copied {actual_size}"
            ),
            Self::NumericOverflow { field_name } => {
                write!(f, "numeric overflow while deriving {field_name}")
            }
            Self::EncodedBoxSizeOverflow => {
                f.write_str("encoded rewritten sidx box size overflowed the supported range")
            }
        }
    }
}

impl Error for SidxAnalysisError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::Extract(error) => Some(error),
            Self::NotFragmented
            | Self::MissingMovieBox
            | Self::MissingMovieExtendsBox
            | Self::MissingTracks
            | Self::MissingMediaSegments
            | Self::MissingRequiredChild { .. }
            | Self::UnexpectedChildCount { .. }
            | Self::MissingTrackExtends { .. }
            | Self::SegmentWithoutMovieFragment { .. }
            | Self::MissingTimingTrackFragments { .. }
            | Self::MissingTrackFragmentDecodeTime { .. }
            | Self::TrunSampleCountMismatch { .. }
            | Self::UnsupportedTopLevelSidxIndirectEntry { .. }
            | Self::SidxEntryCountMismatch { .. }
            | Self::NumericOverflow { .. } => None,
        }
    }
}

impl Error for SidxPlanError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Analysis(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::UnsupportedTopLevelSidxCount { .. }
            | Self::UnsupportedReplacementPlacement { .. }
            | Self::TooManyEntries { .. }
            | Self::SegmentSizeOverflow { .. }
            | Self::SegmentDurationOverflow { .. }
            | Self::EntryEndOffsetOverflow { .. }
            | Self::EncodedBoxSizeOverflow => None,
        }
    }
}

impl Error for SidxRewriteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::EmptyPlanEntries
            | Self::PlannedEntryCountMismatch { .. }
            | Self::UnsupportedSidxVersion { .. }
            | Self::PlannedBoxMismatch { .. }
            | Self::InvalidRewrittenLayout { .. }
            | Self::InvalidEntrySpan { .. }
            | Self::TargetSizeOverflow { .. }
            | Self::IncompleteCopy { .. }
            | Self::NumericOverflow { .. }
            | Self::EncodedBoxSizeOverflow => None,
        }
    }
}

impl From<io::Error> for SidxAnalysisError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<HeaderError> for SidxAnalysisError {
    fn from(value: HeaderError) -> Self {
        Self::Header(value)
    }
}

impl From<CodecError> for SidxAnalysisError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}

impl From<ExtractError> for SidxAnalysisError {
    fn from(value: ExtractError) -> Self {
        Self::Extract(value)
    }
}

impl From<SidxAnalysisError> for SidxPlanError {
    fn from(value: SidxAnalysisError) -> Self {
        Self::Analysis(value)
    }
}

impl From<CodecError> for SidxPlanError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}

impl From<io::Error> for SidxRewriteError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<HeaderError> for SidxRewriteError {
    fn from(value: HeaderError) -> Self {
        Self::Header(value)
    }
}

impl From<CodecError> for SidxRewriteError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}

/// Analyzes a fragmented file and returns the default inputs for a top-level `sidx` refresh.
///
/// The derived behavior follows the current fragmented-file defaults:
/// - select a timing track by preferring `vide`, then `soun`, then the first track
/// - group media runs using top-level `styp` boxes or existing top-level `sidx` entries
/// - otherwise treat the first `moof`-started run as one grouped media segment
pub fn analyze_top_level_sidx_update<R>(
    reader: &mut R,
) -> Result<TopLevelSidxUpdateAnalysis, SidxAnalysisError>
where
    R: Read + Seek,
{
    let root_boxes = scan_root_boxes(reader)?;
    let has_fragment_markers = root_boxes
        .iter()
        .any(|info| matches!(info.box_type(), MOOF | STYP | SIDX));

    let moov = root_boxes
        .iter()
        .find(|info| info.box_type() == MOOV)
        .copied()
        .ok_or(SidxAnalysisError::MissingMovieBox)?;

    let has_mvex = !extract_boxes(reader, Some(&moov), &[BoxPath::from([MVEX])])?.is_empty();
    if !has_fragment_markers && !has_mvex {
        return Err(SidxAnalysisError::NotFragmented);
    }
    if !has_mvex {
        return Err(SidxAnalysisError::MissingMovieExtendsBox);
    }

    let init = analyze_init_segment(reader, &moov)?;
    let (segments, existing_top_level_sidxs) = group_media_segments(reader, &root_boxes)?;
    if segments.is_empty() {
        return Err(SidxAnalysisError::MissingMediaSegments);
    }

    let mut analyzed_segments = Vec::with_capacity(segments.len());
    for (segment_index, segment) in segments.iter().enumerate() {
        analyzed_segments.push(analyze_segment(
            reader,
            segment_index + 1,
            segment,
            init.timing_track.track_id,
            &init.trex,
        )?);
    }

    Ok(TopLevelSidxUpdateAnalysis {
        timing_track: init.timing_track,
        placement: TopLevelSidxPlacement {
            insertion_box: segments[0].first_box,
            existing_top_level_sidxs: existing_top_level_sidxs
                .into_iter()
                .map(|entry| entry.public)
                .collect(),
        },
        segments: analyzed_segments,
    })
}

/// Analyzes a fragmented byte slice and returns the default inputs for a top-level `sidx`
/// refresh.
pub fn analyze_top_level_sidx_update_bytes(
    input: &[u8],
) -> Result<TopLevelSidxUpdateAnalysis, SidxAnalysisError> {
    let mut reader = Cursor::new(input);
    analyze_top_level_sidx_update(&mut reader)
}

/// Analyzes a fragmented file through the additive Tokio-based async library surface and returns
/// the default inputs for a top-level `sidx` refresh.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn analyze_top_level_sidx_update_async<R>(
    reader: &mut R,
) -> Result<TopLevelSidxUpdateAnalysis, SidxAnalysisError>
where
    R: AsyncReadSeek,
{
    let input = read_all_bytes_async(reader).await?;
    analyze_top_level_sidx_update_bytes(&input)
}

/// Builds a deterministic top-level `sidx` refresh plan from analyzed file data.
///
/// Returns `Ok(None)` when the file does not currently contain a top-level `sidx` and
/// `add_if_not_exists` is `false`, leaving the input unchanged.
pub fn build_top_level_sidx_plan(
    analysis: &TopLevelSidxUpdateAnalysis,
    options: TopLevelSidxPlanOptions,
) -> Result<Option<TopLevelSidxPlan>, SidxPlanError> {
    if analysis.segments.is_empty() {
        return Err(SidxAnalysisError::MissingMediaSegments.into());
    }

    let action = match analysis.placement.existing_top_level_sidxs.as_slice() {
        [] if options.add_if_not_exists => TopLevelSidxPlanAction::Insert,
        [] => return Ok(None),
        [existing] => {
            if existing.segment_starts.first().copied()
                != Some(analysis.placement.insertion_box.offset())
            {
                return Err(SidxPlanError::UnsupportedReplacementPlacement {
                    existing_offset: existing.info.offset(),
                    media_start_offset: analysis.placement.insertion_box.offset(),
                });
            }
            TopLevelSidxPlanAction::Replace {
                existing: existing.clone(),
            }
        }
        existing => {
            return Err(SidxPlanError::UnsupportedTopLevelSidxCount {
                count: existing.len(),
            });
        }
    };

    let entry_count =
        u16::try_from(analysis.segments.len()).map_err(|_| SidxPlanError::TooManyEntries {
            count: analysis.segments.len(),
        })?;

    let mut sidx = Sidx::default();
    sidx.set_version(1);
    sidx.reference_id = 1;
    sidx.timescale = analysis.timing_track.timescale;
    sidx.earliest_presentation_time_v1 = if options.non_zero_ept {
        analysis.segments[0].presentation_time
    } else {
        0
    };
    sidx.first_offset_v1 = 0;
    sidx.reference_count = entry_count;

    let mut entries = Vec::with_capacity(analysis.segments.len());
    let mut sidx_entries = Vec::with_capacity(analysis.segments.len());
    for (index, segment) in analysis.segments.iter().enumerate() {
        let target_size =
            u32::try_from(segment.size).map_err(|_| SidxPlanError::SegmentSizeOverflow {
                segment_index: index + 1,
                size: segment.size,
            })?;
        if target_size > 0x7fff_ffff {
            return Err(SidxPlanError::SegmentSizeOverflow {
                segment_index: index + 1,
                size: segment.size,
            });
        }
        let subsegment_duration = u32::try_from(segment.duration).map_err(|_| {
            SidxPlanError::SegmentDurationOverflow {
                segment_index: index + 1,
                duration: segment.duration,
            }
        })?;
        let end_offset = segment.first_box.offset().checked_add(segment.size).ok_or(
            SidxPlanError::EntryEndOffsetOverflow {
                segment_index: index + 1,
                start_offset: segment.first_box.offset(),
                size: segment.size,
            },
        )?;

        sidx_entries.push(SidxReference {
            reference_type: false,
            referenced_size: target_size,
            subsegment_duration,
            starts_with_sap: true,
            sap_type: 1,
            sap_delta_time: 0,
        });
        entries.push(TopLevelSidxEntryPlan {
            entry_index: (index + 1) as u32,
            start_offset: segment.first_box.offset(),
            end_offset,
            segment: segment.clone(),
            target_size,
            subsegment_duration,
        });
    }
    sidx.references = sidx_entries;

    let payload_size = encoded_payload_size(&sidx)?;
    let encoded_box_size = payload_size
        .checked_add(box_header_size_for_payload(payload_size))
        .ok_or(SidxPlanError::EncodedBoxSizeOverflow)?;

    Ok(Some(TopLevelSidxPlan {
        timing_track: analysis.timing_track.clone(),
        sidx,
        action,
        insertion_box: analysis.placement.insertion_box,
        encoded_box_size,
        entries,
    }))
}

/// Analyzes a fragmented file and builds the deterministic top-level `sidx` refresh plan.
///
/// Returns `Ok(None)` when `add_if_not_exists` is `false` and no file-level top-level `sidx`
/// exists yet.
pub fn plan_top_level_sidx_update<R>(
    reader: &mut R,
    options: TopLevelSidxPlanOptions,
) -> Result<Option<TopLevelSidxPlan>, SidxPlanError>
where
    R: Read + Seek,
{
    let analysis = analyze_top_level_sidx_update(reader)?;
    build_top_level_sidx_plan(&analysis, options)
}

/// Analyzes a fragmented byte slice and builds the deterministic top-level `sidx` refresh plan.
pub fn plan_top_level_sidx_update_bytes(
    input: &[u8],
    options: TopLevelSidxPlanOptions,
) -> Result<Option<TopLevelSidxPlan>, SidxPlanError> {
    let mut reader = Cursor::new(input);
    plan_top_level_sidx_update(&mut reader, options)
}

/// Analyzes a fragmented file through the additive Tokio-based async library surface and builds
/// the deterministic top-level `sidx` refresh plan.
///
/// Returns `Ok(None)` when `add_if_not_exists` is `false` and no file-level top-level `sidx`
/// exists yet.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn plan_top_level_sidx_update_async<R>(
    reader: &mut R,
    options: TopLevelSidxPlanOptions,
) -> Result<Option<TopLevelSidxPlan>, SidxPlanError>
where
    R: AsyncReadSeek,
{
    let analysis = analyze_top_level_sidx_update_async(reader).await?;
    build_top_level_sidx_plan(&analysis, options)
}

/// Applies a deterministic top-level `sidx` plan to a fragmented file and writes the updated bytes
/// to `writer`.
///
/// The helper only rewrites the planned top-level `sidx` span. All other bytes are copied through
/// verbatim.
pub fn apply_top_level_sidx_plan<R, W>(
    reader: &mut R,
    mut writer: W,
    plan: &TopLevelSidxPlan,
) -> Result<AppliedTopLevelSidx, SidxRewriteError>
where
    R: Read + Seek,
    W: Write,
{
    validate_rewrite_plan(plan)?;

    validate_root_box(reader, &plan.insertion_box)?;
    let (write_offset, removed_size) = match &plan.action {
        TopLevelSidxPlanAction::Insert => (plan.insertion_box.offset(), 0),
        TopLevelSidxPlanAction::Replace { existing } => {
            validate_root_box(reader, &existing.info)?;
            (existing.info.offset(), existing.info.size())
        }
    };

    let rewritten = build_rewritten_sidx(plan, write_offset, removed_size)?;
    let input_end = reader.seek(SeekFrom::End(0))?;
    let removed_end = checked_add_rewrite(write_offset, removed_size, "planned removed span end")?;
    let trailing_size =
        input_end
            .checked_sub(removed_end)
            .ok_or(SidxRewriteError::NumericOverflow {
                field_name: "trailing rewrite bytes",
            })?;

    copy_range_exact(reader, &mut writer, 0, write_offset)?;
    writer.write_all(&rewritten.bytes)?;
    copy_range_exact(reader, &mut writer, removed_end, trailing_size)?;

    Ok(rewritten.applied)
}

/// Applies a deterministic top-level `sidx` plan to an in-memory MP4 byte slice and returns the
/// rewritten bytes.
pub fn apply_top_level_sidx_plan_bytes(
    input: &[u8],
    plan: &TopLevelSidxPlan,
) -> Result<Vec<u8>, SidxRewriteError> {
    let mut reader = Cursor::new(input);
    let mut writer = Vec::with_capacity(input.len().saturating_add(plan.encoded_box_size as usize));
    apply_top_level_sidx_plan(&mut reader, &mut writer, plan)?;
    Ok(writer)
}

/// Applies a deterministic top-level `sidx` plan through the additive Tokio-based async library
/// surface and writes the updated bytes to `writer`.
///
/// The helper only rewrites the planned top-level `sidx` span. All other bytes are copied through
/// verbatim.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn apply_top_level_sidx_plan_async<R, W>(
    reader: &mut R,
    writer: &mut W,
    plan: &TopLevelSidxPlan,
) -> Result<AppliedTopLevelSidx, SidxRewriteError>
where
    R: AsyncReadSeek,
    W: AsyncWriteSeek,
{
    validate_rewrite_plan(plan)?;

    validate_root_box_async(reader, &plan.insertion_box).await?;
    let (write_offset, removed_size) = match &plan.action {
        TopLevelSidxPlanAction::Insert => (plan.insertion_box.offset(), 0),
        TopLevelSidxPlanAction::Replace { existing } => {
            validate_root_box_async(reader, &existing.info).await?;
            (existing.info.offset(), existing.info.size())
        }
    };

    let rewritten = build_rewritten_sidx(plan, write_offset, removed_size)?;
    let input_end = reader.seek(SeekFrom::End(0)).await?;
    let removed_end = checked_add_rewrite(write_offset, removed_size, "planned removed span end")?;
    let trailing_size =
        input_end
            .checked_sub(removed_end)
            .ok_or(SidxRewriteError::NumericOverflow {
                field_name: "trailing rewrite bytes",
            })?;

    copy_range_exact_async(reader, writer, 0, write_offset).await?;
    writer.write_all(&rewritten.bytes).await?;
    copy_range_exact_async(reader, writer, removed_end, trailing_size).await?;

    Ok(rewritten.applied)
}

fn scan_root_boxes<R>(reader: &mut R) -> Result<Vec<BoxInfo>, SidxAnalysisError>
where
    R: Read + Seek,
{
    let end = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;

    let mut boxes = Vec::new();
    while reader.stream_position()? < end {
        let info = BoxInfo::read(reader)?;
        boxes.push(info);
        info.seek_to_end(reader)?;
    }

    Ok(boxes)
}

fn analyze_init_segment<R>(
    reader: &mut R,
    moov: &BoxInfo,
) -> Result<InitAnalysis, SidxAnalysisError>
where
    R: Read + Seek,
{
    let mvex = require_single_child_info(reader, moov, MVEX)?;
    let traks = extract_boxes(reader, Some(moov), &[BoxPath::from([TRAK])])?;
    if traks.is_empty() {
        return Err(SidxAnalysisError::MissingTracks);
    }

    let mut tracks = Vec::with_capacity(traks.len());
    for trak in traks {
        let tkhd = require_single_child_as::<_, Tkhd>(reader, &trak, TKHD)?;
        let mdhd = require_single_nested_child_as::<_, Mdhd>(reader, &trak, MDIA, MDHD)?;
        let handler_type = extract_optional_handler_type(reader, &trak)?;
        tracks.push(InitTrackInfo {
            track_id: tkhd.track_id,
            handler_type,
            timescale: mdhd.timescale,
        });
    }

    let timing_track = select_timing_track(&tracks)?;
    let trex = extract_box_as::<_, Trex>(reader, Some(&mvex), BoxPath::from([TREX]))?
        .into_iter()
        .find(|entry| entry.track_id == timing_track.track_id)
        .ok_or(SidxAnalysisError::MissingTrackExtends {
            track_id: timing_track.track_id,
        })?;

    Ok(InitAnalysis { timing_track, trex })
}

fn select_timing_track(tracks: &[InitTrackInfo]) -> Result<SidxTimingTrackInfo, SidxAnalysisError> {
    let track = tracks
        .iter()
        .find(|track| track.handler_type == Some(VIDE))
        .or_else(|| tracks.iter().find(|track| track.handler_type == Some(SOUN)))
        .or_else(|| tracks.first())
        .ok_or(SidxAnalysisError::MissingTracks)?;

    Ok(SidxTimingTrackInfo {
        track_id: track.track_id,
        handler_type: track.handler_type,
        timescale: track.timescale,
    })
}

fn extract_optional_handler_type<R>(
    reader: &mut R,
    trak: &BoxInfo,
) -> Result<Option<FourCc>, SidxAnalysisError>
where
    R: Read + Seek,
{
    let handlers = extract_box_as::<_, crate::boxes::iso14496_12::Hdlr>(
        reader,
        Some(trak),
        BoxPath::from([MDIA, HDLR]),
    )?;
    match handlers.len() {
        0 => Ok(None),
        1 => Ok(Some(handlers[0].handler_type)),
        count => Err(SidxAnalysisError::UnexpectedChildCount {
            parent_box_type: trak.box_type(),
            parent_offset: trak.offset(),
            child_box_type: HDLR,
            count,
        }),
    }
}

fn require_single_child_info<R>(
    reader: &mut R,
    parent: &BoxInfo,
    child_box_type: FourCc,
) -> Result<BoxInfo, SidxAnalysisError>
where
    R: Read + Seek,
{
    let infos = extract_boxes(reader, Some(parent), &[BoxPath::from([child_box_type])])?;
    match infos.len() {
        0 => Err(SidxAnalysisError::MissingRequiredChild {
            parent_box_type: parent.box_type(),
            parent_offset: parent.offset(),
            child_box_type,
        }),
        1 => Ok(infos[0]),
        count => Err(SidxAnalysisError::UnexpectedChildCount {
            parent_box_type: parent.box_type(),
            parent_offset: parent.offset(),
            child_box_type,
            count,
        }),
    }
}

fn require_single_child_as<R, B>(
    reader: &mut R,
    parent: &BoxInfo,
    child_box_type: FourCc,
) -> Result<B, SidxAnalysisError>
where
    R: Read + Seek,
    B: CodecBox + Clone + 'static,
{
    let boxes = extract_box_as::<_, B>(reader, Some(parent), BoxPath::from([child_box_type]))?;
    match boxes.len() {
        0 => Err(SidxAnalysisError::MissingRequiredChild {
            parent_box_type: parent.box_type(),
            parent_offset: parent.offset(),
            child_box_type,
        }),
        1 => Ok(boxes.into_iter().next().unwrap()),
        count => Err(SidxAnalysisError::UnexpectedChildCount {
            parent_box_type: parent.box_type(),
            parent_offset: parent.offset(),
            child_box_type,
            count,
        }),
    }
}

fn require_single_nested_child_as<R, B>(
    reader: &mut R,
    parent: &BoxInfo,
    intermediate_box_type: FourCc,
    child_box_type: FourCc,
) -> Result<B, SidxAnalysisError>
where
    R: Read + Seek,
    B: CodecBox + Clone + 'static,
{
    let boxes = extract_box_as::<_, B>(
        reader,
        Some(parent),
        BoxPath::from([intermediate_box_type, child_box_type]),
    )?;
    match boxes.len() {
        0 => Err(SidxAnalysisError::MissingRequiredChild {
            parent_box_type: parent.box_type(),
            parent_offset: parent.offset(),
            child_box_type,
        }),
        1 => Ok(boxes.into_iter().next().unwrap()),
        count => Err(SidxAnalysisError::UnexpectedChildCount {
            parent_box_type: parent.box_type(),
            parent_offset: parent.offset(),
            child_box_type,
            count,
        }),
    }
}

fn group_media_segments<R>(
    reader: &mut R,
    root_boxes: &[BoxInfo],
) -> Result<(Vec<SegmentAccumulator>, Vec<ExistingTopLevelSidxInternal>), SidxAnalysisError>
where
    R: Read + Seek,
{
    let mut segments = Vec::new();
    let mut existing_top_level_sidxs = Vec::new();
    let mut previous_box_type = None;

    for info in root_boxes {
        match info.box_type() {
            STYP => {
                start_segment(&mut segments, *info);
                add_segment_size(
                    segments.last_mut().unwrap(),
                    info.size(),
                    "media segment size",
                )?;
            }
            SIDX => {
                if segments.is_empty() && previous_box_type != Some(MDAT) {
                    let decoded = read_payload_as::<_, Sidx>(reader, info)?;
                    let internal = analyze_existing_top_level_sidx(info, &decoded)?;
                    existing_top_level_sidxs.push(internal);
                } else if previous_box_type == Some(STYP)
                    && segments.last().is_some_and(is_pending_styp_prelude_segment)
                {
                    let decoded = read_payload_as::<_, Sidx>(reader, info)?;
                    let internal = analyze_existing_top_level_sidx(info, &decoded)?;
                    existing_top_level_sidxs.push(internal);
                    segments.pop();
                } else if previous_box_type == Some(MDAT) {
                    start_segment(&mut segments, *info);
                    let segment = segments.last_mut().unwrap();
                    segment.segment_sidx_count = 1;
                    add_segment_size(segment, info.size(), "media segment size")?;
                } else if let Some(segment) = segments.last_mut() {
                    if segment.segment_sidx_count == 0 {
                        add_segment_size(segment, info.size(), "media segment size")?;
                    }
                    segment.segment_sidx_count += 1;
                }
            }
            EMSG | MOOF => {
                if should_start_segment(info.offset(), segments.len(), &existing_top_level_sidxs) {
                    start_segment(&mut segments, *info);
                }

                if let Some(segment) = segments.last_mut() {
                    add_segment_size(segment, info.size(), "media segment size")?;
                    if info.box_type() == MOOF {
                        segment.moofs.push(*info);
                    }
                }
            }
            MDAT => {
                if let Some(segment) = segments.last_mut() {
                    add_segment_size(segment, info.size(), "media segment size")?;
                }
            }
            _ => {}
        }

        previous_box_type = Some(info.box_type());
    }

    Ok((segments, existing_top_level_sidxs))
}

fn is_pending_styp_prelude_segment(segment: &SegmentAccumulator) -> bool {
    segment.first_box.box_type() == STYP
        && segment.moofs.is_empty()
        && segment.segment_sidx_count == 0
        && segment.size == segment.first_box.size()
}

fn start_segment(segments: &mut Vec<SegmentAccumulator>, first_box: BoxInfo) {
    segments.push(SegmentAccumulator {
        first_box,
        moofs: Vec::new(),
        size: 0,
        segment_sidx_count: 0,
    });
}

fn should_start_segment(
    box_offset: u64,
    segment_count: usize,
    existing_top_level_sidxs: &[ExistingTopLevelSidxInternal],
) -> bool {
    if existing_top_level_sidxs.is_empty() {
        return segment_count == 0;
    }

    let mut next_segment_index = 0usize;
    for sidx in existing_top_level_sidxs {
        for segment_start in &sidx.public.segment_starts {
            if next_segment_index == segment_count {
                return box_offset == *segment_start;
            }
            next_segment_index += 1;
        }
    }

    false
}

fn analyze_existing_top_level_sidx(
    info: &BoxInfo,
    sidx: &Sidx,
) -> Result<ExistingTopLevelSidxInternal, SidxAnalysisError> {
    if usize::from(sidx.reference_count) != sidx.references.len() {
        return Err(SidxAnalysisError::SidxEntryCountMismatch {
            sidx_offset: info.offset(),
            declared: sidx.reference_count,
            actual: sidx.references.len(),
        });
    }

    let anchor_point = checked_add(
        checked_add(info.offset(), info.size(), "top-level sidx end offset")?,
        sidx.first_offset(),
        "top-level sidx anchor point",
    )?;
    let mut current_start = anchor_point;
    let mut segment_starts = Vec::with_capacity(sidx.references.len());

    for (index, entry) in sidx.references.iter().enumerate() {
        if entry.reference_type {
            return Err(SidxAnalysisError::UnsupportedTopLevelSidxIndirectEntry {
                sidx_offset: info.offset(),
                entry_index: index + 1,
            });
        }
        segment_starts.push(current_start);
        current_start = checked_add(
            current_start,
            u64::from(entry.referenced_size),
            "top-level sidx segment start",
        )?;
    }

    Ok(ExistingTopLevelSidxInternal {
        public: ExistingTopLevelSidx {
            info: *info,
            anchor_point,
            segment_starts,
        },
    })
}

fn analyze_segment<R>(
    reader: &mut R,
    segment_index: usize,
    segment: &SegmentAccumulator,
    timing_track_id: u32,
    trex: &Trex,
) -> Result<SidxMediaSegment, SidxAnalysisError>
where
    R: Read + Seek,
{
    let first_moof =
        segment
            .moofs
            .first()
            .copied()
            .ok_or(SidxAnalysisError::SegmentWithoutMovieFragment {
                segment_index,
                segment_offset: segment.first_box.offset(),
            })?;

    let mut base_decode_time = 0_u64;
    let mut first_composition_time_offset = 0_i64;
    let mut duration = 0_u64;
    let mut timing_fragment_count = 0_usize;
    let mut matched_any_fragment = false;

    for (fragment_index, moof) in segment.moofs.iter().enumerate() {
        let trafs = extract_boxes(reader, Some(moof), &[BoxPath::from([TRAF])])?;
        let mut matched_timing_fragment = false;

        for traf in trafs {
            let tfhd = require_single_child_as::<_, Tfhd>(reader, &traf, TFHD)?;
            if tfhd.track_id != timing_track_id {
                continue;
            }

            let tfdt = require_single_child_as::<_, Tfdt>(reader, &traf, TFDT).map_err(
                |error| match error {
                    SidxAnalysisError::MissingRequiredChild { .. } => {
                        SidxAnalysisError::MissingTrackFragmentDecodeTime {
                            segment_index,
                            moof_offset: moof.offset(),
                            track_id: timing_track_id,
                        }
                    }
                    other => other,
                },
            )?;
            let truns = extract_box_as::<_, Trun>(reader, Some(&traf), BoxPath::from([TRUN]))?;

            if !matched_timing_fragment {
                timing_fragment_count += 1;
                matched_timing_fragment = true;
            }
            matched_any_fragment = true;

            if fragment_index == 0 {
                base_decode_time = tfdt.base_media_decode_time();
            }

            for (trun_index, trun) in truns.iter().enumerate() {
                validate_trun_sample_count(trun, moof, timing_track_id)?;

                if fragment_index == 0 && trun_index == 0 && trun.sample_count > 0 {
                    first_composition_time_offset = effective_first_composition_time_offset(trun)?;
                }

                duration = checked_add(
                    duration,
                    effective_trun_duration(trun, &tfhd, trex),
                    "segment duration",
                )?;
            }
        }
    }

    if !matched_any_fragment {
        return Err(SidxAnalysisError::MissingTimingTrackFragments {
            segment_index,
            track_id: timing_track_id,
        });
    }

    let presentation_time = base_decode_time
        .checked_add_signed(first_composition_time_offset)
        .ok_or(SidxAnalysisError::NumericOverflow {
            field_name: "segment presentation time",
        })?;

    Ok(SidxMediaSegment {
        first_box: segment.first_box,
        first_moof_offset: first_moof.offset(),
        moof_count: segment.moofs.len(),
        timing_fragment_count,
        presentation_time,
        base_decode_time,
        duration,
        size: segment.size,
    })
}

fn validate_trun_sample_count(
    trun: &Trun,
    moof: &BoxInfo,
    timing_track_id: u32,
) -> Result<(), SidxAnalysisError> {
    let per_sample_fields_present = trun.flags()
        & (TRUN_SAMPLE_DURATION_PRESENT
            | crate::boxes::iso14496_12::TRUN_SAMPLE_SIZE_PRESENT
            | crate::boxes::iso14496_12::TRUN_SAMPLE_FLAGS_PRESENT
            | TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT)
        != 0;
    let actual = trun.entries.len();

    if per_sample_fields_present && actual != trun.sample_count as usize {
        return Err(SidxAnalysisError::TrunSampleCountMismatch {
            moof_offset: moof.offset(),
            track_id: timing_track_id,
            declared: trun.sample_count,
            actual,
        });
    }
    if !per_sample_fields_present && actual != 0 {
        return Err(SidxAnalysisError::TrunSampleCountMismatch {
            moof_offset: moof.offset(),
            track_id: timing_track_id,
            declared: trun.sample_count,
            actual,
        });
    }

    Ok(())
}

fn effective_first_composition_time_offset(trun: &Trun) -> Result<i64, SidxAnalysisError> {
    if trun.sample_count == 0 {
        return Ok(0);
    }
    if trun.flags() & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT == 0 {
        return Ok(0);
    }

    if trun.entries.is_empty() {
        return Err(SidxAnalysisError::NumericOverflow {
            field_name: "first composition time offset",
        });
    }

    Ok(trun.sample_composition_time_offset(0))
}

fn effective_trun_duration(trun: &Trun, tfhd: &Tfhd, trex: &Trex) -> u64 {
    if trun.flags() & TRUN_SAMPLE_DURATION_PRESENT != 0 {
        return trun
            .entries
            .iter()
            .map(|entry| u64::from(entry.sample_duration))
            .sum();
    }

    let default_sample_duration = if tfhd.flags() & TFHD_DEFAULT_SAMPLE_DURATION_PRESENT != 0 {
        tfhd.default_sample_duration
    } else {
        trex.default_sample_duration
    };
    u64::from(trun.sample_count) * u64::from(default_sample_duration)
}

fn read_payload_as<R, B>(reader: &mut R, info: &BoxInfo) -> Result<B, SidxAnalysisError>
where
    R: Read + Seek,
    B: CodecBox + Default,
{
    info.seek_to_payload(reader)?;
    let mut decoded = B::default();
    unmarshal(reader, info.payload_size()?, &mut decoded, None)?;
    Ok(decoded)
}

fn add_segment_size(
    segment: &mut SegmentAccumulator,
    size: u64,
    field_name: &'static str,
) -> Result<(), SidxAnalysisError> {
    segment.size = checked_add(segment.size, size, field_name)?;
    Ok(())
}

fn checked_add(lhs: u64, rhs: u64, field_name: &'static str) -> Result<u64, SidxAnalysisError> {
    lhs.checked_add(rhs)
        .ok_or(SidxAnalysisError::NumericOverflow { field_name })
}

#[cfg(feature = "async")]
async fn read_all_bytes_async<R>(reader: &mut R) -> Result<Vec<u8>, SidxAnalysisError>
where
    R: AsyncReadSeek,
{
    reader.seek(SeekFrom::Start(0)).await?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await?;
    reader.seek(SeekFrom::Start(0)).await?;
    Ok(bytes)
}

fn encoded_payload_size(sidx: &Sidx) -> Result<u64, CodecError> {
    let mut payload = Vec::new();
    marshal(&mut payload, sidx, None)?;
    Ok(payload.len() as u64)
}

fn box_header_size_for_payload(payload_size: u64) -> u64 {
    if payload_size.saturating_add(SMALL_HEADER_SIZE) > u32::MAX as u64 {
        LARGE_HEADER_SIZE
    } else {
        SMALL_HEADER_SIZE
    }
}

struct EncodedRewrittenSidx {
    applied: AppliedTopLevelSidx,
    bytes: Vec<u8>,
}

fn validate_rewrite_plan(plan: &TopLevelSidxPlan) -> Result<(), SidxRewriteError> {
    if plan.entries.is_empty() {
        return Err(SidxRewriteError::EmptyPlanEntries);
    }

    let sidx_entries = plan.sidx.references.len();
    if usize::from(plan.sidx.reference_count) != sidx_entries || sidx_entries != plan.entries.len()
    {
        return Err(SidxRewriteError::PlannedEntryCountMismatch {
            declared: plan.sidx.reference_count,
            sidx_entries,
            plan_entries: plan.entries.len(),
        });
    }

    match plan.sidx.version() {
        0 | 1 => Ok(()),
        version => Err(SidxRewriteError::UnsupportedSidxVersion { version }),
    }
}

fn validate_root_box<R>(reader: &mut R, expected: &BoxInfo) -> Result<(), SidxRewriteError>
where
    R: Read + Seek,
{
    reader.seek(SeekFrom::Start(expected.offset()))?;
    let actual = BoxInfo::read(reader)?;
    if actual.box_type() != expected.box_type() || actual.size() != expected.size() {
        return Err(SidxRewriteError::PlannedBoxMismatch {
            expected_type: expected.box_type(),
            expected_offset: expected.offset(),
            expected_size: expected.size(),
            actual_type: actual.box_type(),
            actual_offset: actual.offset(),
            actual_size: actual.size(),
        });
    }

    Ok(())
}

#[cfg(feature = "async")]
async fn validate_root_box_async<R>(
    reader: &mut R,
    expected: &BoxInfo,
) -> Result<(), SidxRewriteError>
where
    R: AsyncReadSeek,
{
    reader.seek(SeekFrom::Start(expected.offset())).await?;
    let actual = BoxInfo::read_async(reader).await?;
    if actual.box_type() != expected.box_type() || actual.size() != expected.size() {
        return Err(SidxRewriteError::PlannedBoxMismatch {
            expected_type: expected.box_type(),
            expected_offset: expected.offset(),
            expected_size: expected.size(),
            actual_type: actual.box_type(),
            actual_offset: actual.offset(),
            actual_size: actual.size(),
        });
    }

    Ok(())
}

fn build_rewritten_sidx(
    plan: &TopLevelSidxPlan,
    write_offset: u64,
    removed_size: u64,
) -> Result<EncodedRewrittenSidx, SidxRewriteError> {
    let removed_end = checked_add_rewrite(write_offset, removed_size, "planned removed span end")?;
    let (_, initial_info) = encode_sidx_box(&plan.sidx)?;
    let mut encoded_box_size = initial_info.size();
    let mut last = None;

    // Serialize until the header width stabilizes, then derive offsets from that final size.
    for _ in 0..3 {
        let mut sidx = plan.sidx.clone();
        let first_segment_start = shift_offset_after_rewrite(
            plan.entries[0].start_offset,
            removed_end,
            encoded_box_size,
            removed_size,
            "rewritten first segment start offset",
        )?;
        let sidx_end =
            checked_add_rewrite(write_offset, encoded_box_size, "rewritten sidx end offset")?;
        let first_offset = first_segment_start.checked_sub(sidx_end).ok_or(
            SidxRewriteError::InvalidRewrittenLayout {
                sidx_end_offset: sidx_end,
                first_segment_start_offset: first_segment_start,
            },
        )?;
        set_sidx_first_offset(&mut sidx, first_offset)?;

        for (index, entry) in plan.entries.iter().enumerate() {
            let start_offset = shift_offset_after_rewrite(
                entry.start_offset,
                removed_end,
                encoded_box_size,
                removed_size,
                "rewritten entry start offset",
            )?;
            let end_offset = shift_offset_after_rewrite(
                entry.end_offset,
                removed_end,
                encoded_box_size,
                removed_size,
                "rewritten entry end offset",
            )?;
            let size =
                end_offset
                    .checked_sub(start_offset)
                    .ok_or(SidxRewriteError::InvalidEntrySpan {
                        entry_index: entry.entry_index,
                        start_offset,
                        end_offset,
                    })?;
            if size > 0x7fff_ffff {
                return Err(SidxRewriteError::TargetSizeOverflow {
                    entry_index: entry.entry_index,
                    size,
                });
            }
            sidx.references[index].referenced_size =
                u32::try_from(size).map_err(|_| SidxRewriteError::TargetSizeOverflow {
                    entry_index: entry.entry_index,
                    size,
                })?;
        }

        let (bytes, info) = encode_sidx_box(&sidx)?;
        let applied = AppliedTopLevelSidx {
            info: info.with_offset(write_offset),
            sidx,
        };
        let actual_size = applied.info.size();
        last = Some(EncodedRewrittenSidx { applied, bytes });
        if actual_size == encoded_box_size {
            return Ok(last.unwrap());
        }
        encoded_box_size = actual_size;
    }

    Ok(last.unwrap())
}

fn shift_offset_after_rewrite(
    offset: u64,
    removed_end: u64,
    inserted_size: u64,
    removed_size: u64,
    field_name: &'static str,
) -> Result<u64, SidxRewriteError> {
    if offset < removed_end {
        return Ok(offset);
    }

    if inserted_size >= removed_size {
        offset
            .checked_add(inserted_size - removed_size)
            .ok_or(SidxRewriteError::NumericOverflow { field_name })
    } else {
        offset
            .checked_sub(removed_size - inserted_size)
            .ok_or(SidxRewriteError::NumericOverflow { field_name })
    }
}

fn set_sidx_first_offset(sidx: &mut Sidx, first_offset: u64) -> Result<(), SidxRewriteError> {
    match sidx.version() {
        0 => {
            sidx.first_offset_v0 =
                u32::try_from(first_offset).map_err(|_| SidxRewriteError::NumericOverflow {
                    field_name: "rewritten first offset",
                })?;
            Ok(())
        }
        1 => {
            sidx.first_offset_v1 = first_offset;
            Ok(())
        }
        version => Err(SidxRewriteError::UnsupportedSidxVersion { version }),
    }
}

fn encode_sidx_box(sidx: &Sidx) -> Result<(Vec<u8>, BoxInfo), SidxRewriteError> {
    let mut payload = Vec::new();
    marshal(&mut payload, sidx, None)?;

    let payload_size = payload.len() as u64;
    let header_size = box_header_size_for_payload(payload_size);
    let total_size = payload_size
        .checked_add(header_size)
        .ok_or(SidxRewriteError::EncodedBoxSizeOverflow)?;
    let info = BoxInfo::new(SIDX, total_size).with_header_size(header_size);

    let mut bytes = info.encode();
    bytes.extend_from_slice(&payload);
    Ok((bytes, info))
}

fn copy_range_exact<R, W>(
    reader: &mut R,
    writer: &mut W,
    start: u64,
    len: u64,
) -> Result<(), SidxRewriteError>
where
    R: Read + Seek,
    W: Write,
{
    if len == 0 {
        return Ok(());
    }

    reader.seek(SeekFrom::Start(start))?;
    let mut limited = reader.take(len);
    let copied = io::copy(&mut limited, writer)?;
    if copied != len {
        return Err(SidxRewriteError::IncompleteCopy {
            expected_size: len,
            actual_size: copied,
        });
    }

    Ok(())
}

#[cfg(feature = "async")]
async fn copy_range_exact_async<R, W>(
    reader: &mut R,
    writer: &mut W,
    start: u64,
    len: u64,
) -> Result<(), SidxRewriteError>
where
    R: AsyncReadSeek,
    W: AsyncWriteSeek,
{
    if len == 0 {
        return Ok(());
    }

    reader.seek(SeekFrom::Start(start)).await?;
    let mut limited = (&mut *reader).take(len);
    let copied = tokio::io::copy(&mut limited, writer).await?;
    if copied != len {
        return Err(SidxRewriteError::IncompleteCopy {
            expected_size: len,
            actual_size: copied,
        });
    }

    Ok(())
}

fn checked_add_rewrite(
    lhs: u64,
    rhs: u64,
    field_name: &'static str,
) -> Result<u64, SidxRewriteError> {
    lhs.checked_add(rhs)
        .ok_or(SidxRewriteError::NumericOverflow { field_name })
}
