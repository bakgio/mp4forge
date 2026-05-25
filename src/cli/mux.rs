//! Mux command support.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use super::{format_post_run_diagnostics_unavailable, write_error_line, write_warning_lines};
use crate::cli::divide::collect_fragmented_file_warnings;
use crate::mux::{
    MuxDestinationMode, MuxDurationMode, MuxError, MuxOutputLayout, MuxRequest, MuxTrackSpec,
    mux_fragmented_to_paths, mux_into_path, mux_to_path,
};

/// Runs the mux subcommand with `args`, writing failures to `stderr`.
pub fn run<E>(args: &[String], stderr: &mut E) -> i32
where
    E: Write,
{
    match run_inner(args, stderr) {
        Ok(()) => 0,
        Err(MuxCliError::UsageRequested) => {
            let _ = write_usage(stderr);
            1
        }
        Err(error) => {
            let _ = write_error_line(stderr, &error, error.diagnostic_context());
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
        "USAGE: mp4forge mux --track <SPEC> [--track <SPEC> ...] [--layout <flat|fragmented>] [--segment_duration <SECONDS> | --fragment_duration <SECONDS>] [--out <PATH> | --init_out <PATH> --media_out <PATH>] [DEST]"
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
        "                               Current path-only auto-detection covers MP4, VobSub, supported AVI audio streams plus H.263/JPEG/PNG/MPEG-4 Part 2/H.264/AVC1 video streams, supported MPEG-PS MPEG audio streams plus LPCM audio plus MPEG-4 Part 2/H.264/H.265/VVC video streams, supported MPEG-TS MPEG audio streams plus AAC LATM/MHAS plus AC-3/E-AC-3/AC-4/DTS/TrueHD audio plus MPEG-2/AV1/AVS3/MPEG-4 Part 2/H.264/H.265/VVC video streams, AAC ADTS, AAC LATM, MP3, AC-3, E-AC-3, AC-4, AMR, AMR-WB, QCP voice audio, DTS-family core audio, Dolby TrueHD, leading-sync MHAS MPEG-H, IAMF, H.263 elementary video, MPEG-2 elementary video, MPEG-4 Part 2 elementary video, H.264 Annex B, H.265 Annex B, VVC Annex B, raw AV1 OBU, raw AV1 Annex B, IVF AV1/VP8/VP9/VP10, JPEG still images, PNG still images, WAVE/AIFF/AIFC PCM, native FLAC, Ogg FLAC, Ogg Opus, Ogg Vorbis, Ogg Speex, Ogg Theora, and CAF ALAC"
    )?;
    writeln!(
        writer,
        "                               Broader DTS-family sample-entry variants remain supported through MP4 track import"
    )?;
    writeln!(
        writer,
        "  --segment_duration <SECONDS> Set one target segment duration for supported fragmented jobs"
    )?;
    writeln!(
        writer,
        "  --fragment_duration <SECONDS> Set one target fragment duration for supported fragmented jobs"
    )?;
    writeln!(
        writer,
        "  --layout <flat|fragmented>   Choose the output container layout; defaults to flat"
    )?;
    writeln!(
        writer,
        "  --out <PATH>                 Force one newly created output destination at PATH"
    )?;
    writeln!(
        writer,
        "  --init_out <PATH>            Write fragmented initialization boxes to PATH"
    )?;
    writeln!(
        writer,
        "  --media_out <PATH>           Write fragmented index and media fragments to PATH"
    )?;
    writeln!(
        writer,
        "  -warnings                    Emit warning-grade diagnostics to stderr after a successful run"
    )?;
    writeln!(writer)?;
    writeln!(
        writer,
        "The current mux command supports at most one video track plus one or more audio and text/subtitle tracks. One positional DEST path follows the update-or-create destination flow: if DEST is an existing MP4, its current tracks are preserved and the requested tracks are imported into it; otherwise DEST is treated as the newly created output file. `--out PATH` is the explicit force-new path. `--init_out PATH --media_out PATH` writes a fragmented job as separate outputs. Flat output rejects duration modes. Fragmented output currently requires exactly one duration mode and should be paired with `--out PATH` or separate fragmented outputs. Path-only MP4 inputs import all supported tracks unless you add one selector suffix."
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

impl MuxCliError {
    fn diagnostic_context(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Mux(error) => Some((error.stage(), error.category())),
            Self::InvalidArgument(..) => Some(("request", "input")),
            Self::UsageRequested => None,
        }
    }
}

struct ParsedMuxArgs {
    request: MuxRequest,
    target: MuxCliTarget,
    emit_warnings: bool,
}

enum MuxCliTarget {
    Destination(PathBuf),
    Out(PathBuf),
    Split {
        init_path: PathBuf,
        media_path: PathBuf,
    },
}

fn run_inner<E>(args: &[String], stderr: &mut E) -> Result<(), MuxCliError>
where
    E: Write,
{
    let parsed = parse_args(args)?;
    let output_path = parsed.target.primary_output_path().to_path_buf();
    let output_layout = parsed.request.output_layout();
    let warnings_target_supported = matches!(
        &parsed.target,
        MuxCliTarget::Out(_) | MuxCliTarget::Destination(_)
    );
    match &parsed.target {
        MuxCliTarget::Destination(destination_path) => {
            mux_into_path(&parsed.request, destination_path)?
        }
        MuxCliTarget::Out(output_path) => mux_to_path(&parsed.request, output_path)?,
        MuxCliTarget::Split {
            init_path,
            media_path,
        } => mux_fragmented_to_paths(&parsed.request, init_path, media_path)?,
    }
    if parsed.emit_warnings
        && matches!(output_layout, MuxOutputLayout::Fragmented)
        && warnings_target_supported
    {
        emit_fragmented_mux_warnings(&output_path, stderr);
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<ParsedMuxArgs, MuxCliError> {
    let mut tracks = Vec::new();
    let mut output_layout = MuxOutputLayout::Flat;
    let mut destination_mode = MuxDestinationMode::UpdateOrCreateDestination;
    let mut duration_mode = None::<MuxDurationMode>;
    let mut out_path = None::<PathBuf>;
    let mut init_out_path = None::<PathBuf>;
    let mut media_out_path = None::<PathBuf>;
    let mut emit_warnings = false;
    let mut positional = Vec::new();
    let mut index = 0usize;

    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => return Err(MuxCliError::UsageRequested),
            "-warnings" | "--warnings" => {
                emit_warnings = true;
                index += 1;
            }
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
            "--init_out" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(MuxCliError::InvalidArgument(
                        "missing value for --init_out".to_string(),
                    ));
                };
                if init_out_path.is_some() {
                    return Err(MuxCliError::InvalidArgument(
                        "--init_out may only be supplied once".to_string(),
                    ));
                }
                init_out_path = Some(PathBuf::from(value));
                destination_mode = MuxDestinationMode::CreateNew;
                index += 2;
            }
            "--media_out" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(MuxCliError::InvalidArgument(
                        "missing value for --media_out".to_string(),
                    ));
                };
                if media_out_path.is_some() {
                    return Err(MuxCliError::InvalidArgument(
                        "--media_out may only be supplied once".to_string(),
                    ));
                }
                media_out_path = Some(PathBuf::from(value));
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
    let target = match (out_path, init_out_path, media_out_path, positional.len()) {
        (Some(path), None, None, 0) => MuxCliTarget::Out(path),
        (Some(_), None, None, _) => {
            return Err(MuxCliError::InvalidArgument(
                "--out <PATH> may not be used together with a positional DEST path".to_string(),
            ));
        }
        (Some(_), _, _, 0) => {
            return Err(MuxCliError::InvalidArgument(
                "--out <PATH> may not be used together with separate outputs".to_string(),
            ));
        }
        (Some(_), _, _, _) => {
            return Err(MuxCliError::InvalidArgument(
                "--out <PATH> may not be used together with separate outputs or a positional DEST path".to_string(),
            ));
        }
        (None, Some(init_path), Some(media_path), 0) => MuxCliTarget::Split {
            init_path,
            media_path,
        },
        (None, Some(_), None, _) => {
            return Err(MuxCliError::InvalidArgument(
                "--init_out requires --media_out".to_string(),
            ));
        }
        (None, None, Some(_), _) => {
            return Err(MuxCliError::InvalidArgument(
                "--media_out requires --init_out".to_string(),
            ));
        }
        (None, Some(_), Some(_), _) => {
            return Err(MuxCliError::InvalidArgument(
                "separate fragmented outputs may not be used with a positional DEST path"
                    .to_string(),
            ));
        }
        (None, None, None, 1) => MuxCliTarget::Destination(positional.remove(0)),
        (None, None, None, _) => return Err(MuxCliError::UsageRequested),
    };

    let mut request = MuxRequest::new(tracks)
        .with_output_layout(output_layout)
        .with_destination_mode(destination_mode);
    if let Some(duration_mode) = duration_mode {
        request = request.with_duration_mode(duration_mode);
    }

    validate_mux_cli_request_shape(&request, &target)?;

    Ok(ParsedMuxArgs {
        request,
        target,
        emit_warnings,
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

fn validate_mux_cli_request_shape(
    request: &MuxRequest,
    target: &MuxCliTarget,
) -> Result<(), MuxCliError> {
    let output_path = match target {
        MuxCliTarget::Destination(path) | MuxCliTarget::Out(path) => path.as_path(),
        MuxCliTarget::Split { media_path, .. } => media_path.as_path(),
    };

    if matches!(target, MuxCliTarget::Split { .. })
        && !matches!(request.output_layout(), MuxOutputLayout::Fragmented)
    {
        return Err(MuxError::InvalidOutputLayout {
            layout: request.output_layout().label(),
            message: "separate fragmented output requires fragmented layout".to_string(),
        }
        .into());
    }

    if let MuxCliTarget::Split {
        init_path,
        media_path,
    } = target
        && absolute_cli_path(init_path)? == absolute_cli_path(media_path)?
    {
        return Err(MuxError::InvalidDestinationMode {
            mode: MuxDestinationMode::CreateNew.label(),
            message: "separate fragmented output paths must be distinct".to_string(),
        }
        .into());
    }

    match (request.output_layout(), request.duration_mode()) {
        (MuxOutputLayout::Flat, Some(duration_mode)) => {
            return Err(MuxError::InvalidOutputLayout {
                layout: request.output_layout().label(),
                message: format!(
                    "flat output does not support `--{}`; use `--layout fragmented` instead",
                    duration_mode.label()
                ),
            }
            .into());
        }
        (MuxOutputLayout::Fragmented, None) => {
            return Err(MuxError::InvalidOutputLayout {
                layout: request.output_layout().label(),
                message: "fragmented output requires exactly one of `--segment_duration` or `--fragment_duration`".to_string(),
            }
            .into());
        }
        _ => {}
    }

    if matches!(target, MuxCliTarget::Destination(_))
        && matches!(request.output_layout(), MuxOutputLayout::Fragmented)
        && is_existing_mp4_destination(output_path)
    {
        return Err(MuxError::InvalidDestinationMode {
            mode: request.destination_mode().label(),
            message: "the current destination-path mux mode only supports flat output; use `--out PATH` for create-new fragmented output".to_string(),
        }
        .into());
    }

    let video_count = request
        .tracks()
        .iter()
        .filter(|track| {
            matches!(
                track,
                MuxTrackSpec::Path {
                    selector: Some(crate::mux::MuxMp4TrackSelector::Video),
                    ..
                }
            )
        })
        .count();
    if video_count > 1 {
        return Err(MuxError::MultipleVideoTracks { count: video_count }.into());
    }

    let output_absolute = absolute_cli_path(output_path)?;
    for track in request.tracks() {
        let input_absolute = absolute_cli_path(track.input_path())?;
        if input_absolute == output_absolute {
            return Err(MuxError::OutputPathConflict {
                output: output_absolute,
                input: input_absolute,
            }
            .into());
        }
        if let MuxCliTarget::Split { init_path, .. } = target {
            let init_absolute = absolute_cli_path(init_path)?;
            if input_absolute == init_absolute {
                return Err(MuxError::OutputPathConflict {
                    output: init_absolute,
                    input: input_absolute,
                }
                .into());
            }
        }
    }

    Ok(())
}

fn absolute_cli_path(path: &Path) -> Result<PathBuf, MuxCliError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .map_err(MuxError::from)
        .map_err(MuxCliError::from)?
        .join(path))
}

fn is_existing_mp4_destination(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut prefix = [0_u8; 16];
    let Ok(read) = file.read(&mut prefix) else {
        return false;
    };
    read >= 8 && &prefix[4..8] == b"ftyp"
}

impl MuxCliTarget {
    fn primary_output_path(&self) -> &Path {
        match self {
            Self::Destination(path) | Self::Out(path) => path.as_path(),
            Self::Split { media_path, .. } => media_path.as_path(),
        }
    }
}

fn emit_fragmented_mux_warnings<E>(output_path: &Path, stderr: &mut E)
where
    E: Write,
{
    let warnings = match File::open(output_path) {
        Ok(mut file) => match collect_fragmented_file_warnings(&mut file) {
            Ok(warnings) => warnings,
            Err(error) => vec![format_post_run_diagnostics_unavailable(
                "fragmented output diagnostics",
                output_path,
                &error,
            )],
        },
        Err(error) => vec![format_post_run_diagnostics_unavailable(
            "fragmented output diagnostics",
            output_path,
            &error,
        )],
    };
    let _ = write_warning_lines(stderr, &warnings);
}
