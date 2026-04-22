//! Stable text rendering for descriptor-backed boxes.

use std::error::Error;
use std::fmt;

use crate::codec::{
    CodecDescription, FieldFormat, FieldHooks, FieldResolutionError, FieldValue, FieldValueError,
    ResolvedField,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StructuredStringifyField {
    pub name: &'static str,
    pub value: FieldValue,
    pub rendered_value: String,
    pub include_display_value: bool,
}

pub(crate) fn collect_structured_fields(
    src: &dyn CodecDescription,
    hooks: Option<&dyn FieldHooks>,
) -> Result<Vec<StructuredStringifyField>, StringifyError> {
    let mut resolved = src.field_table().resolve_active(src, hooks)?;
    resolved.sort_by_key(ResolvedField::display_order);
    let mut rendered_fields = Vec::new();

    for field in resolved {
        if field.descriptor.constant.is_some() || field.descriptor.display.hidden {
            continue;
        }

        let (value, rendered_value, include_display_value) = collect_field(src, field)?;
        rendered_fields.push(StructuredStringifyField {
            name: field.name(),
            value,
            rendered_value,
            include_display_value,
        });
    }

    Ok(rendered_fields)
}

/// Renders a descriptor-backed box into the compact single-line form used by tests and CLI output.
pub fn stringify(
    src: &dyn CodecDescription,
    hooks: Option<&dyn FieldHooks>,
) -> Result<String, StringifyError> {
    stringify_with_indent(src, "", hooks)
}

/// Renders a descriptor-backed box with one field per line using the supplied indentation prefix.
pub fn stringify_with_indent(
    src: &dyn CodecDescription,
    indent: &str,
    hooks: Option<&dyn FieldHooks>,
) -> Result<String, StringifyError> {
    let rendered_fields = collect_structured_fields(src, hooks)?
        .into_iter()
        .map(|field| format!("{}={}", field.name, field.rendered_value))
        .collect::<Vec<_>>();

    if indent.is_empty() {
        return Ok(rendered_fields.join(" "));
    }

    let mut rendered = String::new();
    for field in rendered_fields {
        rendered.push_str(indent);
        rendered.push_str(&field);
        rendered.push('\n');
    }
    Ok(rendered)
}

fn collect_field(
    src: &dyn CodecDescription,
    field: ResolvedField<'_>,
) -> Result<(FieldValue, String, bool), StringifyError> {
    match field.descriptor.role {
        crate::codec::FieldRole::Version => {
            let value = FieldValue::Unsigned(u64::from(src.version()));
            let rendered = value_string(field, src, &value)?;
            Ok((value, rendered, false))
        }
        crate::codec::FieldRole::Flags => {
            let value = FieldValue::Unsigned(u64::from(src.flags()));
            let rendered = render_flags(src.flags(), field);
            Ok((value, rendered, true))
        }
        crate::codec::FieldRole::Data => {
            let value = src.field_value(field.name())?;
            let rendered = value_string(field, src, &value)?;
            let include_display_value = src.display_field(field.name()).is_some()
                || !matches!(
                    field.descriptor.display.format,
                    FieldFormat::Default | FieldFormat::Decimal
                );
            Ok((value, rendered, include_display_value))
        }
    }
}

fn value_string(
    field: ResolvedField<'_>,
    src: &dyn CodecDescription,
    value: &FieldValue,
) -> Result<String, StringifyError> {
    match field.descriptor.role {
        crate::codec::FieldRole::Version => render_default_value(value),
        crate::codec::FieldRole::Flags => Ok(render_flags(src.flags(), field)),
        crate::codec::FieldRole::Data => {
            if let Some(rendered) = src.display_field(field.name()) {
                Ok(rendered)
            } else {
                render_value(field, value)
            }
        }
    }
}

fn render_flags(value: u32, field: ResolvedField<'_>) -> String {
    let width = field.bit_width.unwrap_or(24).div_ceil(4) as usize;
    format!("0x{value:0width$x}")
}

fn render_value(field: ResolvedField<'_>, value: &FieldValue) -> Result<String, StringifyError> {
    match field.descriptor.display.format {
        FieldFormat::Default | FieldFormat::Decimal => render_default_value(value),
        FieldFormat::Hex => render_hex_value(field.name(), value),
        FieldFormat::Iso639_2 => render_iso639_2_value(field.name(), value),
        FieldFormat::Uuid => render_uuid_value(field.name(), value),
        FieldFormat::String(_) => render_string_value(value),
    }
}

fn render_default_value(value: &FieldValue) -> Result<String, StringifyError> {
    match value {
        FieldValue::Unsigned(value) => Ok(value.to_string()),
        FieldValue::Signed(value) => Ok(value.to_string()),
        FieldValue::Boolean(value) => Ok(value.to_string()),
        FieldValue::Bytes(bytes) => Ok(render_bytes(bytes)),
        FieldValue::String(value) => Ok(quote_string(value)),
        FieldValue::UnsignedArray(values) => Ok(render_array(
            values.iter().map(u64::to_string).collect::<Vec<_>>(),
        )),
        FieldValue::SignedArray(values) => Ok(render_array(
            values.iter().map(i64::to_string).collect::<Vec<_>>(),
        )),
        FieldValue::BooleanArray(values) => Ok(render_array(
            values.iter().map(bool::to_string).collect::<Vec<_>>(),
        )),
    }
}

fn render_hex_value(
    field_name: &'static str,
    value: &FieldValue,
) -> Result<String, StringifyError> {
    match value {
        FieldValue::Unsigned(value) => Ok(format!("0x{value:x}")),
        FieldValue::Signed(value) => Ok(render_signed_hex(*value)),
        FieldValue::UnsignedArray(values) => Ok(render_array(
            values.iter().map(|value| format!("0x{value:x}")).collect(),
        )),
        FieldValue::SignedArray(values) => Ok(render_array(
            values
                .iter()
                .map(|value| render_signed_hex(*value))
                .collect(),
        )),
        other => Err(StringifyError::InvalidFormat {
            field_name,
            reason: invalid_format_reason("hex", other),
        }),
    }
}

fn render_iso639_2_value(
    field_name: &'static str,
    value: &FieldValue,
) -> Result<String, StringifyError> {
    let bytes = match value {
        FieldValue::Bytes(bytes) => bytes.clone(),
        FieldValue::UnsignedArray(values) => values
            .iter()
            .map(|value| {
                u8::try_from(*value).map_err(|_| StringifyError::InvalidFormat {
                    field_name,
                    reason: "ISO-639-2 values must fit in one byte",
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        FieldValue::Unsigned(value) => {
            let value = u8::try_from(*value).map_err(|_| StringifyError::InvalidFormat {
                field_name,
                reason: "ISO-639-2 values must fit in one byte",
            })?;
            vec![value]
        }
        other => {
            return Err(StringifyError::InvalidFormat {
                field_name,
                reason: invalid_format_reason("ISO-639-2", other),
            });
        }
    };

    let mapped = bytes
        .into_iter()
        .map(|byte| char::from(byte.saturating_add(0x60)))
        .collect::<String>();
    Ok(quote_string(&mapped))
}

fn render_uuid_value(
    field_name: &'static str,
    value: &FieldValue,
) -> Result<String, StringifyError> {
    let bytes = match value {
        FieldValue::Bytes(bytes) => bytes.clone(),
        FieldValue::UnsignedArray(values) => values
            .iter()
            .map(|value| {
                u8::try_from(*value).map_err(|_| StringifyError::InvalidFormat {
                    field_name,
                    reason: "UUID values must fit in one byte",
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        other => {
            return Err(StringifyError::InvalidFormat {
                field_name,
                reason: invalid_format_reason("UUID", other),
            });
        }
    };

    if bytes.len() != 16 {
        return Err(StringifyError::InvalidFormat {
            field_name,
            reason: "UUID values must be exactly 16 bytes",
        });
    }

    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}

fn render_string_value(value: &FieldValue) -> Result<String, StringifyError> {
    match value {
        FieldValue::String(value) => Ok(quote_string(value)),
        FieldValue::Bytes(bytes) => Ok(quote_string(&escape_bytes(bytes))),
        FieldValue::UnsignedArray(values) => {
            let bytes = values
                .iter()
                .map(|value| u8::try_from(*value).unwrap_or(b'.'))
                .collect::<Vec<_>>();
            Ok(quote_string(&escape_bytes(&bytes)))
        }
        other => Err(StringifyError::InvalidFormat {
            field_name: "",
            reason: invalid_format_reason("string", other),
        }),
    }
}

fn render_bytes(bytes: &[u8]) -> String {
    render_array(bytes.iter().map(|byte| format!("0x{byte:x}")).collect())
}

fn render_array(values: Vec<String>) -> String {
    format!("[{}]", values.join(", "))
}

fn render_signed_hex(value: i64) -> String {
    if value < 0 {
        format!("-0x{:x}", value.unsigned_abs())
    } else {
        format!("0x{:x}", value as u64)
    }
}

fn quote_string(value: &str) -> String {
    format!("\"{}\"", escape_text(value))
}

fn escape_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| escape_char(char::from(*byte)))
        .collect::<String>()
}

fn escape_text(value: &str) -> String {
    value.chars().map(escape_char).collect()
}

fn escape_char(value: char) -> char {
    if value.is_control() || !value.is_ascii_graphic() && value != ' ' {
        '.'
    } else {
        value
    }
}

fn invalid_format_reason(expected_format: &'static str, value: &FieldValue) -> &'static str {
    match expected_format {
        "hex" => "hex formatting requires integer values",
        "ISO-639-2" => "ISO-639-2 formatting requires byte or unsigned values",
        "UUID" => "UUID formatting requires a 16-byte value",
        "string" => "string formatting requires text or byte values",
        _ => {
            let _ = value;
            "unsupported field formatting"
        }
    }
}

/// Errors raised while converting a descriptor-backed box into text.
#[derive(Debug)]
pub enum StringifyError {
    FieldResolution(FieldResolutionError),
    FieldValue(FieldValueError),
    InvalidFormat {
        field_name: &'static str,
        reason: &'static str,
    },
}

impl fmt::Display for StringifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FieldResolution(error) => error.fmt(f),
            Self::FieldValue(error) => error.fmt(f),
            Self::InvalidFormat {
                field_name: "",
                reason,
            } => {
                write!(f, "{reason}")
            }
            Self::InvalidFormat { field_name, reason } => {
                write!(f, "invalid display value for {field_name}: {reason}")
            }
        }
    }
}

impl Error for StringifyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FieldResolution(error) => Some(error),
            Self::FieldValue(error) => Some(error),
            Self::InvalidFormat { .. } => None,
        }
    }
}

impl From<FieldResolutionError> for StringifyError {
    fn from(error: FieldResolutionError) -> Self {
        Self::FieldResolution(error)
    }
}

impl From<FieldValueError> for StringifyError {
    fn from(error: FieldValueError) -> Self {
        Self::FieldValue(error)
    }
}
