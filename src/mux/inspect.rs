//! Public direct-ingest inspection and export helpers built on the mux parser path.
//!
//! This additive surface lets callers inspect one path-first direct-ingest input without writing
//! an MP4 first. Reports intentionally reuse the same native detection and staging path that the
//! real mux task uses, so successful reports describe the same staged sources, track metadata, and
//! sample timing that the retained flat mux surface would consume.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::MuxError;

/// Structured output formats supported by the direct-ingest report writers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectIngestReportFormat {
    /// Pretty-printed JSON output with stable field ordering.
    Json,
    /// Stable YAML output with one explicit field order.
    Yaml,
    /// Stable NHML-like XML sidecar output for the track-oriented direct-ingest view.
    Nhml,
    /// Stable NHNT-like XML sidecar output for the packet-oriented direct-ingest view.
    Nhnt,
}

/// Top-level detection result for one path-first direct-ingest input.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectIngestDetectedKind {
    /// The input path was recognized as one MP4-family source.
    Mp4,
    /// The input path was recognized as one supported carried container family.
    Container {
        /// Stable lowercase container-family label.
        container: String,
    },
    /// The input path was recognized as one supported raw direct-ingest family.
    Raw {
        /// Stable lowercase codec-family label.
        codec: String,
    },
    /// The input path was recognized, but it is currently import-only on the native direct path.
    ImportOnly {
        /// Stable human-readable family label explaining the import-only state.
        family: String,
    },
    /// The input path was not recognized by the current native direct-ingest detector.
    Unknown,
}

/// One staged source referenced by the inspected direct-ingest path.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectIngestStagedSourceReport {
    /// Stable staged source index referenced by the per-sample reports.
    pub source_index: usize,
    /// Backing filesystem path used by the staged source.
    pub path: PathBuf,
    /// Whether this staged source is segmented rather than one direct file range.
    pub segmented: bool,
    /// Total logical payload size exposed by this staged source.
    pub total_size: u64,
    /// Number of logical segments when this staged source is segmented.
    pub segment_count: Option<usize>,
    /// Logical segment detail when this staged source is segmented.
    pub segments: Option<Vec<DirectIngestSourceSegmentReport>>,
}

/// One logical segment inside a segmented staged source.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectIngestSourceSegmentReport {
    /// Stable lowercase segment-kind label.
    pub kind: String,
    /// Logical staged byte offset where this segment begins.
    pub logical_offset: u64,
    /// Logical staged payload size exposed by this segment.
    pub logical_size: u64,
    /// Backing file byte offset when the segment reads directly from the source file.
    pub source_offset: Option<u64>,
    /// Backing filesystem path when the segment reads directly from one external file.
    pub source_path: Option<PathBuf>,
    /// Inline payload bytes as one lowercase hexadecimal string when the segment is inline.
    pub data_hex: Option<String>,
}

/// One staged sample entry in the direct-ingest report.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectIngestSampleReport {
    /// Staged source index that supplies this sample's bytes.
    pub source_index: usize,
    /// Logical staged byte offset used by the mux path for this sample.
    pub data_offset: u64,
    /// Number of staged payload bytes used by this sample.
    pub data_size: u32,
    /// Decode time assigned by the native direct-ingest path before movie-timescale normalization.
    pub decode_time: u64,
    /// Decode-time delta from the previous sample in this track, when one exists.
    pub previous_decode_delta: Option<u64>,
    /// Composition-time offset carried by this sample.
    pub composition_time_offset: i32,
    /// Presentation timestamp derived from decode time and composition offset.
    pub presentation_time: i64,
    /// Presentation end timestamp derived from presentation time and duration.
    pub presentation_end_time: i64,
    /// Presentation-time delta from the previous sample in this track, when one exists.
    pub previous_presentation_delta: Option<i64>,
    /// Decode duration carried by this sample.
    pub duration: u32,
    /// Whether this sample is marked as a sync sample.
    pub is_sync_sample: bool,
}

/// One staged track entry in the direct-ingest report.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectIngestTrackReport {
    /// Track identifier that the direct-ingest path would assign or preserve.
    pub track_id: u32,
    /// Stable lowercase track-kind label used by the landed mux surface.
    pub kind: String,
    /// Media timescale carried by this track.
    pub timescale: u32,
    /// Three-letter ISO-639-2 language code.
    pub language: String,
    /// Authored handler-name string used by the current native path.
    pub handler_name: String,
    /// Sample-entry box type authored or preserved for this track.
    pub sample_entry_type: String,
    /// Full sample-entry box bytes as one lowercase hexadecimal string.
    pub sample_entry_box_hex: String,
    /// Visual width when the track is visual.
    pub width: Option<u16>,
    /// Visual height when the track is visual.
    pub height: Option<u16>,
    /// Source edit media time when the direct-ingest path preserves one.
    pub source_edit_media_time: Option<u64>,
    /// Sample roll distance when the current native path preserves one.
    pub sample_roll_distance: Option<i16>,
    /// Number of staged samples carried by this track.
    pub sample_count: usize,
    /// Number of sync samples carried by this track.
    pub sync_sample_count: usize,
    /// Whether the first staged sample in this track is marked as a sync sample.
    pub starts_with_sync_sample: bool,
    /// Sum of staged sample durations on the native media timescale.
    pub total_duration: u64,
    /// Sum of staged payload sizes in bytes.
    pub total_payload_size: u64,
    /// Average staged sample payload size in bytes.
    pub average_sample_size: Option<u64>,
    /// Smallest staged sample payload size in bytes.
    pub minimum_sample_size: Option<u32>,
    /// Largest staged sample payload size in bytes.
    pub maximum_sample_size: Option<u32>,
    /// Smallest staged sample duration on the native media timescale.
    pub minimum_sample_duration: Option<u32>,
    /// Largest staged sample duration on the native media timescale.
    pub maximum_sample_duration: Option<u32>,
    /// Average authored track bitrate on the native media timescale.
    pub average_bitrate_bits_per_second: Option<u64>,
    /// Smallest sync-sample payload size in bytes.
    pub minimum_sync_sample_size: Option<u32>,
    /// Largest sync-sample payload size in bytes.
    pub maximum_sync_sample_size: Option<u32>,
    /// Average sync-sample payload size in bytes.
    pub average_sync_sample_size: Option<u64>,
    /// Average non-sync-sample payload size in bytes.
    pub average_non_sync_sample_size: Option<u64>,
    /// Smallest composition-time offset observed across the staged samples.
    pub minimum_composition_time_offset: Option<i32>,
    /// Largest composition-time offset observed across the staged samples.
    pub maximum_composition_time_offset: Option<i32>,
    /// Smallest presentation timestamp observed across the staged samples.
    pub minimum_presentation_time: Option<i64>,
    /// Largest presentation end timestamp observed across the staged samples.
    pub maximum_presentation_end_time: Option<i64>,
    /// Smallest presentation-time delta observed between consecutive samples.
    pub minimum_previous_presentation_delta: Option<i64>,
    /// Largest presentation-time delta observed between consecutive samples.
    pub maximum_previous_presentation_delta: Option<i64>,
    /// Smallest decode-time delta observed between consecutive samples.
    pub minimum_previous_decode_delta: Option<u64>,
    /// Largest decode-time delta observed between consecutive samples.
    pub maximum_previous_decode_delta: Option<u64>,
    /// Number of times one sample started after the previous presentation end time.
    pub presentation_gap_count: usize,
    /// Number of times one sample started before the previous presentation end time.
    pub presentation_overlap_count: usize,
    /// Number of times one sample presentation time moved backward relative to the previous one.
    pub presentation_regression_count: usize,
    /// Number of times adjacent samples changed decode duration.
    pub duration_change_count: usize,
    /// Number of times adjacent samples changed composition-time offset.
    pub composition_time_offset_change_count: usize,
    /// Smallest distance in samples between consecutive sync samples.
    pub minimum_sync_sample_distance: Option<u32>,
    /// Largest distance in samples between consecutive sync samples.
    pub maximum_sync_sample_distance: Option<u32>,
    /// Average distance in samples between consecutive sync samples.
    pub average_sync_sample_distance: Option<u64>,
    /// Smallest decode-time delta between consecutive sync samples.
    pub minimum_sync_sample_decode_delta: Option<u64>,
    /// Largest decode-time delta between consecutive sync samples.
    pub maximum_sync_sample_decode_delta: Option<u64>,
    /// Average decode-time delta between consecutive sync samples.
    pub average_sync_sample_decode_delta: Option<u64>,
    /// Zero-based index of the first sync sample when one exists.
    pub first_sync_sample_index: Option<usize>,
    /// Zero-based index of the last sync sample when one exists.
    pub last_sync_sample_index: Option<usize>,
    /// Decode time of the first sync sample when one exists.
    pub first_sync_decode_time: Option<u64>,
    /// Decode time of the last sync sample when one exists.
    pub last_sync_decode_time: Option<u64>,
    /// Presentation time of the first sync sample when one exists.
    pub first_sync_presentation_time: Option<i64>,
    /// Presentation time of the last sync sample when one exists.
    pub last_sync_presentation_time: Option<i64>,
    /// Decode time of the first staged sample in this track.
    pub first_decode_time: u64,
    /// Decode end time of the last staged sample in this track.
    pub end_decode_time: u64,
    /// Per-sample staged metadata in native decode order.
    pub samples: Vec<DirectIngestSampleReport>,
}

/// Top-level direct-ingest report for one input path.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectIngestReport {
    /// Input path inspected by the direct-ingest detector.
    pub input_path: PathBuf,
    /// High-level detection result for this input path.
    pub detected_kind: DirectIngestDetectedKind,
    /// Whether the current native direct-ingest path supports this input directly.
    pub supports_flat_mux: bool,
    /// Optional explanatory note for import-only or unknown inputs.
    pub note: Option<String>,
    /// Number of staged tracks surfaced by the native direct-ingest path.
    pub track_count: usize,
    /// Total number of staged samples across every reported track.
    pub total_sample_count: usize,
    /// Total number of sync samples across every reported track.
    pub total_sync_sample_count: usize,
    /// Sum of staged payload sizes across every reported track.
    pub total_payload_size: u64,
    /// Staged sources referenced by the reported samples.
    pub staged_sources: Vec<DirectIngestStagedSourceReport>,
    /// Track reports surfaced by the native direct-ingest path.
    pub tracks: Vec<DirectIngestTrackReport>,
}

/// One flattened packet entry in the additive packet-focused direct-ingest view.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectIngestPacketEntry {
    /// Track identifier that the direct-ingest path would assign or preserve.
    pub track_id: u32,
    /// Zero-based packet index in native decode order within the track.
    pub packet_index: usize,
    /// Stable lowercase track-kind label used by the landed mux surface.
    pub track_kind: String,
    /// Media timescale carried by this packet's track.
    pub timescale: u32,
    /// Sample-entry box type authored or preserved for this packet's track.
    pub sample_entry_type: String,
    /// Staged source index that supplies this packet's bytes.
    pub source_index: usize,
    /// Logical staged byte offset used by the mux path for this packet.
    pub data_offset: u64,
    /// Number of staged payload bytes used by this packet.
    pub data_size: u32,
    /// Decode time assigned by the native direct-ingest path before movie-timescale normalization.
    pub decode_time: u64,
    /// Composition-time offset carried by this packet.
    pub composition_time_offset: i32,
    /// Presentation timestamp derived from decode time and composition offset.
    pub presentation_time: i64,
    /// Presentation end timestamp derived from presentation time and duration.
    pub presentation_end_time: i64,
    /// Presentation-time delta from the previous packet in this track, when one exists.
    pub previous_presentation_delta: Option<i64>,
    /// Decode duration carried by this packet.
    pub duration: u32,
    /// Decode-time delta from the previous packet in this track, when one exists.
    pub previous_decode_delta: Option<u64>,
    /// CRC-32 of the staged packet payload bytes.
    pub payload_crc32: u32,
    /// Whether this packet is marked as a sync sample.
    pub is_sync_sample: bool,
}

/// Top-level packet-focused direct-ingest report for one input path.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectIngestPacketReport {
    /// Input path inspected by the direct-ingest detector.
    pub input_path: PathBuf,
    /// High-level detection result for this input path.
    pub detected_kind: DirectIngestDetectedKind,
    /// Whether the current native direct-ingest path supports this input directly.
    pub supports_flat_mux: bool,
    /// Optional explanatory note for import-only or unknown inputs.
    pub note: Option<String>,
    /// Number of staged tracks surfaced by the native direct-ingest path.
    pub track_count: usize,
    /// Number of flattened packets emitted by this report.
    pub packet_count: usize,
    /// Number of sync packets emitted by this report.
    pub sync_packet_count: usize,
    /// Whether the first flattened packet is marked as a sync packet.
    pub starts_with_sync_packet: bool,
    /// Sum of staged payload sizes across every flattened packet.
    pub total_payload_size: u64,
    /// Smallest flattened packet payload size in bytes.
    pub minimum_packet_size: Option<u32>,
    /// Largest flattened packet payload size in bytes.
    pub maximum_packet_size: Option<u32>,
    /// Smallest sync-packet payload size in bytes.
    pub minimum_sync_packet_size: Option<u32>,
    /// Largest sync-packet payload size in bytes.
    pub maximum_sync_packet_size: Option<u32>,
    /// Average sync-packet payload size in bytes.
    pub average_sync_packet_size: Option<u64>,
    /// Average non-sync-packet payload size in bytes.
    pub average_non_sync_packet_size: Option<u64>,
    /// Smallest flattened packet duration on the native media timescale.
    pub minimum_packet_duration: Option<u32>,
    /// Largest flattened packet duration on the native media timescale.
    pub maximum_packet_duration: Option<u32>,
    /// Smallest previous decode delta surfaced by the flattened packet view.
    pub minimum_previous_decode_delta: Option<u64>,
    /// Largest previous decode delta surfaced by the flattened packet view.
    pub maximum_previous_decode_delta: Option<u64>,
    /// Smallest composition-time offset surfaced by the flattened packet view.
    pub minimum_composition_time_offset: Option<i32>,
    /// Largest composition-time offset surfaced by the flattened packet view.
    pub maximum_composition_time_offset: Option<i32>,
    /// Smallest presentation timestamp surfaced by the flattened packet view.
    pub minimum_presentation_time: Option<i64>,
    /// Largest presentation end timestamp surfaced by the flattened packet view.
    pub maximum_presentation_end_time: Option<i64>,
    /// Smallest presentation-time delta surfaced by the flattened packet view.
    pub minimum_previous_presentation_delta: Option<i64>,
    /// Largest presentation-time delta surfaced by the flattened packet view.
    pub maximum_previous_presentation_delta: Option<i64>,
    /// Number of times one packet started after the previous presentation end time within a track.
    pub presentation_gap_count: usize,
    /// Number of times one packet started before the previous presentation end time within a track.
    pub presentation_overlap_count: usize,
    /// Number of times one packet presentation time moved backward within a track.
    pub presentation_regression_count: usize,
    /// Number of times adjacent packets changed decode duration within a track.
    pub duration_change_count: usize,
    /// Number of times adjacent packets changed composition-time offset within a track.
    pub composition_time_offset_change_count: usize,
    /// Smallest per-track packet distance between consecutive sync packets.
    pub minimum_sync_packet_distance: Option<u32>,
    /// Largest per-track packet distance between consecutive sync packets.
    pub maximum_sync_packet_distance: Option<u32>,
    /// Average per-track packet distance between consecutive sync packets.
    pub average_sync_packet_distance: Option<u64>,
    /// Smallest per-track decode-time delta between consecutive sync packets.
    pub minimum_sync_packet_decode_delta: Option<u64>,
    /// Largest per-track decode-time delta between consecutive sync packets.
    pub maximum_sync_packet_decode_delta: Option<u64>,
    /// Average per-track decode-time delta between consecutive sync packets.
    pub average_sync_packet_decode_delta: Option<u64>,
    /// Track identifier of the first sync packet when one exists.
    pub first_sync_packet_track_id: Option<u32>,
    /// Zero-based packet index of the first sync packet when one exists.
    pub first_sync_packet_index: Option<usize>,
    /// Track identifier of the last sync packet when one exists.
    pub last_sync_packet_track_id: Option<u32>,
    /// Zero-based packet index of the last sync packet when one exists.
    pub last_sync_packet_index: Option<usize>,
    /// Decode time of the first sync packet when one exists.
    pub first_sync_decode_time: Option<u64>,
    /// Decode time of the last sync packet when one exists.
    pub last_sync_decode_time: Option<u64>,
    /// Presentation time of the first sync packet when one exists.
    pub first_sync_presentation_time: Option<i64>,
    /// Presentation time of the last sync packet when one exists.
    pub last_sync_presentation_time: Option<i64>,
    /// Track reports surfaced by the native direct-ingest path before packet flattening.
    pub tracks: Vec<DirectIngestTrackReport>,
    /// Staged sources referenced by the reported packets.
    pub staged_sources: Vec<DirectIngestStagedSourceReport>,
    /// Flattened packet entries in track and decode order.
    pub packets: Vec<DirectIngestPacketEntry>,
}

/// Inspects one path-first direct-ingest input with the synchronous mux parser path.
pub fn inspect_direct_ingest_path(path: impl AsRef<Path>) -> Result<DirectIngestReport, MuxError> {
    super::import::inspect_direct_ingest_path_sync(path.as_ref())
}

/// Inspects one path-first direct-ingest input with the additive async mux parser path.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn inspect_direct_ingest_path_async(
    path: impl AsRef<Path>,
) -> Result<DirectIngestReport, MuxError> {
    super::import::inspect_direct_ingest_path_async(path.as_ref()).await
}

/// Inspects one path-first direct-ingest input and flattens the staged track view into packets.
pub fn inspect_direct_ingest_packets(
    path: impl AsRef<Path>,
) -> Result<DirectIngestPacketReport, MuxError> {
    super::import::inspect_direct_ingest_packets_sync(path.as_ref())
}

/// Async companion to [`inspect_direct_ingest_packets`].
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn inspect_direct_ingest_packets_async(
    path: impl AsRef<Path>,
) -> Result<DirectIngestPacketReport, MuxError> {
    super::import::inspect_direct_ingest_packets_async(path.as_ref()).await
}

/// Collects warning-grade diagnostics from one track-focused direct-ingest report.
pub fn collect_track_report_warnings(report: &DirectIngestReport) -> Vec<String> {
    let mut warnings = Vec::new();
    for track in &report.tracks {
        if !track.starts_with_sync_sample {
            warnings.push(format!(
                "track {} ({}) does not start with a sync sample",
                track.track_id, track.kind
            ));
        }
        if track.sync_sample_count == 0 {
            warnings.push(format!(
                "track {} ({}) has no sync samples",
                track.track_id, track.kind
            ));
        }
        if track.presentation_gap_count != 0 {
            warnings.push(format!(
                "track {} ({}) has {} presentation gap(s)",
                track.track_id, track.kind, track.presentation_gap_count
            ));
        }
        if track.presentation_overlap_count != 0 {
            warnings.push(format!(
                "track {} ({}) has {} presentation overlap(s)",
                track.track_id, track.kind, track.presentation_overlap_count
            ));
        }
        if track.presentation_regression_count != 0 {
            warnings.push(format!(
                "track {} ({}) has {} presentation regression(s)",
                track.track_id, track.kind, track.presentation_regression_count
            ));
        }
        if track.duration_change_count != 0
            && track.minimum_sample_duration != track.maximum_sample_duration
        {
            warnings.push(format!(
                "track {} ({}) changes decode duration {} time(s)",
                track.track_id, track.kind, track.duration_change_count
            ));
        }
        if track.composition_time_offset_change_count != 0
            && track.minimum_composition_time_offset != track.maximum_composition_time_offset
        {
            warnings.push(format!(
                "track {} ({}) changes composition offset {} time(s)",
                track.track_id, track.kind, track.composition_time_offset_change_count
            ));
        }
    }
    warnings
}

/// Collects warning-grade diagnostics from one packet-focused direct-ingest report.
pub fn collect_packet_report_warnings(report: &DirectIngestPacketReport) -> Vec<String> {
    let mut warnings = Vec::new();
    if report.packet_count != 0 && !report.starts_with_sync_packet {
        warnings.push("packet view does not start with a sync packet".to_string());
    }
    if report.packet_count != 0 && report.sync_packet_count == 0 {
        warnings.push("packet view has no sync packets".to_string());
    }
    if report.presentation_gap_count != 0 {
        warnings.push(format!(
            "packet view has {} presentation gap(s)",
            report.presentation_gap_count
        ));
    }
    if report.presentation_overlap_count != 0 {
        warnings.push(format!(
            "packet view has {} presentation overlap(s)",
            report.presentation_overlap_count
        ));
    }
    if report.presentation_regression_count != 0 {
        warnings.push(format!(
            "packet view has {} presentation regression(s)",
            report.presentation_regression_count
        ));
    }
    if report.duration_change_count != 0
        && report.minimum_packet_duration != report.maximum_packet_duration
    {
        warnings.push(format!(
            "packet view changes decode duration {} time(s)",
            report.duration_change_count
        ));
    }
    if report.composition_time_offset_change_count != 0
        && report.minimum_composition_time_offset != report.maximum_composition_time_offset
    {
        warnings.push(format!(
            "packet view changes composition offset {} time(s)",
            report.composition_time_offset_change_count
        ));
    }
    warnings
}

/// Writes one direct-ingest report in the requested stable structured format.
pub fn write_report<W>(
    writer: &mut W,
    report: &DirectIngestReport,
    format: DirectIngestReportFormat,
) -> io::Result<()>
where
    W: Write,
{
    match format {
        DirectIngestReportFormat::Json => write_json_report(writer, report),
        DirectIngestReportFormat::Yaml => write_yaml_report(writer, report),
        DirectIngestReportFormat::Nhml => write_nhml_report(writer, report),
        DirectIngestReportFormat::Nhnt => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NHNT output requires the packet inspection view",
        )),
    }
}

/// Writes one packet-focused direct-ingest report in the requested stable structured format.
pub fn write_packet_report<W>(
    writer: &mut W,
    report: &DirectIngestPacketReport,
    format: DirectIngestReportFormat,
) -> io::Result<()>
where
    W: Write,
{
    match format {
        DirectIngestReportFormat::Json => write_json_packet_report(writer, report),
        DirectIngestReportFormat::Yaml => write_yaml_packet_report(writer, report),
        DirectIngestReportFormat::Nhml => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NHML output requires the track inspection view",
        )),
        DirectIngestReportFormat::Nhnt => write_nhnt_report(writer, report),
    }
}

fn detected_kind_name(kind: &DirectIngestDetectedKind) -> &'static str {
    match kind {
        DirectIngestDetectedKind::Mp4 => "mp4",
        DirectIngestDetectedKind::Container { .. } => "container",
        DirectIngestDetectedKind::Raw { .. } => "raw",
        DirectIngestDetectedKind::ImportOnly { .. } => "import_only",
        DirectIngestDetectedKind::Unknown => "unknown",
    }
}

fn write_json_report<W>(writer: &mut W, report: &DirectIngestReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "{{")?;
    write_json_field(
        writer,
        1,
        "InputPath",
        &json_string(&report.input_path.display().to_string()),
        true,
    )?;
    write_json_detected_kind(writer, &report.detected_kind)?;
    write_json_field(
        writer,
        1,
        "SupportsFlatMux",
        if report.supports_flat_mux {
            "true"
        } else {
            "false"
        },
        true,
    )?;
    if let Some(note) = &report.note {
        write_json_field(writer, 1, "Note", &json_string(note), true)?;
    }
    write_json_field(
        writer,
        1,
        "TrackCount",
        &report.track_count.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "TotalSampleCount",
        &report.total_sample_count.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "TotalSyncSampleCount",
        &report.total_sync_sample_count.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "TotalPayloadSize",
        &report.total_payload_size.to_string(),
        true,
    )?;
    writeln!(writer, "  \"StagedSources\": [")?;
    for (index, source) in report.staged_sources.iter().enumerate() {
        write_json_source(writer, source, index + 1 != report.staged_sources.len())?;
    }
    writeln!(writer, "  ],")?;
    writeln!(writer, "  \"Tracks\": [")?;
    for (index, track) in report.tracks.iter().enumerate() {
        write_json_track(writer, track, index + 1 != report.tracks.len())?;
    }
    writeln!(writer, "  ]")?;
    writeln!(writer, "}}")
}

fn write_json_packet_report<W>(writer: &mut W, report: &DirectIngestPacketReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "{{")?;
    write_json_field(
        writer,
        1,
        "InputPath",
        &json_string(&report.input_path.display().to_string()),
        true,
    )?;
    write_json_detected_kind(writer, &report.detected_kind)?;
    write_json_field(
        writer,
        1,
        "SupportsFlatMux",
        if report.supports_flat_mux {
            "true"
        } else {
            "false"
        },
        true,
    )?;
    if let Some(note) = &report.note {
        write_json_field(writer, 1, "Note", &json_string(note), true)?;
    }
    write_json_field(
        writer,
        1,
        "TrackCount",
        &report.track_count.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "PacketCount",
        &report.packet_count.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "SyncPacketCount",
        &report.sync_packet_count.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "StartsWithSyncPacket",
        if report.starts_with_sync_packet {
            "true"
        } else {
            "false"
        },
        true,
    )?;
    write_json_field(
        writer,
        1,
        "TotalPayloadSize",
        &report.total_payload_size.to_string(),
        true,
    )?;
    if let Some(minimum_packet_size) = report.minimum_packet_size {
        write_json_field(
            writer,
            1,
            "MinimumPacketSize",
            &minimum_packet_size.to_string(),
            true,
        )?;
    }
    if let Some(maximum_packet_size) = report.maximum_packet_size {
        write_json_field(
            writer,
            1,
            "MaximumPacketSize",
            &maximum_packet_size.to_string(),
            true,
        )?;
    }
    if let Some(minimum_sync_packet_size) = report.minimum_sync_packet_size {
        write_json_field(
            writer,
            1,
            "MinimumSyncPacketSize",
            &minimum_sync_packet_size.to_string(),
            true,
        )?;
    }
    if let Some(maximum_sync_packet_size) = report.maximum_sync_packet_size {
        write_json_field(
            writer,
            1,
            "MaximumSyncPacketSize",
            &maximum_sync_packet_size.to_string(),
            true,
        )?;
    }
    if let Some(average_sync_packet_size) = report.average_sync_packet_size {
        write_json_field(
            writer,
            1,
            "AverageSyncPacketSize",
            &average_sync_packet_size.to_string(),
            true,
        )?;
    }
    if let Some(average_non_sync_packet_size) = report.average_non_sync_packet_size {
        write_json_field(
            writer,
            1,
            "AverageNonSyncPacketSize",
            &average_non_sync_packet_size.to_string(),
            true,
        )?;
    }
    if let Some(minimum_packet_duration) = report.minimum_packet_duration {
        write_json_field(
            writer,
            1,
            "MinimumPacketDuration",
            &minimum_packet_duration.to_string(),
            true,
        )?;
    }
    if let Some(maximum_packet_duration) = report.maximum_packet_duration {
        write_json_field(
            writer,
            1,
            "MaximumPacketDuration",
            &maximum_packet_duration.to_string(),
            true,
        )?;
    }
    if let Some(minimum_previous_decode_delta) = report.minimum_previous_decode_delta {
        write_json_field(
            writer,
            1,
            "MinimumPreviousDecodeDelta",
            &minimum_previous_decode_delta.to_string(),
            true,
        )?;
    }
    if let Some(maximum_previous_decode_delta) = report.maximum_previous_decode_delta {
        write_json_field(
            writer,
            1,
            "MaximumPreviousDecodeDelta",
            &maximum_previous_decode_delta.to_string(),
            true,
        )?;
    }
    if let Some(minimum_composition_time_offset) = report.minimum_composition_time_offset {
        write_json_field(
            writer,
            1,
            "MinimumCompositionTimeOffset",
            &minimum_composition_time_offset.to_string(),
            true,
        )?;
    }
    if let Some(maximum_composition_time_offset) = report.maximum_composition_time_offset {
        write_json_field(
            writer,
            1,
            "MaximumCompositionTimeOffset",
            &maximum_composition_time_offset.to_string(),
            true,
        )?;
    }
    if let Some(minimum_presentation_time) = report.minimum_presentation_time {
        write_json_field(
            writer,
            1,
            "MinimumPresentationTime",
            &minimum_presentation_time.to_string(),
            true,
        )?;
    }
    if let Some(maximum_presentation_end_time) = report.maximum_presentation_end_time {
        write_json_field(
            writer,
            1,
            "MaximumPresentationEndTime",
            &maximum_presentation_end_time.to_string(),
            true,
        )?;
    }
    if let Some(minimum_previous_presentation_delta) = report.minimum_previous_presentation_delta {
        write_json_field(
            writer,
            1,
            "MinimumPreviousPresentationDelta",
            &minimum_previous_presentation_delta.to_string(),
            true,
        )?;
    }
    if let Some(maximum_previous_presentation_delta) = report.maximum_previous_presentation_delta {
        write_json_field(
            writer,
            1,
            "MaximumPreviousPresentationDelta",
            &maximum_previous_presentation_delta.to_string(),
            true,
        )?;
    }
    write_json_field(
        writer,
        1,
        "PresentationGapCount",
        &report.presentation_gap_count.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "PresentationOverlapCount",
        &report.presentation_overlap_count.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "PresentationRegressionCount",
        &report.presentation_regression_count.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "DurationChangeCount",
        &report.duration_change_count.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "CompositionTimeOffsetChangeCount",
        &report.composition_time_offset_change_count.to_string(),
        true,
    )?;
    if let Some(minimum_sync_packet_distance) = report.minimum_sync_packet_distance {
        write_json_field(
            writer,
            1,
            "MinimumSyncPacketDistance",
            &minimum_sync_packet_distance.to_string(),
            true,
        )?;
    }
    if let Some(maximum_sync_packet_distance) = report.maximum_sync_packet_distance {
        write_json_field(
            writer,
            1,
            "MaximumSyncPacketDistance",
            &maximum_sync_packet_distance.to_string(),
            true,
        )?;
    }
    if let Some(average_sync_packet_distance) = report.average_sync_packet_distance {
        write_json_field(
            writer,
            1,
            "AverageSyncPacketDistance",
            &average_sync_packet_distance.to_string(),
            true,
        )?;
    }
    if let Some(minimum_sync_packet_decode_delta) = report.minimum_sync_packet_decode_delta {
        write_json_field(
            writer,
            1,
            "MinimumSyncPacketDecodeDelta",
            &minimum_sync_packet_decode_delta.to_string(),
            true,
        )?;
    }
    if let Some(maximum_sync_packet_decode_delta) = report.maximum_sync_packet_decode_delta {
        write_json_field(
            writer,
            1,
            "MaximumSyncPacketDecodeDelta",
            &maximum_sync_packet_decode_delta.to_string(),
            true,
        )?;
    }
    if let Some(average_sync_packet_decode_delta) = report.average_sync_packet_decode_delta {
        write_json_field(
            writer,
            1,
            "AverageSyncPacketDecodeDelta",
            &average_sync_packet_decode_delta.to_string(),
            true,
        )?;
    }
    write_json_field(
        writer,
        1,
        "FirstSyncPacketTrackID",
        &report
            .first_sync_packet_track_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "FirstSyncPacketIndex",
        &report
            .first_sync_packet_index
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "LastSyncPacketTrackID",
        &report
            .last_sync_packet_track_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "LastSyncPacketIndex",
        &report
            .last_sync_packet_index
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "FirstSyncDecodeTime",
        &report
            .first_sync_decode_time
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "LastSyncDecodeTime",
        &report
            .last_sync_decode_time
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "FirstSyncPresentationTime",
        &report
            .first_sync_presentation_time
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "LastSyncPresentationTime",
        &report
            .last_sync_presentation_time
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        true,
    )?;
    writeln!(writer, "  \"StagedSources\": [")?;
    for (index, source) in report.staged_sources.iter().enumerate() {
        write_json_source(writer, source, index + 1 != report.staged_sources.len())?;
    }
    writeln!(writer, "  ],")?;
    writeln!(writer, "  \"Packets\": [")?;
    for (index, packet) in report.packets.iter().enumerate() {
        write_json_packet(writer, packet, index + 1 != report.packets.len())?;
    }
    writeln!(writer, "  ]")?;
    writeln!(writer, "}}")
}

fn write_json_detected_kind<W>(writer: &mut W, kind: &DirectIngestDetectedKind) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "  \"DetectedKind\": {{")?;
    write_json_field(
        writer,
        2,
        "Kind",
        &json_string(detected_kind_name(kind)),
        true,
    )?;
    match kind {
        DirectIngestDetectedKind::Container { container } => {
            write_json_field(writer, 2, "Container", &json_string(container), false)?;
        }
        DirectIngestDetectedKind::Raw { codec } => {
            write_json_field(writer, 2, "Codec", &json_string(codec), false)?;
        }
        DirectIngestDetectedKind::ImportOnly { family } => {
            write_json_field(writer, 2, "Family", &json_string(family), false)?;
        }
        DirectIngestDetectedKind::Mp4 | DirectIngestDetectedKind::Unknown => {
            writeln!(writer, "    \"Value\": null")?;
        }
    }
    writeln!(writer, "  }},")
}

fn write_json_source<W>(
    writer: &mut W,
    source: &DirectIngestStagedSourceReport,
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "    {{")?;
    let mut fields = vec![
        ("SourceIndex", source.source_index.to_string()),
        ("Path", json_string(&source.path.display().to_string())),
        (
            "Segmented",
            if source.segmented { "true" } else { "false" }.to_string(),
        ),
        ("TotalSize", source.total_size.to_string()),
    ];
    if let Some(segment_count) = source.segment_count {
        fields.push(("SegmentCount", segment_count.to_string()));
    }
    let has_segments = source
        .segments
        .as_ref()
        .map(|segments| !segments.is_empty())
        .unwrap_or(false);
    for (index, (name, value)) in fields.iter().enumerate() {
        let trailing = index + 1 != fields.len() || has_segments;
        write_json_field(writer, 3, name, value, trailing)?;
    }
    if let Some(segments) = &source.segments
        && !segments.is_empty()
    {
        writeln!(writer, "      \"Segments\": [")?;
        for (index, segment) in segments.iter().enumerate() {
            write_json_source_segment(writer, segment, index + 1 != segments.len())?;
        }
        writeln!(writer, "      ]")?;
    }
    writeln!(writer, "    }}{}", if trailing_comma { "," } else { "" })
}

fn write_json_source_segment<W>(
    writer: &mut W,
    segment: &DirectIngestSourceSegmentReport,
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "        {{")?;
    let mut fields = vec![
        ("Kind", json_string(&segment.kind)),
        ("LogicalOffset", segment.logical_offset.to_string()),
        ("LogicalSize", segment.logical_size.to_string()),
    ];
    if let Some(source_offset) = segment.source_offset {
        fields.push(("SourceOffset", source_offset.to_string()));
    }
    if let Some(data_hex) = &segment.data_hex {
        fields.push(("DataHex", json_string(data_hex)));
    }
    for (index, (name, value)) in fields.iter().enumerate() {
        write_json_field(writer, 5, name, value, index + 1 != fields.len())?;
    }
    writeln!(
        writer,
        "        }}{}",
        if trailing_comma { "," } else { "" }
    )
}

fn write_json_track<W>(
    writer: &mut W,
    track: &DirectIngestTrackReport,
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "    {{")?;
    let mut fields = vec![
        ("TrackID", track.track_id.to_string()),
        ("Kind", json_string(&track.kind)),
        ("Timescale", track.timescale.to_string()),
        ("Language", json_string(&track.language)),
        ("HandlerName", json_string(&track.handler_name)),
        ("SampleEntryType", json_string(&track.sample_entry_type)),
        (
            "SampleEntryBoxHex",
            json_string(&track.sample_entry_box_hex),
        ),
        ("SampleCount", track.sample_count.to_string()),
        ("SyncSampleCount", track.sync_sample_count.to_string()),
        (
            "StartsWithSyncSample",
            if track.starts_with_sync_sample {
                "true"
            } else {
                "false"
            }
            .to_string(),
        ),
        ("TotalDuration", track.total_duration.to_string()),
        ("TotalPayloadSize", track.total_payload_size.to_string()),
        (
            "AverageSampleSize",
            track
                .average_sample_size
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MinimumSampleSize",
            track
                .minimum_sample_size
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MaximumSampleSize",
            track
                .maximum_sample_size
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MinimumSampleDuration",
            track
                .minimum_sample_duration
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MaximumSampleDuration",
            track
                .maximum_sample_duration
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "AverageBitrateBitsPerSecond",
            track
                .average_bitrate_bits_per_second
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MinimumSyncSampleSize",
            track
                .minimum_sync_sample_size
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MaximumSyncSampleSize",
            track
                .maximum_sync_sample_size
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "AverageSyncSampleSize",
            track
                .average_sync_sample_size
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "AverageNonSyncSampleSize",
            track
                .average_non_sync_sample_size
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MinimumCompositionTimeOffset",
            track
                .minimum_composition_time_offset
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MaximumCompositionTimeOffset",
            track
                .maximum_composition_time_offset
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MinimumPresentationTime",
            track
                .minimum_presentation_time
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MaximumPresentationEndTime",
            track
                .maximum_presentation_end_time
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MinimumPreviousDecodeDelta",
            track
                .minimum_previous_decode_delta
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MaximumPreviousDecodeDelta",
            track
                .maximum_previous_decode_delta
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MinimumPreviousPresentationDelta",
            track
                .minimum_previous_presentation_delta
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MaximumPreviousPresentationDelta",
            track
                .maximum_previous_presentation_delta
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "PresentationGapCount",
            track.presentation_gap_count.to_string(),
        ),
        (
            "PresentationOverlapCount",
            track.presentation_overlap_count.to_string(),
        ),
        (
            "PresentationRegressionCount",
            track.presentation_regression_count.to_string(),
        ),
        (
            "DurationChangeCount",
            track.duration_change_count.to_string(),
        ),
        (
            "CompositionTimeOffsetChangeCount",
            track.composition_time_offset_change_count.to_string(),
        ),
        (
            "MinimumSyncSampleDistance",
            track
                .minimum_sync_sample_distance
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MaximumSyncSampleDistance",
            track
                .maximum_sync_sample_distance
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "AverageSyncSampleDistance",
            track
                .average_sync_sample_distance
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MinimumSyncSampleDecodeDelta",
            track
                .minimum_sync_sample_decode_delta
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "MaximumSyncSampleDecodeDelta",
            track
                .maximum_sync_sample_decode_delta
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "AverageSyncSampleDecodeDelta",
            track
                .average_sync_sample_decode_delta
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "FirstSyncSampleIndex",
            track
                .first_sync_sample_index
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "LastSyncSampleIndex",
            track
                .last_sync_sample_index
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "FirstSyncDecodeTime",
            track
                .first_sync_decode_time
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "LastSyncDecodeTime",
            track
                .last_sync_decode_time
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "FirstSyncPresentationTime",
            track
                .first_sync_presentation_time
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "LastSyncPresentationTime",
            track
                .last_sync_presentation_time
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        ("FirstDecodeTime", track.first_decode_time.to_string()),
        ("EndDecodeTime", track.end_decode_time.to_string()),
    ];
    if let Some(width) = track.width {
        fields.push(("Width", width.to_string()));
    }
    if let Some(height) = track.height {
        fields.push(("Height", height.to_string()));
    }
    if let Some(edit_media_time) = track.source_edit_media_time {
        fields.push(("SourceEditMediaTime", edit_media_time.to_string()));
    }
    if let Some(sample_roll_distance) = track.sample_roll_distance {
        fields.push(("SampleRollDistance", sample_roll_distance.to_string()));
    }
    for (index, (name, value)) in fields.iter().enumerate() {
        write_json_field(writer, 3, name, value, true)?;
        if index + 1 == fields.len() {
            break;
        }
    }
    writeln!(writer, "      \"Samples\": [")?;
    for (index, sample) in track.samples.iter().enumerate() {
        write_json_sample(writer, sample, index + 1 != track.samples.len())?;
    }
    writeln!(writer, "      ]")?;
    writeln!(writer, "    }}{}", if trailing_comma { "," } else { "" })
}

fn write_json_sample<W>(
    writer: &mut W,
    sample: &DirectIngestSampleReport,
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "        {{")?;
    let fields = [
        ("SourceIndex", sample.source_index.to_string()),
        ("DataOffset", sample.data_offset.to_string()),
        ("DataSize", sample.data_size.to_string()),
        ("DecodeTime", sample.decode_time.to_string()),
        (
            "PreviousDecodeDelta",
            sample
                .previous_decode_delta
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        (
            "CompositionTimeOffset",
            sample.composition_time_offset.to_string(),
        ),
        ("PresentationTime", sample.presentation_time.to_string()),
        (
            "PresentationEndTime",
            sample.presentation_end_time.to_string(),
        ),
        (
            "PreviousPresentationDelta",
            sample
                .previous_presentation_delta
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        ("Duration", sample.duration.to_string()),
        (
            "IsSyncSample",
            if sample.is_sync_sample {
                "true"
            } else {
                "false"
            }
            .to_string(),
        ),
    ];
    for (index, (name, value)) in fields.iter().enumerate() {
        write_json_field(writer, 5, name, value, index + 1 != fields.len())?;
    }
    writeln!(
        writer,
        "        }}{}",
        if trailing_comma { "," } else { "" }
    )
}

fn write_json_packet<W>(
    writer: &mut W,
    packet: &DirectIngestPacketEntry,
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "    {{")?;
    let fields = [
        ("TrackID", packet.track_id.to_string()),
        ("PacketIndex", packet.packet_index.to_string()),
        ("TrackKind", json_string(&packet.track_kind)),
        ("Timescale", packet.timescale.to_string()),
        ("SampleEntryType", json_string(&packet.sample_entry_type)),
        ("SourceIndex", packet.source_index.to_string()),
        ("DataOffset", packet.data_offset.to_string()),
        ("DataSize", packet.data_size.to_string()),
        ("DecodeTime", packet.decode_time.to_string()),
        (
            "CompositionTimeOffset",
            packet.composition_time_offset.to_string(),
        ),
        ("PresentationTime", packet.presentation_time.to_string()),
        (
            "PresentationEndTime",
            packet.presentation_end_time.to_string(),
        ),
        (
            "PreviousPresentationDelta",
            packet
                .previous_presentation_delta
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        ("Duration", packet.duration.to_string()),
        (
            "PreviousDecodeDelta",
            packet
                .previous_decode_delta
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
        ),
        ("PayloadCrc32", packet.payload_crc32.to_string()),
        (
            "IsSyncSample",
            if packet.is_sync_sample {
                "true"
            } else {
                "false"
            }
            .to_string(),
        ),
    ];
    for (index, (name, value)) in fields.iter().enumerate() {
        write_json_field(writer, 3, name, value, index + 1 != fields.len())?;
    }
    writeln!(writer, "    }}{}", if trailing_comma { "," } else { "" })
}

fn write_json_field<W>(
    writer: &mut W,
    indent_level: usize,
    name: &str,
    value: &str,
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    let indent = "  ".repeat(indent_level);
    writeln!(
        writer,
        "{indent}\"{name}\": {value}{}",
        if trailing_comma { "," } else { "" }
    )
}

fn write_yaml_report<W>(writer: &mut W, report: &DirectIngestReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(
        writer,
        "input_path: {}",
        yaml_string(&report.input_path.display().to_string())
    )?;
    writeln!(writer, "detected_kind:")?;
    writeln!(
        writer,
        "  kind: {}",
        yaml_string(detected_kind_name(&report.detected_kind))
    )?;
    match &report.detected_kind {
        DirectIngestDetectedKind::Container { container } => {
            writeln!(writer, "  container: {}", yaml_string(container))?;
        }
        DirectIngestDetectedKind::Raw { codec } => {
            writeln!(writer, "  codec: {}", yaml_string(codec))?;
        }
        DirectIngestDetectedKind::ImportOnly { family } => {
            writeln!(writer, "  family: {}", yaml_string(family))?;
        }
        DirectIngestDetectedKind::Mp4 | DirectIngestDetectedKind::Unknown => {}
    }
    writeln!(
        writer,
        "supports_flat_mux: {}",
        if report.supports_flat_mux {
            "true"
        } else {
            "false"
        }
    )?;
    if let Some(note) = &report.note {
        writeln!(writer, "note: {}", yaml_string(note))?;
    }
    writeln!(writer, "track_count: {}", report.track_count)?;
    writeln!(writer, "total_sample_count: {}", report.total_sample_count)?;
    writeln!(
        writer,
        "total_sync_sample_count: {}",
        report.total_sync_sample_count
    )?;
    writeln!(writer, "total_payload_size: {}", report.total_payload_size)?;
    writeln!(writer, "staged_sources:")?;
    for source in &report.staged_sources {
        writeln!(writer, "- source_index: {}", source.source_index)?;
        writeln!(
            writer,
            "  path: {}",
            yaml_string(&source.path.display().to_string())
        )?;
        writeln!(
            writer,
            "  segmented: {}",
            if source.segmented { "true" } else { "false" }
        )?;
        writeln!(writer, "  total_size: {}", source.total_size)?;
        if let Some(segment_count) = source.segment_count {
            writeln!(writer, "  segment_count: {}", segment_count)?;
        }
        if let Some(segments) = &source.segments
            && !segments.is_empty()
        {
            writeln!(writer, "  segments:")?;
            for segment in segments {
                writeln!(writer, "  - kind: {}", yaml_string(&segment.kind))?;
                writeln!(writer, "    logical_offset: {}", segment.logical_offset)?;
                writeln!(writer, "    logical_size: {}", segment.logical_size)?;
                match segment.source_offset {
                    Some(source_offset) => {
                        writeln!(writer, "    source_offset: {}", source_offset)?
                    }
                    None => writeln!(writer, "    source_offset: null")?,
                }
                match &segment.data_hex {
                    Some(data_hex) => writeln!(writer, "    data_hex: {}", yaml_string(data_hex))?,
                    None => writeln!(writer, "    data_hex: null")?,
                }
            }
        }
    }
    writeln!(writer, "tracks:")?;
    for track in &report.tracks {
        writeln!(writer, "- track_id: {}", track.track_id)?;
        writeln!(writer, "  kind: {}", yaml_string(&track.kind))?;
        writeln!(writer, "  timescale: {}", track.timescale)?;
        writeln!(writer, "  language: {}", yaml_string(&track.language))?;
        writeln!(
            writer,
            "  handler_name: {}",
            yaml_string(&track.handler_name)
        )?;
        writeln!(
            writer,
            "  sample_entry_type: {}",
            yaml_string(&track.sample_entry_type)
        )?;
        writeln!(
            writer,
            "  sample_entry_box_hex: {}",
            yaml_string(&track.sample_entry_box_hex)
        )?;
        if let Some(width) = track.width {
            writeln!(writer, "  width: {}", width)?;
        }
        if let Some(height) = track.height {
            writeln!(writer, "  height: {}", height)?;
        }
        if let Some(edit_media_time) = track.source_edit_media_time {
            writeln!(writer, "  source_edit_media_time: {}", edit_media_time)?;
        }
        if let Some(sample_roll_distance) = track.sample_roll_distance {
            writeln!(writer, "  sample_roll_distance: {}", sample_roll_distance)?;
        }
        writeln!(writer, "  sample_count: {}", track.sample_count)?;
        writeln!(writer, "  sync_sample_count: {}", track.sync_sample_count)?;
        writeln!(
            writer,
            "  starts_with_sync_sample: {}",
            if track.starts_with_sync_sample {
                "true"
            } else {
                "false"
            }
        )?;
        writeln!(writer, "  total_duration: {}", track.total_duration)?;
        writeln!(writer, "  total_payload_size: {}", track.total_payload_size)?;
        match track.average_sample_size {
            Some(average_sample_size) => {
                writeln!(writer, "  average_sample_size: {}", average_sample_size)?
            }
            None => writeln!(writer, "  average_sample_size: null")?,
        }
        match track.minimum_sample_size {
            Some(minimum_sample_size) => {
                writeln!(writer, "  minimum_sample_size: {}", minimum_sample_size)?
            }
            None => writeln!(writer, "  minimum_sample_size: null")?,
        }
        match track.maximum_sample_size {
            Some(maximum_sample_size) => {
                writeln!(writer, "  maximum_sample_size: {}", maximum_sample_size)?
            }
            None => writeln!(writer, "  maximum_sample_size: null")?,
        }
        match track.minimum_sample_duration {
            Some(minimum_sample_duration) => writeln!(
                writer,
                "  minimum_sample_duration: {}",
                minimum_sample_duration
            )?,
            None => writeln!(writer, "  minimum_sample_duration: null")?,
        }
        match track.maximum_sample_duration {
            Some(maximum_sample_duration) => writeln!(
                writer,
                "  maximum_sample_duration: {}",
                maximum_sample_duration
            )?,
            None => writeln!(writer, "  maximum_sample_duration: null")?,
        }
        match track.average_bitrate_bits_per_second {
            Some(average_bitrate_bits_per_second) => writeln!(
                writer,
                "  average_bitrate_bits_per_second: {}",
                average_bitrate_bits_per_second
            )?,
            None => writeln!(writer, "  average_bitrate_bits_per_second: null")?,
        }
        match track.minimum_sync_sample_size {
            Some(minimum_sync_sample_size) => writeln!(
                writer,
                "  minimum_sync_sample_size: {}",
                minimum_sync_sample_size
            )?,
            None => writeln!(writer, "  minimum_sync_sample_size: null")?,
        }
        match track.maximum_sync_sample_size {
            Some(maximum_sync_sample_size) => writeln!(
                writer,
                "  maximum_sync_sample_size: {}",
                maximum_sync_sample_size
            )?,
            None => writeln!(writer, "  maximum_sync_sample_size: null")?,
        }
        match track.average_sync_sample_size {
            Some(average_sync_sample_size) => writeln!(
                writer,
                "  average_sync_sample_size: {}",
                average_sync_sample_size
            )?,
            None => writeln!(writer, "  average_sync_sample_size: null")?,
        }
        match track.average_non_sync_sample_size {
            Some(average_non_sync_sample_size) => writeln!(
                writer,
                "  average_non_sync_sample_size: {}",
                average_non_sync_sample_size
            )?,
            None => writeln!(writer, "  average_non_sync_sample_size: null")?,
        }
        match track.minimum_composition_time_offset {
            Some(minimum_composition_time_offset) => writeln!(
                writer,
                "  minimum_composition_time_offset: {}",
                minimum_composition_time_offset
            )?,
            None => writeln!(writer, "  minimum_composition_time_offset: null")?,
        }
        match track.maximum_composition_time_offset {
            Some(maximum_composition_time_offset) => writeln!(
                writer,
                "  maximum_composition_time_offset: {}",
                maximum_composition_time_offset
            )?,
            None => writeln!(writer, "  maximum_composition_time_offset: null")?,
        }
        match track.minimum_presentation_time {
            Some(minimum_presentation_time) => writeln!(
                writer,
                "  minimum_presentation_time: {}",
                minimum_presentation_time
            )?,
            None => writeln!(writer, "  minimum_presentation_time: null")?,
        }
        match track.maximum_presentation_end_time {
            Some(maximum_presentation_end_time) => writeln!(
                writer,
                "  maximum_presentation_end_time: {}",
                maximum_presentation_end_time
            )?,
            None => writeln!(writer, "  maximum_presentation_end_time: null")?,
        }
        match track.minimum_previous_decode_delta {
            Some(minimum_previous_decode_delta) => writeln!(
                writer,
                "  minimum_previous_decode_delta: {}",
                minimum_previous_decode_delta
            )?,
            None => writeln!(writer, "  minimum_previous_decode_delta: null")?,
        }
        match track.maximum_previous_decode_delta {
            Some(maximum_previous_decode_delta) => writeln!(
                writer,
                "  maximum_previous_decode_delta: {}",
                maximum_previous_decode_delta
            )?,
            None => writeln!(writer, "  maximum_previous_decode_delta: null")?,
        }
        match track.minimum_previous_presentation_delta {
            Some(minimum_previous_presentation_delta) => writeln!(
                writer,
                "  minimum_previous_presentation_delta: {}",
                minimum_previous_presentation_delta
            )?,
            None => writeln!(writer, "  minimum_previous_presentation_delta: null")?,
        }
        match track.maximum_previous_presentation_delta {
            Some(maximum_previous_presentation_delta) => writeln!(
                writer,
                "  maximum_previous_presentation_delta: {}",
                maximum_previous_presentation_delta
            )?,
            None => writeln!(writer, "  maximum_previous_presentation_delta: null")?,
        }
        writeln!(
            writer,
            "  presentation_gap_count: {}",
            track.presentation_gap_count
        )?;
        writeln!(
            writer,
            "  presentation_overlap_count: {}",
            track.presentation_overlap_count
        )?;
        writeln!(
            writer,
            "  presentation_regression_count: {}",
            track.presentation_regression_count
        )?;
        writeln!(
            writer,
            "  duration_change_count: {}",
            track.duration_change_count
        )?;
        writeln!(
            writer,
            "  composition_time_offset_change_count: {}",
            track.composition_time_offset_change_count
        )?;
        match track.minimum_sync_sample_distance {
            Some(minimum_sync_sample_distance) => writeln!(
                writer,
                "  minimum_sync_sample_distance: {}",
                minimum_sync_sample_distance
            )?,
            None => writeln!(writer, "  minimum_sync_sample_distance: null")?,
        }
        match track.maximum_sync_sample_distance {
            Some(maximum_sync_sample_distance) => writeln!(
                writer,
                "  maximum_sync_sample_distance: {}",
                maximum_sync_sample_distance
            )?,
            None => writeln!(writer, "  maximum_sync_sample_distance: null")?,
        }
        match track.average_sync_sample_distance {
            Some(average_sync_sample_distance) => writeln!(
                writer,
                "  average_sync_sample_distance: {}",
                average_sync_sample_distance
            )?,
            None => writeln!(writer, "  average_sync_sample_distance: null")?,
        }
        match track.minimum_sync_sample_decode_delta {
            Some(minimum_sync_sample_decode_delta) => writeln!(
                writer,
                "  minimum_sync_sample_decode_delta: {}",
                minimum_sync_sample_decode_delta
            )?,
            None => writeln!(writer, "  minimum_sync_sample_decode_delta: null")?,
        }
        match track.maximum_sync_sample_decode_delta {
            Some(maximum_sync_sample_decode_delta) => writeln!(
                writer,
                "  maximum_sync_sample_decode_delta: {}",
                maximum_sync_sample_decode_delta
            )?,
            None => writeln!(writer, "  maximum_sync_sample_decode_delta: null")?,
        }
        match track.average_sync_sample_decode_delta {
            Some(average_sync_sample_decode_delta) => writeln!(
                writer,
                "  average_sync_sample_decode_delta: {}",
                average_sync_sample_decode_delta
            )?,
            None => writeln!(writer, "  average_sync_sample_decode_delta: null")?,
        }
        match track.first_sync_sample_index {
            Some(first_sync_sample_index) => writeln!(
                writer,
                "  first_sync_sample_index: {}",
                first_sync_sample_index
            )?,
            None => writeln!(writer, "  first_sync_sample_index: null")?,
        }
        match track.last_sync_sample_index {
            Some(last_sync_sample_index) => writeln!(
                writer,
                "  last_sync_sample_index: {}",
                last_sync_sample_index
            )?,
            None => writeln!(writer, "  last_sync_sample_index: null")?,
        }
        match track.first_sync_decode_time {
            Some(first_sync_decode_time) => writeln!(
                writer,
                "  first_sync_decode_time: {}",
                first_sync_decode_time
            )?,
            None => writeln!(writer, "  first_sync_decode_time: null")?,
        }
        match track.last_sync_decode_time {
            Some(last_sync_decode_time) => {
                writeln!(writer, "  last_sync_decode_time: {}", last_sync_decode_time)?
            }
            None => writeln!(writer, "  last_sync_decode_time: null")?,
        }
        match track.first_sync_presentation_time {
            Some(first_sync_presentation_time) => writeln!(
                writer,
                "  first_sync_presentation_time: {}",
                first_sync_presentation_time
            )?,
            None => writeln!(writer, "  first_sync_presentation_time: null")?,
        }
        match track.last_sync_presentation_time {
            Some(last_sync_presentation_time) => writeln!(
                writer,
                "  last_sync_presentation_time: {}",
                last_sync_presentation_time
            )?,
            None => writeln!(writer, "  last_sync_presentation_time: null")?,
        }
        writeln!(writer, "  first_decode_time: {}", track.first_decode_time)?;
        writeln!(writer, "  end_decode_time: {}", track.end_decode_time)?;
        writeln!(writer, "  samples:")?;
        for sample in &track.samples {
            writeln!(writer, "  - source_index: {}", sample.source_index)?;
            writeln!(writer, "    data_offset: {}", sample.data_offset)?;
            writeln!(writer, "    data_size: {}", sample.data_size)?;
            writeln!(writer, "    decode_time: {}", sample.decode_time)?;
            match sample.previous_decode_delta {
                Some(previous_decode_delta) => writeln!(
                    writer,
                    "    previous_decode_delta: {}",
                    previous_decode_delta
                )?,
                None => writeln!(writer, "    previous_decode_delta: null")?,
            }
            writeln!(
                writer,
                "    composition_time_offset: {}",
                sample.composition_time_offset
            )?;
            writeln!(
                writer,
                "    presentation_time: {}",
                sample.presentation_time
            )?;
            writeln!(
                writer,
                "    presentation_end_time: {}",
                sample.presentation_end_time
            )?;
            match sample.previous_presentation_delta {
                Some(previous_presentation_delta) => writeln!(
                    writer,
                    "    previous_presentation_delta: {}",
                    previous_presentation_delta
                )?,
                None => writeln!(writer, "    previous_presentation_delta: null")?,
            }
            writeln!(writer, "    duration: {}", sample.duration)?;
            writeln!(
                writer,
                "    is_sync_sample: {}",
                if sample.is_sync_sample {
                    "true"
                } else {
                    "false"
                }
            )?;
        }
    }
    Ok(())
}

fn write_yaml_packet_report<W>(writer: &mut W, report: &DirectIngestPacketReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(
        writer,
        "input_path: {}",
        yaml_string(&report.input_path.display().to_string())
    )?;
    writeln!(writer, "detected_kind:")?;
    writeln!(
        writer,
        "  kind: {}",
        yaml_string(detected_kind_name(&report.detected_kind))
    )?;
    match &report.detected_kind {
        DirectIngestDetectedKind::Container { container } => {
            writeln!(writer, "  container: {}", yaml_string(container))?;
        }
        DirectIngestDetectedKind::Raw { codec } => {
            writeln!(writer, "  codec: {}", yaml_string(codec))?;
        }
        DirectIngestDetectedKind::ImportOnly { family } => {
            writeln!(writer, "  family: {}", yaml_string(family))?;
        }
        DirectIngestDetectedKind::Mp4 | DirectIngestDetectedKind::Unknown => {}
    }
    writeln!(
        writer,
        "supports_flat_mux: {}",
        if report.supports_flat_mux {
            "true"
        } else {
            "false"
        }
    )?;
    if let Some(note) = &report.note {
        writeln!(writer, "note: {}", yaml_string(note))?;
    }
    writeln!(writer, "track_count: {}", report.track_count)?;
    writeln!(writer, "packet_count: {}", report.packet_count)?;
    writeln!(writer, "sync_packet_count: {}", report.sync_packet_count)?;
    writeln!(
        writer,
        "starts_with_sync_packet: {}",
        if report.starts_with_sync_packet {
            "true"
        } else {
            "false"
        }
    )?;
    writeln!(writer, "total_payload_size: {}", report.total_payload_size)?;
    if let Some(minimum_packet_size) = report.minimum_packet_size {
        writeln!(writer, "minimum_packet_size: {}", minimum_packet_size)?;
    }
    if let Some(maximum_packet_size) = report.maximum_packet_size {
        writeln!(writer, "maximum_packet_size: {}", maximum_packet_size)?;
    }
    if let Some(minimum_sync_packet_size) = report.minimum_sync_packet_size {
        writeln!(
            writer,
            "minimum_sync_packet_size: {}",
            minimum_sync_packet_size
        )?;
    }
    if let Some(maximum_sync_packet_size) = report.maximum_sync_packet_size {
        writeln!(
            writer,
            "maximum_sync_packet_size: {}",
            maximum_sync_packet_size
        )?;
    }
    if let Some(average_sync_packet_size) = report.average_sync_packet_size {
        writeln!(
            writer,
            "average_sync_packet_size: {}",
            average_sync_packet_size
        )?;
    }
    if let Some(average_non_sync_packet_size) = report.average_non_sync_packet_size {
        writeln!(
            writer,
            "average_non_sync_packet_size: {}",
            average_non_sync_packet_size
        )?;
    }
    if let Some(minimum_packet_duration) = report.minimum_packet_duration {
        writeln!(
            writer,
            "minimum_packet_duration: {}",
            minimum_packet_duration
        )?;
    }
    if let Some(maximum_packet_duration) = report.maximum_packet_duration {
        writeln!(
            writer,
            "maximum_packet_duration: {}",
            maximum_packet_duration
        )?;
    }
    if let Some(minimum_previous_decode_delta) = report.minimum_previous_decode_delta {
        writeln!(
            writer,
            "minimum_previous_decode_delta: {}",
            minimum_previous_decode_delta
        )?;
    }
    if let Some(maximum_previous_decode_delta) = report.maximum_previous_decode_delta {
        writeln!(
            writer,
            "maximum_previous_decode_delta: {}",
            maximum_previous_decode_delta
        )?;
    }
    if let Some(minimum_composition_time_offset) = report.minimum_composition_time_offset {
        writeln!(
            writer,
            "minimum_composition_time_offset: {}",
            minimum_composition_time_offset
        )?;
    }
    if let Some(maximum_composition_time_offset) = report.maximum_composition_time_offset {
        writeln!(
            writer,
            "maximum_composition_time_offset: {}",
            maximum_composition_time_offset
        )?;
    }
    if let Some(minimum_presentation_time) = report.minimum_presentation_time {
        writeln!(
            writer,
            "minimum_presentation_time: {}",
            minimum_presentation_time
        )?;
    }
    if let Some(maximum_presentation_end_time) = report.maximum_presentation_end_time {
        writeln!(
            writer,
            "maximum_presentation_end_time: {}",
            maximum_presentation_end_time
        )?;
    }
    if let Some(minimum_previous_presentation_delta) = report.minimum_previous_presentation_delta {
        writeln!(
            writer,
            "minimum_previous_presentation_delta: {}",
            minimum_previous_presentation_delta
        )?;
    }
    if let Some(maximum_previous_presentation_delta) = report.maximum_previous_presentation_delta {
        writeln!(
            writer,
            "maximum_previous_presentation_delta: {}",
            maximum_previous_presentation_delta
        )?;
    }
    writeln!(
        writer,
        "presentation_gap_count: {}",
        report.presentation_gap_count
    )?;
    writeln!(
        writer,
        "presentation_overlap_count: {}",
        report.presentation_overlap_count
    )?;
    writeln!(
        writer,
        "presentation_regression_count: {}",
        report.presentation_regression_count
    )?;
    writeln!(
        writer,
        "duration_change_count: {}",
        report.duration_change_count
    )?;
    writeln!(
        writer,
        "composition_time_offset_change_count: {}",
        report.composition_time_offset_change_count
    )?;
    if let Some(minimum_sync_packet_distance) = report.minimum_sync_packet_distance {
        writeln!(
            writer,
            "minimum_sync_packet_distance: {}",
            minimum_sync_packet_distance
        )?;
    }
    if let Some(maximum_sync_packet_distance) = report.maximum_sync_packet_distance {
        writeln!(
            writer,
            "maximum_sync_packet_distance: {}",
            maximum_sync_packet_distance
        )?;
    }
    if let Some(average_sync_packet_distance) = report.average_sync_packet_distance {
        writeln!(
            writer,
            "average_sync_packet_distance: {}",
            average_sync_packet_distance
        )?;
    }
    if let Some(minimum_sync_packet_decode_delta) = report.minimum_sync_packet_decode_delta {
        writeln!(
            writer,
            "minimum_sync_packet_decode_delta: {}",
            minimum_sync_packet_decode_delta
        )?;
    }
    if let Some(maximum_sync_packet_decode_delta) = report.maximum_sync_packet_decode_delta {
        writeln!(
            writer,
            "maximum_sync_packet_decode_delta: {}",
            maximum_sync_packet_decode_delta
        )?;
    }
    if let Some(average_sync_packet_decode_delta) = report.average_sync_packet_decode_delta {
        writeln!(
            writer,
            "average_sync_packet_decode_delta: {}",
            average_sync_packet_decode_delta
        )?;
    }
    match report.first_sync_packet_track_id {
        Some(first_sync_packet_track_id) => writeln!(
            writer,
            "first_sync_packet_track_id: {}",
            first_sync_packet_track_id
        )?,
        None => writeln!(writer, "first_sync_packet_track_id: null")?,
    }
    match report.first_sync_packet_index {
        Some(first_sync_packet_index) => writeln!(
            writer,
            "first_sync_packet_index: {}",
            first_sync_packet_index
        )?,
        None => writeln!(writer, "first_sync_packet_index: null")?,
    }
    match report.last_sync_packet_track_id {
        Some(last_sync_packet_track_id) => writeln!(
            writer,
            "last_sync_packet_track_id: {}",
            last_sync_packet_track_id
        )?,
        None => writeln!(writer, "last_sync_packet_track_id: null")?,
    }
    match report.last_sync_packet_index {
        Some(last_sync_packet_index) => {
            writeln!(writer, "last_sync_packet_index: {}", last_sync_packet_index)?
        }
        None => writeln!(writer, "last_sync_packet_index: null")?,
    }
    match report.first_sync_decode_time {
        Some(first_sync_decode_time) => {
            writeln!(writer, "first_sync_decode_time: {}", first_sync_decode_time)?
        }
        None => writeln!(writer, "first_sync_decode_time: null")?,
    }
    match report.last_sync_decode_time {
        Some(last_sync_decode_time) => {
            writeln!(writer, "last_sync_decode_time: {}", last_sync_decode_time)?
        }
        None => writeln!(writer, "last_sync_decode_time: null")?,
    }
    match report.first_sync_presentation_time {
        Some(first_sync_presentation_time) => writeln!(
            writer,
            "first_sync_presentation_time: {}",
            first_sync_presentation_time
        )?,
        None => writeln!(writer, "first_sync_presentation_time: null")?,
    }
    match report.last_sync_presentation_time {
        Some(last_sync_presentation_time) => writeln!(
            writer,
            "last_sync_presentation_time: {}",
            last_sync_presentation_time
        )?,
        None => writeln!(writer, "last_sync_presentation_time: null")?,
    }
    writeln!(writer, "staged_sources:")?;
    for source in &report.staged_sources {
        writeln!(writer, "- source_index: {}", source.source_index)?;
        writeln!(
            writer,
            "  path: {}",
            yaml_string(&source.path.display().to_string())
        )?;
        writeln!(
            writer,
            "  segmented: {}",
            if source.segmented { "true" } else { "false" }
        )?;
        writeln!(writer, "  total_size: {}", source.total_size)?;
        if let Some(segment_count) = source.segment_count {
            writeln!(writer, "  segment_count: {}", segment_count)?;
        }
        if let Some(segments) = &source.segments
            && !segments.is_empty()
        {
            writeln!(writer, "  segments:")?;
            for segment in segments {
                writeln!(writer, "  - kind: {}", yaml_string(&segment.kind))?;
                writeln!(writer, "    logical_offset: {}", segment.logical_offset)?;
                writeln!(writer, "    logical_size: {}", segment.logical_size)?;
                match segment.source_offset {
                    Some(source_offset) => {
                        writeln!(writer, "    source_offset: {}", source_offset)?
                    }
                    None => writeln!(writer, "    source_offset: null")?,
                }
                match &segment.data_hex {
                    Some(data_hex) => writeln!(writer, "    data_hex: {}", yaml_string(data_hex))?,
                    None => writeln!(writer, "    data_hex: null")?,
                }
            }
        }
    }
    writeln!(writer, "packets:")?;
    for packet in &report.packets {
        writeln!(writer, "- track_id: {}", packet.track_id)?;
        writeln!(writer, "  packet_index: {}", packet.packet_index)?;
        writeln!(writer, "  track_kind: {}", yaml_string(&packet.track_kind))?;
        writeln!(writer, "  timescale: {}", packet.timescale)?;
        writeln!(
            writer,
            "  sample_entry_type: {}",
            yaml_string(&packet.sample_entry_type)
        )?;
        writeln!(writer, "  source_index: {}", packet.source_index)?;
        writeln!(writer, "  data_offset: {}", packet.data_offset)?;
        writeln!(writer, "  data_size: {}", packet.data_size)?;
        writeln!(writer, "  decode_time: {}", packet.decode_time)?;
        writeln!(
            writer,
            "  composition_time_offset: {}",
            packet.composition_time_offset
        )?;
        writeln!(writer, "  presentation_time: {}", packet.presentation_time)?;
        writeln!(
            writer,
            "  presentation_end_time: {}",
            packet.presentation_end_time
        )?;
        match packet.previous_presentation_delta {
            Some(previous_presentation_delta) => writeln!(
                writer,
                "  previous_presentation_delta: {}",
                previous_presentation_delta
            )?,
            None => writeln!(writer, "  previous_presentation_delta: null")?,
        }
        writeln!(writer, "  duration: {}", packet.duration)?;
        match packet.previous_decode_delta {
            Some(previous_decode_delta) => {
                writeln!(writer, "  previous_decode_delta: {}", previous_decode_delta)?;
            }
            None => {
                writeln!(writer, "  previous_decode_delta: null")?;
            }
        }
        writeln!(writer, "  payload_crc32: {}", packet.payload_crc32)?;
        writeln!(
            writer,
            "  is_sync_sample: {}",
            if packet.is_sync_sample {
                "true"
            } else {
                "false"
            }
        )?;
    }
    Ok(())
}

fn write_nhml_report<W>(writer: &mut W, report: &DirectIngestReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
    write!(
        writer,
        "<nhml inputPath=\"{}\" detectedKind=\"{}\" supportsFlatMux=\"{}\" trackCount=\"{}\" totalSampleCount=\"{}\" totalSyncSampleCount=\"{}\" totalPayloadSize=\"{}\"",
        xml_escape_attr(&report.input_path.display().to_string()),
        xml_escape_attr(detected_kind_name(&report.detected_kind)),
        if report.supports_flat_mux {
            "true"
        } else {
            "false"
        },
        report.track_count,
        report.total_sample_count,
        report.total_sync_sample_count,
        report.total_payload_size
    )?;
    match &report.detected_kind {
        DirectIngestDetectedKind::Container { container } => {
            write!(writer, " container=\"{}\"", xml_escape_attr(container))?;
        }
        DirectIngestDetectedKind::Raw { codec } => {
            write!(writer, " codec=\"{}\"", xml_escape_attr(codec))?;
        }
        DirectIngestDetectedKind::ImportOnly { family } => {
            write!(writer, " family=\"{}\"", xml_escape_attr(family))?;
        }
        DirectIngestDetectedKind::Mp4 | DirectIngestDetectedKind::Unknown => {}
    }
    if let Some(note) = &report.note {
        write!(writer, " note=\"{}\"", xml_escape_attr(note))?;
    }
    writeln!(writer, ">")?;
    for source in &report.staged_sources {
        write!(
            writer,
            "  <source index=\"{}\" path=\"{}\" segmented=\"{}\" totalSize=\"{}\"",
            source.source_index,
            xml_escape_attr(&source.path.display().to_string()),
            if source.segmented { "true" } else { "false" },
            source.total_size
        )?;
        if let Some(segment_count) = source.segment_count {
            write!(writer, " segmentCount=\"{}\"", segment_count)?;
        }
        if let Some(segments) = &source.segments
            && !segments.is_empty()
        {
            writeln!(writer, ">")?;
            for segment in segments {
                write!(
                    writer,
                    "    <segment kind=\"{}\" logicalOffset=\"{}\" logicalSize=\"{}\"",
                    xml_escape_attr(&segment.kind),
                    segment.logical_offset,
                    segment.logical_size
                )?;
                if let Some(source_offset) = segment.source_offset {
                    write!(writer, " sourceOffset=\"{}\"", source_offset)?;
                }
                if let Some(source_path) = &segment.source_path {
                    write!(
                        writer,
                        " sourcePath=\"{}\"",
                        xml_escape_attr(&source_path.display().to_string())
                    )?;
                }
                if let Some(data_hex) = &segment.data_hex {
                    write!(writer, " dataHex=\"{}\"", xml_escape_attr(data_hex))?;
                }
                writeln!(writer, " />")?;
            }
            writeln!(writer, "  </source>")?;
        } else {
            writeln!(writer, " />")?;
        }
    }
    for track in &report.tracks {
        write!(
            writer,
            "  <track trackID=\"{}\" kind=\"{}\" timescale=\"{}\" language=\"{}\" handlerName=\"{}\" sampleEntryType=\"{}\" sampleEntryBoxHex=\"{}\" sampleCount=\"{}\" syncSampleCount=\"{}\" startsWithSyncSample=\"{}\" totalDuration=\"{}\" totalPayloadSize=\"{}\" firstDecodeTime=\"{}\" endDecodeTime=\"{}\"",
            track.track_id,
            xml_escape_attr(&track.kind),
            track.timescale,
            xml_escape_attr(&track.language),
            xml_escape_attr(&track.handler_name),
            xml_escape_attr(&track.sample_entry_type),
            xml_escape_attr(&track.sample_entry_box_hex),
            track.sample_count,
            track.sync_sample_count,
            if track.starts_with_sync_sample {
                "true"
            } else {
                "false"
            },
            track.total_duration,
            track.total_payload_size,
            track.first_decode_time,
            track.end_decode_time
        )?;
        if let Some(width) = track.width {
            write!(writer, " width=\"{}\"", width)?;
        }
        if let Some(height) = track.height {
            write!(writer, " height=\"{}\"", height)?;
        }
        if let Some(edit_media_time) = track.source_edit_media_time {
            write!(writer, " sourceEditMediaTime=\"{}\"", edit_media_time)?;
        }
        if let Some(sample_roll_distance) = track.sample_roll_distance {
            write!(writer, " sampleRollDistance=\"{}\"", sample_roll_distance)?;
        }
        if let Some(minimum_sample_size) = track.minimum_sample_size {
            write!(writer, " minimumSampleSize=\"{}\"", minimum_sample_size)?;
        }
        if let Some(maximum_sample_size) = track.maximum_sample_size {
            write!(writer, " maximumSampleSize=\"{}\"", maximum_sample_size)?;
        }
        if let Some(minimum_sample_duration) = track.minimum_sample_duration {
            write!(
                writer,
                " minimumSampleDuration=\"{}\"",
                minimum_sample_duration
            )?;
        }
        if let Some(maximum_sample_duration) = track.maximum_sample_duration {
            write!(
                writer,
                " maximumSampleDuration=\"{}\"",
                maximum_sample_duration
            )?;
        }
        if let Some(average_bitrate_bits_per_second) = track.average_bitrate_bits_per_second {
            write!(
                writer,
                " averageBitrateBitsPerSecond=\"{}\"",
                average_bitrate_bits_per_second
            )?;
        }
        if let Some(average_sample_size) = track.average_sample_size {
            write!(writer, " averageSampleSize=\"{}\"", average_sample_size)?;
        }
        if let Some(minimum_sync_sample_size) = track.minimum_sync_sample_size {
            write!(
                writer,
                " minimumSyncSampleSize=\"{}\"",
                minimum_sync_sample_size
            )?;
        }
        if let Some(maximum_sync_sample_size) = track.maximum_sync_sample_size {
            write!(
                writer,
                " maximumSyncSampleSize=\"{}\"",
                maximum_sync_sample_size
            )?;
        }
        if let Some(average_sync_sample_size) = track.average_sync_sample_size {
            write!(
                writer,
                " averageSyncSampleSize=\"{}\"",
                average_sync_sample_size
            )?;
        }
        if let Some(average_non_sync_sample_size) = track.average_non_sync_sample_size {
            write!(
                writer,
                " averageNonSyncSampleSize=\"{}\"",
                average_non_sync_sample_size
            )?;
        }
        if let Some(minimum_composition_time_offset) = track.minimum_composition_time_offset {
            write!(
                writer,
                " minimumCompositionTimeOffset=\"{}\"",
                minimum_composition_time_offset
            )?;
        }
        if let Some(maximum_composition_time_offset) = track.maximum_composition_time_offset {
            write!(
                writer,
                " maximumCompositionTimeOffset=\"{}\"",
                maximum_composition_time_offset
            )?;
        }
        if let Some(minimum_presentation_time) = track.minimum_presentation_time {
            write!(
                writer,
                " minimumPresentationTime=\"{}\"",
                minimum_presentation_time
            )?;
        }
        if let Some(maximum_presentation_end_time) = track.maximum_presentation_end_time {
            write!(
                writer,
                " maximumPresentationEndTime=\"{}\"",
                maximum_presentation_end_time
            )?;
        }
        if let Some(minimum_previous_decode_delta) = track.minimum_previous_decode_delta {
            write!(
                writer,
                " minimumPreviousDecodeDelta=\"{}\"",
                minimum_previous_decode_delta
            )?;
        }
        if let Some(maximum_previous_decode_delta) = track.maximum_previous_decode_delta {
            write!(
                writer,
                " maximumPreviousDecodeDelta=\"{}\"",
                maximum_previous_decode_delta
            )?;
        }
        if let Some(minimum_previous_presentation_delta) = track.minimum_previous_presentation_delta
        {
            write!(
                writer,
                " minimumPreviousPresentationDelta=\"{}\"",
                minimum_previous_presentation_delta
            )?;
        }
        if let Some(maximum_previous_presentation_delta) = track.maximum_previous_presentation_delta
        {
            write!(
                writer,
                " maximumPreviousPresentationDelta=\"{}\"",
                maximum_previous_presentation_delta
            )?;
        }
        write!(
            writer,
            " presentationGapCount=\"{}\" presentationOverlapCount=\"{}\" presentationRegressionCount=\"{}\" durationChangeCount=\"{}\" compositionTimeOffsetChangeCount=\"{}\"",
            track.presentation_gap_count,
            track.presentation_overlap_count,
            track.presentation_regression_count,
            track.duration_change_count,
            track.composition_time_offset_change_count
        )?;
        if let Some(minimum_sync_sample_distance) = track.minimum_sync_sample_distance {
            write!(
                writer,
                " minimumSyncSampleDistance=\"{}\"",
                minimum_sync_sample_distance
            )?;
        }
        if let Some(maximum_sync_sample_distance) = track.maximum_sync_sample_distance {
            write!(
                writer,
                " maximumSyncSampleDistance=\"{}\"",
                maximum_sync_sample_distance
            )?;
        }
        if let Some(average_sync_sample_distance) = track.average_sync_sample_distance {
            write!(
                writer,
                " averageSyncSampleDistance=\"{}\"",
                average_sync_sample_distance
            )?;
        }
        if let Some(minimum_sync_sample_decode_delta) = track.minimum_sync_sample_decode_delta {
            write!(
                writer,
                " minimumSyncSampleDecodeDelta=\"{}\"",
                minimum_sync_sample_decode_delta
            )?;
        }
        if let Some(maximum_sync_sample_decode_delta) = track.maximum_sync_sample_decode_delta {
            write!(
                writer,
                " maximumSyncSampleDecodeDelta=\"{}\"",
                maximum_sync_sample_decode_delta
            )?;
        }
        if let Some(average_sync_sample_decode_delta) = track.average_sync_sample_decode_delta {
            write!(
                writer,
                " averageSyncSampleDecodeDelta=\"{}\"",
                average_sync_sample_decode_delta
            )?;
        }
        if let Some(first_sync_sample_index) = track.first_sync_sample_index {
            write!(
                writer,
                " firstSyncSampleIndex=\"{}\"",
                first_sync_sample_index
            )?;
        }
        if let Some(last_sync_sample_index) = track.last_sync_sample_index {
            write!(
                writer,
                " lastSyncSampleIndex=\"{}\"",
                last_sync_sample_index
            )?;
        }
        if let Some(first_sync_decode_time) = track.first_sync_decode_time {
            write!(
                writer,
                " firstSyncDecodeTime=\"{}\"",
                first_sync_decode_time
            )?;
        }
        if let Some(last_sync_decode_time) = track.last_sync_decode_time {
            write!(writer, " lastSyncDecodeTime=\"{}\"", last_sync_decode_time)?;
        }
        if let Some(first_sync_presentation_time) = track.first_sync_presentation_time {
            write!(
                writer,
                " firstSyncPresentationTime=\"{}\"",
                first_sync_presentation_time
            )?;
        }
        if let Some(last_sync_presentation_time) = track.last_sync_presentation_time {
            write!(
                writer,
                " lastSyncPresentationTime=\"{}\"",
                last_sync_presentation_time
            )?;
        }
        writeln!(writer, ">")?;
        for sample in &track.samples {
            writeln!(
                writer,
                "    <sample sourceIndex=\"{}\" dataOffset=\"{}\" dataSize=\"{}\" decodeTime=\"{}\"{} compositionTimeOffset=\"{}\" presentationTime=\"{}\" presentationEndTime=\"{}\"{} duration=\"{}\" sync=\"{}\" />",
                sample.source_index,
                sample.data_offset,
                sample.data_size,
                sample.decode_time,
                sample
                    .previous_decode_delta
                    .map(|value| format!(" previousDecodeDelta=\"{value}\""))
                    .unwrap_or_default(),
                sample.composition_time_offset,
                sample.presentation_time,
                sample.presentation_end_time,
                sample
                    .previous_presentation_delta
                    .map(|value| format!(" previousPresentationDelta=\"{value}\""))
                    .unwrap_or_default(),
                sample.duration,
                if sample.is_sync_sample {
                    "true"
                } else {
                    "false"
                }
            )?;
        }
        writeln!(writer, "  </track>")?;
    }
    writeln!(writer, "</nhml>")
}

fn write_nhnt_report<W>(writer: &mut W, report: &DirectIngestPacketReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
    write!(
        writer,
        "<nhnt inputPath=\"{}\" detectedKind=\"{}\" supportsFlatMux=\"{}\" trackCount=\"{}\" packetCount=\"{}\" syncPacketCount=\"{}\" totalPayloadSize=\"{}\"",
        xml_escape_attr(&report.input_path.display().to_string()),
        xml_escape_attr(detected_kind_name(&report.detected_kind)),
        if report.supports_flat_mux {
            "true"
        } else {
            "false"
        },
        report.track_count,
        report.packet_count,
        report.sync_packet_count,
        report.total_payload_size
    )?;
    write!(
        writer,
        " startsWithSyncPacket=\"{}\"",
        if report.starts_with_sync_packet {
            "true"
        } else {
            "false"
        }
    )?;
    match &report.detected_kind {
        DirectIngestDetectedKind::Container { container } => {
            write!(writer, " container=\"{}\"", xml_escape_attr(container))?;
        }
        DirectIngestDetectedKind::Raw { codec } => {
            write!(writer, " codec=\"{}\"", xml_escape_attr(codec))?;
        }
        DirectIngestDetectedKind::ImportOnly { family } => {
            write!(writer, " family=\"{}\"", xml_escape_attr(family))?;
        }
        DirectIngestDetectedKind::Mp4 | DirectIngestDetectedKind::Unknown => {}
    }
    if let Some(note) = &report.note {
        write!(writer, " note=\"{}\"", xml_escape_attr(note))?;
    }
    if let Some(minimum_packet_size) = report.minimum_packet_size {
        write!(writer, " minimumPacketSize=\"{}\"", minimum_packet_size)?;
    }
    if let Some(maximum_packet_size) = report.maximum_packet_size {
        write!(writer, " maximumPacketSize=\"{}\"", maximum_packet_size)?;
    }
    if let Some(minimum_sync_packet_size) = report.minimum_sync_packet_size {
        write!(
            writer,
            " minimumSyncPacketSize=\"{}\"",
            minimum_sync_packet_size
        )?;
    }
    if let Some(maximum_sync_packet_size) = report.maximum_sync_packet_size {
        write!(
            writer,
            " maximumSyncPacketSize=\"{}\"",
            maximum_sync_packet_size
        )?;
    }
    if let Some(average_sync_packet_size) = report.average_sync_packet_size {
        write!(
            writer,
            " averageSyncPacketSize=\"{}\"",
            average_sync_packet_size
        )?;
    }
    if let Some(average_non_sync_packet_size) = report.average_non_sync_packet_size {
        write!(
            writer,
            " averageNonSyncPacketSize=\"{}\"",
            average_non_sync_packet_size
        )?;
    }
    if let Some(minimum_packet_duration) = report.minimum_packet_duration {
        write!(
            writer,
            " minimumPacketDuration=\"{}\"",
            minimum_packet_duration
        )?;
    }
    if let Some(maximum_packet_duration) = report.maximum_packet_duration {
        write!(
            writer,
            " maximumPacketDuration=\"{}\"",
            maximum_packet_duration
        )?;
    }
    if let Some(minimum_previous_decode_delta) = report.minimum_previous_decode_delta {
        write!(
            writer,
            " minimumPreviousDecodeDelta=\"{}\"",
            minimum_previous_decode_delta
        )?;
    }
    if let Some(maximum_previous_decode_delta) = report.maximum_previous_decode_delta {
        write!(
            writer,
            " maximumPreviousDecodeDelta=\"{}\"",
            maximum_previous_decode_delta
        )?;
    }
    if let Some(minimum_composition_time_offset) = report.minimum_composition_time_offset {
        write!(
            writer,
            " minimumCompositionTimeOffset=\"{}\"",
            minimum_composition_time_offset
        )?;
    }
    if let Some(maximum_composition_time_offset) = report.maximum_composition_time_offset {
        write!(
            writer,
            " maximumCompositionTimeOffset=\"{}\"",
            maximum_composition_time_offset
        )?;
    }
    if let Some(minimum_presentation_time) = report.minimum_presentation_time {
        write!(
            writer,
            " minimumPresentationTime=\"{}\"",
            minimum_presentation_time
        )?;
    }
    if let Some(maximum_presentation_end_time) = report.maximum_presentation_end_time {
        write!(
            writer,
            " maximumPresentationEndTime=\"{}\"",
            maximum_presentation_end_time
        )?;
    }
    if let Some(minimum_previous_presentation_delta) = report.minimum_previous_presentation_delta {
        write!(
            writer,
            " minimumPreviousPresentationDelta=\"{}\"",
            minimum_previous_presentation_delta
        )?;
    }
    if let Some(maximum_previous_presentation_delta) = report.maximum_previous_presentation_delta {
        write!(
            writer,
            " maximumPreviousPresentationDelta=\"{}\"",
            maximum_previous_presentation_delta
        )?;
    }
    write!(
        writer,
        " presentationGapCount=\"{}\" presentationOverlapCount=\"{}\" presentationRegressionCount=\"{}\" durationChangeCount=\"{}\" compositionTimeOffsetChangeCount=\"{}\"",
        report.presentation_gap_count,
        report.presentation_overlap_count,
        report.presentation_regression_count,
        report.duration_change_count,
        report.composition_time_offset_change_count
    )?;
    if let Some(minimum_sync_packet_distance) = report.minimum_sync_packet_distance {
        write!(
            writer,
            " minimumSyncPacketDistance=\"{}\"",
            minimum_sync_packet_distance
        )?;
    }
    if let Some(maximum_sync_packet_distance) = report.maximum_sync_packet_distance {
        write!(
            writer,
            " maximumSyncPacketDistance=\"{}\"",
            maximum_sync_packet_distance
        )?;
    }
    if let Some(average_sync_packet_distance) = report.average_sync_packet_distance {
        write!(
            writer,
            " averageSyncPacketDistance=\"{}\"",
            average_sync_packet_distance
        )?;
    }
    if let Some(minimum_sync_packet_decode_delta) = report.minimum_sync_packet_decode_delta {
        write!(
            writer,
            " minimumSyncPacketDecodeDelta=\"{}\"",
            minimum_sync_packet_decode_delta
        )?;
    }
    if let Some(maximum_sync_packet_decode_delta) = report.maximum_sync_packet_decode_delta {
        write!(
            writer,
            " maximumSyncPacketDecodeDelta=\"{}\"",
            maximum_sync_packet_decode_delta
        )?;
    }
    if let Some(average_sync_packet_decode_delta) = report.average_sync_packet_decode_delta {
        write!(
            writer,
            " averageSyncPacketDecodeDelta=\"{}\"",
            average_sync_packet_decode_delta
        )?;
    }
    if let Some(first_sync_packet_track_id) = report.first_sync_packet_track_id {
        write!(
            writer,
            " firstSyncPacketTrackID=\"{}\"",
            first_sync_packet_track_id
        )?;
    }
    if let Some(first_sync_packet_index) = report.first_sync_packet_index {
        write!(
            writer,
            " firstSyncPacketIndex=\"{}\"",
            first_sync_packet_index
        )?;
    }
    if let Some(last_sync_packet_track_id) = report.last_sync_packet_track_id {
        write!(
            writer,
            " lastSyncPacketTrackID=\"{}\"",
            last_sync_packet_track_id
        )?;
    }
    if let Some(last_sync_packet_index) = report.last_sync_packet_index {
        write!(
            writer,
            " lastSyncPacketIndex=\"{}\"",
            last_sync_packet_index
        )?;
    }
    if let Some(first_sync_decode_time) = report.first_sync_decode_time {
        write!(
            writer,
            " firstSyncDecodeTime=\"{}\"",
            first_sync_decode_time
        )?;
    }
    if let Some(last_sync_decode_time) = report.last_sync_decode_time {
        write!(writer, " lastSyncDecodeTime=\"{}\"", last_sync_decode_time)?;
    }
    if let Some(first_sync_presentation_time) = report.first_sync_presentation_time {
        write!(
            writer,
            " firstSyncPresentationTime=\"{}\"",
            first_sync_presentation_time
        )?;
    }
    if let Some(last_sync_presentation_time) = report.last_sync_presentation_time {
        write!(
            writer,
            " lastSyncPresentationTime=\"{}\"",
            last_sync_presentation_time
        )?;
    }
    writeln!(writer, ">")?;
    for source in &report.staged_sources {
        write!(
            writer,
            "  <source index=\"{}\" path=\"{}\" segmented=\"{}\" totalSize=\"{}\"",
            source.source_index,
            xml_escape_attr(&source.path.display().to_string()),
            if source.segmented { "true" } else { "false" },
            source.total_size
        )?;
        if let Some(segment_count) = source.segment_count {
            write!(writer, " segmentCount=\"{}\"", segment_count)?;
        }
        if let Some(segments) = &source.segments
            && !segments.is_empty()
        {
            writeln!(writer, ">")?;
            for segment in segments {
                write!(
                    writer,
                    "    <segment kind=\"{}\" logicalOffset=\"{}\" logicalSize=\"{}\"",
                    xml_escape_attr(&segment.kind),
                    segment.logical_offset,
                    segment.logical_size
                )?;
                if let Some(source_offset) = segment.source_offset {
                    write!(writer, " sourceOffset=\"{}\"", source_offset)?;
                }
                if let Some(source_path) = &segment.source_path {
                    write!(
                        writer,
                        " sourcePath=\"{}\"",
                        xml_escape_attr(&source_path.display().to_string())
                    )?;
                }
                if let Some(data_hex) = &segment.data_hex {
                    write!(writer, " dataHex=\"{}\"", xml_escape_attr(data_hex))?;
                }
                writeln!(writer, " />")?;
            }
            writeln!(writer, "  </source>")?;
        } else {
            writeln!(writer, " />")?;
        }
    }
    for track in &report.tracks {
        write!(
            writer,
            "  <track trackID=\"{}\" kind=\"{}\" timescale=\"{}\" language=\"{}\" handlerName=\"{}\" sampleEntryType=\"{}\" sampleEntryBoxHex=\"{}\"",
            track.track_id,
            xml_escape_attr(&track.kind),
            track.timescale,
            xml_escape_attr(&track.language),
            xml_escape_attr(&track.handler_name),
            xml_escape_attr(&track.sample_entry_type),
            xml_escape_attr(&track.sample_entry_box_hex)
        )?;
        if let Some(width) = track.width {
            write!(writer, " width=\"{}\"", width)?;
        }
        if let Some(height) = track.height {
            write!(writer, " height=\"{}\"", height)?;
        }
        if let Some(edit_media_time) = track.source_edit_media_time {
            write!(writer, " sourceEditMediaTime=\"{}\"", edit_media_time)?;
        }
        if let Some(sample_roll_distance) = track.sample_roll_distance {
            write!(writer, " sampleRollDistance=\"{}\"", sample_roll_distance)?;
        }
        writeln!(
            writer,
            " sampleCount=\"{}\" syncSampleCount=\"{}\" totalDuration=\"{}\" totalPayloadSize=\"{}\" />",
            track.sample_count,
            track.sync_sample_count,
            track.total_duration,
            track.total_payload_size
        )?;
    }
    for packet in &report.packets {
        writeln!(
            writer,
            "  <packet trackID=\"{}\" packetIndex=\"{}\" trackKind=\"{}\" timescale=\"{}\" sampleEntryType=\"{}\" sourceIndex=\"{}\" dataOffset=\"{}\" dataSize=\"{}\" decodeTime=\"{}\" compositionTimeOffset=\"{}\" presentationTime=\"{}\" presentationEndTime=\"{}\"{} duration=\"{}\"{} payloadCrc32=\"{}\" sync=\"{}\" />",
            packet.track_id,
            packet.packet_index,
            xml_escape_attr(&packet.track_kind),
            packet.timescale,
            xml_escape_attr(&packet.sample_entry_type),
            packet.source_index,
            packet.data_offset,
            packet.data_size,
            packet.decode_time,
            packet.composition_time_offset,
            packet.presentation_time,
            packet.presentation_end_time,
            packet
                .previous_presentation_delta
                .map(|value| format!(" previousPresentationDelta=\"{value}\""))
                .unwrap_or_default(),
            packet.duration,
            packet
                .previous_decode_delta
                .map(|value| format!(" previousDecodeDelta=\"{value}\""))
                .unwrap_or_default(),
            packet.payload_crc32,
            if packet.is_sync_sample {
                "true"
            } else {
                "false"
            }
        )?;
    }
    writeln!(writer, "</nhnt>")
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(escaped, "\\u{:04X}", ch as u32);
            }
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn yaml_string(value: &str) -> String {
    if value.is_empty()
        || value.starts_with(|ch: char| {
            ch.is_whitespace() || matches!(ch, '-' | '?' | ':' | '[' | ']' | '{' | '}' | ',')
        })
        || value.ends_with(char::is_whitespace)
        || value.contains(|ch: char| {
            ch.is_control() || matches!(ch, ':' | '#' | '"' | '\'' | '\n' | '\r' | '\t')
        })
    {
        let mut escaped = String::with_capacity(value.len() + 2);
        escaped.push('"');
        for ch in value.chars() {
            match ch {
                '"' => escaped.push_str("\\\""),
                '\\' => escaped.push_str("\\\\"),
                '\n' => escaped.push_str("\\n"),
                '\r' => escaped.push_str("\\r"),
                '\t' => escaped.push_str("\\t"),
                ch => escaped.push(ch),
            }
        }
        escaped.push('"');
        escaped
    } else {
        value.to_string()
    }
}

fn xml_escape_attr(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            ch if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') => {}
            ch => escaped.push(ch),
        }
    }
    escaped
}
