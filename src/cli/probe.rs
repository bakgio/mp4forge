//! Probe command support and stable report rendering.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, Write};

use crate::probe::{
    ProbeError, TrackCodec, average_sample_bitrate, average_segment_bitrate, find_idr_frames,
    max_sample_bitrate, max_segment_bitrate, probe,
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

/// Top-level probe report used by the CLI layer.
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
    Ok(())
}

/// Builds a CLI probe report from an MP4 reader.
pub fn build_report<R>(reader: &mut R) -> Result<ProbeReport, ProbeCliError>
where
    R: Read + Seek,
{
    let summary = probe(reader)?;

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
        let mut bitrate = average_sample_bitrate(&track.samples, track.timescale);
        let mut max_bitrate =
            max_sample_bitrate(&track.samples, track.timescale, track.timescale.into());
        if bitrate == 0 || max_bitrate == 0 {
            bitrate = average_segment_bitrate(&summary.segments, track.track_id, track.timescale);
            max_bitrate = max_segment_bitrate(&summary.segments, track.track_id, track.timescale);
        }

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
            row.idr_frame_num = idr_frame_count(reader, track)?;
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

fn run_inner<W>(args: &[String], stdout: &mut W) -> Result<(), ProbeCliError>
where
    W: Write,
{
    let mut format = ProbeFormat::Json;
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
    let report = build_report(&mut file)?;
    write_report(stdout, &report, format)
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
