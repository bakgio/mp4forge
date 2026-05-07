//! Mux command support.

use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;
use std::str::FromStr;

use crate::mux::{
    MuxDestinationMode, MuxDurationMode, MuxError, MuxOutputLayout, MuxRequest, MuxTrackSpec,
    mux_into_path, mux_to_path,
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
        "USAGE: mp4forge mux --track <SPEC> [--track <SPEC> ...] [--layout <flat|fragmented>] [--segment_duration <SECONDS> | --fragment_duration <SECONDS>] [--out <PATH>] [DEST]"
    )?;
    writeln!(writer)?;
    writeln!(writer, "OPTIONS:")?;
    writeln!(
        writer,
        "  --track <SPEC>                Add one mux input using the path-first track-spec grammar"
    )?;
    writeln!(writer, "                               Path only: PATH")?;
    writeln!(
        writer,
        "                               Select one MP4 track when needed with: PATH#video, PATH#audio, PATH#audio:N, PATH#text, PATH#text:N, PATH#track:ID"
    )?;
    writeln!(
        writer,
        "                               Current path-only auto-detection covers MP4, VobSub, supported AVI audio streams plus H.263/JPEG/PNG/MPEG-4 Part 2/H.264/AVC1 video streams, supported MPEG-PS MPEG audio streams plus MPEG-4 Part 2/H.264/H.265/VVC video streams, supported MPEG-TS MPEG audio streams plus AC-3/E-AC-3 audio plus MPEG-4 Part 2/H.264/H.265/VVC video streams, AAC ADTS, AAC LATM, MP3, AC-3, E-AC-3, AC-4, AMR, AMR-WB, QCP voice audio, DTS core audio, Dolby TrueHD, leading-sync MHAS MPEG-H, IAMF, H.263 elementary video, MPEG-4 Part 2 elementary video, H.264 Annex B, H.265 Annex B, VVC Annex B, IVF AV1/VP8/VP9/VP10, JPEG still images, PNG still images, WAVE/AIFF/AIFC PCM, native FLAC, Ogg FLAC, Ogg Opus, Ogg Vorbis, Ogg Speex, Ogg Theora, and CAF ALAC"
    )?;
    writeln!(
        writer,
        "                               Broader DTS-family sample-entry variants remain supported through MP4 track import"
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
    writeln!(
        writer,
        "  --out <PATH>                 Force one newly created output destination at PATH"
    )?;
    writeln!(writer)?;
    writeln!(
        writer,
        "The current mux command supports at most one video track plus one or more audio and text/subtitle tracks. One positional DEST path follows the update-or-create destination flow: if DEST is an existing MP4, its current tracks are preserved and the requested tracks are imported into it; otherwise DEST is treated as the newly created output file. `--out PATH` is the explicit force-new path. Flat output rejects duration modes. Fragmented output currently requires exactly one duration mode and should be paired with `--out PATH`. Path-only MP4 inputs import all supported tracks unless you add one selector suffix."
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
    target: MuxCliTarget,
}

enum MuxCliTarget {
    Destination(PathBuf),
    Out(PathBuf),
}

fn run_inner(args: &[String]) -> Result<(), MuxCliError> {
    let parsed = parse_args(args)?;
    match parsed.target {
        MuxCliTarget::Destination(destination_path) => {
            mux_into_path(&parsed.request, &destination_path)?
        }
        MuxCliTarget::Out(output_path) => mux_to_path(&parsed.request, &output_path)?,
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<ParsedMuxArgs, MuxCliError> {
    let mut tracks = Vec::new();
    let mut output_layout = MuxOutputLayout::Flat;
    let mut destination_mode = MuxDestinationMode::UpdateOrCreateDestination;
    let mut duration_mode = None::<MuxDurationMode>;
    let mut out_path = None::<PathBuf>;
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
            "--out" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(MuxCliError::InvalidArgument(
                        "missing value for --out".to_string(),
                    ));
                };
                if out_path.is_some() {
                    return Err(MuxCliError::InvalidArgument(
                        "--out may only be supplied once".to_string(),
                    ));
                }
                out_path = Some(PathBuf::from(value));
                destination_mode = MuxDestinationMode::CreateNew;
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

    if tracks.is_empty() {
        return Err(MuxCliError::UsageRequested);
    }
    let target = match (out_path, positional.len()) {
        (Some(path), 0) => MuxCliTarget::Out(path),
        (Some(_), _) => {
            return Err(MuxCliError::InvalidArgument(
                "--out <PATH> may not be used together with a positional DEST path".to_string(),
            ));
        }
        (None, 1) => MuxCliTarget::Destination(positional.remove(0)),
        (None, _) => return Err(MuxCliError::UsageRequested),
    };

    let mut request = MuxRequest::new(tracks)
        .with_output_layout(output_layout)
        .with_destination_mode(destination_mode);
    if let Some(duration_mode) = duration_mode {
        request = request.with_duration_mode(duration_mode);
    }

    Ok(ParsedMuxArgs { request, target })
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
