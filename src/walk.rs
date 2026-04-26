//! Depth-first box traversal with path tracking and lazy payload access.

use std::error::Error;
use std::fmt;
#[cfg(feature = "async")]
use std::future::Future;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Deref;
#[cfg(feature = "async")]
use std::pin::Pin;
use std::str::FromStr;

use crate::FourCc;
#[cfg(feature = "async")]
use crate::async_io::{AsyncReadSeek, AsyncWrite};
use crate::boxes::iso14496_12::{
    Ftyp, VisualSampleEntry, split_box_children_with_optional_trailing_bytes,
};
use crate::boxes::metadata::Keys;
use crate::boxes::{BoxLookupContext, BoxRegistry, default_registry};
use crate::codec::{CodecError, DynCodecBox, unmarshal, unmarshal_any_with_context};
use crate::fourcc::ParseFourCcError;
use crate::header::{BoxInfo, HeaderError, SMALL_HEADER_SIZE};
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

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
    children_layout: Option<ChildrenLayout>,
}

/// Boxed future type used by closure-based async walk visitors.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub type AsyncWalkFuture<'a> =
    Pin<Box<dyn Future<Output = Result<WalkControl, WalkError>> + Send + 'a>>;

/// Tokio-based async visitor view of one box during a depth-first structure walk.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub struct AsyncWalkHandle<'a, R> {
    reader: &'a mut R,
    registry: &'a BoxRegistry,
    info: BoxInfo,
    path: BoxPath,
    descendant_lookup_context: BoxLookupContext,
    children_layout: Option<ChildrenLayout>,
}

/// Async visitor interface for the Tokio-based structure walker.
///
/// The first async traversal rollout keeps the existing visitor-driven depth-first walk model but
/// allows the visitor to await payload decode or raw byte reads on the current box.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub trait AsyncWalkVisitor<R>
where
    R: AsyncReadSeek,
    Self: Send,
{
    /// Future returned for one visited box.
    type Future<'a>: Future<Output = Result<WalkControl, WalkError>> + Send + 'a
    where
        Self: 'a,
        R: 'a;

    /// Visits one box and decides whether the walker should descend into its children.
    fn visit<'a, 'r>(&'a mut self, handle: &'a mut AsyncWalkHandle<'r, R>) -> Self::Future<'a>
    where
        'r: 'a;
}

#[cfg(feature = "async")]
impl<R, F> AsyncWalkVisitor<R> for F
where
    R: AsyncReadSeek,
    F: Send + for<'a, 'r> FnMut(&'a mut AsyncWalkHandle<'r, R>) -> AsyncWalkFuture<'a>,
{
    type Future<'a>
        = AsyncWalkFuture<'a>
    where
        Self: 'a,
        R: 'a;

    fn visit<'a, 'r>(&'a mut self, handle: &'a mut AsyncWalkHandle<'r, R>) -> Self::Future<'a>
    where
        'r: 'a,
    {
        self(handle)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChildrenLayout {
    offset: u64,
    size: u64,
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
        self.children_layout = Some(children_layout_for_payload(
            self.reader,
            &self.info,
            payload_size,
            read,
            boxed.as_ref(),
        )?);
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

    fn ensure_children_layout(&mut self) -> Result<ChildrenLayout, WalkError> {
        if let Some(children_layout) = self.children_layout {
            return Ok(children_layout);
        }

        self.read_payload()?;
        if let Some(children_layout) = self.children_layout {
            Ok(children_layout)
        } else {
            unreachable!("read_payload always computes children layout")
        }
    }
}

#[cfg(feature = "async")]
impl<'a, R> AsyncWalkHandle<'a, R>
where
    R: AsyncReadSeek,
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
    pub async fn read_payload_async(&mut self) -> Result<(Box<dyn DynCodecBox>, u64), WalkError> {
        self.info.seek_to_payload_async(self.reader).await?;
        let payload_size = self.info.payload_size()?;
        let payload = crate::codec::read_exact_vec_untrusted_async(
            self.reader,
            usize::try_from(payload_size)
                .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?,
        )
        .await?;
        self.info.seek_to_payload_async(self.reader).await?;

        let mut payload_reader = std::io::Cursor::new(payload.as_slice());
        let (boxed, read) = crate::codec::unmarshal_any_with_context(
            &mut payload_reader,
            payload_size,
            self.info.box_type(),
            self.registry,
            self.info.lookup_context(),
            None,
        )?;
        self.children_layout = Some(children_layout_for_buffered_payload(
            &self.info,
            payload_size,
            read,
            boxed.as_any().is::<VisualSampleEntry>(),
            &payload,
        )?);
        Ok((boxed, read))
    }

    /// Copies the raw payload bytes into `writer` without decoding them.
    pub async fn read_data_async<W>(&mut self, writer: &mut W) -> Result<u64, WalkError>
    where
        W: AsyncWrite + Unpin,
    {
        self.info.seek_to_payload_async(self.reader).await?;
        let payload_size = self.info.payload_size()?;
        let mut limited = (&mut *self.reader).take(payload_size);
        tokio::io::copy(&mut limited, writer)
            .await
            .map_err(WalkError::Io)
    }

    async fn ensure_children_layout_async(&mut self) -> Result<ChildrenLayout, WalkError> {
        if let Some(children_layout) = self.children_layout {
            return Ok(children_layout);
        }

        self.read_payload_async().await?;
        if let Some(children_layout) = self.children_layout {
            Ok(children_layout)
        } else {
            unreachable!("read_payload_async always computes children layout")
        }
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

/// Walks the file from the start in depth-first order through the additive Tokio-based async
/// surface using the built-in registry.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn walk_structure_async<R, V>(reader: &mut R, visitor: V) -> Result<(), WalkError>
where
    R: AsyncReadSeek,
    V: AsyncWalkVisitor<R> + Send,
{
    let registry = default_registry();
    walk_structure_with_registry_async(reader, &registry, visitor).await
}

/// Walks the file from the start in depth-first order through the additive Tokio-based async
/// surface using `registry`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn walk_structure_with_registry_async<R, V>(
    reader: &mut R,
    registry: &BoxRegistry,
    mut visitor: V,
) -> Result<(), WalkError>
where
    R: AsyncReadSeek,
    V: AsyncWalkVisitor<R> + Send,
{
    reader.seek(SeekFrom::Start(0)).await?;
    walk_sequence_async(
        reader,
        registry,
        &mut visitor,
        0,
        true,
        &BoxPath::default(),
        BoxLookupContext::new(),
    )
    .await
}

/// Walks `parent` and any expanded descendants through the additive Tokio-based async surface
/// using the built-in registry.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn walk_structure_from_box_async<R, V>(
    reader: &mut R,
    parent: &BoxInfo,
    visitor: V,
) -> Result<(), WalkError>
where
    R: AsyncReadSeek,
    V: AsyncWalkVisitor<R> + Send,
{
    let registry = default_registry();
    walk_structure_from_box_with_registry_async(reader, parent, &registry, visitor).await
}

/// Walks `parent` and any expanded descendants through the additive Tokio-based async surface
/// using `registry`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn walk_structure_from_box_with_registry_async<R, V>(
    reader: &mut R,
    parent: &BoxInfo,
    registry: &BoxRegistry,
    mut visitor: V,
) -> Result<(), WalkError>
where
    R: AsyncReadSeek,
    V: AsyncWalkVisitor<R> + Send,
{
    let mut parent = *parent;
    walk_box_async(
        reader,
        registry,
        &mut visitor,
        &mut parent,
        &BoxPath::default(),
    )
    .await
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
        children_layout: None,
    };

    let control = visitor(&mut handle)?;
    if matches!(control, WalkControl::Descend) {
        let children_layout = handle.ensure_children_layout()?;
        handle
            .reader
            .seek(SeekFrom::Start(children_layout.offset))?;
        walk_sequence(
            handle.reader,
            handle.registry,
            visitor,
            children_layout.size,
            false,
            &handle.path,
            handle.descendant_lookup_context,
        )?;
    }

    handle.info.seek_to_end(handle.reader)?;
    Ok(())
}

#[cfg(feature = "async")]
async fn walk_sequence_async<R, V>(
    reader: &mut R,
    registry: &BoxRegistry,
    visitor: &mut V,
    mut remaining_size: u64,
    is_root: bool,
    path: &BoxPath,
    mut sibling_lookup_context: BoxLookupContext,
) -> Result<(), WalkError>
where
    R: AsyncReadSeek,
    V: AsyncWalkVisitor<R> + Send,
{
    loop {
        if !is_root && remaining_size < SMALL_HEADER_SIZE {
            break;
        }

        let start = reader.stream_position().await?;
        let mut info = match BoxInfo::read_async(reader).await {
            Ok(info) => info,
            Err(HeaderError::Io(error))
                if is_root && clean_root_eof_async(reader, start, &error).await? =>
            {
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
        walk_box_async(reader, registry, visitor, &mut info, path).await?;

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

#[cfg(feature = "async")]
async fn walk_box_async<R, V>(
    reader: &mut R,
    registry: &BoxRegistry,
    visitor: &mut V,
    info: &mut BoxInfo,
    path: &BoxPath,
) -> Result<(), WalkError>
where
    R: AsyncReadSeek,
    V: AsyncWalkVisitor<R> + Send,
{
    inspect_context_carriers_async(reader, info, path).await?;

    let path = path.child_path(info.box_type());
    let descendant_lookup_context = info.lookup_context().enter(info.box_type());
    let mut handle = AsyncWalkHandle {
        reader,
        registry,
        info: *info,
        path,
        descendant_lookup_context,
        children_layout: None,
    };

    let control = {
        let future = visitor.visit(&mut handle);
        future.await?
    };
    if matches!(control, WalkControl::Descend) {
        let children_layout = handle.ensure_children_layout_async().await?;
        let path = handle.path.clone();
        let descendant_lookup_context = handle.descendant_lookup_context;
        handle
            .reader
            .seek(SeekFrom::Start(children_layout.offset))
            .await?;
        Box::pin(walk_sequence_async(
            handle.reader,
            handle.registry,
            visitor,
            children_layout.size,
            false,
            &path,
            descendant_lookup_context,
        ))
        .await?;
    }

    let info = handle.info;
    info.seek_to_end_async(handle.reader).await?;
    Ok(())
}

fn children_layout_for_payload<R>(
    reader: &mut R,
    info: &BoxInfo,
    payload_size: u64,
    payload_read: u64,
    payload: &dyn DynCodecBox,
) -> Result<ChildrenLayout, WalkError>
where
    R: Read + Seek,
{
    let offset = info.offset() + info.header_size() + payload_read;
    let size = if payload.as_any().is::<VisualSampleEntry>() {
        visual_sample_entry_child_payload_size(
            reader,
            offset,
            payload_size.saturating_sub(payload_read),
        )?
    } else {
        payload_size.saturating_sub(payload_read)
    };

    Ok(ChildrenLayout { offset, size })
}

#[cfg(feature = "async")]
fn children_layout_for_buffered_payload(
    info: &BoxInfo,
    payload_size: u64,
    payload_read: u64,
    is_visual_sample_entry: bool,
    payload: &[u8],
) -> Result<ChildrenLayout, WalkError> {
    let offset = info.offset() + info.header_size() + payload_read;
    let size = if is_visual_sample_entry {
        let payload_read = usize::try_from(payload_read)
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        let remaining = payload
            .get(payload_read..)
            .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
        split_box_children_with_optional_trailing_bytes(remaining) as u64
    } else {
        payload_size.saturating_sub(payload_read)
    };

    Ok(ChildrenLayout { offset, size })
}

fn visual_sample_entry_child_payload_size<R>(
    reader: &mut R,
    extension_offset: u64,
    extension_size: u64,
) -> Result<u64, WalkError>
where
    R: Read + Seek,
{
    let checkpoint = reader.stream_position()?;
    reader.seek(SeekFrom::Start(extension_offset))?;
    let bytes = read_extension_bytes(reader, extension_size)?;
    reader.seek(SeekFrom::Start(checkpoint))?;
    Ok(split_box_children_with_optional_trailing_bytes(&bytes) as u64)
}

fn read_extension_bytes<R>(reader: &mut R, extension_size: u64) -> Result<Vec<u8>, WalkError>
where
    R: Read,
{
    let extension_len = usize::try_from(extension_size).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "payload extension is too large")
    })?;
    let mut bytes = vec![0; extension_len];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
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

#[cfg(feature = "async")]
async fn inspect_context_carriers_async<R>(
    reader: &mut R,
    info: &mut BoxInfo,
    path: &BoxPath,
) -> Result<(), WalkError>
where
    R: AsyncReadSeek,
{
    if path.is_empty() && info.box_type() == FTYP {
        let ftyp = decode_box_async::<_, Ftyp>(reader, info).await?;
        if ftyp.has_compatible_brand(QT_BRAND) {
            info.set_lookup_context(info.lookup_context().with_quicktime_compatible(true));
        }
    }

    if info.box_type() == KEYS {
        let keys = decode_box_async::<_, Keys>(reader, info).await?;
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

#[cfg(feature = "async")]
async fn decode_box_async<R, B>(reader: &mut R, info: &BoxInfo) -> Result<B, WalkError>
where
    R: AsyncReadSeek,
    B: Default + crate::codec::CodecBox + Send,
{
    info.seek_to_payload_async(reader).await?;
    let mut decoded = B::default();
    crate::codec::unmarshal_async(reader, info.payload_size()?, &mut decoded, None).await?;
    info.seek_to_payload_async(reader).await?;
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

#[cfg(feature = "async")]
async fn clean_root_eof_async<R>(
    reader: &mut R,
    start: u64,
    error: &io::Error,
) -> Result<bool, io::Error>
where
    R: AsyncReadSeek,
{
    if error.kind() != io::ErrorKind::UnexpectedEof {
        return Ok(false);
    }

    let end = reader.seek(SeekFrom::End(0)).await?;
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
