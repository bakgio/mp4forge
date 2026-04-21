//! Tree-dump command support.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, Write};

use terminal_size::{Width, terminal_size};

use crate::FourCc;
use crate::codec::CodecError;
use crate::header::HeaderError;
use crate::stringify::{StringifyError, stringify};
use crate::walk::{WalkControl, WalkError, walk_structure};

use super::util::should_have_no_children;

const DEFAULT_TERMINAL_WIDTH: usize = 180;
const FREE: FourCc = FourCc::from_bytes(*b"free");
const MDAT: FourCc = FourCc::from_bytes(*b"mdat");
const SKIP: FourCc = FourCc::from_bytes(*b"skip");

/// Formatting controls for the dump command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DumpOptions {
    /// Box types that should always render their full payload.
    pub full_box_types: BTreeSet<FourCc>,
    /// Whether all supported boxes should render their full payload.
    pub show_all: bool,
    /// Whether each line should include the box offset.
    pub show_offset: bool,
    /// Whether sizes and offsets should render in hexadecimal.
    pub hex: bool,
    /// Maximum line width before supported payload text is elided.
    pub terminal_width: usize,
}

impl Default for DumpOptions {
    fn default() -> Self {
        Self {
            full_box_types: BTreeSet::new(),
            show_all: false,
            show_offset: false,
            hex: false,
            terminal_width: detect_terminal_width(),
        }
    }
}

impl DumpOptions {
    fn is_full(&self, box_type: FourCc) -> bool {
        self.full_box_types.contains(&box_type)
    }
}

fn detect_terminal_width() -> usize {
    terminal_size()
        .map(|(Width(width), _)| usize::from(width))
        .filter(|width| *width > 0)
        .unwrap_or(DEFAULT_TERMINAL_WIDTH)
}

/// Runs the dump subcommand with `args`, writing output to `stdout`.
pub fn run<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    match run_inner(args, stdout) {
        Ok(()) => 0,
        Err(DumpError::UsageRequested) => {
            let _ = write_usage(stderr);
            1
        }
        Err(error) => {
            let _ = writeln!(stderr, "Error: {error}");
            1
        }
    }
}

/// Writes the dump subcommand usage text.
pub fn write_usage<W>(writer: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "USAGE: mp4forge dump [OPTIONS] INPUT.mp4")?;
    writeln!(writer)?;
    writeln!(writer, "OPTIONS:")?;
    writeln!(
        writer,
        "  -full <type,type>      Show full content for the listed box types"
    )?;
    writeln!(
        writer,
        "  -a                     Show full content for supported boxes"
    )?;
    writeln!(
        writer,
        "  -mdat                  Deprecated shorthand for -full mdat"
    )?;
    writeln!(
        writer,
        "  -free                  Deprecated shorthand for -full free,skip"
    )?;
    writeln!(writer, "  -offset                Show box offsets")?;
    writeln!(
        writer,
        "  -hex                   Use hexadecimal size and offset values"
    )?;
    Ok(())
}

/// Dumps one MP4 reader using the provided formatting `options`.
pub fn dump_reader<R, W>(
    reader: &mut R,
    options: &DumpOptions,
    writer: &mut W,
) -> Result<(), DumpError>
where
    R: Read + Seek,
    W: Write,
{
    let mut dump_error = None;
    let result = walk_structure(reader, |handle| {
        let info = *handle.info();
        let mut line = " ".repeat(handle.path().len().saturating_sub(1) * 2);
        line.push('[');
        line.push_str(&info.box_type().to_string());
        line.push(']');

        if !handle.is_supported_type() {
            line.push_str(" (unsupported box type)");
        }

        if options.show_offset {
            line.push_str(" Offset=");
            line.push_str(&format_number(info.offset(), options.hex));
        }
        line.push_str(" Size=");
        line.push_str(&format_number(info.size(), options.hex));

        let is_full = options.is_full(info.box_type());
        if !is_full && matches!(info.box_type(), MDAT | FREE | SKIP) {
            line.push_str(&format!(
                " Data=[...] (use \"-full {}\" to show all)",
                info.box_type()
            ));
            writeln!(writer, "{line}")?;
            return Ok(WalkControl::Continue);
        }

        let is_full = is_full || options.show_all;
        if handle.is_supported_type() {
            if !is_full && info.payload_size()? >= 64 && should_have_no_children(info.box_type()) {
                line.push_str(&format!(
                    " ... (use \"-full {}\" to show all)",
                    info.box_type()
                ));
                writeln!(writer, "{line}")?;
                return Ok(WalkControl::Continue);
            }

            match handle.read_payload() {
                Ok((payload, _)) => {
                    let rendered = match stringify(payload.as_ref(), None) {
                        Ok(rendered) => rendered,
                        Err(error) => {
                            dump_error = Some(error.into());
                            return Err(io::Error::other("dump stringify failed").into());
                        }
                    };
                    if !rendered.is_empty() {
                        if !is_full && line.len() + rendered.len() + 1 > options.terminal_width {
                            line.push_str(&format!(
                                " ... (use \"-full {}\" to show all)",
                                info.box_type()
                            ));
                        } else {
                            line.push(' ');
                            line.push_str(&rendered);
                        }
                    }

                    writeln!(writer, "{line}")?;
                    return Ok(WalkControl::Descend);
                }
                Err(WalkError::Codec(CodecError::UnsupportedVersion { .. })) => {
                    line.push_str(" (unsupported box version)");
                }
                Err(error) => return Err(error),
            }
        }

        if is_full {
            let capacity = match usize::try_from(info.payload_size()?) {
                Ok(capacity) => capacity,
                Err(_) => {
                    dump_error = Some(DumpError::NumericOverflow);
                    return Err(io::Error::other("dump payload too large").into());
                }
            };
            let mut bytes = Vec::with_capacity(capacity);
            handle.read_data(&mut bytes)?;
            line.push_str(" Data=[");
            line.push_str(&render_hex_bytes(&bytes));
            line.push(']');
        } else {
            line.push_str(&format!(
                " Data=[...] (use \"-full {}\" to show all)",
                info.box_type()
            ));
        }

        writeln!(writer, "{line}")?;
        Ok(WalkControl::Continue)
    });

    if let Some(error) = dump_error {
        return Err(error);
    }
    result?;

    Ok(())
}

fn run_inner<W>(args: &[String], stdout: &mut W) -> Result<(), DumpError>
where
    W: Write,
{
    let mut options = DumpOptions::default();
    let mut input_path = None;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-full" | "--full" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DumpError::InvalidArgument(
                        "missing value for -full".to_string(),
                    ));
                };
                parse_full_box_types(value, &mut options.full_box_types)?;
                index += 2;
            }
            "-a" | "--a" => {
                options.show_all = true;
                index += 1;
            }
            "-mdat" | "--mdat" => {
                options.full_box_types.insert(MDAT);
                index += 1;
            }
            "-free" | "--free" => {
                options.full_box_types.insert(FREE);
                options.full_box_types.insert(SKIP);
                index += 1;
            }
            "-offset" | "--offset" => {
                options.show_offset = true;
                index += 1;
            }
            "-hex" | "--hex" => {
                options.hex = true;
                index += 1;
            }
            "-h" | "--help" => return Err(DumpError::UsageRequested),
            value if value.starts_with('-') => {
                return Err(DumpError::InvalidArgument(format!(
                    "unknown dump option: {value}"
                )));
            }
            value => {
                if input_path.is_some() {
                    return Err(DumpError::InvalidArgument(
                        "dump accepts exactly one input path".to_string(),
                    ));
                }
                input_path = Some(value);
                index += 1;
            }
        }
    }

    let Some(input_path) = input_path else {
        return Err(DumpError::UsageRequested);
    };

    let mut file = File::open(input_path)?;
    dump_reader(&mut file, &options, stdout)
}

fn parse_full_box_types(value: &str, dst: &mut BTreeSet<FourCc>) -> Result<(), DumpError> {
    for name in value.split(',').filter(|entry| !entry.is_empty()) {
        let box_type = FourCc::try_from(name).map_err(|_| {
            DumpError::InvalidArgument(format!("box types passed to -full must be 4 bytes: {name}"))
        })?;
        dst.insert(box_type);
    }
    Ok(())
}

fn format_number(value: u64, hex: bool) -> String {
    if hex {
        format!("0x{value:x}")
    } else {
        value.to_string()
    }
}

fn render_hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("0x{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Errors raised while parsing dump arguments or rendering dump output.
#[derive(Debug)]
pub enum DumpError {
    Io(io::Error),
    Header(HeaderError),
    Walk(WalkError),
    Stringify(StringifyError),
    InvalidArgument(String),
    NumericOverflow,
    UsageRequested,
}

impl fmt::Display for DumpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Header(error) => error.fmt(f),
            Self::Walk(error) => error.fmt(f),
            Self::Stringify(error) => error.fmt(f),
            Self::InvalidArgument(message) => f.write_str(message),
            Self::NumericOverflow => f.write_str("numeric value does not fit in memory"),
            Self::UsageRequested => f.write_str("usage requested"),
        }
    }
}

impl Error for DumpError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::Walk(error) => Some(error),
            Self::Stringify(error) => Some(error),
            Self::InvalidArgument(..) | Self::NumericOverflow | Self::UsageRequested => None,
        }
    }
}

impl From<io::Error> for DumpError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<HeaderError> for DumpError {
    fn from(value: HeaderError) -> Self {
        Self::Header(value)
    }
}

impl From<WalkError> for DumpError {
    fn from(value: WalkError) -> Self {
        Self::Walk(value)
    }
}

impl From<StringifyError> for DumpError {
    fn from(value: StringifyError) -> Self {
        Self::Stringify(value)
    }
}
