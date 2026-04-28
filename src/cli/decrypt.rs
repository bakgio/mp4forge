//! Decrypt command support.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::decrypt::{
    DecryptError, DecryptOptions, DecryptProgress, DecryptProgressPhase, ParseDecryptionKeyError,
    decrypt_file, decrypt_file_with_progress,
};

/// Runs the decrypt subcommand with `args`, writing progress and failures to `stderr`.
pub fn run<E>(args: &[String], stderr: &mut E) -> i32
where
    E: Write,
{
    match run_inner(args, stderr) {
        Ok(()) => 0,
        Err(DecryptCliError::UsageRequested) => {
            let _ = write_usage(stderr);
            1
        }
        Err(error) => {
            let _ = writeln!(stderr, "Error: {error}");
            1
        }
    }
}

/// Writes the decrypt subcommand usage text.
pub fn write_usage<W>(writer: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(
        writer,
        "USAGE: mp4forge decrypt --key <ID:KEY> [--key <ID:KEY> ...] [--fragments-info FILE] [--show-progress] INPUT OUTPUT"
    )?;
    writeln!(writer)?;
    writeln!(writer, "OPTIONS:")?;
    writeln!(
        writer,
        "  --key <ID:KEY>             Add one decryption key addressed by decimal track ID or 128-bit KID"
    )?;
    writeln!(
        writer,
        "  --fragments-info <FILE>    Read matching initialization-segment bytes for standalone media-segment decrypt"
    )?;
    writeln!(
        writer,
        "  --show-progress            Write coarse decrypt progress snapshots to stderr"
    )?;
    writeln!(writer)?;
    writeln!(writer, "Key syntax:")?;
    writeln!(writer, "  --key <id>:<key>")?;
    writeln!(
        writer,
        "      <id> is either a track ID in decimal or a 128-bit KID in hex"
    )?;
    writeln!(writer, "      <key> is a 128-bit decryption key in hex")?;
    writeln!(
        writer,
        "      note: --fragments-info is typically the init segment when decrypting fragmented media segments"
    )
}

#[derive(Debug)]
enum DecryptCliError {
    Io(io::Error),
    Decrypt(DecryptError),
    ParseKey(ParseDecryptionKeyError),
    InvalidArgument(String),
    UsageRequested,
}

impl fmt::Display for DecryptCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Decrypt(error) => error.fmt(f),
            Self::ParseKey(error) => error.fmt(f),
            Self::InvalidArgument(message) => f.write_str(message),
            Self::UsageRequested => f.write_str("usage requested"),
        }
    }
}

impl Error for DecryptCliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Decrypt(error) => Some(error),
            Self::ParseKey(error) => Some(error),
            Self::InvalidArgument(..) | Self::UsageRequested => None,
        }
    }
}

impl From<io::Error> for DecryptCliError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<DecryptError> for DecryptCliError {
    fn from(value: DecryptError) -> Self {
        Self::Decrypt(value)
    }
}

impl From<ParseDecryptionKeyError> for DecryptCliError {
    fn from(value: ParseDecryptionKeyError) -> Self {
        Self::ParseKey(value)
    }
}

struct ParsedArgs {
    show_progress: bool,
    key_specs: Vec<String>,
    fragments_info: Option<PathBuf>,
    input: PathBuf,
    output: PathBuf,
}

fn run_inner<E>(args: &[String], stderr: &mut E) -> Result<(), DecryptCliError>
where
    E: Write,
{
    let parsed = parse_args(args)?;
    let mut options = DecryptOptions::new();
    for key_spec in &parsed.key_specs {
        options.add_key_spec(key_spec)?;
    }

    if let Some(path) = &parsed.fragments_info {
        options.set_fragments_info_bytes(fs::read(path)?);
    }

    if parsed.show_progress {
        decrypt_file_with_cli_progress(&parsed.input, &parsed.output, &options, stderr)
    } else {
        decrypt_file(&parsed.input, &parsed.output, &options).map_err(Into::into)
    }
}

fn parse_args(args: &[String]) -> Result<ParsedArgs, DecryptCliError> {
    let mut show_progress = false;
    let mut key_specs = Vec::new();
    let mut fragments_info = None;
    let mut positional = Vec::new();
    let mut index = 0usize;

    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => return Err(DecryptCliError::UsageRequested),
            "--show-progress" | "-show-progress" => {
                show_progress = true;
                index += 1;
            }
            "--key" | "-key" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DecryptCliError::InvalidArgument(
                        "missing value for --key".to_string(),
                    ));
                };
                key_specs.push(value.clone());
                index += 2;
            }
            "--fragments-info" | "-fragments-info" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DecryptCliError::InvalidArgument(
                        "missing value for --fragments-info".to_string(),
                    ));
                };
                if fragments_info.is_some() {
                    return Err(DecryptCliError::InvalidArgument(
                        "--fragments-info may only be provided once".to_string(),
                    ));
                }
                fragments_info = Some(PathBuf::from(value));
                index += 2;
            }
            value if value.starts_with('-') => {
                return Err(DecryptCliError::InvalidArgument(format!(
                    "unknown decrypt option: {value}"
                )));
            }
            value => {
                positional.push(PathBuf::from(value));
                index += 1;
            }
        }
    }

    if positional.len() != 2 {
        return Err(DecryptCliError::UsageRequested);
    }
    if key_specs.is_empty() {
        return Err(DecryptCliError::InvalidArgument(
            "at least one --key <ID:KEY> is required".to_string(),
        ));
    }

    Ok(ParsedArgs {
        show_progress,
        key_specs,
        fragments_info,
        input: positional.remove(0),
        output: positional.remove(0),
    })
}

fn decrypt_file_with_cli_progress<E>(
    input: &Path,
    output: &Path,
    options: &DecryptOptions,
    stderr: &mut E,
) -> Result<(), DecryptCliError>
where
    E: Write,
{
    let mut progress_write_error = None;
    decrypt_file_with_progress(input, output, options, |snapshot| {
        if progress_write_error.is_none()
            && let Err(error) = write_progress_snapshot(stderr, snapshot)
        {
            progress_write_error = Some(error);
        }
    })?;

    if let Some(error) = progress_write_error {
        return Err(DecryptCliError::Io(error));
    }

    Ok(())
}

fn write_progress_snapshot<W>(writer: &mut W, snapshot: DecryptProgress) -> io::Result<()>
where
    W: Write,
{
    match snapshot.total {
        Some(total) => writeln!(
            writer,
            "{} {}/{}",
            progress_phase_name(snapshot.phase),
            snapshot.completed,
            total
        ),
        None => writeln!(
            writer,
            "{} {}",
            progress_phase_name(snapshot.phase),
            snapshot.completed
        ),
    }
}

fn progress_phase_name(phase: DecryptProgressPhase) -> &'static str {
    match phase {
        DecryptProgressPhase::OpenInput => "OpenInput",
        DecryptProgressPhase::OpenOutput => "OpenOutput",
        DecryptProgressPhase::OpenFragmentsInfo => "OpenFragmentsInfo",
        DecryptProgressPhase::InspectStructure => "InspectStructure",
        DecryptProgressPhase::ProcessSamples => "ProcessSamples",
        DecryptProgressPhase::FinalizeOutput => "FinalizeOutput",
    }
}
