//! Raw-box extraction command support.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, Write};

use crate::FourCc;
use crate::codec::CodecError;
use crate::extract::{ExtractError, extract_boxes_bytes};
use crate::header::HeaderError;
use crate::walk::{BoxPath, WalkControl, WalkError, walk_structure};

use super::util::should_have_no_children;

/// Runs the extract subcommand with `args`, writing output to `stdout`.
pub fn run<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    match run_inner(args, stdout) {
        Ok(()) => 0,
        Err(ExtractCliError::UsageRequested) => {
            let _ = write_usage(stderr);
            1
        }
        Err(error) => {
            let _ = writeln!(stderr, "Error: {error}");
            1
        }
    }
}

/// Writes the extract subcommand usage text.
pub fn write_usage<W>(writer: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "USAGE: mp4forge extract BOX_TYPE INPUT.mp4")?;
    writeln!(
        writer,
        "       mp4forge extract -path <box/path> [-path <box/path> ...] INPUT.mp4"
    )?;
    writeln!(writer)?;
    writeln!(writer, "OPTIONS:")?;
    writeln!(
        writer,
        "  -path <box/path>      Extract raw boxes that match the parsed slash-delimited box path"
    )
}

/// Extracts every box of type `box_type` from `reader`, preserving raw bytes.
pub fn extract_reader<R, W>(
    reader: &mut R,
    box_type: FourCc,
    writer: &mut W,
) -> Result<(), ExtractCliError>
where
    R: Read + Seek,
    W: Write,
{
    walk_structure(reader, |handle| {
        if handle.info().box_type() == box_type {
            writer.write_all(&handle.info().encode())?;
            handle.read_data(writer)?;
        }

        if !handle.is_supported_type() {
            return Ok(WalkControl::Continue);
        }

        if handle.info().payload_size()? >= 256 && should_have_no_children(handle.info().box_type())
        {
            return Ok(WalkControl::Continue);
        }

        match handle.read_payload() {
            Ok(_) => Ok(WalkControl::Descend),
            Err(WalkError::Codec(CodecError::UnsupportedVersion { .. })) => {
                Ok(WalkControl::Continue)
            }
            Err(error) => Err(error),
        }
    })?;

    Ok(())
}

/// Extracts every box that matches any path in `paths` from `reader`, preserving raw bytes.
///
/// Paths use the existing [`BoxPath`] parser, including slash-delimited segments and `*`
/// wildcards. Each match is copied with its original box header and payload bytes intact.
pub fn extract_reader_paths<R, W>(
    reader: &mut R,
    paths: &[BoxPath],
    writer: &mut W,
) -> Result<(), ExtractCliError>
where
    R: Read + Seek,
    W: Write,
{
    for bytes in extract_boxes_bytes(reader, None, paths).map_err(map_extract_error)? {
        writer.write_all(&bytes)?;
    }
    Ok(())
}

fn run_inner<W>(args: &[String], stdout: &mut W) -> Result<(), ExtractCliError>
where
    W: Write,
{
    let mut paths = Vec::new();
    let mut positional = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-path" | "--path" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(ExtractCliError::InvalidArgument(
                        "missing value for -path".to_string(),
                    ));
                };
                let path = BoxPath::parse(value).map_err(|error| {
                    ExtractCliError::InvalidArgument(format!("invalid box path: {error}"))
                })?;
                paths.push(path);
                index += 2;
            }
            "-h" | "--help" => return Err(ExtractCliError::UsageRequested),
            value if value.starts_with('-') => {
                return Err(ExtractCliError::InvalidArgument(format!(
                    "unknown extract option: {value}"
                )));
            }
            value => {
                positional.push(value);
                index += 1;
            }
        }
    }

    if paths.is_empty() {
        if positional.len() != 2 {
            return Err(ExtractCliError::UsageRequested);
        }

        let box_type = FourCc::try_from(positional[0]).map_err(|_| {
            ExtractCliError::InvalidArgument(format!("invalid box type: {}", positional[0]))
        })?;
        let mut file = File::open(positional[1])?;
        return extract_reader(&mut file, box_type, stdout);
    }

    if positional.len() != 1 {
        return Err(ExtractCliError::InvalidArgument(
            "extract with -path accepts exactly one input path".to_string(),
        ));
    }

    let mut file = File::open(positional[0])?;
    extract_reader_paths(&mut file, &paths, stdout)
}

fn map_extract_error(error: ExtractError) -> ExtractCliError {
    match error {
        ExtractError::Io(error) => ExtractCliError::Io(error),
        ExtractError::Header(error) => ExtractCliError::Header(error),
        ExtractError::Codec(error) => ExtractCliError::Walk(WalkError::Codec(error)),
        ExtractError::Walk(error) => ExtractCliError::Walk(error),
        ExtractError::EmptyPath => {
            ExtractCliError::InvalidArgument("box path must not be empty".to_string())
        }
        ExtractError::PayloadDecode { source, .. } => {
            ExtractCliError::Walk(WalkError::Codec(source))
        }
        unexpected @ ExtractError::UnexpectedPayloadType { .. } => {
            ExtractCliError::InvalidArgument(unexpected.to_string())
        }
    }
}

/// Errors raised while parsing extract arguments or copying raw boxes.
#[derive(Debug)]
pub enum ExtractCliError {
    Io(io::Error),
    Header(HeaderError),
    Walk(WalkError),
    InvalidArgument(String),
    UsageRequested,
}

impl fmt::Display for ExtractCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Header(error) => error.fmt(f),
            Self::Walk(error) => error.fmt(f),
            Self::InvalidArgument(message) => f.write_str(message),
            Self::UsageRequested => f.write_str("usage requested"),
        }
    }
}

impl Error for ExtractCliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::Walk(error) => Some(error),
            Self::InvalidArgument(..) | Self::UsageRequested => None,
        }
    }
}

impl From<io::Error> for ExtractCliError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<HeaderError> for ExtractCliError {
    fn from(value: HeaderError) -> Self {
        Self::Header(value)
    }
}

impl From<WalkError> for ExtractCliError {
    fn from(value: WalkError) -> Self {
        Self::Walk(value)
    }
}
