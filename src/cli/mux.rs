//! Mux command support.

use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;
use std::str::FromStr;

use crate::mux::{
    MuxDurationMode, MuxError, MuxOutputLayout, MuxRequest, MuxTrackSpec, mux_to_path,
};

/// Runs the mux subcommand with `args`, writing failures to `stderr`.
pub fn run<E>(args: &[String], stderr: &mut E) -> i32
where
    E: Write,
{
    match run_inner(args) {
        Ok(()) => 0,
        Err(MuxCliError::UsageRequested) => {
            let _ = write_usage(stderr);
            1
        }
        Err(error) => {
            let _ = writeln!(stderr, "Error: {error}");
            1
        }
    }
}

/// Writes the mux subcommand usage text.
pub fn write_usage<W>(writer: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(
        writer,
        "USAGE: mp4forge mux --track <SPEC> [--track <SPEC> ...] [--layout <flat|fragmented>] [--segment_duration <SECONDS> | --fragment_duration <SECONDS>] OUTPUT"
    )?;
    writeln!(writer)?;
    writeln!(writer, "OPTIONS:")?;
    writeln!(
        writer,
        "  --track <SPEC>                Add one mux input using the widened track-spec grammar"
    )?;
    writeln!(
        writer,
        "                               Raw: <codec>:PATH[#key=value[,key=value...]]"
    )?;
    writeln!(
        writer,
        "                               Some raw codecs require explicit layout parameters such as width/height or sample_rate/channel_count"
    )?;
    writeln!(
        writer,
        "                               MP4: PATH.mp4#video, PATH.mp4#audio, PATH.mp4#audio:N, PATH.mp4#text, PATH.mp4#text:N, PATH.mp4#track:ID"
    )?;
    writeln!(
        writer,
        "  --segment_duration <SECONDS> Set one target segment duration for supported single-input jobs"
    )?;
    writeln!(
        writer,
        "  --fragment_duration <SECONDS> Set one target fragment duration for supported single-input jobs"
    )?;
    writeln!(
        writer,
        "  --layout <flat|fragmented>   Choose the output container layout; defaults to flat"
    )?;
    writeln!(writer)?;
    writeln!(
        writer,
        "The current mux command supports at most one video track plus one or more audio and text/subtitle tracks and always writes one explicit output MP4 file. Flat output rejects duration modes. Fragmented output currently requires exactly one duration mode."
    )
}

#[derive(Debug)]
enum MuxCliError {
    Mux(MuxError),
    InvalidArgument(String),
    UsageRequested,
}

impl fmt::Display for MuxCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mux(error) => error.fmt(f),
            Self::InvalidArgument(message) => f.write_str(message),
            Self::UsageRequested => f.write_str("usage requested"),
        }
    }
}

impl Error for MuxCliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Mux(error) => Some(error),
            Self::InvalidArgument(..) | Self::UsageRequested => None,
        }
    }
}

impl From<MuxError> for MuxCliError {
    fn from(value: MuxError) -> Self {
        Self::Mux(value)
    }
}

struct ParsedMuxArgs {
    request: MuxRequest,
    output_path: PathBuf,
}

fn run_inner(args: &[String]) -> Result<(), MuxCliError> {
    let parsed = parse_args(args)?;
    mux_to_path(&parsed.request, &parsed.output_path)?;
    Ok(())
}

fn parse_args(args: &[String]) -> Result<ParsedMuxArgs, MuxCliError> {
    let mut tracks = Vec::new();
    let mut output_layout = MuxOutputLayout::Flat;
    let mut duration_mode = None::<MuxDurationMode>;
    let mut positional = Vec::new();
    let mut index = 0usize;

    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => return Err(MuxCliError::UsageRequested),
            "--track" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(MuxCliError::InvalidArgument(
                        "missing value for --track".to_string(),
                    ));
                };
                tracks.push(MuxTrackSpec::from_str(value).map_err(MuxCliError::from)?);
                index += 2;
            }
            "--segment_duration" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(MuxCliError::InvalidArgument(
                        "missing value for --segment_duration".to_string(),
                    ));
                };
                set_duration_mode(
                    &mut duration_mode,
                    MuxDurationMode::Segment {
                        seconds: parse_seconds("--segment_duration", value)?,
                    },
                )?;
                index += 2;
            }
            "--fragment_duration" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(MuxCliError::InvalidArgument(
                        "missing value for --fragment_duration".to_string(),
                    ));
                };
                set_duration_mode(
                    &mut duration_mode,
                    MuxDurationMode::Fragment {
                        seconds: parse_seconds("--fragment_duration", value)?,
                    },
                )?;
                index += 2;
            }
            "--layout" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(MuxCliError::InvalidArgument(
                        "missing value for --layout".to_string(),
                    ));
                };
                output_layout = parse_layout(value)?;
                index += 2;
            }
            value if value.starts_with('-') => {
                return Err(MuxCliError::InvalidArgument(format!(
                    "unknown mux option: {value}"
                )));
            }
            value => {
                positional.push(PathBuf::from(value));
                index += 1;
            }
        }
    }

    if positional.len() != 1 {
        return Err(MuxCliError::UsageRequested);
    }
    if tracks.is_empty() {
        return Err(MuxCliError::InvalidArgument(
            "at least one --track <SPEC> is required".to_string(),
        ));
    }

    let mut request = MuxRequest::new(tracks).with_output_layout(output_layout);
    if let Some(duration_mode) = duration_mode {
        request = request.with_duration_mode(duration_mode);
    }

    Ok(ParsedMuxArgs {
        request,
        output_path: positional.remove(0),
    })
}

fn set_duration_mode(
    current: &mut Option<MuxDurationMode>,
    next: MuxDurationMode,
) -> Result<(), MuxCliError> {
    if let Some(existing) = current {
        return Err(MuxCliError::InvalidArgument(format!(
            "--{} and --{} may not be used together",
            existing.label(),
            next.label()
        )));
    }
    *current = Some(next);
    Ok(())
}

fn parse_seconds(option: &str, value: &str) -> Result<f64, MuxCliError> {
    value.parse::<f64>().map_err(|_| {
        MuxCliError::InvalidArgument(format!(
            "invalid value for {option}: expected a floating-point duration in seconds"
        ))
    })
}

fn parse_layout(value: &str) -> Result<MuxOutputLayout, MuxCliError> {
    match value {
        "flat" => Ok(MuxOutputLayout::Flat),
        "fragmented" => Ok(MuxOutputLayout::Fragmented),
        _ => Err(MuxCliError::InvalidArgument(
            "invalid value for --layout: expected `flat` or `fragmented`".to_string(),
        )),
    }
}
