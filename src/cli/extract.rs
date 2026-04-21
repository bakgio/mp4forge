//! Raw-box extraction command support.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, Write};

use crate::FourCc;
use crate::codec::CodecError;
use crate::header::HeaderError;
use crate::walk::{WalkControl, WalkError, walk_structure};

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
    Ok(())
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

fn run_inner<W>(args: &[String], stdout: &mut W) -> Result<(), ExtractCliError>
where
    W: Write,
{
    if args.len() != 2 {
        return Err(ExtractCliError::UsageRequested);
    }

    let box_type = FourCc::try_from(args[0].as_str())
        .map_err(|_| ExtractCliError::InvalidArgument(format!("invalid box type: {}", args[0])))?;
    let mut file = File::open(&args[1])?;
    extract_reader(&mut file, box_type, stdout)
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
