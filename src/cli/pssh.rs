//! Protection-system summary command support.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, Write};

use crate::FourCc;
use crate::boxes::iso23001_7::Pssh;
use crate::codec::ImmutableBox;
use crate::extract::{ExtractError, extract_boxes_with_payload};
use crate::walk::BoxPath;

const MOOV: FourCc = FourCc::from_bytes(*b"moov");
const MOOF: FourCc = FourCc::from_bytes(*b"moof");
const PSSH: FourCc = FourCc::from_bytes(*b"pssh");

/// Runs the pssh-dump subcommand with `args`, writing output to `stdout`.
pub fn run<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    match run_inner(args, stdout) {
        Ok(()) => 0,
        Err(PsshDumpError::UsageRequested) => {
            let _ = write_usage(stderr);
            1
        }
        Err(error) => {
            let _ = writeln!(stderr, "Error: {error}");
            1
        }
    }
}

/// Writes the pssh-dump usage text.
pub fn write_usage<W>(writer: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "USAGE: mp4forge psshdump INPUT.mp4")
}

/// Writes formatted `pssh` summaries discovered in `reader`.
pub fn dump_pssh<R, W>(reader: &mut R, writer: &mut W) -> Result<(), PsshDumpError>
where
    R: Read + Seek,
    W: Write,
{
    let extracted = extract_boxes_with_payload(
        reader,
        None,
        &[BoxPath::from([MOOV, PSSH]), BoxPath::from([MOOF, PSSH])],
    )?;

    for (index, entry) in extracted.iter().enumerate() {
        let pssh = entry
            .payload
            .as_ref()
            .as_any()
            .downcast_ref::<Pssh>()
            .ok_or(PsshDumpError::UnexpectedPayloadType)?;

        entry.info.seek_to_start(reader)?;
        let raw_len =
            usize::try_from(entry.info.size()).map_err(|_| PsshDumpError::NumericOverflow)?;
        let mut raw = vec![0_u8; raw_len];
        reader.read_exact(&mut raw)?;

        writeln!(writer, "{index}:")?;
        writeln!(writer, "  offset: {}", entry.info.offset())?;
        writeln!(writer, "  size: {}", entry.info.size())?;
        writeln!(writer, "  version: {}", pssh.version())?;
        writeln!(writer, "  flags: 0x{:06x}", pssh.flags())?;
        writeln!(writer, "  systemId: {}", format_uuid(&pssh.system_id))?;
        writeln!(writer, "  dataSize: {}", pssh.data_size)?;
        writeln!(writer, "  base64: \"{}\"", encode_base64(&raw))?;
        writeln!(writer)?;
    }

    Ok(())
}

fn run_inner<W>(args: &[String], stdout: &mut W) -> Result<(), PsshDumpError>
where
    W: Write,
{
    if args.len() != 1 {
        return Err(PsshDumpError::UsageRequested);
    }

    let mut file = File::open(&args[0])?;
    dump_pssh(&mut file, stdout)
}

fn format_uuid(value: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        value[0],
        value[1],
        value[2],
        value[3],
        value[4],
        value[5],
        value[6],
        value[7],
        value[8],
        value[9],
        value[10],
        value[11],
        value[12],
        value[13],
        value[14],
        value[15]
    )
}

fn encode_base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut encoded = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let combined = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);

        encoded.push(ALPHABET[((combined >> 18) & 0x3f) as usize] as char);
        encoded.push(ALPHABET[((combined >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(ALPHABET[((combined >> 6) & 0x3f) as usize] as char);
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(ALPHABET[(combined & 0x3f) as usize] as char);
        } else {
            encoded.push('=');
        }
    }

    encoded
}

/// Errors raised while parsing `psshdump` arguments or formatting summaries.
#[derive(Debug)]
pub enum PsshDumpError {
    Io(io::Error),
    Extract(ExtractError),
    UnexpectedPayloadType,
    NumericOverflow,
    UsageRequested,
}

impl fmt::Display for PsshDumpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Extract(error) => error.fmt(f),
            Self::UnexpectedPayloadType => {
                f.write_str("unexpected payload type while reading pssh")
            }
            Self::NumericOverflow => f.write_str("numeric value does not fit in memory"),
            Self::UsageRequested => f.write_str("usage requested"),
        }
    }
}

impl Error for PsshDumpError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Extract(error) => Some(error),
            Self::UnexpectedPayloadType | Self::NumericOverflow | Self::UsageRequested => None,
        }
    }
}

impl From<io::Error> for PsshDumpError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ExtractError> for PsshDumpError {
    fn from(value: ExtractError) -> Self {
        Self::Extract(value)
    }
}
