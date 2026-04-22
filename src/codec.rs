//! Descriptor-driven binary codec support for MP4 boxes.

use std::any::Any;
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::FourCc;
use crate::bitio::{BitReader, BitWriter};
use crate::boxes::{BoxLookupContext, BoxRegistry};

/// Sentinel version used before a concrete box version has been read.
pub const ANY_VERSION: u8 = u8::MAX;
/// Sentinel length used by callers that want a remainder-consuming field.
pub const UNBOUNDED_LENGTH: u32 = u32::MAX;
const MAX_UNTRUSTED_PREALLOC: usize = 64 * 1024;

/// Object-safe alias for readers that also support seeking.
pub trait ReadSeek: Read + Seek {}

impl<T> ReadSeek for T where T: Read + Seek {}

pub(crate) fn untrusted_prealloc_hint(count: usize) -> usize {
    count.min(MAX_UNTRUSTED_PREALLOC)
}

pub(crate) fn read_exact_vec_untrusted<R>(reader: &mut R, len: usize) -> io::Result<Vec<u8>>
where
    R: Read + ?Sized,
{
    let mut data = Vec::with_capacity(untrusted_prealloc_hint(len));
    let mut chunk = [0_u8; 4096];
    let mut remaining = len;
    while remaining != 0 {
        let to_read = remaining.min(chunk.len());
        reader.read_exact(&mut chunk[..to_read])?;
        data.extend_from_slice(&chunk[..to_read]);
        remaining -= to_read;
    }
    Ok(data)
}

/// Box-specific overrides used by the generic descriptor codec.
pub trait FieldHooks {
    /// Returns a dynamic bit width for `name` when the descriptor requests one.
    fn field_size(&self, _name: &'static str) -> Option<u32> {
        None
    }

    /// Returns a dynamic element count for `name` when the descriptor requests one.
    fn field_length(&self, _name: &'static str) -> Option<u32> {
        None
    }

    /// Returns whether a dynamically gated field should be active.
    fn field_enabled(&self, _name: &'static str) -> Option<bool> {
        None
    }

    /// Chooses whether a Pascal-compatible string should decode in Pascal mode.
    fn is_pascal_string(
        &self,
        _name: &'static str,
        _data: &[u8],
        _remaining_bytes: u64,
    ) -> Option<bool> {
        None
    }

    /// Overrides the rendered value for a field in the shared stringifier.
    fn display_field(&self, _name: &'static str) -> Option<String> {
        None
    }

    /// Allows terminal string fields to consume trailing padding after the first NUL byte.
    fn consume_remaining_bytes_after_string(&self, _name: &'static str) -> Option<bool> {
        None
    }
}

/// Default hook set that leaves every behavior on the generic path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoopFieldHooks;

impl FieldHooks for NoopFieldHooks {}

/// Read-only box behavior required by the descriptor codec.
pub trait ImmutableBox: FieldHooks {
    /// Returns the four-character type for the box.
    fn box_type(&self) -> FourCc;

    /// Returns the parsed version, or [`ANY_VERSION`] before it is known.
    fn version(&self) -> u8 {
        ANY_VERSION
    }

    /// Returns the parsed 24-bit flag value.
    fn flags(&self) -> u32 {
        0
    }

    /// Returns `true` when `flag` is set in the current box flags.
    fn check_flag(&self, flag: u32) -> bool {
        self.flags() & flag != 0
    }
}

/// Mutable box behavior required during decode.
pub trait MutableBox: ImmutableBox {
    /// Updates the stored box version.
    fn set_version(&mut self, _version: u8) {}

    /// Updates the stored box flags.
    fn set_flags(&mut self, _flags: u32) {}

    /// Runs before descriptor-driven decode for boxes that must inspect payload bytes first.
    fn before_unmarshal(
        &mut self,
        _reader: &mut dyn ReadSeek,
        _payload_size: u64,
    ) -> Result<(), CodecError> {
        Ok(())
    }

    /// Sets `flag` in the stored flag word.
    fn add_flag(&mut self, flag: u32) {
        self.set_flags(self.flags() | flag);
    }

    /// Clears `flag` from the stored flag word.
    fn remove_flag(&mut self, flag: u32) {
        self.set_flags(self.flags() & !flag);
    }
}

/// String storage mode used by descriptor-backed fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringFieldMode {
    NullTerminated,
    PascalCompatible,
    RawBox,
}

/// Logical field kind used by the generic codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Unsigned,
    Signed,
    Boolean,
    Bytes,
    String(StringFieldMode),
}

/// Text-format hint used by the shared stringifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldFormat {
    Default,
    Decimal,
    Hex,
    Iso639_2,
    Uuid,
    String(StringFieldMode),
}

/// Rendering controls attached to a field descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldDisplay {
    /// Preferred display format for the field.
    pub format: FieldFormat,
    /// Whether the field should be hidden from shared string output.
    pub hidden: bool,
}

impl FieldDisplay {
    /// Creates the default display configuration.
    pub const fn new() -> Self {
        Self {
            format: FieldFormat::Default,
            hidden: false,
        }
    }
}

impl Default for FieldDisplay {
    fn default() -> Self {
        Self::new()
    }
}

/// Bit-width source for a field descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldBitWidth {
    Unspecified,
    Fixed(u32),
    Dynamic,
}

/// Element-count source for a field descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldLength {
    Unbounded,
    Fixed(u32),
    Dynamic,
}

/// Version gate applied to a descriptor field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionGate {
    Any,
    Exact(u8),
    Not(u8),
}

/// Special handling role applied to a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldRole {
    Data,
    Version,
    Flags,
}

/// Presence conditions applied to a field descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldGate {
    /// Version-based presence rule.
    pub version: VersionGate,
    /// Flags that must be present for the field to be active.
    pub required_flags: u32,
    /// Flags that must be absent for the field to be active.
    pub forbidden_flags: u32,
    /// Whether field presence is delegated to [`FieldHooks::field_enabled`].
    pub dynamic_presence: bool,
}

impl FieldGate {
    /// Creates an unconstrained field gate.
    pub const fn new() -> Self {
        Self {
            version: VersionGate::Any,
            required_flags: 0,
            forbidden_flags: 0,
            dynamic_presence: false,
        }
    }
}

impl Default for FieldGate {
    fn default() -> Self {
        Self::new()
    }
}

/// Compile-time description of a single logical field in a box payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldDescriptor {
    /// Field name used for reflection-style access and formatting.
    pub name: &'static str,
    /// Stable field order within the payload.
    pub order: u16,
    /// Stable display order within shared string output.
    pub display_order: u16,
    /// Logical field kind.
    pub kind: FieldKind,
    /// Special role, if the field drives version or flags.
    pub role: FieldRole,
    /// Bit-width source.
    pub bit_width: FieldBitWidth,
    /// Element-count source.
    pub length: FieldLength,
    /// Presence gate.
    pub gate: FieldGate,
    /// Display hints.
    pub display: FieldDisplay,
    /// Whether the field uses the internal unsigned-varint encoding.
    pub varint: bool,
    /// Reserved for extended field handling.
    pub extend: bool,
    /// Constant value expected during encode and decode.
    pub constant: Option<&'static str>,
}

impl FieldDescriptor {
    /// Creates a default unsigned data descriptor with the supplied name and order.
    pub const fn new(name: &'static str, order: u16) -> Self {
        Self {
            name,
            order,
            display_order: order,
            kind: FieldKind::Unsigned,
            role: FieldRole::Data,
            bit_width: FieldBitWidth::Unspecified,
            length: FieldLength::Unbounded,
            gate: FieldGate::new(),
            display: FieldDisplay::new(),
            varint: false,
            extend: false,
            constant: None,
        }
    }

    /// Uses a fixed bit width for the field.
    pub const fn with_bit_width(mut self, bit_width: u32) -> Self {
        self.bit_width = FieldBitWidth::Fixed(bit_width);
        self
    }

    /// Defers bit-width resolution to [`FieldHooks::field_size`].
    pub const fn with_dynamic_bit_width(mut self) -> Self {
        self.bit_width = FieldBitWidth::Dynamic;
        self
    }

    /// Uses a fixed element count for the field.
    pub const fn with_length(mut self, length: u32) -> Self {
        self.length = FieldLength::Fixed(length);
        self
    }

    /// Uses an explicit string-rendering order without affecting wire order.
    pub const fn with_display_order(mut self, display_order: u16) -> Self {
        self.display_order = display_order;
        self
    }

    /// Defers element-count resolution to [`FieldHooks::field_length`].
    pub const fn with_dynamic_length(mut self) -> Self {
        self.length = FieldLength::Dynamic;
        self
    }

    /// Restricts the field to one concrete box version.
    pub const fn with_version(mut self, version: u8) -> Self {
        self.gate.version = VersionGate::Exact(version);
        self
    }

    /// Disables the field for one concrete box version.
    pub const fn without_version(mut self, version: u8) -> Self {
        self.gate.version = VersionGate::Not(version);
        self
    }

    /// Requires the provided flags to be set before the field becomes active.
    pub const fn with_required_flags(mut self, required_flags: u32) -> Self {
        self.gate.required_flags = required_flags;
        self
    }

    /// Requires the provided flags to be clear before the field becomes active.
    pub const fn with_forbidden_flags(mut self, forbidden_flags: u32) -> Self {
        self.gate.forbidden_flags = forbidden_flags;
        self
    }

    /// Defers presence decisions to [`FieldHooks::field_enabled`].
    pub const fn with_dynamic_presence(mut self) -> Self {
        self.gate.dynamic_presence = true;
        self
    }

    /// Requires the field to match a constant value during encode and decode.
    pub const fn with_constant(mut self, constant: &'static str) -> Self {
        self.constant = Some(constant);
        self
    }

    /// Marks the field as an unsigned integer.
    pub const fn as_unsigned(mut self) -> Self {
        self.kind = FieldKind::Unsigned;
        self
    }

    /// Marks the field as a signed integer.
    pub const fn as_signed(mut self) -> Self {
        self.kind = FieldKind::Signed;
        self
    }

    /// Marks the field as a boolean value.
    pub const fn as_boolean(mut self) -> Self {
        self.kind = FieldKind::Boolean;
        self
    }

    /// Marks the field as an opaque byte sequence.
    pub const fn as_bytes(mut self) -> Self {
        self.kind = FieldKind::Bytes;
        self
    }

    /// Enables the internal unsigned-varint encoding for the field.
    pub const fn as_varint(mut self) -> Self {
        self.varint = true;
        self
    }

    /// Marks the field as the box version field.
    pub const fn as_version_field(mut self) -> Self {
        self.role = FieldRole::Version;
        self.kind = FieldKind::Unsigned;
        self
    }

    /// Marks the field as the box flags field.
    pub const fn as_flags_field(mut self) -> Self {
        self.role = FieldRole::Flags;
        self.kind = FieldKind::Unsigned;
        self
    }

    /// Marks the field as extended metadata for future codec use.
    pub const fn as_extended(mut self) -> Self {
        self.extend = true;
        self
    }

    /// Hides the field from the shared stringifier.
    pub const fn as_hidden(mut self) -> Self {
        self.display.hidden = true;
        self
    }

    /// Formats the field as a decimal value.
    pub const fn as_decimal(mut self) -> Self {
        self.display.format = FieldFormat::Decimal;
        self
    }

    /// Formats the field as a hexadecimal value.
    pub const fn as_hex(mut self) -> Self {
        self.display.format = FieldFormat::Hex;
        self
    }

    /// Formats the field as an ISO-639-2 language code.
    pub const fn as_iso639_2(mut self) -> Self {
        self.display.format = FieldFormat::Iso639_2;
        self
    }

    /// Formats the field as a UUID.
    pub const fn as_uuid(mut self) -> Self {
        self.display.format = FieldFormat::Uuid;
        self
    }

    /// Marks the field as a string with the provided storage mode.
    pub const fn as_string(mut self, mode: StringFieldMode) -> Self {
        self.kind = FieldKind::String(mode);
        self.display.format = FieldFormat::String(mode);
        self
    }

    /// Returns `true` when the descriptor is active for the current box state.
    pub fn is_active(&self, owner: &dyn ImmutableBox, hooks: Option<&dyn FieldHooks>) -> bool {
        let version = owner.version();
        if version != ANY_VERSION {
            match self.gate.version {
                VersionGate::Any => {}
                VersionGate::Exact(required) if version != required => return false,
                VersionGate::Not(excluded) if version == excluded => return false,
                VersionGate::Exact(_) | VersionGate::Not(_) => {}
            }
        }

        if self.gate.required_flags != 0 && owner.flags() & self.gate.required_flags == 0 {
            return false;
        }

        if self.gate.forbidden_flags != 0 && owner.flags() & self.gate.forbidden_flags != 0 {
            return false;
        }

        if self.gate.dynamic_presence {
            return select_hooks(owner, hooks)
                .field_enabled(self.name)
                .unwrap_or(false);
        }

        true
    }

    /// Resolves the descriptor into a concrete field instance for the current box state.
    pub fn resolve(
        &self,
        owner: &dyn ImmutableBox,
        hooks: Option<&dyn FieldHooks>,
    ) -> Result<Option<ResolvedField<'_>>, FieldResolutionError> {
        if !self.is_active(owner, hooks) {
            return Ok(None);
        }

        let hooks = select_hooks(owner, hooks);
        let bit_width = match self.bit_width {
            FieldBitWidth::Unspecified => None,
            FieldBitWidth::Fixed(bit_width) => Some(bit_width),
            FieldBitWidth::Dynamic => Some(hooks.field_size(self.name).ok_or(
                FieldResolutionError::MissingDynamicBitWidth {
                    field_name: self.name,
                },
            )?),
        };

        let length = match self.length {
            FieldLength::Unbounded => ResolvedFieldLength::Unbounded,
            FieldLength::Fixed(length) => ResolvedFieldLength::Fixed(length),
            FieldLength::Dynamic => {
                ResolvedFieldLength::Fixed(hooks.field_length(self.name).ok_or(
                    FieldResolutionError::MissingDynamicLength {
                        field_name: self.name,
                    },
                )?)
            }
        };

        Ok(Some(ResolvedField {
            descriptor: self,
            bit_width,
            length,
        }))
    }
}

/// Concrete field length after all dynamic resolution has finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedFieldLength {
    Unbounded,
    Fixed(u32),
}

/// Runtime field description ready for encode, decode, or rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedField<'a> {
    /// Source descriptor.
    pub descriptor: &'a FieldDescriptor,
    /// Resolved bit width, when applicable.
    pub bit_width: Option<u32>,
    /// Resolved element count.
    pub length: ResolvedFieldLength,
}

impl ResolvedField<'_> {
    /// Returns the field name.
    pub const fn name(&self) -> &'static str {
        self.descriptor.name
    }

    /// Returns the stable field order.
    pub const fn order(&self) -> u16 {
        self.descriptor.order
    }

    /// Returns the stable display order.
    pub const fn display_order(&self) -> u16 {
        self.descriptor.display_order
    }
}

/// Errors raised while resolving dynamic descriptor properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldResolutionError {
    MissingDynamicBitWidth { field_name: &'static str },
    MissingDynamicLength { field_name: &'static str },
}

impl fmt::Display for FieldResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDynamicBitWidth { field_name } => {
                write!(f, "missing dynamic bit width for field {field_name}")
            }
            Self::MissingDynamicLength { field_name } => {
                write!(f, "missing dynamic length for field {field_name}")
            }
        }
    }
}

impl Error for FieldResolutionError {}

/// Ordered collection of field descriptors for one box type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldTable {
    fields: &'static [FieldDescriptor],
}

impl FieldTable {
    /// Creates a new field table.
    pub const fn new(fields: &'static [FieldDescriptor]) -> Self {
        Self { fields }
    }

    /// Returns the raw descriptor slice.
    pub const fn fields(&self) -> &'static [FieldDescriptor] {
        self.fields
    }

    /// Returns descriptors sorted by their declared field order.
    pub fn ordered(&self) -> Vec<&'static FieldDescriptor> {
        let mut ordered = self.fields.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|field| field.order);
        ordered
    }

    /// Resolves every active descriptor for the current box state.
    pub fn resolve_active(
        &self,
        owner: &dyn ImmutableBox,
        hooks: Option<&dyn FieldHooks>,
    ) -> Result<Vec<ResolvedField<'static>>, FieldResolutionError> {
        let mut resolved = Vec::with_capacity(self.fields.len());
        for field in self.ordered() {
            if let Some(field) = field.resolve(owner, hooks)? {
                resolved.push(field);
            }
        }
        Ok(resolved)
    }
}

fn select_hooks<'a>(
    owner: &'a dyn FieldHooks,
    hooks: Option<&'a dyn FieldHooks>,
) -> &'a dyn FieldHooks {
    hooks.unwrap_or(owner)
}

/// Owned field value transferred between descriptor code and concrete boxes.
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(tag = "kind", content = "value", rename_all = "snake_case")
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    Unsigned(u64),
    Signed(i64),
    Boolean(bool),
    Bytes(Vec<u8>),
    String(String),
    UnsignedArray(Vec<u64>),
    SignedArray(Vec<i64>),
    BooleanArray(Vec<bool>),
}

impl FieldValue {
    /// Returns a human-readable name for the current value kind.
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Unsigned(_) => "unsigned integer",
            Self::Signed(_) => "signed integer",
            Self::Boolean(_) => "boolean",
            Self::Bytes(_) => "byte sequence",
            Self::String(_) => "string",
            Self::UnsignedArray(_) => "unsigned integer array",
            Self::SignedArray(_) => "signed integer array",
            Self::BooleanArray(_) => "boolean array",
        }
    }
}

/// Errors raised while getting or setting a concrete field value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValueError {
    MissingField {
        field_name: &'static str,
    },
    UnexpectedType {
        field_name: &'static str,
        expected: &'static str,
        actual: &'static str,
    },
    InvalidValue {
        field_name: &'static str,
        reason: &'static str,
    },
}

impl fmt::Display for FieldValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField { field_name } => write!(f, "missing field value for {field_name}"),
            Self::UnexpectedType {
                field_name,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "unexpected field value type for {field_name}: expected {expected}, got {actual}"
                )
            }
            Self::InvalidValue { field_name, reason } => {
                write!(f, "invalid field value for {field_name}: {reason}")
            }
        }
    }
}

impl Error for FieldValueError {}

/// Read access to logical field values on a box.
pub trait FieldValueRead {
    /// Returns the current value for `field_name`.
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError>;
}

/// Write access to logical field values on a box.
pub trait FieldValueWrite {
    /// Applies `value` to `field_name`.
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError>;
}

/// Compile-time descriptor contract for a concrete box type.
pub trait CodecBox: MutableBox + FieldValueRead + FieldValueWrite {
    /// Static descriptor table for the box payload.
    const FIELD_TABLE: FieldTable;
    /// Supported versions for the box type. An empty slice means any version is accepted.
    const SUPPORTED_VERSIONS: &'static [u8] = &[];

    /// Returns `true` when `version` is accepted for this concrete box type.
    fn is_supported_version(&self, version: u8) -> bool {
        Self::SUPPORTED_VERSIONS.is_empty() || Self::SUPPORTED_VERSIONS.contains(&version)
    }

    /// Encodes the full payload manually when the generic descriptor path is not expressive enough.
    fn custom_marshal(&self, _writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        Ok(None)
    }

    /// Decodes the full payload manually when the generic descriptor path is not expressive enough.
    fn custom_unmarshal(
        &mut self,
        _reader: &mut dyn ReadSeek,
        _payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        Ok(None)
    }
}

/// Object-safe view of the descriptor-backed box surface.
pub trait CodecDescription: MutableBox + FieldValueRead + FieldValueWrite {
    /// Returns the runtime field table for the box.
    fn field_table(&self) -> FieldTable;

    /// Returns the supported versions for the box type.
    fn supported_versions(&self) -> &'static [u8];

    /// Returns `true` when `version` is supported.
    fn is_supported_version(&self, version: u8) -> bool;

    /// Encodes the full payload manually when the generic descriptor path is not expressive enough.
    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError>;

    /// Decodes the full payload manually when the generic descriptor path is not expressive enough.
    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError>;
}

impl<T> CodecDescription for T
where
    T: CodecBox + ?Sized,
{
    fn field_table(&self) -> FieldTable {
        T::FIELD_TABLE
    }

    fn supported_versions(&self) -> &'static [u8] {
        T::SUPPORTED_VERSIONS
    }

    fn is_supported_version(&self, version: u8) -> bool {
        <T as CodecBox>::is_supported_version(self, version)
    }

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        <T as CodecBox>::custom_marshal(self, writer)
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        <T as CodecBox>::custom_unmarshal(self, reader, payload_size)
    }
}

/// Runtime-erased descriptor-backed box that still supports downcasting.
pub trait DynCodecBox: CodecDescription {
    /// Returns a shared `Any` view for downcasting.
    fn as_any(&self) -> &dyn Any;

    /// Returns a mutable `Any` view for downcasting.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T> DynCodecBox for T
where
    T: CodecBox + 'static,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Errors raised while encoding or decoding descriptor-backed boxes.
#[derive(Debug)]
pub enum CodecError {
    Io(io::Error),
    FieldResolution(FieldResolutionError),
    FieldValue(FieldValueError),
    MissingBitWidth {
        field_name: &'static str,
    },
    InvalidBitWidth {
        field_name: &'static str,
        bit_width: u32,
    },
    InvalidLength {
        field_name: &'static str,
        expected: usize,
        actual: usize,
    },
    NumericOverflow {
        field_name: &'static str,
        bit_width: u32,
    },
    ConstantMismatch {
        field_name: &'static str,
        constant: &'static str,
    },
    InvalidConstant {
        field_name: &'static str,
        constant: &'static str,
    },
    UnsupportedVarintWidth {
        field_name: &'static str,
    },
    VarintOverflow {
        field_name: &'static str,
        value: u64,
    },
    UnsupportedVersion {
        box_type: FourCc,
        version: u8,
    },
    UnknownBoxType {
        box_type: FourCc,
    },
    InvalidUtf8 {
        field_name: &'static str,
    },
    InvalidUnboundedLength {
        field_name: &'static str,
        bit_width: u32,
        remaining_bits: u64,
    },
    UnsupportedFixedLengthString {
        field_name: &'static str,
    },
    InvalidBoxAlignment {
        box_type: FourCc,
        bit_count: u64,
    },
    Overrun {
        box_type: FourCc,
        payload_size: u64,
        bit_count: u64,
    },
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::FieldResolution(error) => error.fmt(f),
            Self::FieldValue(error) => error.fmt(f),
            Self::MissingBitWidth { field_name } => {
                write!(f, "missing bit width for field {field_name}")
            }
            Self::InvalidBitWidth {
                field_name,
                bit_width,
            } => {
                write!(f, "invalid bit width for field {field_name}: {bit_width}")
            }
            Self::InvalidLength {
                field_name,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "invalid element count for field {field_name}: expected {expected}, got {actual}"
                )
            }
            Self::NumericOverflow {
                field_name,
                bit_width,
            } => {
                write!(
                    f,
                    "numeric value does not fit field {field_name} with width {bit_width}"
                )
            }
            Self::ConstantMismatch {
                field_name,
                constant,
            } => {
                write!(
                    f,
                    "constant mismatch for field {field_name}: expected {constant}"
                )
            }
            Self::InvalidConstant {
                field_name,
                constant,
            } => {
                write!(f, "invalid constant for field {field_name}: {constant}")
            }
            Self::UnsupportedVarintWidth { field_name } => {
                write!(f, "field {field_name} uses an unsupported varint width")
            }
            Self::VarintOverflow { field_name, value } => {
                write!(f, "varint value {value} does not fit field {field_name}")
            }
            Self::UnsupportedVersion { box_type, version } => {
                write!(f, "unsupported box version {version} for type {box_type}")
            }
            Self::UnknownBoxType { box_type } => {
                write!(f, "no registered box definition for type {box_type}")
            }
            Self::InvalidUtf8 { field_name } => {
                write!(f, "field {field_name} does not contain valid UTF-8")
            }
            Self::InvalidUnboundedLength {
                field_name,
                bit_width,
                remaining_bits,
            } => {
                write!(
                    f,
                    "field {field_name} cannot consume {remaining_bits} remaining bits with width {bit_width}"
                )
            }
            Self::UnsupportedFixedLengthString { field_name } => {
                write!(
                    f,
                    "field {field_name} uses a fixed-length string mode that is not supported"
                )
            }
            Self::InvalidBoxAlignment {
                box_type,
                bit_count,
            } => {
                write!(
                    f,
                    "box size is not multiple of 8 bits: type={box_type}, bits={bit_count}"
                )
            }
            Self::Overrun {
                box_type,
                payload_size,
                bit_count,
            } => {
                write!(
                    f,
                    "overrun error: type={box_type}, size={payload_size}, bits={bit_count}"
                )
            }
        }
    }
}

impl Error for CodecError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::FieldResolution(error) => Some(error),
            Self::FieldValue(error) => Some(error),
            Self::MissingBitWidth { .. }
            | Self::InvalidBitWidth { .. }
            | Self::InvalidLength { .. }
            | Self::NumericOverflow { .. }
            | Self::ConstantMismatch { .. }
            | Self::InvalidConstant { .. }
            | Self::UnsupportedVarintWidth { .. }
            | Self::VarintOverflow { .. }
            | Self::UnsupportedVersion { .. }
            | Self::UnknownBoxType { .. }
            | Self::InvalidUtf8 { .. }
            | Self::InvalidUnboundedLength { .. }
            | Self::UnsupportedFixedLengthString { .. }
            | Self::InvalidBoxAlignment { .. }
            | Self::Overrun { .. } => None,
        }
    }
}

impl From<io::Error> for CodecError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FieldResolutionError> for CodecError {
    fn from(error: FieldResolutionError) -> Self {
        Self::FieldResolution(error)
    }
}

impl From<FieldValueError> for CodecError {
    fn from(error: FieldValueError) -> Self {
        Self::FieldValue(error)
    }
}

/// Encodes a concrete box payload into `writer`.
pub fn marshal<W, B>(writer: W, src: &B, hooks: Option<&dyn FieldHooks>) -> Result<u64, CodecError>
where
    W: Write,
    B: CodecBox,
{
    marshal_codec(writer, src, hooks)
}

/// Encodes a runtime-erased descriptor-backed box payload into `writer`.
pub fn marshal_dyn<W>(
    writer: W,
    src: &dyn CodecDescription,
    hooks: Option<&dyn FieldHooks>,
) -> Result<u64, CodecError>
where
    W: Write,
{
    marshal_codec(writer, src, hooks)
}

fn marshal_codec<W>(
    writer: W,
    src: &dyn CodecDescription,
    hooks: Option<&dyn FieldHooks>,
) -> Result<u64, CodecError>
where
    W: Write,
{
    let mut writer = writer;
    if let Some(written) = src.custom_marshal(&mut writer)? {
        return Ok(written);
    }

    let fields = src.field_table().resolve_active(src, hooks)?;
    let mut encoder = Encoder::new(writer, src.box_type());
    for field in fields {
        encoder.encode_field(src, field)?;
    }

    if !encoder.written_bits.is_multiple_of(8) {
        return Err(CodecError::InvalidBoxAlignment {
            box_type: src.box_type(),
            bit_count: encoder.written_bits,
        });
    }

    Ok(encoder.written_bits / 8)
}

/// Decodes a concrete box payload from `reader`.
pub fn unmarshal<R, B>(
    reader: &mut R,
    payload_size: u64,
    dst: &mut B,
    hooks: Option<&dyn FieldHooks>,
) -> Result<u64, CodecError>
where
    R: Read + Seek,
    B: CodecBox,
{
    unmarshal_codec(reader, payload_size, dst, hooks)
}

/// Decodes a runtime-erased descriptor-backed box payload from `reader`.
pub fn unmarshal_dyn<R>(
    reader: &mut R,
    payload_size: u64,
    dst: &mut dyn CodecDescription,
    hooks: Option<&dyn FieldHooks>,
) -> Result<u64, CodecError>
where
    R: Read + Seek,
{
    unmarshal_codec(reader, payload_size, dst, hooks)
}

/// Constructs and decodes a box using the registry entry for `box_type`.
pub fn unmarshal_any<R>(
    reader: &mut R,
    payload_size: u64,
    box_type: FourCc,
    registry: &BoxRegistry,
    hooks: Option<&dyn FieldHooks>,
) -> Result<(Box<dyn DynCodecBox>, u64), CodecError>
where
    R: Read + Seek,
{
    unmarshal_any_with_context(
        reader,
        payload_size,
        box_type,
        registry,
        BoxLookupContext::new(),
        hooks,
    )
}

/// Constructs and decodes a box using the registration active for `box_type` in `context`.
pub fn unmarshal_any_with_context<R>(
    reader: &mut R,
    payload_size: u64,
    box_type: FourCc,
    registry: &BoxRegistry,
    context: BoxLookupContext,
    hooks: Option<&dyn FieldHooks>,
) -> Result<(Box<dyn DynCodecBox>, u64), CodecError>
where
    R: Read + Seek,
{
    let mut boxed = registry
        .new_box_with_context(box_type, context)
        .ok_or(CodecError::UnknownBoxType { box_type })?;
    let read = unmarshal_dyn(reader, payload_size, boxed.as_mut(), hooks)?;
    Ok((boxed, read))
}

fn unmarshal_codec<R>(
    reader: &mut R,
    payload_size: u64,
    dst: &mut dyn CodecDescription,
    hooks: Option<&dyn FieldHooks>,
) -> Result<u64, CodecError>
where
    R: Read + Seek,
{
    let start = reader.stream_position()?;
    let original_version = dst.version();
    let original_flags = dst.flags();
    dst.set_version(ANY_VERSION);
    dst.before_unmarshal(reader, payload_size)?;

    let result = if let Some(read) = dst.custom_unmarshal(reader, payload_size)? {
        Ok(read)
    } else {
        let mut decoder = Decoder::new(reader, payload_size, dst.box_type());
        decoder
            .decode_box(dst, hooks)
            .map(|read_bits| read_bits / 8)
    };

    match result {
        Ok(read_bytes) => Ok(read_bytes),
        Err(error @ CodecError::UnsupportedVersion { .. }) => {
            reader.seek(SeekFrom::Start(start))?;
            dst.set_version(original_version);
            dst.set_flags(original_flags);
            Err(error)
        }
        Err(error) => Err(error),
    }
}

struct Encoder<W> {
    writer: BitWriter<W>,
    written_bits: u64,
}

impl<W: Write> Encoder<W> {
    fn new(writer: W, _box_type: FourCc) -> Self {
        Self {
            writer: BitWriter::new(writer),
            written_bits: 0,
        }
    }

    fn encode_field(
        &mut self,
        src: &dyn CodecDescription,
        field: ResolvedField<'_>,
    ) -> Result<(), CodecError> {
        if let Some(value) = constant_field_value(field)? {
            return self.encode_value(field, value);
        }

        match field.descriptor.role {
            FieldRole::Version => {
                let bit_width = require_bit_width(field)?;
                self.write_unsigned(field.name(), u64::from(src.version()), bit_width)?;
            }
            FieldRole::Flags => {
                let bit_width = require_bit_width(field)?;
                self.write_unsigned(field.name(), u64::from(src.flags()), bit_width)?;
            }
            FieldRole::Data => {
                let value = src.field_value(field.name())?;
                self.encode_value(field, value)?;
            }
        }

        Ok(())
    }

    fn encode_value(
        &mut self,
        field: ResolvedField<'_>,
        value: FieldValue,
    ) -> Result<(), CodecError> {
        match field.descriptor.kind {
            FieldKind::Unsigned => {
                if field.descriptor.varint {
                    let value = expect_unsigned(field.name(), &value)?;
                    self.write_uvarint(field.name(), value)?;
                    return Ok(());
                }

                let width = require_bit_width(field)?;
                match value {
                    FieldValue::Unsigned(value) => {
                        self.require_scalar_length(field)?;
                        self.write_unsigned(field.name(), value, width)?;
                    }
                    FieldValue::UnsignedArray(values) => {
                        self.require_length(field, values.len())?;
                        for value in values {
                            self.write_unsigned(field.name(), value, width)?;
                        }
                    }
                    other => {
                        return Err(FieldValueError::UnexpectedType {
                            field_name: field.name(),
                            expected: "unsigned integer",
                            actual: other.kind_name(),
                        }
                        .into());
                    }
                }
            }
            FieldKind::Signed => {
                let width = require_bit_width(field)?;
                match value {
                    FieldValue::Signed(value) => {
                        self.require_scalar_length(field)?;
                        self.write_signed(field.name(), value, width)?;
                    }
                    FieldValue::SignedArray(values) => {
                        self.require_length(field, values.len())?;
                        for value in values {
                            self.write_signed(field.name(), value, width)?;
                        }
                    }
                    other => {
                        return Err(FieldValueError::UnexpectedType {
                            field_name: field.name(),
                            expected: "signed integer",
                            actual: other.kind_name(),
                        }
                        .into());
                    }
                }
            }
            FieldKind::Boolean => {
                let width = require_bit_width(field)?;
                match value {
                    FieldValue::Boolean(value) => {
                        self.require_scalar_length(field)?;
                        self.write_boolean(field.name(), value, width)?;
                    }
                    FieldValue::BooleanArray(values) => {
                        self.require_length(field, values.len())?;
                        for value in values {
                            self.write_boolean(field.name(), value, width)?;
                        }
                    }
                    other => {
                        return Err(FieldValueError::UnexpectedType {
                            field_name: field.name(),
                            expected: "boolean",
                            actual: other.kind_name(),
                        }
                        .into());
                    }
                }
            }
            FieldKind::Bytes => {
                let width = require_bit_width(field)?;
                if width != 8 {
                    return Err(CodecError::InvalidBitWidth {
                        field_name: field.name(),
                        bit_width: width,
                    });
                }

                let bytes = expect_bytes(field.name(), &value)?;
                self.require_length(field, bytes.len())?;
                self.write_bytes(bytes)?;
            }
            FieldKind::String(mode) => {
                let string = expect_string(field.name(), &value)?;
                self.write_string(field, string, mode)?;
            }
        }

        Ok(())
    }

    fn require_scalar_length(&self, field: ResolvedField<'_>) -> Result<(), CodecError> {
        match field.length {
            ResolvedFieldLength::Unbounded | ResolvedFieldLength::Fixed(1) => Ok(()),
            ResolvedFieldLength::Fixed(expected) => Err(CodecError::InvalidLength {
                field_name: field.name(),
                expected: expected as usize,
                actual: 1,
            }),
        }
    }

    fn require_length(
        &self,
        field: ResolvedField<'_>,
        actual_len: usize,
    ) -> Result<(), CodecError> {
        if let ResolvedFieldLength::Fixed(expected) = field.length
            && actual_len != expected as usize
        {
            return Err(CodecError::InvalidLength {
                field_name: field.name(),
                expected: expected as usize,
                actual: actual_len,
            });
        }

        Ok(())
    }

    fn write_unsigned(
        &mut self,
        field_name: &'static str,
        value: u64,
        bit_width: u32,
    ) -> Result<(), CodecError> {
        validate_unsigned_width(field_name, value, bit_width)?;
        self.writer
            .write_bits(&value.to_be_bytes(), bit_width as usize)?;
        self.written_bits += u64::from(bit_width);
        Ok(())
    }

    fn write_signed(
        &mut self,
        field_name: &'static str,
        value: i64,
        bit_width: u32,
    ) -> Result<(), CodecError> {
        let encoded = encode_signed(field_name, value, bit_width)?;
        self.writer
            .write_bits(&encoded.to_be_bytes(), bit_width as usize)?;
        self.written_bits += u64::from(bit_width);
        Ok(())
    }

    fn write_boolean(
        &mut self,
        field_name: &'static str,
        value: bool,
        bit_width: u32,
    ) -> Result<(), CodecError> {
        validate_width(field_name, bit_width)?;
        let bits = if value {
            if bit_width == 64 {
                u64::MAX
            } else {
                (1_u64 << bit_width) - 1
            }
        } else {
            0
        };

        self.writer
            .write_bits(&bits.to_be_bytes(), bit_width as usize)?;
        self.written_bits += u64::from(bit_width);
        Ok(())
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        for byte in bytes {
            self.writer.write_bits(&[*byte], 8)?;
            self.written_bits += 8;
        }
        Ok(())
    }

    fn write_string(
        &mut self,
        field: ResolvedField<'_>,
        value: &str,
        mode: StringFieldMode,
    ) -> Result<(), CodecError> {
        match (mode, field.length) {
            (StringFieldMode::RawBox, ResolvedFieldLength::Fixed(expected)) => {
                if value.len() != expected as usize {
                    return Err(CodecError::InvalidLength {
                        field_name: field.name(),
                        expected: expected as usize,
                        actual: value.len(),
                    });
                }
            }
            (StringFieldMode::RawBox, ResolvedFieldLength::Unbounded) => {}
            (_, ResolvedFieldLength::Unbounded) => {}
            (_, ResolvedFieldLength::Fixed(expected)) => {
                let actual = value.len() + 1;
                if actual != expected as usize {
                    return Err(CodecError::InvalidLength {
                        field_name: field.name(),
                        expected: expected as usize,
                        actual,
                    });
                }
            }
        }

        self.write_bytes(value.as_bytes())?;
        if !matches!(mode, StringFieldMode::RawBox) {
            self.write_bytes(&[0])?;
        }
        Ok(())
    }

    fn write_uvarint(&mut self, field_name: &'static str, value: u64) -> Result<(), CodecError> {
        if value > 0x0fff_ffff {
            return Err(CodecError::VarintOverflow { field_name, value });
        }

        for shift in [21_u32, 14, 7] {
            let octet = (((value >> shift) as u8) & 0x7f) | 0x80;
            self.write_bytes(&[octet])?;
        }
        self.write_bytes(&[(value as u8) & 0x7f])?;
        Ok(())
    }
}

struct Decoder<'a, R> {
    reader: BitReader<&'a mut R>,
    box_type: FourCc,
    payload_size: u64,
    read_bits: u64,
}

impl<'a, R: Read + Seek> Decoder<'a, R> {
    fn new(reader: &'a mut R, payload_size: u64, box_type: FourCc) -> Self {
        Self {
            reader: BitReader::new(reader),
            box_type,
            payload_size,
            read_bits: 0,
        }
    }

    fn decode_box(
        &mut self,
        dst: &mut dyn CodecDescription,
        hooks: Option<&dyn FieldHooks>,
    ) -> Result<u64, CodecError> {
        for descriptor in dst.field_table().ordered() {
            if let Some(field) = descriptor.resolve(dst, hooks)? {
                self.decode_field(dst, field, hooks)?;
            }
        }

        if !self.read_bits.is_multiple_of(8) {
            return Err(CodecError::InvalidBoxAlignment {
                box_type: self.box_type,
                bit_count: self.read_bits,
            });
        }

        if self.read_bits > self.payload_size.saturating_mul(8) {
            return Err(CodecError::Overrun {
                box_type: self.box_type,
                payload_size: self.payload_size,
                bit_count: self.read_bits,
            });
        }

        Ok(self.read_bits)
    }

    fn decode_field(
        &mut self,
        dst: &mut dyn CodecDescription,
        field: ResolvedField<'_>,
        hooks: Option<&dyn FieldHooks>,
    ) -> Result<(), CodecError> {
        if let Some(constant) = field.descriptor.constant {
            self.verify_constant(field, constant)?;
            return Ok(());
        }

        match field.descriptor.role {
            FieldRole::Version => {
                let bit_width = require_bit_width(field)?;
                let version = self.read_unsigned(field.name(), bit_width)?;
                let version = u8::try_from(version).map_err(|_| CodecError::NumericOverflow {
                    field_name: field.name(),
                    bit_width,
                })?;
                dst.set_version(version);
                if !CodecDescription::is_supported_version(dst, version) {
                    return Err(CodecError::UnsupportedVersion {
                        box_type: dst.box_type(),
                        version,
                    });
                }
            }
            FieldRole::Flags => {
                let bit_width = require_bit_width(field)?;
                let flags = self.read_unsigned(field.name(), bit_width)?;
                let flags = u32::try_from(flags).map_err(|_| CodecError::NumericOverflow {
                    field_name: field.name(),
                    bit_width,
                })?;
                dst.set_flags(flags);
            }
            FieldRole::Data => {
                let value = self.read_value(field, select_hooks(dst, hooks))?;
                dst.set_field_value(field.name(), value)?;
            }
        }

        Ok(())
    }

    fn read_value(
        &mut self,
        field: ResolvedField<'_>,
        hooks: &dyn FieldHooks,
    ) -> Result<FieldValue, CodecError> {
        match field.descriptor.kind {
            FieldKind::Unsigned => {
                if field.descriptor.varint {
                    return Ok(FieldValue::Unsigned(self.read_uvarint(field.name())?));
                }

                let width = require_bit_width(field)?;
                if field_is_scalar(field) {
                    Ok(FieldValue::Unsigned(
                        self.read_unsigned(field.name(), width)?,
                    ))
                } else {
                    let count = self.element_count(field, width)?;
                    let mut values = Vec::with_capacity(untrusted_prealloc_hint(count));
                    for _ in 0..count {
                        values.push(self.read_unsigned(field.name(), width)?);
                    }
                    Ok(FieldValue::UnsignedArray(values))
                }
            }
            FieldKind::Signed => {
                let width = require_bit_width(field)?;
                if field_is_scalar(field) {
                    Ok(FieldValue::Signed(self.read_signed(field.name(), width)?))
                } else {
                    let count = self.element_count(field, width)?;
                    let mut values = Vec::with_capacity(untrusted_prealloc_hint(count));
                    for _ in 0..count {
                        values.push(self.read_signed(field.name(), width)?);
                    }
                    Ok(FieldValue::SignedArray(values))
                }
            }
            FieldKind::Boolean => {
                let width = require_bit_width(field)?;
                if field_is_scalar(field) {
                    Ok(FieldValue::Boolean(self.read_boolean(field.name(), width)?))
                } else {
                    let count = self.element_count(field, width)?;
                    let mut values = Vec::with_capacity(untrusted_prealloc_hint(count));
                    for _ in 0..count {
                        values.push(self.read_boolean(field.name(), width)?);
                    }
                    Ok(FieldValue::BooleanArray(values))
                }
            }
            FieldKind::Bytes => {
                let width = require_bit_width(field)?;
                if width != 8 {
                    return Err(CodecError::InvalidBitWidth {
                        field_name: field.name(),
                        bit_width: width,
                    });
                }
                let count = self.element_count(field, width)?;
                Ok(FieldValue::Bytes(self.read_exact_bytes(count)?))
            }
            FieldKind::String(mode) => {
                Ok(FieldValue::String(self.read_string(field, mode, hooks)?))
            }
        }
    }

    fn verify_constant(
        &mut self,
        field: ResolvedField<'_>,
        constant: &'static str,
    ) -> Result<(), CodecError> {
        match field.descriptor.kind {
            FieldKind::Unsigned => {
                if field.descriptor.varint {
                    let value = self.read_uvarint(field.name())?;
                    let expected = parse_unsigned_constant(field.name(), constant)?;
                    if value != expected {
                        return Err(CodecError::ConstantMismatch {
                            field_name: field.name(),
                            constant,
                        });
                    }
                } else {
                    let bit_width = require_bit_width(field)?;
                    let value = self.read_unsigned(field.name(), bit_width)?;
                    let expected = parse_unsigned_constant(field.name(), constant)?;
                    if value != expected {
                        return Err(CodecError::ConstantMismatch {
                            field_name: field.name(),
                            constant,
                        });
                    }
                }
            }
            FieldKind::Signed => {
                let bit_width = require_bit_width(field)?;
                let value = self.read_signed(field.name(), bit_width)?;
                let expected = parse_signed_constant(field.name(), constant)?;
                if value != expected {
                    return Err(CodecError::ConstantMismatch {
                        field_name: field.name(),
                        constant,
                    });
                }
            }
            FieldKind::Boolean => {
                let bit_width = require_bit_width(field)?;
                let value = self.read_boolean(field.name(), bit_width)?;
                let expected = parse_unsigned_constant(field.name(), constant)? != 0;
                if value != expected {
                    return Err(CodecError::ConstantMismatch {
                        field_name: field.name(),
                        constant,
                    });
                }
            }
            FieldKind::Bytes | FieldKind::String(_) => {
                return Err(CodecError::InvalidConstant {
                    field_name: field.name(),
                    constant,
                });
            }
        }

        Ok(())
    }

    fn element_count(&self, field: ResolvedField<'_>, bit_width: u32) -> Result<usize, CodecError> {
        match field.length {
            ResolvedFieldLength::Fixed(length) => Ok(length as usize),
            ResolvedFieldLength::Unbounded => {
                let remaining_bits = self.remaining_bits();
                if !remaining_bits.is_multiple_of(u64::from(bit_width)) {
                    return Err(CodecError::InvalidUnboundedLength {
                        field_name: field.name(),
                        bit_width,
                        remaining_bits,
                    });
                }
                Ok((remaining_bits / u64::from(bit_width)) as usize)
            }
        }
    }

    fn read_unsigned(
        &mut self,
        field_name: &'static str,
        bit_width: u32,
    ) -> Result<u64, CodecError> {
        validate_width(field_name, bit_width)?;
        let data = self.reader.read_bits(bit_width as usize)?;
        self.read_bits += u64::from(bit_width);

        let mut value = 0_u64;
        for byte in data {
            value = (value << 8) | u64::from(byte);
        }
        Ok(value)
    }

    fn read_signed(&mut self, field_name: &'static str, bit_width: u32) -> Result<i64, CodecError> {
        let value = self.read_unsigned(field_name, bit_width)?;
        if bit_width == 64 {
            return Ok(value as i64);
        }

        let sign_mask = 1_u64 << (bit_width - 1);
        if value & sign_mask == 0 {
            return Ok(value as i64);
        }

        let extended = value | (!0_u64 << bit_width);
        Ok(extended as i64)
    }

    fn read_boolean(
        &mut self,
        field_name: &'static str,
        bit_width: u32,
    ) -> Result<bool, CodecError> {
        Ok(self.read_unsigned(field_name, bit_width)? != 0)
    }

    fn read_exact_bytes(&mut self, count: usize) -> Result<Vec<u8>, CodecError> {
        // Box lengths come from the bitstream, so avoid trusting them for large
        // upfront allocations before we have actually read the bytes.
        read_exact_vec_untrusted(&mut self.reader, count)
            .inspect(|_| {
                self.read_bits += (count as u64) * 8;
            })
            .map_err(CodecError::Io)
    }

    fn read_string(
        &mut self,
        field: ResolvedField<'_>,
        mode: StringFieldMode,
        hooks: &dyn FieldHooks,
    ) -> Result<String, CodecError> {
        let width = require_bit_width(field)?;
        if width != 8 {
            return Err(CodecError::InvalidBitWidth {
                field_name: field.name(),
                bit_width: width,
            });
        }

        let bytes = match mode {
            StringFieldMode::RawBox => {
                let count = match field.length {
                    ResolvedFieldLength::Fixed(length) => length as usize,
                    ResolvedFieldLength::Unbounded => {
                        let remaining_bits = self.remaining_bits();
                        if !remaining_bits.is_multiple_of(8) {
                            return Err(CodecError::InvalidUnboundedLength {
                                field_name: field.name(),
                                bit_width: 8,
                                remaining_bits,
                            });
                        }
                        (remaining_bits / 8) as usize
                    }
                };
                self.read_exact_bytes(count)?
            }
            StringFieldMode::NullTerminated => {
                self.read_c_string(field.name(), string_budget(field.length), hooks)?
            }
            StringFieldMode::PascalCompatible => {
                if let Some(string) =
                    self.try_read_pascal_string(field.name(), string_budget(field.length), hooks)?
                {
                    string.into_bytes()
                } else {
                    self.read_c_string(field.name(), string_budget(field.length), hooks)?
                }
            }
        };

        String::from_utf8(bytes).map_err(|_| CodecError::InvalidUtf8 {
            field_name: field.name(),
        })
    }

    fn read_c_string(
        &mut self,
        field_name: &'static str,
        budget: Option<usize>,
        hooks: &dyn FieldHooks,
    ) -> Result<Vec<u8>, CodecError> {
        let mut bytes = Vec::new();
        let mut terminated = false;

        loop {
            if self.remaining_bits() == 0 {
                break;
            }

            if let Some(limit) = budget
                && bytes.len() >= limit
            {
                break;
            }

            let octet = self.reader.read_bits(8)?;
            self.read_bits += 8;
            if octet[0] == 0 {
                terminated = true;
                break;
            }

            bytes.push(octet[0]);
        }

        if budget.is_none()
            && terminated
            && hooks
                .consume_remaining_bytes_after_string(field_name)
                .unwrap_or(false)
        {
            // Unbounded C-style strings occupy the rest of the payload even when
            // the visible text ends earlier, so consume any trailing padding.
            while self.remaining_bits() >= 8 {
                self.reader.read_bits(8)?;
                self.read_bits += 8;
            }
        }

        Ok(bytes)
    }

    fn try_read_pascal_string(
        &mut self,
        field_name: &'static str,
        budget: Option<usize>,
        hooks: &dyn FieldHooks,
    ) -> Result<Option<String>, CodecError> {
        let remaining_bytes = self.remaining_bits() / 8;
        if remaining_bytes < 2 {
            return Ok(None);
        }

        if let Some(limit) = budget
            && limit < 2
        {
            return Ok(None);
        }

        let start = self.reader.stream_position()?;

        let mut length = [0_u8; 1];
        self.reader.read_exact(&mut length)?;
        let payload_len = length[0] as usize;

        if let Some(limit) = budget
            && payload_len + 1 > limit
        {
            self.reader.seek(SeekFrom::Start(start))?;
            return Ok(None);
        }

        if payload_len as u64 > remaining_bytes - 1 {
            self.reader.seek(SeekFrom::Start(start))?;
            return Ok(None);
        }

        let mut payload = vec![0_u8; payload_len];
        self.reader.read_exact(&mut payload)?;

        let remaining_after_payload = remaining_bytes - payload_len as u64 - 1;
        let is_pascal = hooks
            .is_pascal_string(field_name, &payload, remaining_after_payload)
            .unwrap_or(false);

        if !is_pascal {
            self.reader.seek(SeekFrom::Start(start))?;
            return Ok(None);
        }

        self.read_bits += ((payload_len + 1) * 8) as u64;
        let string =
            String::from_utf8(payload).map_err(|_| CodecError::InvalidUtf8 { field_name })?;
        Ok(Some(string))
    }

    fn read_uvarint(&mut self, _field_name: &'static str) -> Result<u64, CodecError> {
        let mut value = 0_u64;
        loop {
            let octet = self.reader.read_bits(8)?;
            self.read_bits += 8;

            value = (value << 7) | u64::from(octet[0] & 0x7f);
            if octet[0] & 0x80 == 0 {
                return Ok(value);
            }
        }
    }

    fn remaining_bits(&self) -> u64 {
        self.payload_size
            .saturating_mul(8)
            .saturating_sub(self.read_bits)
    }
}

fn require_bit_width(field: ResolvedField<'_>) -> Result<u32, CodecError> {
    field.bit_width.ok_or(CodecError::MissingBitWidth {
        field_name: field.name(),
    })
}

fn validate_width(field_name: &'static str, bit_width: u32) -> Result<(), CodecError> {
    if bit_width == 0 || bit_width > 64 {
        return Err(CodecError::InvalidBitWidth {
            field_name,
            bit_width,
        });
    }
    Ok(())
}

fn validate_unsigned_width(
    field_name: &'static str,
    value: u64,
    bit_width: u32,
) -> Result<(), CodecError> {
    validate_width(field_name, bit_width)?;
    if bit_width < 64 && value >= (1_u64 << bit_width) {
        return Err(CodecError::NumericOverflow {
            field_name,
            bit_width,
        });
    }
    Ok(())
}

fn encode_signed(field_name: &'static str, value: i64, bit_width: u32) -> Result<u64, CodecError> {
    validate_width(field_name, bit_width)?;

    if bit_width == 64 {
        return Ok(value as u64);
    }

    let minimum = -(1_i128 << (bit_width - 1));
    let maximum = (1_i128 << (bit_width - 1)) - 1;
    let value_i128 = i128::from(value);
    if value_i128 < minimum || value_i128 > maximum {
        return Err(CodecError::NumericOverflow {
            field_name,
            bit_width,
        });
    }

    if value >= 0 {
        Ok(value as u64)
    } else {
        Ok(((1_i128 << bit_width) + value_i128) as u64)
    }
}

fn constant_field_value(field: ResolvedField<'_>) -> Result<Option<FieldValue>, CodecError> {
    let Some(constant) = field.descriptor.constant else {
        return Ok(None);
    };

    let value = match field.descriptor.kind {
        FieldKind::Unsigned => {
            FieldValue::Unsigned(parse_unsigned_constant(field.name(), constant)?)
        }
        FieldKind::Signed => FieldValue::Signed(parse_signed_constant(field.name(), constant)?),
        FieldKind::Boolean => {
            let value = parse_unsigned_constant(field.name(), constant)? != 0;
            FieldValue::Boolean(value)
        }
        FieldKind::Bytes | FieldKind::String(_) => {
            return Err(CodecError::InvalidConstant {
                field_name: field.name(),
                constant,
            });
        }
    };

    Ok(Some(value))
}

fn parse_unsigned_constant(
    field_name: &'static str,
    constant: &'static str,
) -> Result<u64, CodecError> {
    if let Some(hex) = constant
        .strip_prefix("0x")
        .or_else(|| constant.strip_prefix("0X"))
    {
        return u64::from_str_radix(hex, 16).map_err(|_| CodecError::InvalidConstant {
            field_name,
            constant,
        });
    }

    constant
        .parse::<u64>()
        .map_err(|_| CodecError::InvalidConstant {
            field_name,
            constant,
        })
}

fn parse_signed_constant(
    field_name: &'static str,
    constant: &'static str,
) -> Result<i64, CodecError> {
    if let Some(hex) = constant
        .strip_prefix("0x")
        .or_else(|| constant.strip_prefix("0X"))
    {
        return i64::from_str_radix(hex, 16).map_err(|_| CodecError::InvalidConstant {
            field_name,
            constant,
        });
    }

    constant
        .parse::<i64>()
        .map_err(|_| CodecError::InvalidConstant {
            field_name,
            constant,
        })
}

fn expect_unsigned(field_name: &'static str, value: &FieldValue) -> Result<u64, CodecError> {
    match value {
        FieldValue::Unsigned(value) => Ok(*value),
        other => Err(FieldValueError::UnexpectedType {
            field_name,
            expected: "unsigned integer",
            actual: other.kind_name(),
        }
        .into()),
    }
}

fn expect_bytes<'a>(
    field_name: &'static str,
    value: &'a FieldValue,
) -> Result<&'a [u8], CodecError> {
    match value {
        FieldValue::Bytes(bytes) => Ok(bytes.as_slice()),
        other => Err(FieldValueError::UnexpectedType {
            field_name,
            expected: "byte sequence",
            actual: other.kind_name(),
        }
        .into()),
    }
}

fn expect_string<'a>(
    field_name: &'static str,
    value: &'a FieldValue,
) -> Result<&'a str, CodecError> {
    match value {
        FieldValue::String(string) => Ok(string.as_str()),
        other => Err(FieldValueError::UnexpectedType {
            field_name,
            expected: "string",
            actual: other.kind_name(),
        }
        .into()),
    }
}

fn field_is_scalar(field: ResolvedField<'_>) -> bool {
    match field.length {
        ResolvedFieldLength::Unbounded => true,
        ResolvedFieldLength::Fixed(1) => !matches!(field.descriptor.length, FieldLength::Dynamic),
        ResolvedFieldLength::Fixed(_) => false,
    }
}

fn string_budget(length: ResolvedFieldLength) -> Option<usize> {
    match length {
        ResolvedFieldLength::Unbounded => None,
        ResolvedFieldLength::Fixed(length) => Some(length as usize),
    }
}

#[macro_export]
macro_rules! codec_field {
    ($name:literal, $order:expr $(, $method:ident ( $($arg:expr),* $(,)? ) )* $(,)?) => {{
        $crate::codec::FieldDescriptor::new($name, $order)$(.$method($($arg),*))*
    }};
}
