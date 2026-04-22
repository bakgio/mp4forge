//! Protection-system summary command support.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, Write};

use crate::FourCc;
use crate::boxes::iso23001_7::Pssh;
use crate::codec::ImmutableBox;
use crate::extract::ExtractError;
use crate::walk::{BoxPath, WalkControl, WalkError, WalkHandle, walk_structure};

const MOOV: FourCc = FourCc::from_bytes(*b"moov");
const MOOF: FourCc = FourCc::from_bytes(*b"moof");
const PSSH: FourCc = FourCc::from_bytes(*b"pssh");

/// Structured output format supported by the pssh-dump command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsshDumpFormat {
    /// Pretty-printed JSON output.
    Json,
    /// Simple YAML output with stable field order.
    Yaml,
}

impl PsshDumpFormat {
    fn parse(value: &str) -> Result<Option<Self>, PsshDumpError> {
        match value {
            "text" => Ok(None),
            "json" => Ok(Some(Self::Json)),
            "yaml" => Ok(Some(Self::Yaml)),
            other => Err(invalid_argument(format!(
                "unsupported psshdump format: {other}"
            ))),
        }
    }
}

/// Additive selection controls for reusable `pssh` reports.
///
/// Filters inside one category are combined with OR semantics, while different categories are
/// combined with AND semantics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PsshReportFilter {
    /// Parsed subtree selectors that scope which `pssh` paths are eligible for inclusion.
    ///
    /// These reuse the existing [`BoxPath`] parser, including `*` wildcard segments and the
    /// special `<root>` marker. The filter behaves like subtree selection, so `moov` matches
    /// `moov/pssh` and `<root>` matches every discovered `pssh` box.
    pub paths: Vec<BoxPath>,
    /// Protection-system UUIDs that are allowed to match.
    pub system_ids: Vec<[u8; 16]>,
    /// Key IDs that are allowed to match.
    pub kids: Vec<[u8; 16]>,
}

impl PsshReportFilter {
    /// Returns `true` when the filter leaves the report unscoped.
    pub fn is_unfiltered(&self) -> bool {
        self.paths.is_empty() && self.system_ids.is_empty() && self.kids.is_empty()
    }
}

/// Top-level structured `pssh` summary report used by JSON and YAML output.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PsshReport {
    /// Parsed `pssh` entries in file order.
    pub entries: Vec<PsshEntryReport>,
}

/// One parsed `pssh` summary entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PsshEntryReport {
    /// Zero-based file-order index used by the text formatter.
    pub index: usize,
    /// Slash-delimited path from the file root to the matched `pssh` box.
    pub path: String,
    /// Absolute file offset of the `pssh` header.
    pub offset: u64,
    /// Total box size including the header.
    pub size: u64,
    /// Parsed full-box version.
    pub version: u8,
    /// Parsed full-box flags.
    pub flags: u32,
    /// Formatted protection-system UUID.
    pub system_id: String,
    /// Parsed KID count field.
    pub kid_count: u32,
    /// Formatted key IDs carried by version `1` entries.
    pub kids: Vec<String>,
    /// Parsed data-size field.
    pub data_size: u32,
    /// Raw `Data` field bytes.
    pub data_bytes: Vec<u8>,
    /// Base64 encoding of the exact serialized box bytes, including the header.
    pub raw_box_base64: String,
}

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
    writeln!(writer, "USAGE: mp4forge psshdump [OPTIONS] INPUT.mp4")?;
    writeln!(writer)?;
    writeln!(writer, "OPTIONS:")?;
    writeln!(
        writer,
        "  -format <text|json|yaml>  Output format (default: text)"
    )?;
    writeln!(
        writer,
        "  -path <box/path>          Limit results to matching parsed subtrees (repeatable)"
    )?;
    writeln!(
        writer,
        "  -system-id <uuid>         Limit results to matching protection-system IDs (repeatable)"
    )?;
    writeln!(
        writer,
        "  -kid <uuid>               Limit results to matching key IDs (repeatable)"
    )?;
    Ok(())
}

/// Writes the existing human-readable `pssh` summaries discovered in `reader`.
pub fn dump_pssh<R, W>(reader: &mut R, writer: &mut W) -> Result<(), PsshDumpError>
where
    R: Read + Seek,
    W: Write,
{
    let report = build_pssh_report(reader)?;
    write_text_report(writer, &report)
}

/// Builds a reusable structured `pssh` summary report from one MP4 reader.
pub fn build_pssh_report<R>(reader: &mut R) -> Result<PsshReport, PsshDumpError>
where
    R: Read + Seek,
{
    build_pssh_report_with_filters(reader, &PsshReportFilter::default())
}

/// Writes the existing human-readable `pssh` summaries discovered in `reader`, limited by
/// `filters`.
pub fn dump_pssh_with_filters<R, W>(
    reader: &mut R,
    filters: &PsshReportFilter,
    writer: &mut W,
) -> Result<(), PsshDumpError>
where
    R: Read + Seek,
    W: Write,
{
    let report = build_pssh_report_with_filters(reader, filters)?;
    write_text_report(writer, &report)
}

/// Builds a reusable structured `pssh` summary report from one MP4 reader, limited by `filters`.
pub fn build_pssh_report_with_filters<R>(
    reader: &mut R,
    filters: &PsshReportFilter,
) -> Result<PsshReport, PsshDumpError>
where
    R: Read + Seek,
{
    let mut collector = PsshReportCollector {
        filters,
        next_index: 0,
        entries: Vec::new(),
        build_error: None,
    };
    let result = walk_structure(reader, |handle| {
        collect_pssh_report_entry(handle, &mut collector)
    });
    if let Some(error) = collector.build_error {
        return Err(error);
    }
    result.map_err(walk_error_as_extract)?;
    Ok(PsshReport {
        entries: collector.entries,
    })
}

/// Writes a structured `pssh` `report` in the selected `format`.
pub fn write_pssh_report<W>(
    writer: &mut W,
    report: &PsshReport,
    format: PsshDumpFormat,
) -> Result<(), PsshDumpError>
where
    W: Write,
{
    match format {
        PsshDumpFormat::Json => write_json_pssh_report(writer, report).map_err(PsshDumpError::Io),
        PsshDumpFormat::Yaml => write_yaml_pssh_report(writer, report).map_err(PsshDumpError::Io),
    }
}

/// Writes one MP4 reader as a structured `pssh` JSON or YAML report.
pub fn dump_pssh_structured<R, W>(
    reader: &mut R,
    format: PsshDumpFormat,
    writer: &mut W,
) -> Result<(), PsshDumpError>
where
    R: Read + Seek,
    W: Write,
{
    dump_pssh_structured_with_filters(reader, &PsshReportFilter::default(), format, writer)
}

/// Writes one MP4 reader as a structured `pssh` JSON or YAML report, limited by `filters`.
pub fn dump_pssh_structured_with_filters<R, W>(
    reader: &mut R,
    filters: &PsshReportFilter,
    format: PsshDumpFormat,
    writer: &mut W,
) -> Result<(), PsshDumpError>
where
    R: Read + Seek,
    W: Write,
{
    let report = build_pssh_report_with_filters(reader, filters)?;
    write_pssh_report(writer, &report, format)
}

fn run_inner<W>(args: &[String], stdout: &mut W) -> Result<(), PsshDumpError>
where
    W: Write,
{
    let mut format = None;
    let mut filters = PsshReportFilter::default();
    let mut input_path = None;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-format" | "--format" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(invalid_argument("missing value for -format"));
                };
                format = PsshDumpFormat::parse(value)?;
                index += 2;
            }
            "-path" | "--path" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(invalid_argument("missing value for -path"));
                };
                let path =
                    BoxPath::parse(value).map_err(|error| invalid_argument(error.to_string()))?;
                filters.paths.push(path);
                index += 2;
            }
            "-system-id" | "--system-id" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(invalid_argument("missing value for -system-id"));
                };
                let system_id = parse_uuid_filter(value, "system ID")?;
                filters.system_ids.push(system_id);
                index += 2;
            }
            "-kid" | "--kid" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(invalid_argument("missing value for -kid"));
                };
                let kid = parse_uuid_filter(value, "KID")?;
                filters.kids.push(kid);
                index += 2;
            }
            "-h" | "--help" => return Err(PsshDumpError::UsageRequested),
            value if value.starts_with('-') => {
                return Err(invalid_argument(format!(
                    "unknown psshdump option: {value}"
                )));
            }
            value => {
                if input_path.is_some() {
                    return Err(invalid_argument("psshdump accepts exactly one input path"));
                }
                input_path = Some(value);
                index += 1;
            }
        }
    }

    let Some(input_path) = input_path else {
        return Err(PsshDumpError::UsageRequested);
    };

    let mut file = File::open(input_path)?;
    match format {
        Some(format) => dump_pssh_structured_with_filters(&mut file, &filters, format, stdout),
        None => dump_pssh_with_filters(&mut file, &filters, stdout),
    }
}

struct PsshReportCollector<'a> {
    filters: &'a PsshReportFilter,
    next_index: usize,
    entries: Vec<PsshEntryReport>,
    build_error: Option<PsshDumpError>,
}

fn collect_pssh_report_entry<R>(
    handle: &mut WalkHandle<'_, R>,
    collector: &mut PsshReportCollector<'_>,
) -> Result<WalkControl, WalkError>
where
    R: Read + Seek,
{
    if should_descend_pssh_path(handle.path().as_slice()) {
        return Ok(WalkControl::Descend);
    }

    if !is_pssh_path(handle.path().as_slice()) {
        return Ok(WalkControl::Continue);
    }

    let entry_index = collector.next_index;
    collector.next_index += 1;
    if !matches_path_filters(collector.filters, handle.path()) {
        return Ok(WalkControl::Continue);
    }

    let (payload, _) = handle.read_payload()?;
    let Some(pssh) = payload.as_ref().as_any().downcast_ref::<Pssh>() else {
        collector.build_error = Some(PsshDumpError::UnexpectedPayloadType);
        return Err(io::Error::other("unexpected pssh payload type").into());
    };
    if !matches_system_id_filters(collector.filters, &pssh.system_id)
        || !matches_kid_filters(collector.filters, &pssh.kids)
    {
        return Ok(WalkControl::Continue);
    }

    let payload_bytes = read_payload_bytes(handle, &mut collector.build_error)?;
    let mut raw_box = handle.info().encode();
    raw_box.extend_from_slice(&payload_bytes);

    collector.entries.push(PsshEntryReport {
        index: entry_index,
        path: handle.path().to_string(),
        offset: handle.info().offset(),
        size: handle.info().size(),
        version: pssh.version(),
        flags: pssh.flags(),
        system_id: format_uuid(&pssh.system_id),
        kid_count: pssh.kid_count,
        kids: pssh.kids.iter().map(|kid| format_uuid(&kid.kid)).collect(),
        data_size: pssh.data_size,
        data_bytes: pssh.data.clone(),
        raw_box_base64: encode_base64(&raw_box),
    });

    Ok(WalkControl::Continue)
}

fn should_descend_pssh_path(path: &[FourCc]) -> bool {
    matches!(path, [MOOV] | [MOOF])
}

fn is_pssh_path(path: &[FourCc]) -> bool {
    matches!(path, [MOOV, PSSH] | [MOOF, PSSH])
}

fn matches_path_filters(filters: &PsshReportFilter, entry_path: &BoxPath) -> bool {
    filters.paths.is_empty()
        || filters.paths.iter().any(|path| {
            let selected_vs_entry = path.compare_with(entry_path);
            selected_vs_entry.exact_match || selected_vs_entry.forward_match
        })
}

fn matches_system_id_filters(filters: &PsshReportFilter, system_id: &[u8; 16]) -> bool {
    filters.system_ids.is_empty()
        || filters
            .system_ids
            .iter()
            .any(|candidate| candidate == system_id)
}

fn matches_kid_filters(
    filters: &PsshReportFilter,
    kids: &[crate::boxes::iso23001_7::PsshKid],
) -> bool {
    filters.kids.is_empty()
        || kids
            .iter()
            .any(|kid| filters.kids.iter().any(|candidate| candidate == &kid.kid))
}

fn parse_uuid_filter(value: &str, label: &str) -> Result<[u8; 16], PsshDumpError> {
    let mut digits = String::with_capacity(32);
    for ch in value.chars() {
        if ch == '-' {
            continue;
        }
        digits.push(ch);
    }

    if digits.len() != 32 {
        return Err(invalid_argument(format!(
            "invalid {label}: expected 32 hexadecimal digits with optional hyphens"
        )));
    }

    let mut parsed = [0u8; 16];
    let bytes = digits.as_bytes();
    for (index, slot) in parsed.iter_mut().enumerate() {
        let high = decode_hex_nibble(bytes[index * 2]).ok_or_else(|| {
            invalid_argument(format!(
                "invalid {label}: expected 32 hexadecimal digits with optional hyphens"
            ))
        })?;
        let low = decode_hex_nibble(bytes[index * 2 + 1]).ok_or_else(|| {
            invalid_argument(format!(
                "invalid {label}: expected 32 hexadecimal digits with optional hyphens"
            ))
        })?;
        *slot = (high << 4) | low;
    }

    Ok(parsed)
}

fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn read_payload_bytes<R>(
    handle: &mut WalkHandle<'_, R>,
    build_error: &mut Option<PsshDumpError>,
) -> Result<Vec<u8>, WalkError>
where
    R: Read + Seek,
{
    let payload_size = handle.info().payload_size().map_err(WalkError::Header)?;
    let capacity = match usize::try_from(payload_size) {
        Ok(capacity) => capacity,
        Err(_) => {
            *build_error = Some(PsshDumpError::NumericOverflow);
            return Err(io::Error::other("payload too large").into());
        }
    };
    let mut payload = Vec::with_capacity(capacity);
    handle.read_data(&mut payload)?;
    Ok(payload)
}

fn write_text_report<W>(writer: &mut W, report: &PsshReport) -> Result<(), PsshDumpError>
where
    W: Write,
{
    for entry in &report.entries {
        writeln!(writer, "{}:", entry.index)?;
        writeln!(writer, "  offset: {}", entry.offset)?;
        writeln!(writer, "  size: {}", entry.size)?;
        writeln!(writer, "  version: {}", entry.version)?;
        writeln!(writer, "  flags: 0x{:06x}", entry.flags)?;
        writeln!(writer, "  systemId: {}", entry.system_id)?;
        writeln!(writer, "  dataSize: {}", entry.data_size)?;
        writeln!(writer, "  base64: \"{}\"", entry.raw_box_base64)?;
        writeln!(writer)?;
    }

    Ok(())
}

fn write_json_pssh_report<W>(writer: &mut W, report: &PsshReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "{{")?;
    writeln!(writer, "  \"Entries\": [")?;
    for (index, entry) in report.entries.iter().enumerate() {
        write_json_pssh_entry(writer, entry, 2)?;
        let trailing = if index + 1 == report.entries.len() {
            ""
        } else {
            ","
        };
        writeln!(writer, "{trailing}")?;
    }
    writeln!(writer, "  ]")?;
    writeln!(writer, "}}")
}

fn write_json_pssh_entry<W>(
    writer: &mut W,
    entry: &PsshEntryReport,
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
        "Index",
        &entry.index.to_string(),
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
        "Version",
        &entry.version.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "Flags",
        &entry.flags.to_string(),
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "SystemId",
        &json_string(&entry.system_id),
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "KidCount",
        &entry.kid_count.to_string(),
        true,
    )?;
    write_json_string_array_field(writer, indent_level + 1, "Kids", &entry.kids, true)?;
    write_json_field(
        writer,
        indent_level + 1,
        "DataSize",
        &entry.data_size.to_string(),
        true,
    )?;
    write_json_u8_array_field(
        writer,
        indent_level + 1,
        "DataBytes",
        &entry.data_bytes,
        true,
    )?;
    write_json_field(
        writer,
        indent_level + 1,
        "RawBoxBase64",
        &json_string(&entry.raw_box_base64),
        false,
    )?;
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

fn write_json_string_array_field<W>(
    writer: &mut W,
    indent_level: usize,
    name: &str,
    values: &[String],
    trailing_comma: bool,
) -> io::Result<()>
where
    W: Write,
{
    write_json_array_field(
        writer,
        indent_level,
        name,
        &values
            .iter()
            .map(|value| json_string(value))
            .collect::<Vec<_>>(),
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

fn write_yaml_pssh_report<W>(writer: &mut W, report: &PsshReport) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "entries:")?;
    for entry in &report.entries {
        write_yaml_pssh_entry(writer, entry, 0)?;
    }
    Ok(())
}

fn write_yaml_pssh_entry<W>(
    writer: &mut W,
    entry: &PsshEntryReport,
    indent_level: usize,
) -> io::Result<()>
where
    W: Write,
{
    let indent = "  ".repeat(indent_level);
    let child_indent = "  ".repeat(indent_level + 1);
    writeln!(writer, "{indent}- index: {}", entry.index)?;
    writeln!(writer, "{child_indent}path: {}", yaml_string(&entry.path))?;
    writeln!(writer, "{child_indent}offset: {}", entry.offset)?;
    writeln!(writer, "{child_indent}size: {}", entry.size)?;
    writeln!(writer, "{child_indent}version: {}", entry.version)?;
    writeln!(writer, "{child_indent}flags: {}", entry.flags)?;
    writeln!(
        writer,
        "{child_indent}system_id: {}",
        yaml_string(&entry.system_id)
    )?;
    writeln!(writer, "{child_indent}kid_count: {}", entry.kid_count)?;
    if entry.kids.is_empty() {
        writeln!(writer, "{child_indent}kids: []")?;
    } else {
        writeln!(writer, "{child_indent}kids:")?;
        for kid in &entry.kids {
            writeln!(
                writer,
                "{}- {}",
                "  ".repeat(indent_level + 2),
                yaml_string(kid)
            )?;
        }
    }
    writeln!(writer, "{child_indent}data_size: {}", entry.data_size)?;
    if entry.data_bytes.is_empty() {
        writeln!(writer, "{child_indent}data_bytes: []")?;
    } else {
        writeln!(writer, "{child_indent}data_bytes:")?;
        for value in &entry.data_bytes {
            writeln!(writer, "{}- {value}", "  ".repeat(indent_level + 2))?;
        }
    }
    writeln!(
        writer,
        "{child_indent}raw_box_base64: {}",
        yaml_string(&entry.raw_box_base64)
    )?;
    Ok(())
}

fn walk_error_as_extract(error: WalkError) -> PsshDumpError {
    PsshDumpError::Extract(ExtractError::from(error))
}

fn invalid_argument(message: impl Into<String>) -> PsshDumpError {
    PsshDumpError::Io(io::Error::new(io::ErrorKind::InvalidInput, message.into()))
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
