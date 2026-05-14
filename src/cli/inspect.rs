//! Direct-ingest inspection command support.

use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;

use super::{write_error_line, write_warning_lines};
use crate::mux::MuxError;
use crate::mux::inspect::{
    DirectIngestPacketReport, DirectIngestReport, DirectIngestReportFormat,
    collect_packet_report_warnings, collect_track_report_warnings, inspect_direct_ingest_packets,
    inspect_direct_ingest_path, write_packet_report, write_report,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectView {
    Tracks,
    Packets,
}

/// Runs the direct-ingest inspection subcommand with `args`, writing output to `stdout`.
pub fn run<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    match run_inner(args, stdout, stderr) {
        Ok(()) => 0,
        Err(InspectCliError::UsageRequested) => {
            let _ = write_usage(stderr);
            1
        }
        Err(error) => {
            let _ = write_error_line(stderr, &error, error.diagnostic_context());
            1
        }
    }
}

/// Writes the direct-ingest inspection subcommand usage text.
pub fn write_usage<W>(writer: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "USAGE: mp4forge inspect [OPTIONS] INPUT")?;
    writeln!(writer)?;
    writeln!(writer, "OPTIONS:")?;
    writeln!(
        writer,
        "  -format <json|yaml|nhml|nhnt>  Output format (default: json)"
    )?;
    writeln!(
        writer,
        "  -view <tracks|packets>  Inspection view (default: tracks)"
    )?;
    writeln!(
        writer,
        "  -warnings  Emit warning-grade diagnostics to stderr after a successful report"
    )?;
    Ok(())
}

/// Builds one direct-ingest inspection report for `input_path`.
pub fn build_report(input_path: &PathBuf) -> Result<DirectIngestReport, InspectCliError> {
    inspect_direct_ingest_path(input_path).map_err(InspectCliError::Mux)
}

/// Builds one packet-focused direct-ingest inspection report for `input_path`.
pub fn build_packet_report(
    input_path: &PathBuf,
) -> Result<DirectIngestPacketReport, InspectCliError> {
    inspect_direct_ingest_packets(input_path).map_err(InspectCliError::Mux)
}

/// Writes one direct-ingest inspection report in the requested structured format.
pub fn write_inspection_report<W>(
    writer: &mut W,
    report: &DirectIngestReport,
    format: DirectIngestReportFormat,
) -> io::Result<()>
where
    W: Write,
{
    write_report(writer, report, format)
}

/// Writes one packet-focused direct-ingest inspection report in the requested structured format.
pub fn write_packet_inspection_report<W>(
    writer: &mut W,
    report: &DirectIngestPacketReport,
    format: DirectIngestReportFormat,
) -> io::Result<()>
where
    W: Write,
{
    write_packet_report(writer, report, format)
}

#[derive(Debug)]
pub enum InspectCliError {
    UsageRequested,
    InvalidArgument(String),
    Mux(MuxError),
    Io(io::Error),
}

impl fmt::Display for InspectCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UsageRequested => write!(f, "usage requested"),
            Self::InvalidArgument(message) => write!(f, "{message}"),
            Self::Mux(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for InspectCliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Mux(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for InspectCliError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl InspectCliError {
    fn diagnostic_context(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::UsageRequested => None,
            Self::InvalidArgument(..) => Some(("request", "input")),
            Self::Mux(error) => Some((error.stage(), error.category())),
            Self::Io(..) => Some(("io", "io")),
        }
    }
}

fn run_inner<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> Result<(), InspectCliError>
where
    W: Write,
    E: Write,
{
    let (input_path, format, view, emit_warnings) = parse_args(args)?;
    validate_view_format(view, format)?;
    match view {
        InspectView::Tracks => {
            let report = build_report(&input_path)?;
            write_inspection_report(stdout, &report, format)?;
            if emit_warnings {
                write_warning_lines(stderr, &collect_track_report_warnings(&report))?;
            }
        }
        InspectView::Packets => {
            let report = build_packet_report(&input_path)?;
            write_packet_inspection_report(stdout, &report, format)?;
            if emit_warnings {
                write_warning_lines(stderr, &collect_packet_report_warnings(&report))?;
            }
        }
    }
    Ok(())
}

fn parse_args(
    args: &[String],
) -> Result<(PathBuf, DirectIngestReportFormat, InspectView, bool), InspectCliError> {
    let mut format = DirectIngestReportFormat::Json;
    let mut view = InspectView::Tracks;
    let mut emit_warnings = false;
    let mut input_path = None::<PathBuf>;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" | "-help" => return Err(InspectCliError::UsageRequested),
            "-warnings" | "--warnings" => {
                emit_warnings = true;
            }
            "-format" | "--format" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(InspectCliError::InvalidArgument(
                        "missing value for `-format`".to_string(),
                    ));
                };
                format = match value.as_str() {
                    "json" => DirectIngestReportFormat::Json,
                    "yaml" => DirectIngestReportFormat::Yaml,
                    "nhml" => DirectIngestReportFormat::Nhml,
                    "nhnt" => DirectIngestReportFormat::Nhnt,
                    other => {
                        return Err(InspectCliError::InvalidArgument(format!(
                            "unsupported inspect format: {other}"
                        )));
                    }
                };
            }
            "-view" | "--view" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(InspectCliError::InvalidArgument(
                        "missing value for `-view`".to_string(),
                    ));
                };
                view = match value.as_str() {
                    "tracks" => InspectView::Tracks,
                    "packets" => InspectView::Packets,
                    other => {
                        return Err(InspectCliError::InvalidArgument(format!(
                            "unsupported inspect view: {other}"
                        )));
                    }
                };
            }
            value if value.starts_with('-') => {
                return Err(InspectCliError::InvalidArgument(format!(
                    "unsupported inspect option: {value}"
                )));
            }
            value => {
                if input_path.is_some() {
                    return Err(InspectCliError::InvalidArgument(
                        "inspect accepts exactly one input path".to_string(),
                    ));
                }
                input_path = Some(PathBuf::from(value));
            }
        }
        index += 1;
    }
    let Some(input_path) = input_path else {
        return Err(InspectCliError::UsageRequested);
    };
    Ok((input_path, format, view, emit_warnings))
}

fn validate_view_format(
    view: InspectView,
    format: DirectIngestReportFormat,
) -> Result<(), InspectCliError> {
    match (view, format) {
        (InspectView::Tracks, DirectIngestReportFormat::Nhnt) => Err(
            InspectCliError::InvalidArgument("NHNT output requires `-view packets`".to_string()),
        ),
        (InspectView::Packets, DirectIngestReportFormat::Nhml) => Err(
            InspectCliError::InvalidArgument("NHML output requires `-view tracks`".to_string()),
        ),
        _ => Ok(()),
    }
}
