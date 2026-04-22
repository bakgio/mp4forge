//! Probe command support and stable report rendering.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, Write};

use crate::probe::{
    DetailedTrackInfo, ProbeError, ProbeOptions, TrackCodec, TrackCodecDetails, TrackCodecFamily,
    TrackMediaCharacteristics, average_sample_bitrate, average_segment_bitrate, find_idr_frames,
    max_sample_bitrate, max_segment_bitrate, probe_codec_detailed_with_options,
    probe_detailed_with_options, probe_media_characteristics_with_options, probe_with_options,
};

/// Structured output format supported by the probe command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeFormat {
    /// Pretty-printed JSON output.
    Json,
    /// Simple YAML output with stable field order.
    Yaml,
}

impl ProbeFormat {
    fn parse(value: &str) -> Result<Self, ProbeCliError> {
        match value {
            "json" => Ok(Self::Json),
            "yaml" => Ok(Self::Yaml),
            other => Err(ProbeCliError::InvalidArgument(format!(
                "unsupported probe format: {other}"
            ))),
        }
    }
}

/// Additive controls for expensive probe-report rendering work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeReportOptions {
    /// Library-side probe expansion controls.
    pub probe: ProbeOptions,
    /// Whether to aggregate bitrate summaries for each track.
    pub include_bitrate: bool,
    /// Whether to scan AVC samples for IDR frame counts.
    pub include_idr_frame_count: bool,
}

impl ProbeReportOptions {
    /// Returns the existing eager probe-report behavior.
    pub const fn full() -> Self {
        Self {
            probe: ProbeOptions::full(),
            include_bitrate: true,
            include_idr_frame_count: true,
        }
    }

    /// Returns a lighter-weight probe-report behavior for large-file inspection.
    pub const fn lightweight() -> Self {
        Self {
            probe: ProbeOptions::lightweight(),
            include_bitrate: false,
            include_idr_frame_count: false,
        }
    }
}

impl Default for ProbeReportOptions {
    fn default() -> Self {
        Self::full()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProbeDetailLevel {
    Full,
    Light,
}

impl ProbeDetailLevel {
    fn parse(value: &str) -> Result<Self, ProbeCliError> {
        match value {
            "full" => Ok(Self::Full),
            "light" => Ok(Self::Light),
            other => Err(ProbeCliError::InvalidArgument(format!(
                "unsupported probe detail level: {other}"
            ))),
        }
    }

    const fn report_options(self) -> ProbeReportOptions {
        match self {
            Self::Full => ProbeReportOptions::full(),
            Self::Light => ProbeReportOptions::lightweight(),
        }
    }
}

/// Top-level probe report used by the CLI layer.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProbeReport {
    /// Root `ftyp` major brand.
    pub major_brand: String,
    /// Root `ftyp` minor version.
    pub minor_version: u32,
    /// Root `ftyp` compatible brands.
    pub compatible_brands: Vec<String>,
    /// Whether the file places `moov` before the first `mdat`.
    pub fast_start: bool,
    /// Movie timescale from `mvhd`.
    pub timescale: u32,
    /// Movie duration from `mvhd`.
    pub duration: u64,
    /// Movie duration expressed in seconds.
    pub duration_seconds: f32,
    /// Per-track probe summaries.
    pub tracks: Vec<ProbeTrackReport>,
}

/// One track entry in the CLI probe report.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProbeTrackReport {
    /// Track identifier from `tkhd`.
    pub track_id: u32,
    /// Track timescale from `mdhd`.
    pub timescale: u32,
    /// Track duration from `mdhd`.
    pub duration: u64,
    /// Track duration expressed in seconds.
    pub duration_seconds: f32,
    /// Human-readable codec identifier.
    pub codec: String,
    /// Whether the track uses an encrypted sample entry.
    pub encrypted: bool,
    /// Display width for video tracks.
    pub width: Option<u16>,
    /// Display height for video tracks.
    pub height: Option<u16>,
    /// Expanded sample count when present.
    pub sample_num: Option<usize>,
    /// Expanded chunk count when present.
    pub chunk_num: Option<usize>,
    /// Count of samples carrying IDR NAL units.
    pub idr_frame_num: Option<usize>,
    /// Average bitrate in bits per second.
    pub bitrate: Option<u64>,
    /// Maximum bitrate in bits per second.
    pub max_bitrate: Option<u64>,
}

/// Top-level detailed probe report used by the CLI command surface.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DetailedProbeReport {
    /// Root `ftyp` major brand.
    pub major_brand: String,
    /// Root `ftyp` minor version.
    pub minor_version: u32,
    /// Root `ftyp` compatible brands.
    pub compatible_brands: Vec<String>,
    /// Whether the file places `moov` before the first `mdat`.
    pub fast_start: bool,
    /// Movie timescale from `mvhd`.
    pub timescale: u32,
    /// Movie duration from `mvhd`.
    pub duration: u64,
    /// Movie duration expressed in seconds.
    pub duration_seconds: f32,
    /// Per-track detailed probe summaries.
    pub tracks: Vec<DetailedProbeTrackReport>,
}

/// One track entry in the detailed CLI probe report.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DetailedProbeTrackReport {
    /// Track identifier from `tkhd`.
    pub track_id: u32,
    /// Track timescale from `mdhd`.
    pub timescale: u32,
    /// Track duration from `mdhd`.
    pub duration: u64,
    /// Track duration expressed in seconds.
    pub duration_seconds: f32,
    /// Human-readable codec identifier.
    pub codec: String,
    /// Normalized codec-family label.
    pub codec_family: String,
    /// Whether the track uses an encrypted sample entry.
    pub encrypted: bool,
    /// Handler type when present.
    pub handler_type: Option<String>,
    /// ISO-639-2 language code when present.
    pub language: Option<String>,
    /// Sample-entry box type when present.
    pub sample_entry_type: Option<String>,
    /// Protected original-format sample-entry type when present.
    pub original_format: Option<String>,
    /// Protection-scheme type when present.
    pub protection_scheme_type: Option<String>,
    /// Protection-scheme version when present.
    pub protection_scheme_version: Option<u32>,
    /// Display width for visual tracks.
    pub width: Option<u16>,
    /// Display height for visual tracks.
    pub height: Option<u16>,
    /// Channel count for audio tracks.
    pub channel_count: Option<u16>,
    /// Integer sample rate for audio tracks.
    pub sample_rate: Option<u16>,
    /// Expanded sample count when present.
    pub sample_num: Option<usize>,
    /// Expanded chunk count when present.
    pub chunk_num: Option<usize>,
    /// Count of samples carrying IDR NAL units.
    pub idr_frame_num: Option<usize>,
    /// Average bitrate in bits per second.
    pub bitrate: Option<u64>,
    /// Maximum bitrate in bits per second.
    pub max_bitrate: Option<u64>,
}

/// Top-level codec-detailed probe report used by the CLI command surface.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CodecDetailedProbeReport {
    /// Root `ftyp` major brand.
    pub major_brand: String,
    /// Root `ftyp` minor version.
    pub minor_version: u32,
    /// Root `ftyp` compatible brands.
    pub compatible_brands: Vec<String>,
    /// Whether the file places `moov` before the first `mdat`.
    pub fast_start: bool,
    /// Movie timescale from `mvhd`.
    pub timescale: u32,
    /// Movie duration from `mvhd`.
    pub duration: u64,
    /// Movie duration expressed in seconds.
    pub duration_seconds: f32,
    /// Per-track codec-detailed probe summaries.
    pub tracks: Vec<CodecDetailedProbeTrackReport>,
}

/// One track entry in the codec-detailed CLI probe report.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CodecDetailedProbeTrackReport {
    /// Track identifier from `tkhd`.
    pub track_id: u32,
    /// Track timescale from `mdhd`.
    pub timescale: u32,
    /// Track duration from `mdhd`.
    pub duration: u64,
    /// Track duration expressed in seconds.
    pub duration_seconds: f32,
    /// Human-readable codec identifier.
    pub codec: String,
    /// Normalized codec-family label.
    pub codec_family: String,
    /// Parsed codec-specific configuration details.
    pub codec_details: TrackCodecDetails,
    /// Whether the track uses an encrypted sample entry.
    pub encrypted: bool,
    /// Handler type when present.
    pub handler_type: Option<String>,
    /// ISO-639-2 language code when present.
    pub language: Option<String>,
    /// Sample-entry box type when present.
    pub sample_entry_type: Option<String>,
    /// Protected original-format sample-entry type when present.
    pub original_format: Option<String>,
    /// Protection-scheme type when present.
    pub protection_scheme_type: Option<String>,
    /// Protection-scheme version when present.
    pub protection_scheme_version: Option<u32>,
    /// Display width for visual tracks.
    pub width: Option<u16>,
    /// Display height for visual tracks.
    pub height: Option<u16>,
    /// Channel count for audio tracks.
    pub channel_count: Option<u16>,
    /// Integer sample rate for audio tracks.
    pub sample_rate: Option<u16>,
    /// Expanded sample count when present.
    pub sample_num: Option<usize>,
    /// Expanded chunk count when present.
    pub chunk_num: Option<usize>,
    /// Count of samples carrying IDR NAL units.
    pub idr_frame_num: Option<usize>,
    /// Average bitrate in bits per second.
    pub bitrate: Option<u64>,
    /// Maximum bitrate in bits per second.
    pub max_bitrate: Option<u64>,
}

/// Top-level media-characteristics probe report used by the CLI command surface.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaCharacteristicsProbeReport {
    /// Root `ftyp` major brand.
    pub major_brand: String,
    /// Root `ftyp` minor version.
    pub minor_version: u32,
    /// Root `ftyp` compatible brands.
    pub compatible_brands: Vec<String>,
    /// Whether the file places `moov` before the first `mdat`.
    pub fast_start: bool,
    /// Movie timescale from `mvhd`.
    pub timescale: u32,
    /// Movie duration from `mvhd`.
    pub duration: u64,
    /// Movie duration expressed in seconds.
    pub duration_seconds: f32,
    /// Per-track media-characteristics probe summaries.
    pub tracks: Vec<MediaCharacteristicsProbeTrackReport>,
}

/// One track entry in the media-characteristics CLI probe report.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaCharacteristicsProbeTrackReport {
    /// Track identifier from `tkhd`.
    pub track_id: u32,
    /// Track timescale from `mdhd`.
    pub timescale: u32,
    /// Track duration from `mdhd`.
    pub duration: u64,
    /// Track duration expressed in seconds.
    pub duration_seconds: f32,
    /// Human-readable codec identifier.
    pub codec: String,
    /// Normalized codec-family label.
    pub codec_family: String,
    /// Parsed codec-specific configuration details.
    pub codec_details: TrackCodecDetails,
    /// Sample-entry media characteristics already parsed by the crate.
    pub media_characteristics: TrackMediaCharacteristics,
    /// Whether the track uses an encrypted sample entry.
    pub encrypted: bool,
    /// Handler type when present.
    pub handler_type: Option<String>,
    /// ISO-639-2 language code when present.
    pub language: Option<String>,
    /// Sample-entry box type when present.
    pub sample_entry_type: Option<String>,
    /// Protected original-format sample-entry type when present.
    pub original_format: Option<String>,
    /// Protection-scheme type when present.
    pub protection_scheme_type: Option<String>,
    /// Protection-scheme version when present.
    pub protection_scheme_version: Option<u32>,
    /// Display width for visual tracks.
    pub width: Option<u16>,
    /// Display height for visual tracks.
    pub height: Option<u16>,
    /// Channel count for audio tracks.
    pub channel_count: Option<u16>,
    /// Integer sample rate for audio tracks.
    pub sample_rate: Option<u16>,
    /// Expanded sample count when present.
    pub sample_num: Option<usize>,
    /// Expanded chunk count when present.
    pub chunk_num: Option<usize>,
    /// Count of samples carrying IDR NAL units.
    pub idr_frame_num: Option<usize>,
    /// Average bitrate in bits per second.
    pub bitrate: Option<u64>,
    /// Maximum bitrate in bits per second.
    pub max_bitrate: Option<u64>,
}

/// Runs the probe subcommand with `args`, writing output to `stdout`.
pub fn run<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    match run_inner(args, stdout) {
        Ok(()) => 0,
        Err(ProbeCliError::UsageRequested) => {
            let _ = write_usage(stderr);
            1
        }
        Err(error) => {
            let _ = writeln!(stderr, "Error: {error}");
            1
        }
    }
}

/// Writes the probe subcommand usage text.
pub fn write_usage<W>(writer: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "USAGE: mp4forge probe [OPTIONS] INPUT.mp4")?;
    writeln!(writer)?;
    writeln!(writer, "OPTIONS:")?;
    writeln!(
        writer,
        "  -format <json|yaml>    Output format (default: json)"
    )?;
    writeln!(
        writer,
        "  -detail <full|light>  Probe detail level (default: full)"
    )?;
    Ok(())
}

/// Builds a CLI probe report from an MP4 reader.
pub fn build_report<R>(reader: &mut R) -> Result<ProbeReport, ProbeCliError>
where
    R: Read + Seek,
{
    build_report_with_options(reader, ProbeReportOptions::default())
}

/// Builds a CLI probe report from an MP4 reader with additive report controls.
pub fn build_report_with_options<R>(
    reader: &mut R,
    options: ProbeReportOptions,
) -> Result<ProbeReport, ProbeCliError>
where
    R: Read + Seek,
{
    let summary = probe_with_options(reader, options.probe)?;

    let mut report = ProbeReport {
        major_brand: summary.major_brand.to_string(),
        minor_version: summary.minor_version,
        compatible_brands: summary
            .compatible_brands
            .iter()
            .map(ToString::to_string)
            .collect(),
        fast_start: summary.fast_start,
        timescale: summary.timescale,
        duration: summary.duration,
        duration_seconds: seconds(summary.duration, summary.timescale),
        tracks: Vec::with_capacity(summary.tracks.len()),
    };

    for track in &summary.tracks {
        let (bitrate, max_bitrate) = summarize_bitrate(
            &track.samples,
            track.timescale,
            track.track_id,
            &summary.segments,
            options.include_bitrate,
        );

        let mut row = ProbeTrackReport {
            track_id: track.track_id,
            timescale: track.timescale,
            duration: track.duration,
            duration_seconds: seconds(track.duration, track.timescale),
            codec: track_codec_string(track),
            encrypted: track.encrypted,
            width: None,
            height: None,
            sample_num: some_if_nonzero(track.samples.len()),
            chunk_num: some_if_nonzero(track.chunks.len()),
            idr_frame_num: None,
            bitrate: some_if_nonzero(bitrate as usize).map(|_| bitrate),
            max_bitrate: some_if_nonzero(max_bitrate as usize).map(|_| max_bitrate),
        };

        if let Some(avc) = track.avc.as_ref() {
            row.width = Some(avc.width);
            row.height = Some(avc.height);
            if options.include_idr_frame_count && !track.samples.is_empty() {
                row.idr_frame_num = idr_frame_count(reader, track)?;
            }
        }

        report.tracks.push(row);
    }

    Ok(report)
}

/// Builds a detailed CLI probe report from an MP4 reader.
pub fn build_detailed_report<R>(reader: &mut R) -> Result<DetailedProbeReport, ProbeCliError>
where
    R: Read + Seek,
{
    build_detailed_report_with_options(reader, ProbeReportOptions::default())
}

/// Builds a detailed CLI probe report from an MP4 reader with additive report controls.
pub fn build_detailed_report_with_options<R>(
    reader: &mut R,
    options: ProbeReportOptions,
) -> Result<DetailedProbeReport, ProbeCliError>
where
    R: Read + Seek,
{
    let summary = probe_detailed_with_options(reader, options.probe)?;

    let mut report = DetailedProbeReport {
        major_brand: summary.major_brand.to_string(),
        minor_version: summary.minor_version,
        compatible_brands: summary
            .compatible_brands
            .iter()
            .map(ToString::to_string)
            .collect(),
        fast_start: summary.fast_start,
        timescale: summary.timescale,
        duration: summary.duration,
        duration_seconds: seconds(summary.duration, summary.timescale),
        tracks: Vec::with_capacity(summary.tracks.len()),
    };

    for track in &summary.tracks {
        let basic = &track.summary;
        let (bitrate, max_bitrate) = summarize_bitrate(
            &basic.samples,
            basic.timescale,
            basic.track_id,
            &summary.segments,
            options.include_bitrate,
        );

        let mut row = DetailedProbeTrackReport {
            track_id: basic.track_id,
            timescale: basic.timescale,
            duration: basic.duration,
            duration_seconds: seconds(basic.duration, basic.timescale),
            codec: detailed_track_codec_string(track),
            codec_family: track_codec_family_string(track.codec_family).to_string(),
            encrypted: basic.encrypted,
            handler_type: track.handler_type.map(|value| value.to_string()),
            language: track.language.clone(),
            sample_entry_type: track.sample_entry_type.map(|value| value.to_string()),
            original_format: track.original_format.map(|value| value.to_string()),
            protection_scheme_type: track
                .protection_scheme
                .as_ref()
                .map(|value| value.scheme_type.to_string()),
            protection_scheme_version: track
                .protection_scheme
                .as_ref()
                .map(|value| value.scheme_version),
            width: track.display_width,
            height: track.display_height,
            channel_count: track.channel_count,
            sample_rate: track.sample_rate,
            sample_num: some_if_nonzero(basic.samples.len()),
            chunk_num: some_if_nonzero(basic.chunks.len()),
            idr_frame_num: None,
            bitrate: some_if_nonzero(bitrate as usize).map(|_| bitrate),
            max_bitrate: some_if_nonzero(max_bitrate as usize).map(|_| max_bitrate),
        };

        if options.include_idr_frame_count && basic.avc.is_some() && !basic.samples.is_empty() {
            row.idr_frame_num = idr_frame_count(reader, basic)?;
        }

        report.tracks.push(row);
    }

    Ok(report)
}

/// Builds a codec-detailed CLI probe report from an MP4 reader.
pub fn build_codec_detailed_report<R>(
    reader: &mut R,
) -> Result<CodecDetailedProbeReport, ProbeCliError>
where
    R: Read + Seek,
{
    build_codec_detailed_report_with_options(reader, ProbeReportOptions::default())
}

/// Builds a codec-detailed CLI probe report from an MP4 reader with additive report controls.
pub fn build_codec_detailed_report_with_options<R>(
    reader: &mut R,
    options: ProbeReportOptions,
) -> Result<CodecDetailedProbeReport, ProbeCliError>
where
    R: Read + Seek,
{
    let summary = probe_codec_detailed_with_options(reader, options.probe)?;

    let mut report = CodecDetailedProbeReport {
        major_brand: summary.major_brand.to_string(),
        minor_version: summary.minor_version,
        compatible_brands: summary
            .compatible_brands
            .iter()
            .map(ToString::to_string)
            .collect(),
        fast_start: summary.fast_start,
        timescale: summary.timescale,
        duration: summary.duration,
        duration_seconds: seconds(summary.duration, summary.timescale),
        tracks: Vec::with_capacity(summary.tracks.len()),
    };

    for track in &summary.tracks {
        let basic = &track.summary.summary;
        let (bitrate, max_bitrate) = summarize_bitrate(
            &basic.samples,
            basic.timescale,
            basic.track_id,
            &summary.segments,
            options.include_bitrate,
        );

        let mut row = CodecDetailedProbeTrackReport {
            track_id: basic.track_id,
            timescale: basic.timescale,
            duration: basic.duration,
            duration_seconds: seconds(basic.duration, basic.timescale),
            codec: detailed_track_codec_string(&track.summary),
            codec_family: track_codec_family_string(track.summary.codec_family).to_string(),
            codec_details: track.codec_details.clone(),
            encrypted: basic.encrypted,
            handler_type: track.summary.handler_type.map(|value| value.to_string()),
            language: track.summary.language.clone(),
            sample_entry_type: track
                .summary
                .sample_entry_type
                .map(|value| value.to_string()),
            original_format: track.summary.original_format.map(|value| value.to_string()),
            protection_scheme_type: track
                .summary
                .protection_scheme
                .as_ref()
                .map(|value| value.scheme_type.to_string()),
            protection_scheme_version: track
                .summary
                .protection_scheme
                .as_ref()
                .map(|value| value.scheme_version),
            width: track.summary.display_width,
            height: track.summary.display_height,
            channel_count: track.summary.channel_count,
            sample_rate: track.summary.sample_rate,
            sample_num: some_if_nonzero(basic.samples.len()),
            chunk_num: some_if_nonzero(basic.chunks.len()),
            idr_frame_num: None,
            bitrate: some_if_nonzero(bitrate as usize).map(|_| bitrate),
            max_bitrate: some_if_nonzero(max_bitrate as usize).map(|_| max_bitrate),
        };

        if options.include_idr_frame_count && basic.avc.is_some() && !basic.samples.is_empty() {
            row.idr_frame_num = idr_frame_count(reader, basic)?;
        }

        report.tracks.push(row);
    }

    Ok(report)
}

/// Builds a media-characteristics CLI probe report from an MP4 reader.
pub fn build_media_characteristics_report<R>(
    reader: &mut R,
) -> Result<MediaCharacteristicsProbeReport, ProbeCliError>
where
    R: Read + Seek,
{
    build_media_characteristics_report_with_options(reader, ProbeReportOptions::default())
}

/// Builds a media-characteristics CLI probe report from an MP4 reader with additive report
/// controls.
pub fn build_media_characteristics_report_with_options<R>(
    reader: &mut R,
    options: ProbeReportOptions,
) -> Result<MediaCharacteristicsProbeReport, ProbeCliError>
where
    R: Read + Seek,
{
    let summary = probe_media_characteristics_with_options(reader, options.probe)?;

    let mut report = MediaCharacteristicsProbeReport {
        major_brand: summary.major_brand.to_string(),
        minor_version: summary.minor_version,
        compatible_brands: summary
            .compatible_brands
            .iter()
            .map(ToString::to_string)
            .collect(),
        fast_start: summary.fast_start,
        timescale: summary.timescale,
        duration: summary.duration,
        duration_seconds: seconds(summary.duration, summary.timescale),
        tracks: Vec::with_capacity(summary.tracks.len()),
    };

    for track in &summary.tracks {
        let basic = &track.summary.summary;
        let (bitrate, max_bitrate) = summarize_bitrate(
            &basic.samples,
            basic.timescale,
            basic.track_id,
            &summary.segments,
            options.include_bitrate,
        );

        let mut row = MediaCharacteristicsProbeTrackReport {
            track_id: basic.track_id,
            timescale: basic.timescale,
            duration: basic.duration,
            duration_seconds: seconds(basic.duration, basic.timescale),
            codec: detailed_track_codec_string(&track.summary),
            codec_family: track_codec_family_string(track.summary.codec_family).to_string(),
            codec_details: track.codec_details.clone(),
            media_characteristics: track.media_characteristics.clone(),
            encrypted: basic.encrypted,
            handler_type: track.summary.handler_type.map(|value| value.to_string()),
            language: track.summary.language.clone(),
            sample_entry_type: track
                .summary
                .sample_entry_type
                .map(|value| value.to_string()),
            original_format: track.summary.original_format.map(|value| value.to_string()),
            protection_scheme_type: track
                .summary
                .protection_scheme
                .as_ref()
                .map(|value| value.scheme_type.to_string()),
            protection_scheme_version: track
                .summary
                .protection_scheme
                .as_ref()
                .map(|value| value.scheme_version),
            width: track.summary.display_width,
            height: track.summary.display_height,
            channel_count: track.summary.channel_count,
            sample_rate: track.summary.sample_rate,
            sample_num: some_if_nonzero(basic.samples.len()),
            chunk_num: some_if_nonzero(basic.chunks.len()),
            idr_frame_num: None,
            bitrate: some_if_nonzero(bitrate as usize).map(|_| bitrate),
            max_bitrate: some_if_nonzero(max_bitrate as usize).map(|_| max_bitrate),
        };

        if options.include_idr_frame_count && basic.avc.is_some() && !basic.samples.is_empty() {
            row.idr_frame_num = idr_frame_count(reader, basic)?;
        }

        report.tracks.push(row);
    }

    Ok(report)
}

/// Writes `report` in the selected `format`.
pub fn write_report<W>(
    writer: &mut W,
    report: &ProbeReport,
    format: ProbeFormat,
) -> Result<(), ProbeCliError>
where
    W: Write,
{
    match format {
        ProbeFormat::Json => write_json_report(writer, report).map_err(ProbeCliError::Io),
        ProbeFormat::Yaml => write_yaml_report(writer, report).map_err(ProbeCliError::Io),
    }
}

/// Writes `report` in the selected `format`.
pub fn write_detailed_report<W>(
    writer: &mut W,
    report: &DetailedProbeReport,
    format: ProbeFormat,
) -> Result<(), ProbeCliError>
where
    W: Write,
{
    match format {
        ProbeFormat::Json => write_json_detailed_report(writer, report).map_err(ProbeCliError::Io),
        ProbeFormat::Yaml => write_yaml_detailed_report(writer, report).map_err(ProbeCliError::Io),
    }
}

/// Writes `report` in the selected `format`.
pub fn write_codec_detailed_report<W>(
    writer: &mut W,
    report: &CodecDetailedProbeReport,
    format: ProbeFormat,
) -> Result<(), ProbeCliError>
where
    W: Write,
{
    match format {
        ProbeFormat::Json => {
            write_json_codec_detailed_report(writer, report).map_err(ProbeCliError::Io)
        }
        ProbeFormat::Yaml => {
            write_yaml_codec_detailed_report(writer, report).map_err(ProbeCliError::Io)
        }
    }
}

/// Writes `report` in the selected `format`.
pub fn write_media_characteristics_report<W>(
    writer: &mut W,
    report: &MediaCharacteristicsProbeReport,
    format: ProbeFormat,
) -> Result<(), ProbeCliError>
where
    W: Write,
{
    match format {
        ProbeFormat::Json => {
            write_json_media_characteristics_report(writer, report).map_err(ProbeCliError::Io)
        }
        ProbeFormat::Yaml => {
            write_yaml_media_characteristics_report(writer, report).map_err(ProbeCliError::Io)
        }
    }
}

fn run_inner<W>(args: &[String], stdout: &mut W) -> Result<(), ProbeCliError>
where
    W: Write,
{
    let mut format = ProbeFormat::Json;
    let mut detail = ProbeDetailLevel::Full;
    let mut input_path = None;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-format" | "--format" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(ProbeCliError::InvalidArgument(
                        "missing value for -format".to_string(),
                    ));
                };
                format = ProbeFormat::parse(value)?;
                index += 2;
            }
            "-detail" | "--detail" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(ProbeCliError::InvalidArgument(
                        "missing value for -detail".to_string(),
                    ));
                };
                detail = ProbeDetailLevel::parse(value)?;
                index += 2;
            }
            "-h" | "--help" => return Err(ProbeCliError::UsageRequested),
            value if value.starts_with('-') => {
                return Err(ProbeCliError::InvalidArgument(format!(
                    "unknown probe option: {value}"
                )));
            }
            value => {
                if input_path.is_some() {
                    return Err(ProbeCliError::InvalidArgument(
                        "probe accepts exactly one input path".to_string(),
                    ));
                }
                input_path = Some(value);
                index += 1;
            }
        }
    }

    let Some(input_path) = input_path else {
        return Err(ProbeCliError::UsageRequested);
    };

    let mut file = File::open(input_path)?;
    let report =
        build_media_characteristics_report_with_options(&mut file, detail.report_options())?;
    write_media_characteristics_report(stdout, &report, format)
}

fn track_codec_string(track: &crate::probe::TrackInfo) -> String {
    match track.codec {
        TrackCodec::Avc1 => track
            .avc
            .as_ref()
            .map(|avc| {
                format!(
                    "avc1.{:02X}{:02X}{:02X}",
                    avc.profile, avc.profile_compatibility, avc.level
                )
            })
            .unwrap_or_else(|| "avc1".to_string()),
        TrackCodec::Mp4a => track
            .mp4a
            .as_ref()
            .map(|audio| {
                if audio.object_type_indication == 0 {
                    "mp4a".to_string()
                } else if audio.audio_object_type == 0 {
                    format!("mp4a.{:X}", audio.object_type_indication)
                } else {
                    format!(
                        "mp4a.{:X}.{}",
                        audio.object_type_indication, audio.audio_object_type
                    )
                }
            })
            .unwrap_or_else(|| "mp4a".to_string()),
        TrackCodec::Unknown => "unknown".to_string(),
    }
}

fn detailed_track_codec_string(track: &DetailedTrackInfo) -> String {
    let codec_box_type = track.original_format.or(track.sample_entry_type);
    match track.codec_family {
        TrackCodecFamily::Avc | TrackCodecFamily::Mp4Audio => track_codec_string(&track.summary),
        TrackCodecFamily::Unknown => codec_box_type
            .map(|value| value.to_string())
            .unwrap_or_else(|| track_codec_string(&track.summary)),
        _ => codec_box_type
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

fn track_codec_family_string(family: TrackCodecFamily) -> &'static str {
    match family {
        TrackCodecFamily::Unknown => "unknown",
        TrackCodecFamily::Avc => "avc",
        TrackCodecFamily::Hevc => "hevc",
        TrackCodecFamily::Av1 => "av1",
        TrackCodecFamily::Vp8 => "vp8",
        TrackCodecFamily::Vp9 => "vp9",
        TrackCodecFamily::Mp4Audio => "mp4_audio",
        TrackCodecFamily::Opus => "opus",
        TrackCodecFamily::Ac3 => "ac3",
        TrackCodecFamily::Pcm => "pcm",
        TrackCodecFamily::XmlSubtitle => "xml_subtitle",
        TrackCodecFamily::TextSubtitle => "text_subtitle",
        TrackCodecFamily::WebVtt => "webvtt",
    }
}

fn summarize_bitrate(
    samples: &[crate::probe::SampleInfo],
    timescale: u32,
    track_id: u32,
    segments: &[crate::probe::SegmentInfo],
    include_bitrate: bool,
) -> (u64, u64) {
    if !include_bitrate {
        return (0, 0);
    }
    let mut bitrate = average_sample_bitrate(samples, timescale);
    let mut max_bitrate = max_sample_bitrate(samples, timescale, timescale.into());
    if bitrate == 0 || max_bitrate == 0 {
        bitrate = average_segment_bitrate(segments, track_id, timescale);
        max_bitrate = max_segment_bitrate(segments, track_id, timescale);
    }
    (bitrate, max_bitrate)
}

fn idr_frame_count<R>(
    reader: &mut R,
    track: &crate::probe::TrackInfo,
) -> Result<Option<usize>, ProbeCliError>
where
    R: Read + Seek,
{
    match find_idr_frames(reader, track) {
        Ok(indices) => Ok(some_if_nonzero(indices.len())),
        Err(ProbeError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn seconds(duration: u64, timescale: u32) -> f32 {
    if timescale == 0 {
        0.0
    } else {
        duration as f32 / timescale as f32
    }
}

fn some_if_nonzero<T>(value: T) -> Option<T>
where
    T: PartialEq + Default,
{
    if value == T::default() {
        None
    } else {
        Some(value)
    }
}

fn write_json_report<W>(writer: &mut W, report: &ProbeReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "{{")?;
    write_json_field(
        writer,
        1,
        "MajorBrand",
        &json_string(&report.major_brand),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "MinorVersion",
        &report.minor_version.to_string(),
        true,
    )?;
    writeln!(writer, "  \"CompatibleBrands\": [")?;
    for (index, brand) in report.compatible_brands.iter().enumerate() {
        let trailing = if index + 1 == report.compatible_brands.len() {
            ""
        } else {
            ","
        };
        writeln!(writer, "    {}{trailing}", json_string(brand))?;
    }
    writeln!(writer, "  ],")?;
    write_json_field(
        writer,
        1,
        "FastStart",
        if report.fast_start { "true" } else { "false" },
        true,
    )?;
    write_json_field(writer, 1, "Timescale", &report.timescale.to_string(), true)?;
    write_json_field(writer, 1, "Duration", &report.duration.to_string(), true)?;
    write_json_field(
        writer,
        1,
        "DurationSeconds",
        &format_seconds(report.duration_seconds),
        true,
    )?;
    writeln!(writer, "  \"Tracks\": [")?;
    for (index, track) in report.tracks.iter().enumerate() {
        let trailing = if index + 1 == report.tracks.len() {
            ""
        } else {
            ","
        };
        write_json_track(writer, track)?;
        writeln!(writer, "  }}{trailing}")?;
    }
    writeln!(writer, "  ]")?;
    writeln!(writer, "}}")
}

fn write_json_track<W>(writer: &mut W, track: &ProbeTrackReport) -> io::Result<()>
where
    W: Write,
{
    let mut fields = vec![
        ("TrackID", track.track_id.to_string()),
        ("Timescale", track.timescale.to_string()),
        ("Duration", track.duration.to_string()),
        ("DurationSeconds", format_seconds(track.duration_seconds)),
        ("Codec", json_string(&track.codec)),
        (
            "Encrypted",
            if track.encrypted { "true" } else { "false" }.to_string(),
        ),
    ];

    if let Some(width) = track.width {
        fields.push(("Width", width.to_string()));
    }
    if let Some(height) = track.height {
        fields.push(("Height", height.to_string()));
    }
    if let Some(sample_num) = track.sample_num {
        fields.push(("SampleNum", sample_num.to_string()));
    }
    if let Some(chunk_num) = track.chunk_num {
        fields.push(("ChunkNum", chunk_num.to_string()));
    }
    if let Some(idr_frame_num) = track.idr_frame_num {
        fields.push(("IDRFrameNum", idr_frame_num.to_string()));
    }
    if let Some(bitrate) = track.bitrate {
        fields.push(("Bitrate", bitrate.to_string()));
    }
    if let Some(max_bitrate) = track.max_bitrate {
        fields.push(("MaxBitrate", max_bitrate.to_string()));
    }

    writeln!(writer, "    {{")?;
    for (index, (name, value)) in fields.iter().enumerate() {
        write_json_field(writer, 3, name, value, index + 1 != fields.len())?;
    }
    Ok(())
}

fn write_json_detailed_report<W>(writer: &mut W, report: &DetailedProbeReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "{{")?;
    write_json_field(
        writer,
        1,
        "MajorBrand",
        &json_string(&report.major_brand),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "MinorVersion",
        &report.minor_version.to_string(),
        true,
    )?;
    writeln!(writer, "  \"CompatibleBrands\": [")?;
    for (index, brand) in report.compatible_brands.iter().enumerate() {
        let trailing = if index + 1 == report.compatible_brands.len() {
            ""
        } else {
            ","
        };
        writeln!(writer, "    {}{trailing}", json_string(brand))?;
    }
    writeln!(writer, "  ],")?;
    write_json_field(
        writer,
        1,
        "FastStart",
        if report.fast_start { "true" } else { "false" },
        true,
    )?;
    write_json_field(writer, 1, "Timescale", &report.timescale.to_string(), true)?;
    write_json_field(writer, 1, "Duration", &report.duration.to_string(), true)?;
    write_json_field(
        writer,
        1,
        "DurationSeconds",
        &format_seconds(report.duration_seconds),
        true,
    )?;
    writeln!(writer, "  \"Tracks\": [")?;
    for (index, track) in report.tracks.iter().enumerate() {
        let trailing = if index + 1 == report.tracks.len() {
            ""
        } else {
            ","
        };
        write_json_detailed_track(writer, track)?;
        writeln!(writer, "  }}{trailing}")?;
    }
    writeln!(writer, "  ]")?;
    writeln!(writer, "}}")
}

fn write_json_detailed_track<W>(writer: &mut W, track: &DetailedProbeTrackReport) -> io::Result<()>
where
    W: Write,
{
    let mut fields = vec![
        ("TrackID", track.track_id.to_string()),
        ("Timescale", track.timescale.to_string()),
        ("Duration", track.duration.to_string()),
        ("DurationSeconds", format_seconds(track.duration_seconds)),
        ("Codec", json_string(&track.codec)),
        ("CodecFamily", json_string(&track.codec_family)),
        (
            "Encrypted",
            if track.encrypted { "true" } else { "false" }.to_string(),
        ),
    ];

    if let Some(handler_type) = track.handler_type.as_ref() {
        fields.push(("HandlerType", json_string(handler_type)));
    }
    if let Some(language) = track.language.as_ref() {
        fields.push(("Language", json_string(language)));
    }
    if let Some(sample_entry_type) = track.sample_entry_type.as_ref() {
        fields.push(("SampleEntryType", json_string(sample_entry_type)));
    }
    if let Some(original_format) = track.original_format.as_ref() {
        fields.push(("OriginalFormat", json_string(original_format)));
    }
    if let Some(protection_scheme_type) = track.protection_scheme_type.as_ref() {
        fields.push(("ProtectionSchemeType", json_string(protection_scheme_type)));
    }
    if let Some(protection_scheme_version) = track.protection_scheme_version {
        fields.push((
            "ProtectionSchemeVersion",
            protection_scheme_version.to_string(),
        ));
    }
    if let Some(width) = track.width {
        fields.push(("Width", width.to_string()));
    }
    if let Some(height) = track.height {
        fields.push(("Height", height.to_string()));
    }
    if let Some(channel_count) = track.channel_count {
        fields.push(("ChannelCount", channel_count.to_string()));
    }
    if let Some(sample_rate) = track.sample_rate {
        fields.push(("SampleRate", sample_rate.to_string()));
    }
    if let Some(sample_num) = track.sample_num {
        fields.push(("SampleNum", sample_num.to_string()));
    }
    if let Some(chunk_num) = track.chunk_num {
        fields.push(("ChunkNum", chunk_num.to_string()));
    }
    if let Some(idr_frame_num) = track.idr_frame_num {
        fields.push(("IDRFrameNum", idr_frame_num.to_string()));
    }
    if let Some(bitrate) = track.bitrate {
        fields.push(("Bitrate", bitrate.to_string()));
    }
    if let Some(max_bitrate) = track.max_bitrate {
        fields.push(("MaxBitrate", max_bitrate.to_string()));
    }

    writeln!(writer, "    {{")?;
    for (index, (name, value)) in fields.iter().enumerate() {
        write_json_field(writer, 3, name, value, index + 1 != fields.len())?;
    }
    Ok(())
}

fn write_json_codec_detailed_report<W>(
    writer: &mut W,
    report: &CodecDetailedProbeReport,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "{{")?;
    write_json_field(
        writer,
        1,
        "MajorBrand",
        &json_string(&report.major_brand),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "MinorVersion",
        &report.minor_version.to_string(),
        true,
    )?;
    writeln!(writer, "  \"CompatibleBrands\": [")?;
    for (index, brand) in report.compatible_brands.iter().enumerate() {
        let trailing = if index + 1 == report.compatible_brands.len() {
            ""
        } else {
            ","
        };
        writeln!(writer, "    {}{trailing}", json_string(brand))?;
    }
    writeln!(writer, "  ],")?;
    write_json_field(
        writer,
        1,
        "FastStart",
        if report.fast_start { "true" } else { "false" },
        true,
    )?;
    write_json_field(writer, 1, "Timescale", &report.timescale.to_string(), true)?;
    write_json_field(writer, 1, "Duration", &report.duration.to_string(), true)?;
    write_json_field(
        writer,
        1,
        "DurationSeconds",
        &format_seconds(report.duration_seconds),
        true,
    )?;
    writeln!(writer, "  \"Tracks\": [")?;
    for (index, track) in report.tracks.iter().enumerate() {
        let trailing = if index + 1 == report.tracks.len() {
            ""
        } else {
            ","
        };
        write_json_codec_detailed_track(writer, track)?;
        writeln!(writer, "  }}{trailing}")?;
    }
    writeln!(writer, "  ]")?;
    writeln!(writer, "}}")
}

fn write_json_codec_detailed_track<W>(
    writer: &mut W,
    track: &CodecDetailedProbeTrackReport,
) -> io::Result<()>
where
    W: Write,
{
    let mut fields = vec![
        ("TrackID", track.track_id.to_string()),
        ("Timescale", track.timescale.to_string()),
        ("Duration", track.duration.to_string()),
        ("DurationSeconds", format_seconds(track.duration_seconds)),
        ("Codec", json_string(&track.codec)),
        ("CodecFamily", json_string(&track.codec_family)),
        (
            "Encrypted",
            if track.encrypted { "true" } else { "false" }.to_string(),
        ),
    ];

    if let Some(handler_type) = track.handler_type.as_ref() {
        fields.push(("HandlerType", json_string(handler_type)));
    }
    if let Some(language) = track.language.as_ref() {
        fields.push(("Language", json_string(language)));
    }
    if let Some(sample_entry_type) = track.sample_entry_type.as_ref() {
        fields.push(("SampleEntryType", json_string(sample_entry_type)));
    }
    if let Some(original_format) = track.original_format.as_ref() {
        fields.push(("OriginalFormat", json_string(original_format)));
    }
    if let Some(protection_scheme_type) = track.protection_scheme_type.as_ref() {
        fields.push(("ProtectionSchemeType", json_string(protection_scheme_type)));
    }
    if let Some(protection_scheme_version) = track.protection_scheme_version {
        fields.push((
            "ProtectionSchemeVersion",
            protection_scheme_version.to_string(),
        ));
    }
    if let Some(width) = track.width {
        fields.push(("Width", width.to_string()));
    }
    if let Some(height) = track.height {
        fields.push(("Height", height.to_string()));
    }
    if let Some(channel_count) = track.channel_count {
        fields.push(("ChannelCount", channel_count.to_string()));
    }
    if let Some(sample_rate) = track.sample_rate {
        fields.push(("SampleRate", sample_rate.to_string()));
    }
    if let Some(sample_num) = track.sample_num {
        fields.push(("SampleNum", sample_num.to_string()));
    }
    if let Some(chunk_num) = track.chunk_num {
        fields.push(("ChunkNum", chunk_num.to_string()));
    }
    if let Some(idr_frame_num) = track.idr_frame_num {
        fields.push(("IDRFrameNum", idr_frame_num.to_string()));
    }
    if let Some(bitrate) = track.bitrate {
        fields.push(("Bitrate", bitrate.to_string()));
    }
    if let Some(max_bitrate) = track.max_bitrate {
        fields.push(("MaxBitrate", max_bitrate.to_string()));
    }

    writeln!(writer, "    {{")?;
    for (name, value) in &fields {
        write_json_field(writer, 3, name, value, true)?;
    }
    write_json_codec_details(writer, 3, &track.codec_family, &track.codec_details, false)?;
    Ok(())
}

fn write_json_media_characteristics_report<W>(
    writer: &mut W,
    report: &MediaCharacteristicsProbeReport,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "{{")?;
    write_json_field(
        writer,
        1,
        "MajorBrand",
        &json_string(&report.major_brand),
        true,
    )?;
    write_json_field(
        writer,
        1,
        "MinorVersion",
        &report.minor_version.to_string(),
        true,
    )?;
    writeln!(writer, "  \"CompatibleBrands\": [")?;
    for (index, brand) in report.compatible_brands.iter().enumerate() {
        let trailing = if index + 1 == report.compatible_brands.len() {
            ""
        } else {
            ","
        };
        writeln!(writer, "    {}{trailing}", json_string(brand))?;
    }
    writeln!(writer, "  ],")?;
    write_json_field(
        writer,
        1,
        "FastStart",
        if report.fast_start { "true" } else { "false" },
        true,
    )?;
    write_json_field(writer, 1, "Timescale", &report.timescale.to_string(), true)?;
    write_json_field(writer, 1, "Duration", &report.duration.to_string(), true)?;
    write_json_field(
        writer,
        1,
        "DurationSeconds",
        &format_seconds(report.duration_seconds),
        true,
    )?;
    writeln!(writer, "  \"Tracks\": [")?;
    for (index, track) in report.tracks.iter().enumerate() {
        let trailing = if index + 1 == report.tracks.len() {
            ""
        } else {
            ","
        };
        write_json_media_characteristics_track(writer, track)?;
        writeln!(writer, "  }}{trailing}")?;
    }
    writeln!(writer, "  ]")?;
    writeln!(writer, "}}")
}

fn write_json_media_characteristics_track<W>(
    writer: &mut W,
    track: &MediaCharacteristicsProbeTrackReport,
) -> io::Result<()>
where
    W: Write,
{
    let mut fields = vec![
        ("TrackID", track.track_id.to_string()),
        ("Timescale", track.timescale.to_string()),
        ("Duration", track.duration.to_string()),
        ("DurationSeconds", format_seconds(track.duration_seconds)),
        ("Codec", json_string(&track.codec)),
        ("CodecFamily", json_string(&track.codec_family)),
        (
            "Encrypted",
            if track.encrypted { "true" } else { "false" }.to_string(),
        ),
    ];

    if let Some(handler_type) = track.handler_type.as_ref() {
        fields.push(("HandlerType", json_string(handler_type)));
    }
    if let Some(language) = track.language.as_ref() {
        fields.push(("Language", json_string(language)));
    }
    if let Some(sample_entry_type) = track.sample_entry_type.as_ref() {
        fields.push(("SampleEntryType", json_string(sample_entry_type)));
    }
    if let Some(original_format) = track.original_format.as_ref() {
        fields.push(("OriginalFormat", json_string(original_format)));
    }
    if let Some(protection_scheme_type) = track.protection_scheme_type.as_ref() {
        fields.push(("ProtectionSchemeType", json_string(protection_scheme_type)));
    }
    if let Some(protection_scheme_version) = track.protection_scheme_version {
        fields.push((
            "ProtectionSchemeVersion",
            protection_scheme_version.to_string(),
        ));
    }
    if let Some(width) = track.width {
        fields.push(("Width", width.to_string()));
    }
    if let Some(height) = track.height {
        fields.push(("Height", height.to_string()));
    }
    if let Some(channel_count) = track.channel_count {
        fields.push(("ChannelCount", channel_count.to_string()));
    }
    if let Some(sample_rate) = track.sample_rate {
        fields.push(("SampleRate", sample_rate.to_string()));
    }
    if let Some(sample_num) = track.sample_num {
        fields.push(("SampleNum", sample_num.to_string()));
    }
    if let Some(chunk_num) = track.chunk_num {
        fields.push(("ChunkNum", chunk_num.to_string()));
    }
    if let Some(idr_frame_num) = track.idr_frame_num {
        fields.push(("IDRFrameNum", idr_frame_num.to_string()));
    }
    if let Some(bitrate) = track.bitrate {
        fields.push(("Bitrate", bitrate.to_string()));
    }
    if let Some(max_bitrate) = track.max_bitrate {
        fields.push(("MaxBitrate", max_bitrate.to_string()));
    }

    writeln!(writer, "    {{")?;
    for (name, value) in &fields {
        write_json_field(writer, 3, name, value, true)?;
    }
    let include_media = has_media_characteristics(&track.media_characteristics);
    write_json_codec_details(
        writer,
        3,
        &track.codec_family,
        &track.codec_details,
        include_media,
    )?;
    if include_media {
        write_json_media_characteristics(writer, 3, &track.media_characteristics)?;
    }
    Ok(())
}

fn write_json_media_characteristics<W>(
    writer: &mut W,
    indent_level: usize,
    characteristics: &TrackMediaCharacteristics,
) -> io::Result<()>
where
    W: Write,
{
    let section_count = usize::from(characteristics.declared_bitrate.is_some())
        + usize::from(characteristics.color.is_some())
        + usize::from(characteristics.pixel_aspect_ratio.is_some())
        + usize::from(characteristics.field_order.is_some());
    if section_count == 0 {
        return Ok(());
    }

    let indent = "  ".repeat(indent_level);
    writeln!(writer, "{indent}\"MediaCharacteristics\": {{")?;
    let mut written = 0usize;

    if let Some(value) = characteristics.declared_bitrate.as_ref() {
        written += 1;
        writeln!(
            writer,
            "{}\"DeclaredBitrate\": {{",
            "  ".repeat(indent_level + 1)
        )?;
        write_json_field(
            writer,
            indent_level + 2,
            "BufferSizeDB",
            &value.buffer_size_db.to_string(),
            true,
        )?;
        write_json_field(
            writer,
            indent_level + 2,
            "MaxBitrate",
            &value.max_bitrate.to_string(),
            true,
        )?;
        write_json_field(
            writer,
            indent_level + 2,
            "AvgBitrate",
            &value.avg_bitrate.to_string(),
            false,
        )?;
        let trailing = if written == section_count { "" } else { "," };
        writeln!(writer, "{}}}{trailing}", "  ".repeat(indent_level + 1))?;
    }

    if let Some(value) = characteristics.color.as_ref() {
        written += 1;
        writeln!(writer, "{}\"Color\": {{", "  ".repeat(indent_level + 1))?;
        let mut fields = vec![("ColourType", json_string(&value.colour_type.to_string()))];
        if let Some(colour_primaries) = value.colour_primaries {
            fields.push(("ColourPrimaries", colour_primaries.to_string()));
        }
        if let Some(transfer_characteristics) = value.transfer_characteristics {
            fields.push((
                "TransferCharacteristics",
                transfer_characteristics.to_string(),
            ));
        }
        if let Some(matrix_coefficients) = value.matrix_coefficients {
            fields.push(("MatrixCoefficients", matrix_coefficients.to_string()));
        }
        if let Some(full_range) = value.full_range {
            fields.push((
                "FullRange",
                if full_range { "true" } else { "false" }.to_string(),
            ));
        }
        if let Some(profile_size) = value.profile_size {
            fields.push(("ProfileSize", profile_size.to_string()));
        }
        if let Some(unknown_size) = value.unknown_size {
            fields.push(("UnknownSize", unknown_size.to_string()));
        }
        for (index, (name, field_value)) in fields.iter().enumerate() {
            write_json_field(
                writer,
                indent_level + 2,
                name,
                field_value,
                index + 1 != fields.len(),
            )?;
        }
        let trailing = if written == section_count { "" } else { "," };
        writeln!(writer, "{}}}{trailing}", "  ".repeat(indent_level + 1))?;
    }

    if let Some(value) = characteristics.pixel_aspect_ratio.as_ref() {
        written += 1;
        writeln!(
            writer,
            "{}\"PixelAspectRatio\": {{",
            "  ".repeat(indent_level + 1)
        )?;
        write_json_field(
            writer,
            indent_level + 2,
            "HSpacing",
            &value.h_spacing.to_string(),
            true,
        )?;
        write_json_field(
            writer,
            indent_level + 2,
            "VSpacing",
            &value.v_spacing.to_string(),
            false,
        )?;
        let trailing = if written == section_count { "" } else { "," };
        writeln!(writer, "{}}}{trailing}", "  ".repeat(indent_level + 1))?;
    }

    if let Some(value) = characteristics.field_order.as_ref() {
        written += 1;
        writeln!(
            writer,
            "{}\"FieldOrder\": {{",
            "  ".repeat(indent_level + 1)
        )?;
        write_json_field(
            writer,
            indent_level + 2,
            "FieldCount",
            &value.field_count.to_string(),
            true,
        )?;
        write_json_field(
            writer,
            indent_level + 2,
            "FieldOrdering",
            &value.field_ordering.to_string(),
            true,
        )?;
        write_json_field(
            writer,
            indent_level + 2,
            "Interlaced",
            if value.interlaced { "true" } else { "false" },
            false,
        )?;
        let trailing = if written == section_count { "" } else { "," };
        writeln!(writer, "{}}}{trailing}", "  ".repeat(indent_level + 1))?;
    }

    writeln!(writer, "{}}}", indent)
}

fn write_json_codec_details<W>(
    writer: &mut W,
    indent_level: usize,
    codec_family: &str,
    details: &TrackCodecDetails,
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "{}\"CodecDetails\": {{", "  ".repeat(indent_level))?;
    let fields = codec_detail_json_fields(codec_family, details);
    for (index, (name, value)) in fields.iter().enumerate() {
        write_json_field(
            writer,
            indent_level + 1,
            name,
            value,
            index + 1 != fields.len(),
        )?;
    }
    let trailing = if trailing_comma { "," } else { "" };
    writeln!(writer, "{}}}{trailing}", "  ".repeat(indent_level))
}

fn codec_detail_json_fields(
    codec_family: &str,
    details: &TrackCodecDetails,
) -> Vec<(&'static str, String)> {
    let mut fields = vec![("Kind", json_string(codec_family))];
    match details {
        TrackCodecDetails::Unknown => {}
        TrackCodecDetails::Avc(details) => {
            fields.push((
                "ConfigurationVersion",
                details.configuration_version.to_string(),
            ));
            fields.push(("Profile", details.profile.to_string()));
            fields.push((
                "ProfileCompatibility",
                details.profile_compatibility.to_string(),
            ));
            fields.push(("Level", details.level.to_string()));
            fields.push(("LengthSize", details.length_size.to_string()));
            if let Some(chroma_format) = details.chroma_format {
                fields.push(("ChromaFormat", chroma_format.to_string()));
            }
            if let Some(bit_depth_luma) = details.bit_depth_luma {
                fields.push(("BitDepthLuma", bit_depth_luma.to_string()));
            }
            if let Some(bit_depth_chroma) = details.bit_depth_chroma {
                fields.push(("BitDepthChroma", bit_depth_chroma.to_string()));
            }
        }
        TrackCodecDetails::Hevc(details) => {
            fields.push((
                "ConfigurationVersion",
                details.configuration_version.to_string(),
            ));
            fields.push(("ProfileSpace", details.profile_space.to_string()));
            fields.push((
                "TierFlag",
                if details.tier_flag { "true" } else { "false" }.to_string(),
            ));
            fields.push(("ProfileIDC", details.profile_idc.to_string()));
            fields.push((
                "ProfileCompatibilityMask",
                details.profile_compatibility_mask.to_string(),
            ));
            fields.push((
                "ConstraintIndicator",
                json_u8_array(&details.constraint_indicator),
            ));
            fields.push(("LevelIDC", details.level_idc.to_string()));
            fields.push((
                "MinSpatialSegmentationIDC",
                details.min_spatial_segmentation_idc.to_string(),
            ));
            fields.push(("ParallelismType", details.parallelism_type.to_string()));
            fields.push(("ChromaFormatIDC", details.chroma_format_idc.to_string()));
            fields.push(("BitDepthLuma", details.bit_depth_luma.to_string()));
            fields.push(("BitDepthChroma", details.bit_depth_chroma.to_string()));
            fields.push(("AvgFrameRate", details.avg_frame_rate.to_string()));
            fields.push(("ConstantFrameRate", details.constant_frame_rate.to_string()));
            fields.push(("NumTemporalLayers", details.num_temporal_layers.to_string()));
            fields.push(("TemporalIDNested", details.temporal_id_nested.to_string()));
            fields.push(("LengthSize", details.length_size.to_string()));
        }
        TrackCodecDetails::Av1(details) => {
            fields.push(("SeqProfile", details.seq_profile.to_string()));
            fields.push(("SeqLevelIdx0", details.seq_level_idx_0.to_string()));
            fields.push(("SeqTier0", details.seq_tier_0.to_string()));
            fields.push(("BitDepth", details.bit_depth.to_string()));
            fields.push((
                "Monochrome",
                if details.monochrome { "true" } else { "false" }.to_string(),
            ));
            fields.push((
                "ChromaSubsamplingX",
                details.chroma_subsampling_x.to_string(),
            ));
            fields.push((
                "ChromaSubsamplingY",
                details.chroma_subsampling_y.to_string(),
            ));
            fields.push((
                "ChromaSamplePosition",
                details.chroma_sample_position.to_string(),
            ));
            if let Some(delay) = details.initial_presentation_delay_minus_one {
                fields.push(("InitialPresentationDelayMinusOne", delay.to_string()));
            }
        }
        TrackCodecDetails::Vp8(details) | TrackCodecDetails::Vp9(details) => {
            fields.push(("Profile", details.profile.to_string()));
            fields.push(("Level", details.level.to_string()));
            fields.push(("BitDepth", details.bit_depth.to_string()));
            fields.push(("ChromaSubsampling", details.chroma_subsampling.to_string()));
            fields.push((
                "FullRange",
                if details.full_range { "true" } else { "false" }.to_string(),
            ));
            fields.push(("ColourPrimaries", details.colour_primaries.to_string()));
            fields.push((
                "TransferCharacteristics",
                details.transfer_characteristics.to_string(),
            ));
            fields.push((
                "MatrixCoefficients",
                details.matrix_coefficients.to_string(),
            ));
            fields.push((
                "CodecInitializationDataSize",
                details.codec_initialization_data_size.to_string(),
            ));
        }
        TrackCodecDetails::Mp4Audio(details) => {
            fields.push((
                "ObjectTypeIndication",
                details.object_type_indication.to_string(),
            ));
            fields.push(("AudioObjectType", details.audio_object_type.to_string()));
            fields.push(("ChannelCount", details.channel_count.to_string()));
            if let Some(sample_rate) = details.sample_rate {
                fields.push(("SampleRate", sample_rate.to_string()));
            }
        }
        TrackCodecDetails::Opus(details) => {
            fields.push((
                "OutputChannelCount",
                details.output_channel_count.to_string(),
            ));
            fields.push(("PreSkip", details.pre_skip.to_string()));
            fields.push(("InputSampleRate", details.input_sample_rate.to_string()));
            fields.push(("OutputGain", details.output_gain.to_string()));
            fields.push((
                "ChannelMappingFamily",
                details.channel_mapping_family.to_string(),
            ));
            if let Some(stream_count) = details.stream_count {
                fields.push(("StreamCount", stream_count.to_string()));
            }
            if let Some(coupled_count) = details.coupled_count {
                fields.push(("CoupledCount", coupled_count.to_string()));
            }
            if !details.channel_mapping.is_empty() {
                fields.push(("ChannelMapping", json_u8_array(&details.channel_mapping)));
            }
        }
        TrackCodecDetails::Ac3(details) => {
            fields.push(("SampleRateCode", details.sample_rate_code.to_string()));
            fields.push((
                "BitStreamIdentification",
                details.bit_stream_identification.to_string(),
            ));
            fields.push(("BitStreamMode", details.bit_stream_mode.to_string()));
            fields.push(("AudioCodingMode", details.audio_coding_mode.to_string()));
            fields.push((
                "LfeOn",
                if details.lfe_on { "true" } else { "false" }.to_string(),
            ));
            fields.push(("BitRateCode", details.bit_rate_code.to_string()));
        }
        TrackCodecDetails::Pcm(details) => {
            fields.push(("FormatFlags", details.format_flags.to_string()));
            fields.push(("SampleSize", details.sample_size.to_string()));
        }
        TrackCodecDetails::XmlSubtitle(details) => {
            fields.push(("Namespace", json_string(&details.namespace)));
            fields.push(("SchemaLocation", json_string(&details.schema_location)));
            fields.push((
                "AuxiliaryMimeTypes",
                json_string(&details.auxiliary_mime_types),
            ));
        }
        TrackCodecDetails::TextSubtitle(details) => {
            fields.push(("ContentEncoding", json_string(&details.content_encoding)));
            fields.push(("MimeFormat", json_string(&details.mime_format)));
        }
        TrackCodecDetails::WebVtt(details) => {
            if let Some(config) = details.config.as_ref() {
                fields.push(("Config", json_string(config)));
            }
            if let Some(source_label) = details.source_label.as_ref() {
                fields.push(("SourceLabel", json_string(source_label)));
            }
        }
    }

    fields
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
    let trailing = if trailing_comma { "," } else { "" };
    writeln!(
        writer,
        "{}\"{name}\": {value}{trailing}",
        "  ".repeat(indent_level)
    )
}

fn write_yaml_report<W>(writer: &mut W, report: &ProbeReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "major_brand: {}", yaml_string(&report.major_brand))?;
    writeln!(writer, "minor_version: {}", report.minor_version)?;
    writeln!(writer, "compatible_brands:")?;
    for brand in &report.compatible_brands {
        writeln!(writer, "- {}", yaml_string(brand))?;
    }
    writeln!(writer, "fast_start: {}", report.fast_start)?;
    writeln!(writer, "timescale: {}", report.timescale)?;
    writeln!(writer, "duration: {}", report.duration)?;
    writeln!(
        writer,
        "duration_seconds: {}",
        format_seconds(report.duration_seconds)
    )?;
    writeln!(writer, "tracks:")?;
    for track in &report.tracks {
        write_yaml_track(writer, track)?;
    }
    Ok(())
}

fn write_yaml_track<W>(writer: &mut W, track: &ProbeTrackReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "- track_id: {}", track.track_id)?;
    writeln!(writer, "  timescale: {}", track.timescale)?;
    writeln!(writer, "  duration: {}", track.duration)?;
    writeln!(
        writer,
        "  duration_seconds: {}",
        format_seconds(track.duration_seconds)
    )?;
    writeln!(writer, "  codec: {}", yaml_string(&track.codec))?;
    writeln!(writer, "  encrypted: {}", track.encrypted)?;
    if let Some(width) = track.width {
        writeln!(writer, "  width: {width}")?;
    }
    if let Some(height) = track.height {
        writeln!(writer, "  height: {height}")?;
    }
    if let Some(sample_num) = track.sample_num {
        writeln!(writer, "  sample_num: {sample_num}")?;
    }
    if let Some(chunk_num) = track.chunk_num {
        writeln!(writer, "  chunk_num: {chunk_num}")?;
    }
    if let Some(idr_frame_num) = track.idr_frame_num {
        writeln!(writer, "  idr_frame_num: {idr_frame_num}")?;
    }
    if let Some(bitrate) = track.bitrate {
        writeln!(writer, "  bitrate: {bitrate}")?;
    }
    if let Some(max_bitrate) = track.max_bitrate {
        writeln!(writer, "  max_bitrate: {max_bitrate}")?;
    }
    Ok(())
}

fn write_yaml_detailed_report<W>(writer: &mut W, report: &DetailedProbeReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "major_brand: {}", yaml_string(&report.major_brand))?;
    writeln!(writer, "minor_version: {}", report.minor_version)?;
    writeln!(writer, "compatible_brands:")?;
    for brand in &report.compatible_brands {
        writeln!(writer, "- {}", yaml_string(brand))?;
    }
    writeln!(writer, "fast_start: {}", report.fast_start)?;
    writeln!(writer, "timescale: {}", report.timescale)?;
    writeln!(writer, "duration: {}", report.duration)?;
    writeln!(
        writer,
        "duration_seconds: {}",
        format_seconds(report.duration_seconds)
    )?;
    writeln!(writer, "tracks:")?;
    for track in &report.tracks {
        write_yaml_detailed_track(writer, track)?;
    }
    Ok(())
}

fn write_yaml_detailed_track<W>(writer: &mut W, track: &DetailedProbeTrackReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "- track_id: {}", track.track_id)?;
    writeln!(writer, "  timescale: {}", track.timescale)?;
    writeln!(writer, "  duration: {}", track.duration)?;
    writeln!(
        writer,
        "  duration_seconds: {}",
        format_seconds(track.duration_seconds)
    )?;
    writeln!(writer, "  codec: {}", yaml_string(&track.codec))?;
    writeln!(
        writer,
        "  codec_family: {}",
        yaml_string(&track.codec_family)
    )?;
    writeln!(writer, "  encrypted: {}", track.encrypted)?;
    if let Some(handler_type) = track.handler_type.as_ref() {
        writeln!(writer, "  handler_type: {}", yaml_string(handler_type))?;
    }
    if let Some(language) = track.language.as_ref() {
        writeln!(writer, "  language: {}", yaml_string(language))?;
    }
    if let Some(sample_entry_type) = track.sample_entry_type.as_ref() {
        writeln!(
            writer,
            "  sample_entry_type: {}",
            yaml_string(sample_entry_type)
        )?;
    }
    if let Some(original_format) = track.original_format.as_ref() {
        writeln!(
            writer,
            "  original_format: {}",
            yaml_string(original_format)
        )?;
    }
    if let Some(protection_scheme_type) = track.protection_scheme_type.as_ref() {
        writeln!(
            writer,
            "  protection_scheme_type: {}",
            yaml_string(protection_scheme_type)
        )?;
    }
    if let Some(protection_scheme_version) = track.protection_scheme_version {
        writeln!(
            writer,
            "  protection_scheme_version: {protection_scheme_version}"
        )?;
    }
    if let Some(width) = track.width {
        writeln!(writer, "  width: {width}")?;
    }
    if let Some(height) = track.height {
        writeln!(writer, "  height: {height}")?;
    }
    if let Some(channel_count) = track.channel_count {
        writeln!(writer, "  channel_count: {channel_count}")?;
    }
    if let Some(sample_rate) = track.sample_rate {
        writeln!(writer, "  sample_rate: {sample_rate}")?;
    }
    if let Some(sample_num) = track.sample_num {
        writeln!(writer, "  sample_num: {sample_num}")?;
    }
    if let Some(chunk_num) = track.chunk_num {
        writeln!(writer, "  chunk_num: {chunk_num}")?;
    }
    if let Some(idr_frame_num) = track.idr_frame_num {
        writeln!(writer, "  idr_frame_num: {idr_frame_num}")?;
    }
    if let Some(bitrate) = track.bitrate {
        writeln!(writer, "  bitrate: {bitrate}")?;
    }
    if let Some(max_bitrate) = track.max_bitrate {
        writeln!(writer, "  max_bitrate: {max_bitrate}")?;
    }
    Ok(())
}

fn write_yaml_codec_detailed_report<W>(
    writer: &mut W,
    report: &CodecDetailedProbeReport,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "major_brand: {}", yaml_string(&report.major_brand))?;
    writeln!(writer, "minor_version: {}", report.minor_version)?;
    writeln!(writer, "compatible_brands:")?;
    for brand in &report.compatible_brands {
        writeln!(writer, "- {}", yaml_string(brand))?;
    }
    writeln!(writer, "fast_start: {}", report.fast_start)?;
    writeln!(writer, "timescale: {}", report.timescale)?;
    writeln!(writer, "duration: {}", report.duration)?;
    writeln!(
        writer,
        "duration_seconds: {}",
        format_seconds(report.duration_seconds)
    )?;
    writeln!(writer, "tracks:")?;
    for track in &report.tracks {
        write_yaml_codec_detailed_track(writer, track)?;
    }
    Ok(())
}

fn write_yaml_codec_detailed_track<W>(
    writer: &mut W,
    track: &CodecDetailedProbeTrackReport,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "- track_id: {}", track.track_id)?;
    writeln!(writer, "  timescale: {}", track.timescale)?;
    writeln!(writer, "  duration: {}", track.duration)?;
    writeln!(
        writer,
        "  duration_seconds: {}",
        format_seconds(track.duration_seconds)
    )?;
    writeln!(writer, "  codec: {}", yaml_string(&track.codec))?;
    writeln!(
        writer,
        "  codec_family: {}",
        yaml_string(&track.codec_family)
    )?;
    writeln!(writer, "  encrypted: {}", track.encrypted)?;
    if let Some(handler_type) = track.handler_type.as_ref() {
        writeln!(writer, "  handler_type: {}", yaml_string(handler_type))?;
    }
    if let Some(language) = track.language.as_ref() {
        writeln!(writer, "  language: {}", yaml_string(language))?;
    }
    if let Some(sample_entry_type) = track.sample_entry_type.as_ref() {
        writeln!(
            writer,
            "  sample_entry_type: {}",
            yaml_string(sample_entry_type)
        )?;
    }
    if let Some(original_format) = track.original_format.as_ref() {
        writeln!(
            writer,
            "  original_format: {}",
            yaml_string(original_format)
        )?;
    }
    if let Some(protection_scheme_type) = track.protection_scheme_type.as_ref() {
        writeln!(
            writer,
            "  protection_scheme_type: {}",
            yaml_string(protection_scheme_type)
        )?;
    }
    if let Some(protection_scheme_version) = track.protection_scheme_version {
        writeln!(
            writer,
            "  protection_scheme_version: {protection_scheme_version}"
        )?;
    }
    if let Some(width) = track.width {
        writeln!(writer, "  width: {width}")?;
    }
    if let Some(height) = track.height {
        writeln!(writer, "  height: {height}")?;
    }
    if let Some(channel_count) = track.channel_count {
        writeln!(writer, "  channel_count: {channel_count}")?;
    }
    if let Some(sample_rate) = track.sample_rate {
        writeln!(writer, "  sample_rate: {sample_rate}")?;
    }
    if let Some(sample_num) = track.sample_num {
        writeln!(writer, "  sample_num: {sample_num}")?;
    }
    if let Some(chunk_num) = track.chunk_num {
        writeln!(writer, "  chunk_num: {chunk_num}")?;
    }
    if let Some(idr_frame_num) = track.idr_frame_num {
        writeln!(writer, "  idr_frame_num: {idr_frame_num}")?;
    }
    if let Some(bitrate) = track.bitrate {
        writeln!(writer, "  bitrate: {bitrate}")?;
    }
    if let Some(max_bitrate) = track.max_bitrate {
        writeln!(writer, "  max_bitrate: {max_bitrate}")?;
    }
    writeln!(writer, "  codec_details:")?;
    for (name, value) in codec_detail_yaml_fields(&track.codec_family, &track.codec_details) {
        writeln!(writer, "    {name}: {value}")?;
    }
    Ok(())
}

fn write_yaml_media_characteristics_report<W>(
    writer: &mut W,
    report: &MediaCharacteristicsProbeReport,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "major_brand: {}", yaml_string(&report.major_brand))?;
    writeln!(writer, "minor_version: {}", report.minor_version)?;
    writeln!(writer, "compatible_brands:")?;
    for brand in &report.compatible_brands {
        writeln!(writer, "- {}", yaml_string(brand))?;
    }
    writeln!(writer, "fast_start: {}", report.fast_start)?;
    writeln!(writer, "timescale: {}", report.timescale)?;
    writeln!(writer, "duration: {}", report.duration)?;
    writeln!(
        writer,
        "duration_seconds: {}",
        format_seconds(report.duration_seconds)
    )?;
    writeln!(writer, "tracks:")?;
    for track in &report.tracks {
        write_yaml_media_characteristics_track(writer, track)?;
    }
    Ok(())
}

fn write_yaml_media_characteristics_track<W>(
    writer: &mut W,
    track: &MediaCharacteristicsProbeTrackReport,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "- track_id: {}", track.track_id)?;
    writeln!(writer, "  timescale: {}", track.timescale)?;
    writeln!(writer, "  duration: {}", track.duration)?;
    writeln!(
        writer,
        "  duration_seconds: {}",
        format_seconds(track.duration_seconds)
    )?;
    writeln!(writer, "  codec: {}", yaml_string(&track.codec))?;
    writeln!(
        writer,
        "  codec_family: {}",
        yaml_string(&track.codec_family)
    )?;
    writeln!(writer, "  encrypted: {}", track.encrypted)?;
    if let Some(handler_type) = track.handler_type.as_ref() {
        writeln!(writer, "  handler_type: {}", yaml_string(handler_type))?;
    }
    if let Some(language) = track.language.as_ref() {
        writeln!(writer, "  language: {}", yaml_string(language))?;
    }
    if let Some(sample_entry_type) = track.sample_entry_type.as_ref() {
        writeln!(
            writer,
            "  sample_entry_type: {}",
            yaml_string(sample_entry_type)
        )?;
    }
    if let Some(original_format) = track.original_format.as_ref() {
        writeln!(
            writer,
            "  original_format: {}",
            yaml_string(original_format)
        )?;
    }
    if let Some(protection_scheme_type) = track.protection_scheme_type.as_ref() {
        writeln!(
            writer,
            "  protection_scheme_type: {}",
            yaml_string(protection_scheme_type)
        )?;
    }
    if let Some(protection_scheme_version) = track.protection_scheme_version {
        writeln!(
            writer,
            "  protection_scheme_version: {protection_scheme_version}"
        )?;
    }
    if let Some(width) = track.width {
        writeln!(writer, "  width: {width}")?;
    }
    if let Some(height) = track.height {
        writeln!(writer, "  height: {height}")?;
    }
    if let Some(channel_count) = track.channel_count {
        writeln!(writer, "  channel_count: {channel_count}")?;
    }
    if let Some(sample_rate) = track.sample_rate {
        writeln!(writer, "  sample_rate: {sample_rate}")?;
    }
    if let Some(sample_num) = track.sample_num {
        writeln!(writer, "  sample_num: {sample_num}")?;
    }
    if let Some(chunk_num) = track.chunk_num {
        writeln!(writer, "  chunk_num: {chunk_num}")?;
    }
    if let Some(idr_frame_num) = track.idr_frame_num {
        writeln!(writer, "  idr_frame_num: {idr_frame_num}")?;
    }
    if let Some(bitrate) = track.bitrate {
        writeln!(writer, "  bitrate: {bitrate}")?;
    }
    if let Some(max_bitrate) = track.max_bitrate {
        writeln!(writer, "  max_bitrate: {max_bitrate}")?;
    }
    writeln!(writer, "  codec_details:")?;
    for (name, value) in codec_detail_yaml_fields(&track.codec_family, &track.codec_details) {
        writeln!(writer, "    {name}: {value}")?;
    }
    if has_media_characteristics(&track.media_characteristics) {
        writeln!(writer, "  media_characteristics:")?;
        if let Some(value) = track.media_characteristics.declared_bitrate.as_ref() {
            writeln!(writer, "    declared_bitrate:")?;
            writeln!(writer, "      buffer_size_db: {}", value.buffer_size_db)?;
            writeln!(writer, "      max_bitrate: {}", value.max_bitrate)?;
            writeln!(writer, "      avg_bitrate: {}", value.avg_bitrate)?;
        }
        if let Some(value) = track.media_characteristics.color.as_ref() {
            writeln!(writer, "    color:")?;
            writeln!(
                writer,
                "      colour_type: {}",
                yaml_string(&value.colour_type.to_string())
            )?;
            if let Some(colour_primaries) = value.colour_primaries {
                writeln!(writer, "      colour_primaries: {colour_primaries}")?;
            }
            if let Some(transfer_characteristics) = value.transfer_characteristics {
                writeln!(
                    writer,
                    "      transfer_characteristics: {transfer_characteristics}"
                )?;
            }
            if let Some(matrix_coefficients) = value.matrix_coefficients {
                writeln!(writer, "      matrix_coefficients: {matrix_coefficients}")?;
            }
            if let Some(full_range) = value.full_range {
                writeln!(writer, "      full_range: {full_range}")?;
            }
            if let Some(profile_size) = value.profile_size {
                writeln!(writer, "      profile_size: {profile_size}")?;
            }
            if let Some(unknown_size) = value.unknown_size {
                writeln!(writer, "      unknown_size: {unknown_size}")?;
            }
        }
        if let Some(value) = track.media_characteristics.pixel_aspect_ratio.as_ref() {
            writeln!(writer, "    pixel_aspect_ratio:")?;
            writeln!(writer, "      h_spacing: {}", value.h_spacing)?;
            writeln!(writer, "      v_spacing: {}", value.v_spacing)?;
        }
        if let Some(value) = track.media_characteristics.field_order.as_ref() {
            writeln!(writer, "    field_order:")?;
            writeln!(writer, "      field_count: {}", value.field_count)?;
            writeln!(writer, "      field_ordering: {}", value.field_ordering)?;
            writeln!(writer, "      interlaced: {}", value.interlaced)?;
        }
    }
    Ok(())
}

fn codec_detail_yaml_fields(
    codec_family: &str,
    details: &TrackCodecDetails,
) -> Vec<(&'static str, String)> {
    let mut fields = vec![("kind", yaml_string(codec_family))];
    match details {
        TrackCodecDetails::Unknown => {}
        TrackCodecDetails::Avc(details) => {
            fields.push((
                "configuration_version",
                details.configuration_version.to_string(),
            ));
            fields.push(("profile", details.profile.to_string()));
            fields.push((
                "profile_compatibility",
                details.profile_compatibility.to_string(),
            ));
            fields.push(("level", details.level.to_string()));
            fields.push(("length_size", details.length_size.to_string()));
            if let Some(chroma_format) = details.chroma_format {
                fields.push(("chroma_format", chroma_format.to_string()));
            }
            if let Some(bit_depth_luma) = details.bit_depth_luma {
                fields.push(("bit_depth_luma", bit_depth_luma.to_string()));
            }
            if let Some(bit_depth_chroma) = details.bit_depth_chroma {
                fields.push(("bit_depth_chroma", bit_depth_chroma.to_string()));
            }
        }
        TrackCodecDetails::Hevc(details) => {
            fields.push((
                "configuration_version",
                details.configuration_version.to_string(),
            ));
            fields.push(("profile_space", details.profile_space.to_string()));
            fields.push(("tier_flag", details.tier_flag.to_string()));
            fields.push(("profile_idc", details.profile_idc.to_string()));
            fields.push((
                "profile_compatibility_mask",
                details.profile_compatibility_mask.to_string(),
            ));
            fields.push((
                "constraint_indicator",
                yaml_u8_array(&details.constraint_indicator),
            ));
            fields.push(("level_idc", details.level_idc.to_string()));
            fields.push((
                "min_spatial_segmentation_idc",
                details.min_spatial_segmentation_idc.to_string(),
            ));
            fields.push(("parallelism_type", details.parallelism_type.to_string()));
            fields.push(("chroma_format_idc", details.chroma_format_idc.to_string()));
            fields.push(("bit_depth_luma", details.bit_depth_luma.to_string()));
            fields.push(("bit_depth_chroma", details.bit_depth_chroma.to_string()));
            fields.push(("avg_frame_rate", details.avg_frame_rate.to_string()));
            fields.push((
                "constant_frame_rate",
                details.constant_frame_rate.to_string(),
            ));
            fields.push((
                "num_temporal_layers",
                details.num_temporal_layers.to_string(),
            ));
            fields.push(("temporal_id_nested", details.temporal_id_nested.to_string()));
            fields.push(("length_size", details.length_size.to_string()));
        }
        TrackCodecDetails::Av1(details) => {
            fields.push(("seq_profile", details.seq_profile.to_string()));
            fields.push(("seq_level_idx_0", details.seq_level_idx_0.to_string()));
            fields.push(("seq_tier_0", details.seq_tier_0.to_string()));
            fields.push(("bit_depth", details.bit_depth.to_string()));
            fields.push(("monochrome", details.monochrome.to_string()));
            fields.push((
                "chroma_subsampling_x",
                details.chroma_subsampling_x.to_string(),
            ));
            fields.push((
                "chroma_subsampling_y",
                details.chroma_subsampling_y.to_string(),
            ));
            fields.push((
                "chroma_sample_position",
                details.chroma_sample_position.to_string(),
            ));
            if let Some(delay) = details.initial_presentation_delay_minus_one {
                fields.push(("initial_presentation_delay_minus_one", delay.to_string()));
            }
        }
        TrackCodecDetails::Vp8(details) | TrackCodecDetails::Vp9(details) => {
            fields.push(("profile", details.profile.to_string()));
            fields.push(("level", details.level.to_string()));
            fields.push(("bit_depth", details.bit_depth.to_string()));
            fields.push(("chroma_subsampling", details.chroma_subsampling.to_string()));
            fields.push(("full_range", details.full_range.to_string()));
            fields.push(("colour_primaries", details.colour_primaries.to_string()));
            fields.push((
                "transfer_characteristics",
                details.transfer_characteristics.to_string(),
            ));
            fields.push((
                "matrix_coefficients",
                details.matrix_coefficients.to_string(),
            ));
            fields.push((
                "codec_initialization_data_size",
                details.codec_initialization_data_size.to_string(),
            ));
        }
        TrackCodecDetails::Mp4Audio(details) => {
            fields.push((
                "object_type_indication",
                details.object_type_indication.to_string(),
            ));
            fields.push(("audio_object_type", details.audio_object_type.to_string()));
            fields.push(("channel_count", details.channel_count.to_string()));
            if let Some(sample_rate) = details.sample_rate {
                fields.push(("sample_rate", sample_rate.to_string()));
            }
        }
        TrackCodecDetails::Opus(details) => {
            fields.push((
                "output_channel_count",
                details.output_channel_count.to_string(),
            ));
            fields.push(("pre_skip", details.pre_skip.to_string()));
            fields.push(("input_sample_rate", details.input_sample_rate.to_string()));
            fields.push(("output_gain", details.output_gain.to_string()));
            fields.push((
                "channel_mapping_family",
                details.channel_mapping_family.to_string(),
            ));
            if let Some(stream_count) = details.stream_count {
                fields.push(("stream_count", stream_count.to_string()));
            }
            if let Some(coupled_count) = details.coupled_count {
                fields.push(("coupled_count", coupled_count.to_string()));
            }
            if !details.channel_mapping.is_empty() {
                fields.push(("channel_mapping", yaml_u8_array(&details.channel_mapping)));
            }
        }
        TrackCodecDetails::Ac3(details) => {
            fields.push(("sample_rate_code", details.sample_rate_code.to_string()));
            fields.push((
                "bit_stream_identification",
                details.bit_stream_identification.to_string(),
            ));
            fields.push(("bit_stream_mode", details.bit_stream_mode.to_string()));
            fields.push(("audio_coding_mode", details.audio_coding_mode.to_string()));
            fields.push(("lfe_on", details.lfe_on.to_string()));
            fields.push(("bit_rate_code", details.bit_rate_code.to_string()));
        }
        TrackCodecDetails::Pcm(details) => {
            fields.push(("format_flags", details.format_flags.to_string()));
            fields.push(("sample_size", details.sample_size.to_string()));
        }
        TrackCodecDetails::XmlSubtitle(details) => {
            fields.push(("namespace", yaml_string(&details.namespace)));
            fields.push(("schema_location", yaml_string(&details.schema_location)));
            fields.push((
                "auxiliary_mime_types",
                yaml_string(&details.auxiliary_mime_types),
            ));
        }
        TrackCodecDetails::TextSubtitle(details) => {
            fields.push(("content_encoding", yaml_string(&details.content_encoding)));
            fields.push(("mime_format", yaml_string(&details.mime_format)));
        }
        TrackCodecDetails::WebVtt(details) => {
            if let Some(config) = details.config.as_ref() {
                fields.push(("config", yaml_string(config)));
            }
            if let Some(source_label) = details.source_label.as_ref() {
                fields.push(("source_label", yaml_string(source_label)));
            }
        }
    }

    fields
}

fn has_media_characteristics(characteristics: &TrackMediaCharacteristics) -> bool {
    characteristics.declared_bitrate.is_some()
        || characteristics.color.is_some()
        || characteristics.pixel_aspect_ratio.is_some()
        || characteristics.field_order.is_some()
}

fn json_u8_array(values: &[u8]) -> String {
    let mut rendered = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            rendered.push_str(", ");
        }
        rendered.push_str(&value.to_string());
    }
    rendered.push(']');
    rendered
}

fn yaml_u8_array(values: &[u8]) -> String {
    json_u8_array(values)
}

fn json_string(value: &str) -> String {
    let mut escaped = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn yaml_string(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

fn format_seconds(value: f32) -> String {
    let mut rendered = format!("{value:.6}");
    while rendered.contains('.') && rendered.ends_with('0') {
        rendered.pop();
    }
    if rendered.ends_with('.') {
        rendered.pop();
    }
    if rendered.is_empty() {
        "0".to_string()
    } else {
        rendered
    }
}

/// Errors raised while parsing arguments, probing files, or rendering probe output.
#[derive(Debug)]
pub enum ProbeCliError {
    Io(io::Error),
    Probe(ProbeError),
    InvalidArgument(String),
    UsageRequested,
}

impl fmt::Display for ProbeCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Probe(error) => error.fmt(f),
            Self::InvalidArgument(message) => f.write_str(message),
            Self::UsageRequested => f.write_str("usage requested"),
        }
    }
}

impl Error for ProbeCliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Probe(error) => Some(error),
            Self::InvalidArgument(..) | Self::UsageRequested => None,
        }
    }
}

impl From<io::Error> for ProbeCliError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ProbeError> for ProbeCliError {
    fn from(value: ProbeError) -> Self {
        Self::Probe(value)
    }
}
