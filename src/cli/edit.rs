//! Box-edit command support.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use crate::FourCc;
use crate::boxes::iso14496_12::{Ftyp, Tfdt};
use crate::boxes::metadata::Keys;
use crate::boxes::{BoxLookupContext, BoxRegistry, default_registry};
use crate::codec::{
    CodecError, DynCodecBox, ImmutableBox, marshal_dyn, unmarshal, unmarshal_any_with_context,
};
use crate::extract::{ExtractError, extract_boxes_as};
use crate::header::{BoxInfo, HeaderError, SMALL_HEADER_SIZE};
use crate::rewrite::{RewriteError, rewrite_boxes_as};
use crate::walk::{BoxPath, WalkError};
use crate::writer::{Writer, WriterError};

const FTYP: FourCc = FourCc::from_bytes(*b"ftyp");
const KEYS: FourCc = FourCc::from_bytes(*b"keys");
const MDAT: FourCc = FourCc::from_bytes(*b"mdat");
const QT_BRAND: FourCc = FourCc::from_bytes(*b"qt  ");

/// Mutation controls for the edit command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditOptions {
    /// Replacement `tfdt` base media decode time, when provided.
    pub base_media_decode_time: Option<u64>,
    /// Box types that should be removed from the output.
    pub drop_boxes: BTreeSet<FourCc>,
}

/// Runs the edit subcommand with `args`, writing the rewritten file to `OUTPUT.mp4`.
pub fn run<E>(args: &[String], stderr: &mut E) -> i32
where
    E: Write,
{
    match run_inner(args) {
        Ok(()) => 0,
        Err(EditCliError::UsageRequested) => {
            let _ = write_usage(stderr);
            1
        }
        Err(error) => {
            let _ = writeln!(stderr, "Error: {error}");
            1
        }
    }
}

/// Writes the edit subcommand usage text.
pub fn write_usage<W>(writer: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(
        writer,
        "USAGE: mp4forge edit [OPTIONS] INPUT.mp4 OUTPUT.mp4"
    )?;
    writeln!(writer)?;
    writeln!(writer, "OPTIONS:")?;
    writeln!(
        writer,
        "  -base_media_decode_time <value>    Replace tfdt base media decode times"
    )?;
    writeln!(
        writer,
        "  -path <box/path>                   Limit supported typed rewrites to parsed slash-delimited box paths"
    )?;
    writeln!(
        writer,
        "  -drop <type,type>                  Drop boxes by fourcc"
    )?;
    Ok(())
}

/// Rewrites one MP4 stream according to `options`.
pub fn edit_reader<R, W>(reader: &mut R, writer: W, options: &EditOptions) -> Result<(), EditError>
where
    R: Read + Seek,
    W: Write + Seek,
{
    reader.seek(SeekFrom::Start(0))?;
    if options.is_noop() {
        let mut writer = writer;
        io::copy(reader, &mut writer)?;
        return Ok(());
    }

    let registry = default_registry();
    let mut writer = Writer::new(writer);
    rewrite_sequence(
        reader,
        &mut writer,
        &registry,
        options,
        RewriteFrame::root(),
    )?;
    Ok(())
}

fn run_inner(args: &[String]) -> Result<(), EditCliError> {
    let parsed = parse_args(args)?;
    let mut input = File::open(parsed.input_path)?;
    let output = File::create(parsed.output_path)?;
    if parsed.paths.is_empty() {
        return edit_reader(&mut input, output, &parsed.options).map_err(EditCliError::Edit);
    }

    edit_reader_scoped_paths(&mut input, output, &parsed.options, &parsed.paths)
}

#[derive(Debug)]
struct ParsedEditArgs<'a> {
    options: EditOptions,
    paths: Vec<BoxPath>,
    input_path: &'a str,
    output_path: &'a str,
}

fn parse_args(args: &[String]) -> Result<ParsedEditArgs<'_>, EditCliError> {
    let mut options = EditOptions::default();
    let mut paths = Vec::new();
    let mut positional = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-base_media_decode_time" | "--base_media_decode_time" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(EditCliError::InvalidArgument(
                        "missing value for -base_media_decode_time".to_string(),
                    ));
                };
                let value = value.parse::<u64>().map_err(|_| {
                    EditCliError::InvalidArgument(format!(
                        "invalid base media decode time: {value}"
                    ))
                })?;
                options.base_media_decode_time = Some(value);
                index += 2;
            }
            "-path" | "--path" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(EditCliError::InvalidArgument(
                        "missing value for -path".to_string(),
                    ));
                };
                let path = BoxPath::parse(value).map_err(|error| {
                    EditCliError::InvalidArgument(format!("invalid box path: {error}"))
                })?;
                paths.push(path);
                index += 2;
            }
            "-drop" | "--drop" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(EditCliError::InvalidArgument(
                        "missing value for -drop".to_string(),
                    ));
                };
                for name in value.split(',').filter(|entry| !entry.is_empty()) {
                    options
                        .drop_boxes
                        .insert(FourCc::try_from(name).map_err(|_| {
                            EditCliError::InvalidArgument(format!(
                                "box types passed to -drop must be 4 bytes: {name}"
                            ))
                        })?);
                }
                index += 2;
            }
            "-h" | "--help" => return Err(EditCliError::UsageRequested),
            value if value.starts_with('-') => {
                return Err(EditCliError::InvalidArgument(format!(
                    "unknown edit option: {value}"
                )));
            }
            value => {
                positional.push(value);
                index += 1;
            }
        }
    }

    if positional.len() != 2 {
        return Err(EditCliError::UsageRequested);
    }

    if !paths.is_empty() && options.base_media_decode_time.is_none() {
        return Err(EditCliError::InvalidArgument(
            "edit -path currently supports only -base_media_decode_time rewrites".to_string(),
        ));
    }

    Ok(ParsedEditArgs {
        options,
        paths,
        input_path: positional[0],
        output_path: positional[1],
    })
}

fn edit_reader_scoped_paths<R, W>(
    reader: &mut R,
    writer: W,
    options: &EditOptions,
    paths: &[BoxPath],
) -> Result<(), EditCliError>
where
    R: Read + Seek,
    W: Write + Seek,
{
    let Some(base_media_decode_time) = options.base_media_decode_time else {
        return Err(EditCliError::InvalidArgument(
            "edit -path currently supports only -base_media_decode_time rewrites".to_string(),
        ));
    };

    let matched_tfdt =
        extract_boxes_as::<_, Tfdt>(reader, None, paths).map_err(map_scoped_extract_error)?;
    if base_media_decode_time > u64::from(u32::MAX)
        && matched_tfdt.iter().any(|tfdt| tfdt.version() == 0)
    {
        return Err(EditCliError::Edit(EditError::NumericOverflow {
            field_name: "base media decode time",
        }));
    }

    reader.seek(SeekFrom::Start(0))?;
    let mut scoped_output = Cursor::new(Vec::new());
    rewrite_boxes_as::<_, _, Tfdt, _>(reader, &mut scoped_output, paths, |tfdt| {
        if tfdt.version() == 0 {
            tfdt.base_media_decode_time_v0 = base_media_decode_time as u32;
        } else {
            tfdt.base_media_decode_time_v1 = base_media_decode_time;
        }
    })
    .map_err(map_scoped_rewrite_error)?;

    let scoped_bytes = scoped_output.into_inner();
    let follow_up_options = EditOptions {
        base_media_decode_time: None,
        drop_boxes: options.drop_boxes.clone(),
    };
    if follow_up_options.is_noop() {
        let mut writer = writer;
        writer.write_all(&scoped_bytes)?;
        return Ok(());
    }

    let mut scoped_reader = Cursor::new(scoped_bytes);
    edit_reader(&mut scoped_reader, writer, &follow_up_options).map_err(EditCliError::Edit)
}

fn map_scoped_extract_error(error: ExtractError) -> EditCliError {
    match error {
        ExtractError::Io(error) => EditCliError::Edit(EditError::Io(error)),
        ExtractError::Header(error) => EditCliError::Edit(EditError::Header(error)),
        ExtractError::Codec(error) => EditCliError::Edit(EditError::Codec(error)),
        ExtractError::Walk(error) => EditCliError::Edit(map_walk_error(error)),
        ExtractError::EmptyPath => {
            EditCliError::InvalidArgument("box path must not be empty".to_string())
        }
        ExtractError::PayloadDecode { source, .. } => EditCliError::Edit(EditError::Codec(source)),
        ExtractError::UnexpectedPayloadType {
            path,
            box_type,
            offset,
            ..
        } => EditCliError::InvalidArgument(format!(
            "path-based -base_media_decode_time rewrites require tfdt boxes: matched {path} (type={box_type}, offset={offset})"
        )),
    }
}

fn map_scoped_rewrite_error(error: RewriteError) -> EditCliError {
    match error {
        RewriteError::Io(error) => EditCliError::Edit(EditError::Io(error)),
        RewriteError::Header(error) => EditCliError::Edit(EditError::Header(error)),
        RewriteError::Codec(error) => EditCliError::Edit(EditError::Codec(error)),
        RewriteError::Writer(error) => EditCliError::Edit(EditError::Writer(error)),
        RewriteError::EmptyPath => {
            EditCliError::InvalidArgument("box path must not be empty".to_string())
        }
        RewriteError::PayloadDecode { source, .. } | RewriteError::PayloadEncode { source, .. } => {
            EditCliError::Edit(EditError::Codec(source))
        }
        RewriteError::UnexpectedPayloadType {
            path,
            box_type,
            offset,
            ..
        } => EditCliError::InvalidArgument(format!(
            "path-based -base_media_decode_time rewrites require tfdt boxes: matched {path} (type={box_type}, offset={offset})"
        )),
        RewriteError::TooLargeBoxSize {
            box_type,
            size,
            available_size,
        } => EditCliError::Edit(EditError::TooLargeBoxSize {
            box_type,
            size,
            available_size,
        }),
        RewriteError::UnexpectedEof => EditCliError::Edit(EditError::UnexpectedEof),
    }
}

fn map_walk_error(error: WalkError) -> EditError {
    match error {
        WalkError::Io(error) => EditError::Io(error),
        WalkError::Header(error) => EditError::Header(error),
        WalkError::Codec(error) => EditError::Codec(error),
        WalkError::TooLargeBoxSize {
            box_type,
            size,
            available_size,
        } => EditError::TooLargeBoxSize {
            box_type,
            size,
            available_size,
        },
        WalkError::UnexpectedEof => EditError::UnexpectedEof,
    }
}

#[derive(Debug)]
enum EditCliError {
    Edit(EditError),
    InvalidArgument(String),
    UsageRequested,
}

impl fmt::Display for EditCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Edit(error) => error.fmt(f),
            Self::InvalidArgument(message) => f.write_str(message),
            Self::UsageRequested => f.write_str("usage requested"),
        }
    }
}

impl Error for EditCliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Edit(error) => Some(error),
            Self::InvalidArgument(..) | Self::UsageRequested => None,
        }
    }
}

impl From<io::Error> for EditCliError {
    fn from(value: io::Error) -> Self {
        Self::Edit(EditError::Io(value))
    }
}

#[derive(Clone, Copy)]
struct RewriteFrame {
    remaining_size: u64,
    is_root: bool,
    depth: usize,
    sibling_context: BoxLookupContext,
}

impl RewriteFrame {
    const fn root() -> Self {
        Self {
            remaining_size: 0,
            is_root: true,
            depth: 0,
            sibling_context: BoxLookupContext::new(),
        }
    }

    const fn child(remaining_size: u64, depth: usize, sibling_context: BoxLookupContext) -> Self {
        Self {
            remaining_size,
            is_root: false,
            depth,
            sibling_context,
        }
    }
}

fn rewrite_sequence<R, W>(
    reader: &mut R,
    writer: &mut Writer<W>,
    registry: &BoxRegistry,
    options: &EditOptions,
    mut frame: RewriteFrame,
) -> Result<(), EditError>
where
    R: Read + Seek,
    W: Write + Seek,
{
    loop {
        if !frame.is_root && frame.remaining_size < SMALL_HEADER_SIZE {
            break;
        }

        let start = reader.stream_position()?;
        let mut info = match BoxInfo::read(reader) {
            Ok(info) => info,
            Err(HeaderError::Io(error))
                if frame.is_root && clean_root_eof(reader, start, &error)? =>
            {
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };

        if !frame.is_root && info.size() > frame.remaining_size {
            return Err(EditError::TooLargeBoxSize {
                box_type: info.box_type(),
                size: info.size(),
                available_size: frame.remaining_size,
            });
        }
        if !frame.is_root {
            frame.remaining_size -= info.size();
        }

        info.set_lookup_context(frame.sibling_context);
        inspect_context_carriers(reader, &mut info, frame.depth)?;
        process_box(reader, writer, registry, options, &info, frame.depth)?;

        if info.lookup_context().is_quicktime_compatible() {
            frame.sibling_context = frame.sibling_context.with_quicktime_compatible(true);
        }
        if info.box_type() == KEYS {
            frame.sibling_context = frame
                .sibling_context
                .with_metadata_keys_entry_count(info.lookup_context().metadata_keys_entry_count());
        }
    }

    if !frame.is_root
        && frame.remaining_size != 0
        && !frame.sibling_context.is_quicktime_compatible()
    {
        return Err(EditError::UnexpectedEof);
    }

    Ok(())
}

fn process_box<R, W>(
    reader: &mut R,
    writer: &mut Writer<W>,
    registry: &BoxRegistry,
    options: &EditOptions,
    info: &BoxInfo,
    depth: usize,
) -> Result<(), EditError>
where
    R: Read + Seek,
    W: Write + Seek,
{
    if options.drop_boxes.contains(&info.box_type()) {
        info.seek_to_end(reader)?;
        return Ok(());
    }

    if !registry.is_registered_with_context(info.box_type(), info.lookup_context())
        || info.box_type() == MDAT
    {
        copy_raw_box(reader, writer, info)?;
        return Ok(());
    }

    info.seek_to_payload(reader)?;
    let payload_size = info.payload_size()?;
    let decode_result = unmarshal_any_with_context(
        reader,
        payload_size,
        info.box_type(),
        registry,
        info.lookup_context(),
        None,
    );

    let (mut payload, payload_read) = match decode_result {
        Ok(value) => value,
        Err(CodecError::UnsupportedVersion { .. }) => {
            copy_raw_box(reader, writer, info)?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };

    mutate_payload(payload.as_mut(), options)?;

    let placeholder = BoxInfo::new(info.box_type(), info.header_size())
        .with_header_size(info.header_size())
        .with_lookup_context(info.lookup_context())
        .with_extend_to_eof(info.extend_to_eof());
    writer.start_box(placeholder)?;
    marshal_dyn(&mut *writer, payload.as_ref(), None)?;

    let children_offset = info.offset() + info.header_size() + payload_read;
    let children_size = info
        .offset()
        .saturating_add(info.size())
        .saturating_sub(children_offset);
    reader.seek(SeekFrom::Start(children_offset))?;
    rewrite_sequence(
        reader,
        writer,
        registry,
        options,
        RewriteFrame::child(
            children_size,
            depth + 1,
            info.lookup_context().enter(info.box_type()),
        ),
    )?;
    info.seek_to_end(reader)?;
    writer.end_box()?;
    Ok(())
}

fn mutate_payload(payload: &mut dyn DynCodecBox, options: &EditOptions) -> Result<(), EditError> {
    if let Some(base_media_decode_time) = options.base_media_decode_time
        && let Some(tfdt) = payload.as_any_mut().downcast_mut::<Tfdt>()
    {
        if tfdt.version() == 0 {
            tfdt.base_media_decode_time_v0 =
                u32::try_from(base_media_decode_time).map_err(|_| EditError::NumericOverflow {
                    field_name: "base media decode time",
                })?;
        } else {
            tfdt.base_media_decode_time_v1 = base_media_decode_time;
        }
    }

    Ok(())
}

fn copy_raw_box<R, W>(
    reader: &mut R,
    writer: &mut Writer<W>,
    info: &BoxInfo,
) -> Result<(), EditError>
where
    R: Read + Seek,
    W: Write + Seek,
{
    writer.write_all(&info.encode())?;
    info.seek_to_payload(reader)?;
    let mut limited = reader.take(info.payload_size()?);
    io::copy(&mut limited, writer)?;
    Ok(())
}

fn inspect_context_carriers<R>(
    reader: &mut R,
    info: &mut BoxInfo,
    depth: usize,
) -> Result<(), EditError>
where
    R: Read + Seek,
{
    if depth == 0 && info.box_type() == FTYP {
        let ftyp = decode_box::<_, Ftyp>(reader, info)?;
        if ftyp.has_compatible_brand(QT_BRAND) {
            info.set_lookup_context(info.lookup_context().with_quicktime_compatible(true));
        }
    }

    if info.box_type() == KEYS {
        let keys = decode_box::<_, Keys>(reader, info)?;
        info.set_lookup_context(
            info.lookup_context()
                .with_metadata_keys_entry_count(keys.entry_count as usize),
        );
    }

    Ok(())
}

fn decode_box<R, B>(reader: &mut R, info: &BoxInfo) -> Result<B, EditError>
where
    R: Read + Seek,
    B: Default + crate::codec::CodecBox,
{
    info.seek_to_payload(reader)?;
    let mut decoded = B::default();
    unmarshal(reader, info.payload_size()?, &mut decoded, None)?;
    info.seek_to_payload(reader)?;
    Ok(decoded)
}

fn clean_root_eof<R>(reader: &mut R, start: u64, error: &io::Error) -> Result<bool, io::Error>
where
    R: Seek,
{
    if error.kind() != io::ErrorKind::UnexpectedEof {
        return Ok(false);
    }

    let end = reader.seek(SeekFrom::End(0))?;
    Ok(start == end)
}

/// Errors raised while parsing edit arguments or rewriting files.
#[derive(Debug)]
pub enum EditError {
    Io(io::Error),
    Header(HeaderError),
    Codec(CodecError),
    Writer(WriterError),
    InvalidArgument(String),
    TooLargeBoxSize {
        box_type: FourCc,
        size: u64,
        available_size: u64,
    },
    NumericOverflow {
        field_name: &'static str,
    },
    UnexpectedEof,
    UsageRequested,
}

impl EditOptions {
    fn is_noop(&self) -> bool {
        self.base_media_decode_time.is_none() && self.drop_boxes.is_empty()
    }
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Header(error) => error.fmt(f),
            Self::Codec(error) => error.fmt(f),
            Self::Writer(error) => error.fmt(f),
            Self::InvalidArgument(message) => f.write_str(message),
            Self::TooLargeBoxSize {
                box_type,
                size,
                available_size,
            } => write!(
                f,
                "too large box size: type={box_type}, size={size}, actualBufSize={available_size}"
            ),
            Self::NumericOverflow { field_name } => {
                write!(f, "numeric value does not fit while writing {field_name}")
            }
            Self::UnexpectedEof => f.write_str("unexpected EOF"),
            Self::UsageRequested => f.write_str("usage requested"),
        }
    }
}

impl Error for EditError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::Writer(error) => Some(error),
            Self::InvalidArgument(..)
            | Self::TooLargeBoxSize { .. }
            | Self::NumericOverflow { .. }
            | Self::UnexpectedEof
            | Self::UsageRequested => None,
        }
    }
}

impl From<io::Error> for EditError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<HeaderError> for EditError {
    fn from(value: HeaderError) -> Self {
        Self::Header(value)
    }
}

impl From<CodecError> for EditError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}

impl From<WriterError> for EditError {
    fn from(value: WriterError) -> Self {
        Self::Writer(value)
    }
}
