//! Depth-first box traversal with path tracking and lazy payload access.

use std::error::Error;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Deref;
use std::str::FromStr;

use crate::FourCc;
use crate::boxes::iso14496_12::Ftyp;
use crate::boxes::metadata::Keys;
use crate::boxes::{BoxLookupContext, BoxRegistry, default_registry};
use crate::codec::{CodecError, DynCodecBox, unmarshal, unmarshal_any_with_context};
use crate::fourcc::ParseFourCcError;
use crate::header::{BoxInfo, HeaderError, SMALL_HEADER_SIZE};

const FTYP: FourCc = FourCc::from_bytes(*b"ftyp");
const KEYS: FourCc = FourCc::from_bytes(*b"keys");
const QT_BRAND: FourCc = FourCc::from_bytes(*b"qt  ");
const ROOT_MARKER: &str = "<root>";
const WILDCARD_SEGMENT: &str = "*";

/// Depth-first traversal decision returned by a walk visitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkControl {
    /// Skip the current box children and continue with the next sibling.
    Continue,
    /// Expand the current box and visit its children before the next sibling.
    Descend,
}

/// Ordered sequence of box identifiers from the root to the current box.
///
/// Path comparisons used by the extraction and rewrite helpers honor [`FourCc::ANY`] as a
/// wildcard segment.
///
/// In addition to low-level array-based construction, paths can be parsed from slash-delimited
/// strings such as `moov/trak/tkhd`. The segment `*` maps to [`FourCc::ANY`], and the string
/// `<root>` maps to the empty path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoxPath(Vec<FourCc>);

impl BoxPath {
    /// Creates an empty path.
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    /// Returns the path as a borrowed slice.
    pub fn as_slice(&self) -> &[FourCc] {
        &self.0
    }

    /// Returns `true` when the path contains no box identifiers.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the number of box identifiers in the path.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Parses a slash-delimited path string into a [`BoxPath`].
    ///
    /// Each non-wildcard segment must contain exactly four bytes and is parsed using
    /// [`FourCc::from_str`]. The segment `*` maps to [`FourCc::ANY`], and `<root>` returns the
    /// empty path.
    pub fn parse(value: &str) -> Result<Self, ParseBoxPathError> {
        if value == ROOT_MARKER {
            return Ok(Self::empty());
        }

        let mut path = Vec::new();
        for (index, segment) in value.split('/').enumerate() {
            if segment.is_empty() {
                return Err(ParseBoxPathError::EmptySegment { index });
            }
            if segment == ROOT_MARKER {
                return Err(ParseBoxPathError::RootMarkerMustAppearAlone);
            }
            if segment == WILDCARD_SEGMENT {
                path.push(FourCc::ANY);
                continue;
            }

            let box_type =
                FourCc::try_from(segment).map_err(|source| ParseBoxPathError::InvalidSegment {
                    index,
                    segment: segment.to_owned(),
                    source,
                })?;
            path.push(box_type);
        }

        Ok(Self(path))
    }

    fn child_path(&self, box_type: FourCc) -> Self {
        let mut path = self.0.clone();
        path.push(box_type);
        Self(path)
    }

    pub(crate) fn compare_with(&self, other: &Self) -> PathMatch {
        if self.len() > other.len() {
            return PathMatch::default();
        }

        for (lhs, rhs) in self.iter().zip(other.iter()) {
            if !lhs.matches(*rhs) {
                return PathMatch::default();
            }
        }

        if self.len() < other.len() {
            return PathMatch {
                forward_match: true,
                exact_match: false,
            };
        }

        PathMatch {
            forward_match: false,
            exact_match: true,
        }
    }
}

impl Deref for BoxPath {
    type Target = [FourCc];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl fmt::Display for BoxPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("<root>");
        }

        for (index, box_type) in self.0.iter().enumerate() {
            if index != 0 {
                f.write_str("/")?;
            }
            write!(f, "{box_type}")?;
        }

        Ok(())
    }
}

impl From<Vec<FourCc>> for BoxPath {
    fn from(value: Vec<FourCc>) -> Self {
        Self(value)
    }
}

impl TryFrom<&str> for BoxPath {
    type Error = ParseBoxPathError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl<const N: usize> From<[FourCc; N]> for BoxPath {
    fn from(value: [FourCc; N]) -> Self {
        Self(value.into())
    }
}

impl FromIterator<FourCc> for BoxPath {
    fn from_iter<T: IntoIterator<Item = FourCc>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl FromStr for BoxPath {
    type Err = ParseBoxPathError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Error returned when a string cannot be parsed as a [`BoxPath`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseBoxPathError {
    /// One segment between path separators was empty.
    EmptySegment {
        /// Zero-based index of the empty segment.
        index: usize,
    },
    /// One segment was neither `*` nor a valid four-byte [`FourCc`].
    InvalidSegment {
        /// Zero-based index of the invalid segment.
        index: usize,
        /// Original segment text from the parsed path string.
        segment: String,
        /// Underlying four-character-code parse failure.
        source: ParseFourCcError,
    },
    /// The special `<root>` marker was combined with additional segments.
    RootMarkerMustAppearAlone,
}

impl fmt::Display for ParseBoxPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySegment { index } => {
                write!(f, "box path segment {} must not be empty", index + 1)
            }
            Self::InvalidSegment {
                index,
                segment,
                source,
            } => write!(
                f,
                "invalid box path segment {} ({segment:?}): {source}",
                index + 1
            ),
            Self::RootMarkerMustAppearAlone => {
                write!(f, "box path root marker {ROOT_MARKER:?} must appear alone")
            }
        }
    }
}

impl Error for ParseBoxPathError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidSegment { source, .. } => Some(source),
            Self::EmptySegment { .. } | Self::RootMarkerMustAppearAlone => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PathMatch {
    pub(crate) forward_match: bool,
    pub(crate) exact_match: bool,
}

/// Visitor view of one box during a depth-first structure walk.
pub struct WalkHandle<'a, R> {
    reader: &'a mut R,
    registry: &'a BoxRegistry,
    info: BoxInfo,
    path: BoxPath,
    descendant_lookup_context: BoxLookupContext,
    children_offset: Option<u64>,
}

impl<'a, R> WalkHandle<'a, R>
where
    R: Read + Seek,
{
    /// Returns the header metadata for the current box.
    pub const fn info(&self) -> &BoxInfo {
        &self.info
    }

    /// Returns the depth-first path to the current box.
    pub fn path(&self) -> &BoxPath {
        &self.path
    }

    /// Returns the lookup context that will apply to direct children of this box.
    pub const fn descendant_lookup_context(&self) -> BoxLookupContext {
        self.descendant_lookup_context
    }

    /// Returns `true` when the current box type is registered in the active lookup context.
    pub fn is_supported_type(&self) -> bool {
        self.registry
            .is_registered_with_context(self.info.box_type(), self.info.lookup_context())
    }

    /// Decodes the current payload into a descriptor-backed runtime box value.
    pub fn read_payload(&mut self) -> Result<(Box<dyn DynCodecBox>, u64), WalkError> {
        self.info.seek_to_payload(self.reader)?;
        let payload_size = self.info.payload_size()?;
        let (boxed, read) = unmarshal_any_with_context(
            self.reader,
            payload_size,
            self.info.box_type(),
            self.registry,
            self.info.lookup_context(),
            None,
        )?;
        self.children_offset = Some(self.info.offset() + self.info.header_size() + read);
        Ok((boxed, read))
    }

    /// Copies the raw payload bytes into `writer` without decoding them.
    pub fn read_data<W>(&mut self, writer: &mut W) -> Result<u64, WalkError>
    where
        W: Write,
    {
        self.info.seek_to_payload(self.reader)?;
        let payload_size = self.info.payload_size()?;
        let mut limited = (&mut *self.reader).take(payload_size);
        io::copy(&mut limited, writer).map_err(WalkError::Io)
    }

    fn ensure_children_offset(&mut self) -> Result<u64, WalkError> {
        if let Some(children_offset) = self.children_offset {
            return Ok(children_offset);
        }

        let (_, read) = self.read_payload()?;
        Ok(self.info.offset() + self.info.header_size() + read)
    }
}

/// Walks the file from the start in depth-first order using the built-in registry.
pub fn walk_structure<R, F>(reader: &mut R, visitor: F) -> Result<(), WalkError>
where
    R: Read + Seek,
    F: for<'a> FnMut(&mut WalkHandle<'a, R>) -> Result<WalkControl, WalkError>,
{
    let registry = default_registry();
    walk_structure_with_registry(reader, &registry, visitor)
}

/// Walks the file from the start in depth-first order using `registry`.
pub fn walk_structure_with_registry<R, F>(
    reader: &mut R,
    registry: &BoxRegistry,
    mut visitor: F,
) -> Result<(), WalkError>
where
    R: Read + Seek,
    F: for<'a> FnMut(&mut WalkHandle<'a, R>) -> Result<WalkControl, WalkError>,
{
    reader.seek(SeekFrom::Start(0))?;
    walk_sequence(
        reader,
        registry,
        &mut visitor,
        0,
        true,
        &BoxPath::default(),
        BoxLookupContext::new(),
    )
}

/// Walks `parent` and any expanded descendants using the built-in registry.
pub fn walk_structure_from_box<R, F>(
    reader: &mut R,
    parent: &BoxInfo,
    visitor: F,
) -> Result<(), WalkError>
where
    R: Read + Seek,
    F: for<'a> FnMut(&mut WalkHandle<'a, R>) -> Result<WalkControl, WalkError>,
{
    let registry = default_registry();
    walk_structure_from_box_with_registry(reader, parent, &registry, visitor)
}

/// Walks `parent` and any expanded descendants using `registry`.
pub fn walk_structure_from_box_with_registry<R, F>(
    reader: &mut R,
    parent: &BoxInfo,
    registry: &BoxRegistry,
    mut visitor: F,
) -> Result<(), WalkError>
where
    R: Read + Seek,
    F: for<'a> FnMut(&mut WalkHandle<'a, R>) -> Result<WalkControl, WalkError>,
{
    let mut parent = *parent;
    walk_box(
        reader,
        registry,
        &mut visitor,
        &mut parent,
        &BoxPath::default(),
    )
}

fn walk_sequence<R, F>(
    reader: &mut R,
    registry: &BoxRegistry,
    visitor: &mut F,
    mut remaining_size: u64,
    is_root: bool,
    path: &BoxPath,
    mut sibling_lookup_context: BoxLookupContext,
) -> Result<(), WalkError>
where
    R: Read + Seek,
    F: for<'a> FnMut(&mut WalkHandle<'a, R>) -> Result<WalkControl, WalkError>,
{
    loop {
        if !is_root && remaining_size < SMALL_HEADER_SIZE {
            break;
        }

        let start = reader.stream_position()?;
        let mut info = match BoxInfo::read(reader) {
            Ok(info) => info,
            Err(HeaderError::Io(error)) if is_root && clean_root_eof(reader, start, &error)? => {
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };

        if !is_root && info.size() > remaining_size {
            return Err(WalkError::TooLargeBoxSize {
                box_type: info.box_type(),
                size: info.size(),
                available_size: remaining_size,
            });
        }
        if !is_root {
            remaining_size -= info.size();
        }

        info.set_lookup_context(sibling_lookup_context);
        walk_box(reader, registry, visitor, &mut info, path)?;

        if info.lookup_context().is_quicktime_compatible() {
            sibling_lookup_context = sibling_lookup_context.with_quicktime_compatible(true);
        }
        if info.box_type() == KEYS {
            sibling_lookup_context = sibling_lookup_context
                .with_metadata_keys_entry_count(info.lookup_context().metadata_keys_entry_count());
        }
    }

    if !is_root && remaining_size != 0 && !sibling_lookup_context.is_quicktime_compatible() {
        return Err(WalkError::UnexpectedEof);
    }

    Ok(())
}

fn walk_box<R, F>(
    reader: &mut R,
    registry: &BoxRegistry,
    visitor: &mut F,
    info: &mut BoxInfo,
    path: &BoxPath,
) -> Result<(), WalkError>
where
    R: Read + Seek,
    F: for<'a> FnMut(&mut WalkHandle<'a, R>) -> Result<WalkControl, WalkError>,
{
    inspect_context_carriers(reader, info, path)?;

    let path = path.child_path(info.box_type());
    let descendant_lookup_context = info.lookup_context().enter(info.box_type());
    let mut handle = WalkHandle {
        reader,
        registry,
        info: *info,
        path,
        descendant_lookup_context,
        children_offset: None,
    };

    let control = visitor(&mut handle)?;
    if matches!(control, WalkControl::Descend) {
        let children_offset = handle.ensure_children_offset()?;
        let children_size = handle
            .info
            .offset()
            .saturating_add(handle.info.size())
            .saturating_sub(children_offset);
        walk_sequence(
            handle.reader,
            handle.registry,
            visitor,
            children_size,
            false,
            &handle.path,
            handle.descendant_lookup_context,
        )?;
    }

    handle.info.seek_to_end(handle.reader)?;
    Ok(())
}

fn inspect_context_carriers<R>(
    reader: &mut R,
    info: &mut BoxInfo,
    path: &BoxPath,
) -> Result<(), WalkError>
where
    R: Read + Seek,
{
    if path.is_empty() && info.box_type() == FTYP {
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

fn decode_box<R, B>(reader: &mut R, info: &BoxInfo) -> Result<B, WalkError>
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

/// Errors raised while walking a box tree.
#[derive(Debug)]
pub enum WalkError {
    /// An I/O operation failed while reading or seeking.
    Io(io::Error),
    /// Box header metadata was invalid or truncated.
    Header(HeaderError),
    /// Payload decode failed while the walker was inspecting or expanding a box.
    Codec(CodecError),
    /// A child box declared a size larger than the remaining bytes in its parent container.
    TooLargeBoxSize {
        /// Concrete box type whose declared size exceeded the available bytes.
        box_type: FourCc,
        /// Declared child box size.
        size: u64,
        /// Remaining bytes available in the parent container.
        available_size: u64,
    },
    /// A non-QuickTime container ended before all advertised child bytes were consumed.
    UnexpectedEof,
}

impl fmt::Display for WalkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Header(error) => error.fmt(f),
            Self::Codec(error) => error.fmt(f),
            Self::TooLargeBoxSize {
                box_type,
                size,
                available_size,
            } => {
                write!(
                    f,
                    "too large box size: type={box_type}, size={size}, actualBufSize={available_size}"
                )
            }
            Self::UnexpectedEof => f.write_str("unexpected EOF"),
        }
    }
}

impl Error for WalkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::TooLargeBoxSize { .. } | Self::UnexpectedEof => None,
        }
    }
}

impl From<io::Error> for WalkError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<HeaderError> for WalkError {
    fn from(value: HeaderError) -> Self {
        Self::Header(value)
    }
}

impl From<CodecError> for WalkError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}
