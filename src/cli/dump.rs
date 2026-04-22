//! Tree-dump command support.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, Write};

use terminal_size::{Width, terminal_size};

use crate::FourCc;
use crate::codec::{CodecError, FieldValue};
use crate::header::HeaderError;
use crate::stringify::{StringifyError, collect_structured_fields, stringify};
use crate::walk::{BoxPath, WalkControl, WalkError, WalkHandle, walk_structure};

use super::util::should_have_no_children;

const DEFAULT_TERMINAL_WIDTH: usize = 180;
const FREE: FourCc = FourCc::from_bytes(*b"free");
const MDAT: FourCc = FourCc::from_bytes(*b"mdat");
const SKIP: FourCc = FourCc::from_bytes(*b"skip");

/// Structured output format supported by the dump command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructuredDumpFormat {
    /// Pretty-printed JSON output.
    Json,
    /// Simple YAML output with stable field order.
    Yaml,
}

impl StructuredDumpFormat {
    fn parse(value: &str) -> Result<Option<Self>, DumpError> {
        match value {
            "text" => Ok(None),
            "json" => Ok(Some(Self::Json)),
            "yaml" => Ok(Some(Self::Yaml)),
            other => Err(DumpError::InvalidArgument(format!(
                "unsupported dump format: {other}"
            ))),
        }
    }
}

/// Structured payload state recorded for one dumped box.
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DumpPayloadStatus {
    /// The box payload rendered as a descriptor-backed summary string.
    Summary,
    /// The box payload was empty or has no visible summary fields.
    #[default]
    Empty,
    /// Raw payload bytes were included in the structured report.
    Bytes,
    /// Payload bytes were intentionally omitted until `-full` or `-a` is requested.
    Omitted,
    /// The box type is known, but the encoded version is not currently supported.
    UnsupportedVersion,
}

/// Top-level structured dump report used by JSON and YAML tree export.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StructuredDumpReport {
    /// Top-level boxes in file order.
    pub boxes: Vec<StructuredDumpBoxReport>,
}

/// One node in the structured dump tree.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StructuredDumpBoxReport {
    /// Four-character box identifier.
    pub box_type: String,
    /// Slash-delimited path from the file root to this box.
    pub path: String,
    /// Absolute file offset of the box header.
    pub offset: u64,
    /// Total box size including the header.
    pub size: u64,
    /// Whether the current box type is registered in the active lookup context.
    pub supported: bool,
    /// Summary of how payload detail is represented in this report node.
    pub payload_status: DumpPayloadStatus,
    /// Descriptor-backed payload summary when one was rendered.
    pub payload_summary: Option<String>,
    /// Raw payload bytes when full raw expansion was requested.
    pub payload_bytes: Option<Vec<u8>>,
    /// Direct child boxes in file order.
    pub children: Vec<StructuredDumpBoxReport>,
}

/// Additive structured dump report that includes field-level payload data for supported boxes.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldStructuredDumpReport {
    /// Top-level boxes in file order.
    pub boxes: Vec<FieldStructuredDumpBoxReport>,
}

/// One node in the field-level structured dump tree.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldStructuredDumpBoxReport {
    /// Four-character box identifier.
    pub box_type: String,
    /// Slash-delimited path from the file root to this box.
    pub path: String,
    /// Absolute file offset of the box header.
    pub offset: u64,
    /// Total box size including the header.
    pub size: u64,
    /// Whether the current box type is registered in the active lookup context.
    pub supported: bool,
    /// Summary of how payload detail is represented in this report node.
    pub payload_status: DumpPayloadStatus,
    /// Deterministic field-level payload data for supported boxes when it is available.
    pub payload_fields: Vec<StructuredDumpFieldReport>,
    /// Descriptor-backed payload summary when one was rendered.
    pub payload_summary: Option<String>,
    /// Raw payload bytes when full raw expansion was requested.
    pub payload_bytes: Option<Vec<u8>>,
    /// Direct child boxes in file order.
    pub children: Vec<FieldStructuredDumpBoxReport>,
}

/// One field entry in the field-level structured dump payload report.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructuredDumpFieldReport {
    /// Stable field name from the active descriptor table.
    pub name: String,
    /// Machine-readable field value.
    pub value: FieldValue,
    /// Optional human-oriented display projection when the field uses a display override or
    /// non-default formatting.
    pub display_value: Option<String>,
}

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
        "  -format <text|json|yaml>  Output format (default: text)"
    )?;
    writeln!(
        writer,
        "  -path <box/path>      Dump only matched parsed subtrees (repeatable)"
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
    dump_reader_paths(reader, options, &[], writer)
}

/// Dumps only the subtrees that match any parsed `paths` using the provided formatting
/// `options`.
///
/// Paths use the existing [`BoxPath`] parser, including slash-delimited segments, `*` wildcards,
/// and the `<root>` marker. Matching roots become the top-level boxes in the rendered text view,
/// so descendants are indented relative to the selected subtree instead of the original file root.
/// When `paths` is empty, this behaves the same as [`dump_reader`].
pub fn dump_reader_paths<R, W>(
    reader: &mut R,
    options: &DumpOptions,
    paths: &[BoxPath],
    writer: &mut W,
) -> Result<(), DumpError>
where
    R: Read + Seek,
    W: Write,
{
    let mut dump_error = None;
    let result = walk_structure(reader, |handle| {
        let selection = match_dump_paths(paths, handle.path());
        if !selection.include {
            return continue_dump_search(handle, selection.descend);
        }

        let info = *handle.info();
        let mut line = " ".repeat(selection.relative_depth(handle.path()).unwrap_or(0) * 2);
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

/// Builds a structured dump tree from one MP4 reader using the provided formatting `options`.
pub fn build_structured_report<R>(
    reader: &mut R,
    options: &DumpOptions,
) -> Result<StructuredDumpReport, DumpError>
where
    R: Read + Seek,
{
    build_structured_report_paths(reader, options, &[])
}

/// Builds a structured dump tree from only the subtrees that match any parsed `paths`.
///
/// Matching roots become the top-level boxes in the returned report while each node keeps its
/// original full file path. When `paths` is empty, this behaves the same as
/// [`build_structured_report`].
pub fn build_structured_report_paths<R>(
    reader: &mut R,
    options: &DumpOptions,
    paths: &[BoxPath],
) -> Result<StructuredDumpReport, DumpError>
where
    R: Read + Seek,
{
    let mut roots = Vec::new();
    let mut stack = Vec::new();
    let mut dump_error = None;

    let result = walk_structure(reader, |handle| {
        let selection = match_dump_paths(paths, handle.path());
        if !selection.include {
            return continue_dump_search(handle, selection.descend);
        }

        finalize_completed_boxes(
            selection.relative_depth(handle.path()).unwrap_or(0),
            &mut stack,
            &mut roots,
        );
        let (node, control) = build_structured_box_report(handle, options, &mut dump_error)?;
        stack.push(node);
        Ok(control)
    });

    if let Some(error) = dump_error {
        return Err(error);
    }
    result?;
    finalize_completed_boxes(0, &mut stack, &mut roots);

    Ok(StructuredDumpReport { boxes: roots })
}

/// Builds an additive field-level structured dump tree from one MP4 reader using the provided
/// formatting `options`.
pub fn build_field_structured_report<R>(
    reader: &mut R,
    options: &DumpOptions,
) -> Result<FieldStructuredDumpReport, DumpError>
where
    R: Read + Seek,
{
    build_field_structured_report_paths(reader, options, &[])
}

/// Builds an additive field-level structured dump tree from only the subtrees that match any
/// parsed `paths`.
///
/// Matching roots become the top-level boxes in the returned report while each node keeps its
/// original full file path. When `paths` is empty, this behaves the same as
/// [`build_field_structured_report`].
pub fn build_field_structured_report_paths<R>(
    reader: &mut R,
    options: &DumpOptions,
    paths: &[BoxPath],
) -> Result<FieldStructuredDumpReport, DumpError>
where
    R: Read + Seek,
{
    let mut roots = Vec::new();
    let mut stack = Vec::new();
    let mut dump_error = None;

    let result = walk_structure(reader, |handle| {
        let selection = match_dump_paths(paths, handle.path());
        if !selection.include {
            return continue_dump_search(handle, selection.descend);
        }

        finalize_completed_field_boxes(
            selection.relative_depth(handle.path()).unwrap_or(0),
            &mut stack,
            &mut roots,
        );
        let (node, control) = build_field_structured_box_report(handle, options, &mut dump_error)?;
        stack.push(node);
        Ok(control)
    });

    if let Some(error) = dump_error {
        return Err(error);
    }
    result?;
    finalize_completed_field_boxes(0, &mut stack, &mut roots);

    Ok(FieldStructuredDumpReport { boxes: roots })
}

/// Writes a structured dump `report` in the selected `format`.
pub fn write_structured_report<W>(
    writer: &mut W,
    report: &StructuredDumpReport,
    format: StructuredDumpFormat,
) -> Result<(), DumpError>
where
    W: Write,
{
    match format {
        StructuredDumpFormat::Json => {
            write_json_structured_report(writer, report).map_err(DumpError::Io)
        }
        StructuredDumpFormat::Yaml => {
            write_yaml_structured_report(writer, report).map_err(DumpError::Io)
        }
    }
}

/// Writes a field-level structured dump `report` in the selected `format`.
pub fn write_field_structured_report<W>(
    writer: &mut W,
    report: &FieldStructuredDumpReport,
    format: StructuredDumpFormat,
) -> Result<(), DumpError>
where
    W: Write,
{
    match format {
        StructuredDumpFormat::Json => {
            write_json_field_structured_report(writer, report).map_err(DumpError::Io)
        }
        StructuredDumpFormat::Yaml => {
            write_yaml_field_structured_report(writer, report).map_err(DumpError::Io)
        }
    }
}

/// Dumps one MP4 reader as a structured JSON or YAML tree using the provided `options`.
pub fn dump_reader_structured<R, W>(
    reader: &mut R,
    options: &DumpOptions,
    format: StructuredDumpFormat,
    writer: &mut W,
) -> Result<(), DumpError>
where
    R: Read + Seek,
    W: Write,
{
    dump_reader_structured_paths(reader, options, &[], format, writer)
}

/// Dumps only the subtrees that match any parsed `paths` as a structured JSON or YAML tree.
///
/// When `paths` is empty, this behaves the same as [`dump_reader_structured`].
pub fn dump_reader_structured_paths<R, W>(
    reader: &mut R,
    options: &DumpOptions,
    paths: &[BoxPath],
    format: StructuredDumpFormat,
    writer: &mut W,
) -> Result<(), DumpError>
where
    R: Read + Seek,
    W: Write,
{
    let report = build_structured_report_paths(reader, options, paths)?;
    write_structured_report(writer, &report, format)
}

/// Dumps one MP4 reader as an additive field-level structured JSON or YAML tree using the
/// provided `options`.
pub fn dump_reader_field_structured<R, W>(
    reader: &mut R,
    options: &DumpOptions,
    format: StructuredDumpFormat,
    writer: &mut W,
) -> Result<(), DumpError>
where
    R: Read + Seek,
    W: Write,
{
    dump_reader_field_structured_paths(reader, options, &[], format, writer)
}

/// Dumps only the subtrees that match any parsed `paths` as an additive field-level structured
/// JSON or YAML tree.
///
/// When `paths` is empty, this behaves the same as [`dump_reader_field_structured`].
pub fn dump_reader_field_structured_paths<R, W>(
    reader: &mut R,
    options: &DumpOptions,
    paths: &[BoxPath],
    format: StructuredDumpFormat,
    writer: &mut W,
) -> Result<(), DumpError>
where
    R: Read + Seek,
    W: Write,
{
    let report = build_field_structured_report_paths(reader, options, paths)?;
    write_field_structured_report(writer, &report, format)
}

fn run_inner<W>(args: &[String], stdout: &mut W) -> Result<(), DumpError>
where
    W: Write,
{
    let mut options = DumpOptions::default();
    let mut format = None;
    let mut paths = Vec::new();
    let mut input_path = None;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-format" | "--format" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DumpError::InvalidArgument(
                        "missing value for -format".to_string(),
                    ));
                };
                format = StructuredDumpFormat::parse(value)?;
                index += 2;
            }
            "-full" | "--full" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DumpError::InvalidArgument(
                        "missing value for -full".to_string(),
                    ));
                };
                parse_full_box_types(value, &mut options.full_box_types)?;
                index += 2;
            }
            "-path" | "--path" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DumpError::InvalidArgument(
                        "missing value for -path".to_string(),
                    ));
                };
                let path = BoxPath::parse(value).map_err(|error| {
                    DumpError::InvalidArgument(format!("invalid box path: {error}"))
                })?;
                paths.push(path);
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
    match format {
        Some(format) => {
            dump_reader_field_structured_paths(&mut file, &options, &paths, format, stdout)
        }
        None => dump_reader_paths(&mut file, &options, &paths, stdout),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DumpPathMatch {
    include: bool,
    descend: bool,
    display_depth_base: Option<usize>,
}

impl DumpPathMatch {
    fn relative_depth(self, path: &BoxPath) -> Option<usize> {
        self.display_depth_base
            .map(|base| path.len().saturating_sub(base))
    }
}

fn match_dump_paths(paths: &[BoxPath], current: &BoxPath) -> DumpPathMatch {
    if paths.is_empty() {
        return DumpPathMatch {
            include: true,
            descend: true,
            display_depth_base: Some(1),
        };
    }

    let mut matched = DumpPathMatch::default();
    for path in paths {
        let current_vs_selected = current.compare_with(path);
        if current_vs_selected.forward_match {
            matched.descend = true;
        }

        let selected_vs_current = path.compare_with(current);
        if selected_vs_current.exact_match || selected_vs_current.forward_match {
            matched.include = true;
            matched.descend = true;
            let display_depth_base = dump_display_depth_base(path);
            matched.display_depth_base = Some(
                matched
                    .display_depth_base
                    .map_or(display_depth_base, |base| base.min(display_depth_base)),
            );
        }
    }

    matched
}

fn dump_display_depth_base(path: &BoxPath) -> usize {
    if path.is_empty() { 1 } else { path.len() }
}

fn continue_dump_search<R>(
    handle: &mut WalkHandle<'_, R>,
    should_descend: bool,
) -> Result<WalkControl, WalkError>
where
    R: Read + Seek,
{
    if !should_descend {
        return Ok(WalkControl::Continue);
    }

    if !handle.is_supported_type() {
        return Ok(WalkControl::Continue);
    }

    if handle.info().payload_size()? >= 256 && should_have_no_children(handle.info().box_type()) {
        return Ok(WalkControl::Continue);
    }

    match handle.read_payload() {
        Ok(_) => Ok(WalkControl::Descend),
        Err(WalkError::Codec(CodecError::UnsupportedVersion { .. })) => Ok(WalkControl::Continue),
        Err(error) => Err(error),
    }
}

fn build_structured_box_report<R>(
    handle: &mut WalkHandle<'_, R>,
    options: &DumpOptions,
    dump_error: &mut Option<DumpError>,
) -> Result<(StructuredDumpBoxReport, WalkControl), WalkError>
where
    R: Read + Seek,
{
    let info = *handle.info();
    let box_type = info.box_type();
    let is_full = options.show_all || options.is_full(box_type);
    let mut node = StructuredDumpBoxReport {
        box_type: box_type.to_string(),
        path: handle.path().to_string(),
        offset: info.offset(),
        size: info.size(),
        supported: handle.is_supported_type(),
        payload_status: DumpPayloadStatus::Empty,
        payload_summary: None,
        payload_bytes: None,
        children: Vec::new(),
    };

    if !is_full && matches!(box_type, MDAT | FREE | SKIP) {
        node.payload_status = DumpPayloadStatus::Omitted;
        return Ok((node, WalkControl::Continue));
    }

    if handle.is_supported_type() {
        if !is_full && info.payload_size()? >= 64 && should_have_no_children(box_type) {
            node.payload_status = DumpPayloadStatus::Omitted;
            return Ok((node, WalkControl::Continue));
        }

        match handle.read_payload() {
            Ok((payload, _)) => {
                let rendered = match stringify(payload.as_ref(), None) {
                    Ok(rendered) => rendered,
                    Err(error) => {
                        *dump_error = Some(error.into());
                        return Err(io::Error::other("dump stringify failed").into());
                    }
                };
                if rendered.is_empty() {
                    node.payload_status = DumpPayloadStatus::Empty;
                } else {
                    node.payload_status = DumpPayloadStatus::Summary;
                    node.payload_summary = Some(rendered);
                }
                return Ok((node, WalkControl::Descend));
            }
            Err(WalkError::Codec(CodecError::UnsupportedVersion { .. })) => {
                node.payload_status = DumpPayloadStatus::UnsupportedVersion;
            }
            Err(error) => return Err(error),
        }
    }

    if is_full {
        let capacity = match usize::try_from(info.payload_size()?) {
            Ok(capacity) => capacity,
            Err(_) => {
                *dump_error = Some(DumpError::NumericOverflow);
                return Err(io::Error::other("dump payload too large").into());
            }
        };
        let mut bytes = Vec::with_capacity(capacity);
        handle.read_data(&mut bytes)?;
        if !matches!(node.payload_status, DumpPayloadStatus::UnsupportedVersion) {
            node.payload_status = DumpPayloadStatus::Bytes;
        }
        node.payload_bytes = Some(bytes);
    } else if !matches!(node.payload_status, DumpPayloadStatus::UnsupportedVersion) {
        node.payload_status = DumpPayloadStatus::Omitted;
    }

    Ok((node, WalkControl::Continue))
}

fn build_field_structured_box_report<R>(
    handle: &mut WalkHandle<'_, R>,
    options: &DumpOptions,
    dump_error: &mut Option<DumpError>,
) -> Result<(FieldStructuredDumpBoxReport, WalkControl), WalkError>
where
    R: Read + Seek,
{
    let info = *handle.info();
    let box_type = info.box_type();
    let is_full = options.show_all || options.is_full(box_type);
    let mut node = FieldStructuredDumpBoxReport {
        box_type: box_type.to_string(),
        path: handle.path().to_string(),
        offset: info.offset(),
        size: info.size(),
        supported: handle.is_supported_type(),
        payload_status: DumpPayloadStatus::Empty,
        payload_fields: Vec::new(),
        payload_summary: None,
        payload_bytes: None,
        children: Vec::new(),
    };

    if !is_full && matches!(box_type, MDAT | FREE | SKIP) {
        node.payload_status = DumpPayloadStatus::Omitted;
        return Ok((node, WalkControl::Continue));
    }

    if handle.is_supported_type() {
        match handle.read_payload() {
            Ok((payload, _)) => {
                let rendered = match stringify(payload.as_ref(), None) {
                    Ok(rendered) => rendered,
                    Err(error) => {
                        *dump_error = Some(error.into());
                        return Err(io::Error::other("dump stringify failed").into());
                    }
                };
                let fields = match collect_structured_fields(payload.as_ref(), None) {
                    Ok(fields) => fields,
                    Err(error) => {
                        *dump_error = Some(error.into());
                        return Err(io::Error::other("dump field collection failed").into());
                    }
                };
                node.payload_fields = fields
                    .into_iter()
                    .map(|field| StructuredDumpFieldReport {
                        name: field.name.to_string(),
                        value: field.value,
                        display_value: field.include_display_value.then_some(field.rendered_value),
                    })
                    .collect();
                if rendered.is_empty() {
                    node.payload_status = if node.payload_fields.is_empty() {
                        DumpPayloadStatus::Empty
                    } else {
                        DumpPayloadStatus::Summary
                    };
                } else {
                    node.payload_status = DumpPayloadStatus::Summary;
                    node.payload_summary = Some(rendered);
                }
                return Ok((node, WalkControl::Descend));
            }
            Err(WalkError::Codec(CodecError::UnsupportedVersion { .. })) => {
                node.payload_status = DumpPayloadStatus::UnsupportedVersion;
            }
            Err(error) => return Err(error),
        }
    }

    if is_full {
        let capacity = match usize::try_from(info.payload_size()?) {
            Ok(capacity) => capacity,
            Err(_) => {
                *dump_error = Some(DumpError::NumericOverflow);
                return Err(io::Error::other("dump payload too large").into());
            }
        };
        let mut bytes = Vec::with_capacity(capacity);
        handle.read_data(&mut bytes)?;
        if !matches!(node.payload_status, DumpPayloadStatus::UnsupportedVersion) {
            node.payload_status = DumpPayloadStatus::Bytes;
        }
        node.payload_bytes = Some(bytes);
    } else if !matches!(node.payload_status, DumpPayloadStatus::UnsupportedVersion) {
        node.payload_status = DumpPayloadStatus::Omitted;
    }

    Ok((node, WalkControl::Continue))
}

fn finalize_completed_boxes(
    depth: usize,
    stack: &mut Vec<StructuredDumpBoxReport>,
    roots: &mut Vec<StructuredDumpBoxReport>,
) {
    while stack.len() > depth {
        let node = stack.pop().expect("stack length checked before pop");
        if let Some(parent) = stack.last_mut() {
            parent.children.push(node);
        } else {
            roots.push(node);
        }
    }
}

fn finalize_completed_field_boxes(
    depth: usize,
    stack: &mut Vec<FieldStructuredDumpBoxReport>,
    roots: &mut Vec<FieldStructuredDumpBoxReport>,
) {
    while stack.len() > depth {
        let node = stack.pop().expect("stack length checked before pop");
        if let Some(parent) = stack.last_mut() {
            parent.children.push(node);
        } else {
            roots.push(node);
        }
    }
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

fn write_json_structured_report<W>(writer: &mut W, report: &StructuredDumpReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "{{")?;
    writeln!(writer, "  \"Boxes\": [")?;
    for (index, entry) in report.boxes.iter().enumerate() {
        write_json_structured_box(writer, entry, 2)?;
        let trailing = if index + 1 == report.boxes.len() {
            ""
        } else {
            ","
        };
        writeln!(writer, "{trailing}")?;
    }
    writeln!(writer, "  ]")?;
    writeln!(writer, "}}")
}

fn write_json_structured_box<W>(
    writer: &mut W,
    entry: &StructuredDumpBoxReport,
    indent_level: usize,
) -> io::Result<()>
where
    W: Write,
{
    let indent = "  ".repeat(indent_level);
    writeln!(writer, "{indent}{{")?;
    write_json_field(
        writer,
        indent_level + 1,
        "BoxType",
        &json_string(&entry.box_type),
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "Path",
        &json_string(&entry.path),
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "Offset",
        &entry.offset.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "Size",
        &entry.size.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "Supported",
        if entry.supported { "true" } else { "false" },
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "PayloadStatus",
        &json_string(payload_status_name(entry.payload_status)),
        true,
    )?;
    if let Some(summary) = entry.payload_summary.as_ref() {
        write_json_field(
            writer,
            indent_level + 1,
            "PayloadSummary",
            &json_string(summary),
            true,
        )?;
    }
    if let Some(bytes) = entry.payload_bytes.as_ref() {
        write_json_u8_array_field(writer, indent_level + 1, "PayloadBytes", bytes, true)?;
    }

    writeln!(writer, "{}\"Children\": [", "  ".repeat(indent_level + 1))?;
    for (index, child) in entry.children.iter().enumerate() {
        write_json_structured_box(writer, child, indent_level + 2)?;
        let trailing = if index + 1 == entry.children.len() {
            ""
        } else {
            ","
        };
        writeln!(writer, "{trailing}")?;
    }
    writeln!(writer, "{}]", "  ".repeat(indent_level + 1))?;
    write!(writer, "{indent}}}")
}

fn write_json_u8_array_field<W>(
    writer: &mut W,
    indent_level: usize,
    name: &str,
    values: &[u8],
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    write_json_array_field(
        writer,
        indent_level,
        name,
        &values.iter().map(u8::to_string).collect::<Vec<_>>(),
        trailing_comma,
    )
}

fn write_json_array_field<W>(
    writer: &mut W,
    indent_level: usize,
    name: &str,
    values: &[String],
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    let trailing = if trailing_comma { "," } else { "" };
    writeln!(writer, "{}\"{name}\": [", "  ".repeat(indent_level))?;
    for (index, value) in values.iter().enumerate() {
        let trailing_value = if index + 1 == values.len() { "" } else { "," };
        writeln!(
            writer,
            "{}{value}{trailing_value}",
            "  ".repeat(indent_level + 1)
        )?;
    }
    writeln!(writer, "{}]{trailing}", "  ".repeat(indent_level))
}

fn write_json_field<W>(
    writer: &mut W,
    indent_level: usize,
    name: &str,
    value: &str,
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    let trailing = if trailing_comma { "," } else { "" };
    writeln!(
        writer,
        "{}\"{name}\": {value}{trailing}",
        "  ".repeat(indent_level)
    )
}

fn write_yaml_structured_report<W>(writer: &mut W, report: &StructuredDumpReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "boxes:")?;
    for entry in &report.boxes {
        write_yaml_structured_box(writer, entry, 0)?;
    }
    Ok(())
}

fn write_yaml_structured_box<W>(
    writer: &mut W,
    entry: &StructuredDumpBoxReport,
    indent_level: usize,
) -> io::Result<()>
where
    W: Write,
{
    let indent = "  ".repeat(indent_level);
    let child_indent = "  ".repeat(indent_level + 1);

    writeln!(
        writer,
        "{indent}- box_type: {}",
        yaml_string(&entry.box_type)
    )?;
    writeln!(writer, "{child_indent}path: {}", yaml_string(&entry.path))?;
    writeln!(writer, "{child_indent}offset: {}", entry.offset)?;
    writeln!(writer, "{child_indent}size: {}", entry.size)?;
    writeln!(writer, "{child_indent}supported: {}", entry.supported)?;
    writeln!(
        writer,
        "{child_indent}payload_status: {}",
        yaml_string(payload_status_name(entry.payload_status))
    )?;
    if let Some(summary) = entry.payload_summary.as_ref() {
        writeln!(
            writer,
            "{child_indent}payload_summary: {}",
            yaml_string(summary)
        )?;
    }
    if let Some(bytes) = entry.payload_bytes.as_ref() {
        writeln!(writer, "{child_indent}payload_bytes:")?;
        for value in bytes {
            writeln!(writer, "{}- {value}", "  ".repeat(indent_level + 2))?;
        }
    }
    if entry.children.is_empty() {
        writeln!(writer, "{child_indent}children: []")?;
    } else {
        writeln!(writer, "{child_indent}children:")?;
        for child in &entry.children {
            write_yaml_structured_box(writer, child, indent_level + 1)?;
        }
    }
    Ok(())
}

fn write_json_field_structured_report<W>(
    writer: &mut W,
    report: &FieldStructuredDumpReport,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "{{")?;
    writeln!(writer, "  \"Boxes\": [")?;
    for (index, entry) in report.boxes.iter().enumerate() {
        write_json_field_structured_box(writer, entry, 2)?;
        let trailing = if index + 1 == report.boxes.len() {
            ""
        } else {
            ","
        };
        writeln!(writer, "{trailing}")?;
    }
    writeln!(writer, "  ]")?;
    writeln!(writer, "}}")
}

fn write_json_field_structured_box<W>(
    writer: &mut W,
    entry: &FieldStructuredDumpBoxReport,
    indent_level: usize,
) -> io::Result<()>
where
    W: Write,
{
    let indent = "  ".repeat(indent_level);
    writeln!(writer, "{indent}{{")?;
    write_json_field(
        writer,
        indent_level + 1,
        "BoxType",
        &json_string(&entry.box_type),
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "Path",
        &json_string(&entry.path),
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "Offset",
        &entry.offset.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "Size",
        &entry.size.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "Supported",
        if entry.supported { "true" } else { "false" },
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "PayloadStatus",
        &json_string(payload_status_name(entry.payload_status)),
        true,
    )?;
    write_json_payload_fields(writer, indent_level + 1, &entry.payload_fields, true)?;
    if let Some(summary) = entry.payload_summary.as_ref() {
        write_json_field(
            writer,
            indent_level + 1,
            "PayloadSummary",
            &json_string(summary),
            true,
        )?;
    }
    if let Some(bytes) = entry.payload_bytes.as_ref() {
        write_json_u8_array_field(writer, indent_level + 1, "PayloadBytes", bytes, true)?;
    }

    writeln!(writer, "{}\"Children\": [", "  ".repeat(indent_level + 1))?;
    for (index, child) in entry.children.iter().enumerate() {
        write_json_field_structured_box(writer, child, indent_level + 2)?;
        let trailing = if index + 1 == entry.children.len() {
            ""
        } else {
            ","
        };
        writeln!(writer, "{trailing}")?;
    }
    writeln!(writer, "{}]", "  ".repeat(indent_level + 1))?;
    write!(writer, "{indent}}}")
}

fn write_json_payload_fields<W>(
    writer: &mut W,
    indent_level: usize,
    fields: &[StructuredDumpFieldReport],
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    let trailing = if trailing_comma { "," } else { "" };
    writeln!(writer, "{}\"PayloadFields\": [", "  ".repeat(indent_level))?;
    for (index, field) in fields.iter().enumerate() {
        write_json_payload_field(writer, field, indent_level + 1)?;
        let trailing_field = if index + 1 == fields.len() { "" } else { "," };
        writeln!(writer, "{trailing_field}")?;
    }
    writeln!(writer, "{}]{trailing}", "  ".repeat(indent_level))
}

fn write_json_payload_field<W>(
    writer: &mut W,
    field: &StructuredDumpFieldReport,
    indent_level: usize,
) -> io::Result<()>
where
    W: Write,
{
    let indent = "  ".repeat(indent_level);
    writeln!(writer, "{indent}{{")?;
    write_json_field(
        writer,
        indent_level + 1,
        "Name",
        &json_string(&field.name),
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "ValueKind",
        &json_string(structured_field_value_kind_name(&field.value)),
        true,
    )?;
    write_json_dump_field_value(
        writer,
        indent_level + 1,
        "Value",
        &field.value,
        field.display_value.is_some(),
    )?;
    if let Some(display_value) = field.display_value.as_ref() {
        write_json_field(
            writer,
            indent_level + 1,
            "DisplayValue",
            &json_string(display_value),
            false,
        )?;
    }
    write!(writer, "{indent}}}")
}

fn write_json_dump_field_value<W>(
    writer: &mut W,
    indent_level: usize,
    name: &str,
    value: &FieldValue,
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    match value {
        FieldValue::Unsigned(value) => write_json_field(
            writer,
            indent_level,
            name,
            &value.to_string(),
            trailing_comma,
        ),
        FieldValue::Signed(value) => write_json_field(
            writer,
            indent_level,
            name,
            &value.to_string(),
            trailing_comma,
        ),
        FieldValue::Boolean(value) => write_json_field(
            writer,
            indent_level,
            name,
            if *value { "true" } else { "false" },
            trailing_comma,
        ),
        FieldValue::String(value) => write_json_field(
            writer,
            indent_level,
            name,
            &json_string(value),
            trailing_comma,
        ),
        FieldValue::Bytes(values) => {
            write_json_u8_array_field(writer, indent_level, name, values, trailing_comma)
        }
        FieldValue::UnsignedArray(values) => write_json_array_field(
            writer,
            indent_level,
            name,
            &values.iter().map(u64::to_string).collect::<Vec<_>>(),
            trailing_comma,
        ),
        FieldValue::SignedArray(values) => write_json_array_field(
            writer,
            indent_level,
            name,
            &values.iter().map(i64::to_string).collect::<Vec<_>>(),
            trailing_comma,
        ),
        FieldValue::BooleanArray(values) => write_json_array_field(
            writer,
            indent_level,
            name,
            &values.iter().map(bool::to_string).collect::<Vec<_>>(),
            trailing_comma,
        ),
    }
}

fn write_yaml_field_structured_report<W>(
    writer: &mut W,
    report: &FieldStructuredDumpReport,
) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "boxes:")?;
    for entry in &report.boxes {
        write_yaml_field_structured_box(writer, entry, 0)?;
    }
    Ok(())
}

fn write_yaml_field_structured_box<W>(
    writer: &mut W,
    entry: &FieldStructuredDumpBoxReport,
    indent_level: usize,
) -> io::Result<()>
where
    W: Write,
{
    let indent = "  ".repeat(indent_level);
    let child_indent = "  ".repeat(indent_level + 1);

    writeln!(
        writer,
        "{indent}- box_type: {}",
        yaml_string(&entry.box_type)
    )?;
    writeln!(writer, "{child_indent}path: {}", yaml_string(&entry.path))?;
    writeln!(writer, "{child_indent}offset: {}", entry.offset)?;
    writeln!(writer, "{child_indent}size: {}", entry.size)?;
    writeln!(writer, "{child_indent}supported: {}", entry.supported)?;
    writeln!(
        writer,
        "{child_indent}payload_status: {}",
        yaml_string(payload_status_name(entry.payload_status))
    )?;
    if entry.payload_fields.is_empty() {
        writeln!(writer, "{child_indent}payload_fields: []")?;
    } else {
        writeln!(writer, "{child_indent}payload_fields:")?;
        for field in &entry.payload_fields {
            write_yaml_payload_field(writer, field, indent_level + 1)?;
        }
    }
    if let Some(summary) = entry.payload_summary.as_ref() {
        writeln!(
            writer,
            "{child_indent}payload_summary: {}",
            yaml_string(summary)
        )?;
    }
    if let Some(bytes) = entry.payload_bytes.as_ref() {
        writeln!(writer, "{child_indent}payload_bytes:")?;
        for value in bytes {
            writeln!(writer, "{}- {value}", "  ".repeat(indent_level + 2))?;
        }
    }
    if entry.children.is_empty() {
        writeln!(writer, "{child_indent}children: []")?;
    } else {
        writeln!(writer, "{child_indent}children:")?;
        for child in &entry.children {
            write_yaml_field_structured_box(writer, child, indent_level + 1)?;
        }
    }
    Ok(())
}

fn write_yaml_payload_field<W>(
    writer: &mut W,
    field: &StructuredDumpFieldReport,
    indent_level: usize,
) -> io::Result<()>
where
    W: Write,
{
    let indent = "  ".repeat(indent_level + 1);
    let child_indent = "  ".repeat(indent_level + 2);
    writeln!(writer, "{indent}- name: {}", yaml_string(&field.name))?;
    writeln!(
        writer,
        "{child_indent}value_kind: {}",
        yaml_string(structured_field_value_kind_name(&field.value))
    )?;
    write_yaml_dump_field_value(writer, indent_level + 2, "value", &field.value)?;
    if let Some(display_value) = field.display_value.as_ref() {
        writeln!(
            writer,
            "{child_indent}display_value: {}",
            yaml_string(display_value)
        )?;
    }
    Ok(())
}

fn write_yaml_dump_field_value<W>(
    writer: &mut W,
    indent_level: usize,
    name: &str,
    value: &FieldValue,
) -> io::Result<()>
where
    W: Write,
{
    let indent = "  ".repeat(indent_level);
    let child_indent = "  ".repeat(indent_level + 1);
    match value {
        FieldValue::Unsigned(value) => writeln!(writer, "{indent}{name}: {value}"),
        FieldValue::Signed(value) => writeln!(writer, "{indent}{name}: {value}"),
        FieldValue::Boolean(value) => writeln!(writer, "{indent}{name}: {value}"),
        FieldValue::String(value) => writeln!(writer, "{indent}{name}: {}", yaml_string(value)),
        FieldValue::Bytes(values) => {
            if values.is_empty() {
                writeln!(writer, "{indent}{name}: []")
            } else {
                writeln!(writer, "{indent}{name}:")?;
                for value in values {
                    writeln!(writer, "{child_indent}- {value}")?;
                }
                Ok(())
            }
        }
        FieldValue::UnsignedArray(values) => {
            if values.is_empty() {
                writeln!(writer, "{indent}{name}: []")
            } else {
                writeln!(writer, "{indent}{name}:")?;
                for value in values {
                    writeln!(writer, "{child_indent}- {value}")?;
                }
                Ok(())
            }
        }
        FieldValue::SignedArray(values) => {
            if values.is_empty() {
                writeln!(writer, "{indent}{name}: []")
            } else {
                writeln!(writer, "{indent}{name}:")?;
                for value in values {
                    writeln!(writer, "{child_indent}- {value}")?;
                }
                Ok(())
            }
        }
        FieldValue::BooleanArray(values) => {
            if values.is_empty() {
                writeln!(writer, "{indent}{name}: []")
            } else {
                writeln!(writer, "{indent}{name}:")?;
                for value in values {
                    writeln!(writer, "{child_indent}- {value}")?;
                }
                Ok(())
            }
        }
    }
}

fn payload_status_name(status: DumpPayloadStatus) -> &'static str {
    match status {
        DumpPayloadStatus::Summary => "summary",
        DumpPayloadStatus::Empty => "empty",
        DumpPayloadStatus::Bytes => "bytes",
        DumpPayloadStatus::Omitted => "omitted",
        DumpPayloadStatus::UnsupportedVersion => "unsupported_version",
    }
}

fn structured_field_value_kind_name(value: &FieldValue) -> &'static str {
    match value {
        FieldValue::Unsigned(_) => "unsigned",
        FieldValue::Signed(_) => "signed",
        FieldValue::Boolean(_) => "boolean",
        FieldValue::Bytes(_) => "bytes",
        FieldValue::String(_) => "string",
        FieldValue::UnsignedArray(_) => "unsigned_array",
        FieldValue::SignedArray(_) => "signed_array",
        FieldValue::BooleanArray(_) => "boolean_array",
    }
}

fn json_string(value: &str) -> String {
    let mut escaped = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn yaml_string(value: &str) -> String {
    if !value.is_empty()
        && value.trim() == value
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '/' | ' '))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
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
