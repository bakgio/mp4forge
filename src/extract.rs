//! Path-based box extraction helpers built on the structure walker.
//!
//! This module keeps the existing low-level extraction surface available while also exposing thin
//! typed helpers for callers that already know the payload type they expect at a given path,
//! including exact raw-byte helpers and byte-slice convenience wrappers for in-memory workflows.

use std::any::type_name;
use std::error::Error;
use std::fmt;
use std::io::{self, Cursor, Read, Seek};

use crate::BoxInfo;
use crate::FourCc;
use crate::boxes::{BoxRegistry, default_registry};
use crate::codec::{CodecBox, CodecError, DynCodecBox, unmarshal_any_with_context};
use crate::header::HeaderError;
use crate::walk::{
    BoxPath, PathMatch, WalkControl, WalkError, WalkHandle, walk_structure_from_box_with_registry,
    walk_structure_with_registry,
};

/// Header metadata paired with a decoded runtime box payload.
///
/// Use this when the caller needs both the matched [`BoxInfo`] and direct access to the decoded
/// runtime-erased payload. Callers that already know the concrete payload type can usually prefer
/// [`extract_box_as`] or [`extract_boxes_as`] to avoid manual downcasts.
pub struct ExtractedBox {
    /// Header metadata captured during the structure walk.
    pub info: BoxInfo,
    /// Decoded runtime-erased payload for the extracted box.
    pub payload: Box<dyn DynCodecBox>,
}

/// Extracts every box that matches `path` and returns the matching header metadata.
///
/// When `parent` is present, `path` is evaluated relative to that box. Returns an empty vector
/// when no boxes match.
pub fn extract_box<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    path: BoxPath,
) -> Result<Vec<BoxInfo>, ExtractError>
where
    R: Read + Seek,
{
    let paths = [path];
    extract_boxes(reader, parent, &paths)
}

/// Extracts every box that matches any path in `paths` and returns the matching header metadata.
///
/// When `parent` is present, every path is evaluated relative to that box. Returns an empty vector
/// when no boxes match.
pub fn extract_boxes<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
) -> Result<Vec<BoxInfo>, ExtractError>
where
    R: Read + Seek,
{
    let registry = default_registry();
    extract_boxes_with_registry(reader, parent, paths, &registry)
}

/// Extracts every box that matches `path` and decodes the payloads.
///
/// When `parent` is present, `path` is evaluated relative to that box. Each match is returned as
/// an [`ExtractedBox`] so callers can inspect both the header metadata and decoded payload.
pub fn extract_box_with_payload<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    path: BoxPath,
) -> Result<Vec<ExtractedBox>, ExtractError>
where
    R: Read + Seek,
{
    let paths = [path];
    extract_boxes_with_payload(reader, parent, &paths)
}

/// Extracts every box that matches any path in `paths` and decodes the payloads.
///
/// When `parent` is present, every path is evaluated relative to that box.
pub fn extract_boxes_with_payload<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
) -> Result<Vec<ExtractedBox>, ExtractError>
where
    R: Read + Seek,
{
    let registry = default_registry();
    extract_boxes_with_payload_with_registry(reader, parent, paths, &registry)
}

/// Extracts every box that matches `path`, decodes the payloads, and clones them as `T`.
///
/// This is the smallest high-level extraction helper for common read flows that already know the
/// concrete payload type they expect. It keeps the existing low-level extraction layer intact
/// while removing the repeated downcast boilerplate from call sites.
///
/// When `parent` is present, `path` is evaluated relative to that box.
pub fn extract_box_as<R, T>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    path: BoxPath,
) -> Result<Vec<T>, ExtractError>
where
    R: Read + Seek,
    T: CodecBox + Clone + 'static,
{
    let paths = [path];
    extract_boxes_as(reader, parent, &paths)
}

/// Extracts every box that matches any path in `paths`, decodes the payloads, and clones them as
/// `T`.
///
/// Every matched box must decode to `T`, otherwise [`ExtractError::UnexpectedPayloadType`] is
/// returned with the matched path and offset for diagnostics. Returns an empty vector when no
/// boxes match.
pub fn extract_boxes_as<R, T>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
) -> Result<Vec<T>, ExtractError>
where
    R: Read + Seek,
    T: CodecBox + Clone + 'static,
{
    let registry = default_registry();
    extract_boxes_as_with_registry(reader, parent, paths, &registry)
}

/// Extracts every box that matches `path` and returns each match as exact serialized bytes,
/// including the original box header.
///
/// When `parent` is present, `path` is evaluated relative to that box. Returns an empty vector
/// when no boxes match.
pub fn extract_box_bytes<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    path: BoxPath,
) -> Result<Vec<Vec<u8>>, ExtractError>
where
    R: Read + Seek,
{
    let paths = [path];
    extract_boxes_bytes(reader, parent, &paths)
}

/// Extracts every box that matches any path in `paths` and returns each match as exact serialized
/// bytes, including the original box header.
///
/// When `parent` is present, every path is evaluated relative to that box. The returned bytes are
/// copied directly from the source stream without decoding or re-encoding, so the original header
/// form and payload bytes are preserved verbatim.
pub fn extract_boxes_bytes<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
) -> Result<Vec<Vec<u8>>, ExtractError>
where
    R: Read + Seek,
{
    let registry = default_registry();
    extract_boxes_bytes_with_registry(reader, parent, paths, &registry)
}

/// Extracts every box that matches `path` and returns each matched payload as exact on-disk bytes.
///
/// When `parent` is present, `path` is evaluated relative to that box. For container boxes, the
/// returned payload bytes still include any serialized child boxes because those bytes are part of
/// the matched payload.
pub fn extract_box_payload_bytes<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    path: BoxPath,
) -> Result<Vec<Vec<u8>>, ExtractError>
where
    R: Read + Seek,
{
    let paths = [path];
    extract_boxes_payload_bytes(reader, parent, &paths)
}

/// Extracts every box that matches any path in `paths` and returns each matched payload as exact
/// on-disk bytes.
///
/// When `parent` is present, every path is evaluated relative to that box. The returned bytes are
/// copied directly from the source stream without decoding or re-encoding, preserving the payload
/// exactly as stored in the file.
pub fn extract_boxes_payload_bytes<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
) -> Result<Vec<Vec<u8>>, ExtractError>
where
    R: Read + Seek,
{
    let registry = default_registry();
    extract_boxes_payload_bytes_with_registry(reader, parent, paths, &registry)
}

/// Extracts every box that matches `path`, decodes the payloads, and clones them as `T` from an
/// in-memory MP4 byte slice.
///
/// This is equivalent to calling [`extract_box_as`] with `Cursor<&[u8]>` and no parent box. Paths
/// are always evaluated from the file root. Returns an empty vector when no boxes match.
pub fn extract_box_as_bytes<T>(input: &[u8], path: BoxPath) -> Result<Vec<T>, ExtractError>
where
    T: CodecBox + Clone + 'static,
{
    let paths = [path];
    extract_boxes_as_bytes::<T>(input, &paths)
}

/// Extracts every box that matches any path in `paths`, decodes the payloads, and clones them as
/// `T` from an in-memory MP4 byte slice.
///
/// This is equivalent to calling [`extract_boxes_as`] with `Cursor<&[u8]>` and no parent box.
/// Every matched box must decode to `T`, otherwise
/// [`ExtractError::UnexpectedPayloadType`] is returned with the matched path and offset.
pub fn extract_boxes_as_bytes<T>(input: &[u8], paths: &[BoxPath]) -> Result<Vec<T>, ExtractError>
where
    T: CodecBox + Clone + 'static,
{
    let mut reader = Cursor::new(input);
    extract_boxes_as(&mut reader, None, paths)
}

/// Extracts every box that matches any path in `paths` using `registry` and returns the matching
/// header metadata.
///
/// Use this when custom or context-sensitive box registrations must participate in the extraction.
pub fn extract_boxes_with_registry<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
    registry: &BoxRegistry,
) -> Result<Vec<BoxInfo>, ExtractError>
where
    R: Read + Seek,
{
    Ok(collect_matches(reader, parent, paths, registry)?
        .into_iter()
        .map(|matched| matched.info)
        .collect())
}

/// Extracts every box that matches any path in `paths`, then decodes the payloads with `registry`.
///
/// Use this when custom or context-sensitive box registrations must participate in payload decode.
pub fn extract_boxes_with_payload_with_registry<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
    registry: &BoxRegistry,
) -> Result<Vec<ExtractedBox>, ExtractError>
where
    R: Read + Seek,
{
    let matched_boxes = collect_matches(reader, parent, paths, registry)?;
    let mut matches = Vec::with_capacity(matched_boxes.len());

    for matched in matched_boxes {
        let payload = decode_payload(reader, &matched, registry)?;
        matches.push(ExtractedBox {
            info: matched.info,
            payload,
        });
    }

    Ok(matches)
}

/// Extracts every box that matches any path in `paths` using `registry` and returns each match as
/// exact serialized bytes, including the original box header.
///
/// Use this when custom or context-sensitive box registrations are required to walk into matched
/// subtrees while preserving the matched bytes verbatim.
pub fn extract_boxes_bytes_with_registry<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
    registry: &BoxRegistry,
) -> Result<Vec<Vec<u8>>, ExtractError>
where
    R: Read + Seek,
{
    extract_matched_bytes(reader, parent, paths, registry, ExtractedByteRange::FullBox)
}

/// Extracts every box that matches any path in `paths` using `registry` and returns each matched
/// payload as exact on-disk bytes.
///
/// Use this when custom or context-sensitive box registrations are required to walk into matched
/// subtrees while preserving the matched payload bytes verbatim.
pub fn extract_boxes_payload_bytes_with_registry<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
    registry: &BoxRegistry,
) -> Result<Vec<Vec<u8>>, ExtractError>
where
    R: Read + Seek,
{
    extract_matched_bytes(reader, parent, paths, registry, ExtractedByteRange::Payload)
}

/// Extracts every box that matches any path in `paths`, decodes the payloads with `registry`, and
/// clones them as `T`.
///
/// Use this when the active registry may include custom box registrations and all matched boxes are
/// expected to share the same concrete payload type.
pub fn extract_boxes_as_with_registry<R, T>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
    registry: &BoxRegistry,
) -> Result<Vec<T>, ExtractError>
where
    R: Read + Seek,
    T: CodecBox + Clone + 'static,
{
    let matched_boxes = collect_matches(reader, parent, paths, registry)?;
    let mut payloads = Vec::with_capacity(matched_boxes.len());

    for matched in matched_boxes {
        let payload = decode_payload(reader, &matched, registry)?;
        let typed = payload
            .as_ref()
            .as_any()
            .downcast_ref::<T>()
            .cloned()
            .ok_or_else(|| ExtractError::UnexpectedPayloadType {
                path: matched.path.clone(),
                box_type: matched.info.box_type(),
                offset: matched.info.offset(),
                expected_type: type_name::<T>(),
            })?;
        payloads.push(typed);
    }

    Ok(payloads)
}

struct MatchedBox {
    info: BoxInfo,
    path: BoxPath,
}

#[derive(Clone, Copy)]
enum ExtractedByteRange {
    FullBox,
    Payload,
}

fn collect_matches<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
    registry: &BoxRegistry,
) -> Result<Vec<MatchedBox>, ExtractError>
where
    R: Read + Seek,
{
    validate_paths(paths)?;
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let mut matches = Vec::new();
    let mut visitor = |handle: &mut WalkHandle<'_, R>| {
        if handle.info().box_type() == FourCc::ANY {
            return Ok(WalkControl::Continue);
        }

        let relative_path = if parent.is_some() {
            BoxPath::from(handle.path().as_slice()[1..].to_vec())
        } else {
            handle.path().clone()
        };

        let PathMatch {
            forward_match,
            exact_match,
        } = match_paths(paths, &relative_path);
        if exact_match {
            matches.push(MatchedBox {
                info: *handle.info(),
                path: relative_path.clone(),
            });
        }

        Ok(if forward_match {
            WalkControl::Descend
        } else {
            WalkControl::Continue
        })
    };

    if let Some(parent) = parent {
        walk_structure_from_box_with_registry(reader, parent, registry, &mut visitor)?;
    } else {
        walk_structure_with_registry(reader, registry, &mut visitor)?;
    }

    Ok(matches)
}

fn extract_matched_bytes<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
    registry: &BoxRegistry,
    range: ExtractedByteRange,
) -> Result<Vec<Vec<u8>>, ExtractError>
where
    R: Read + Seek,
{
    let matched_boxes = collect_matches(reader, parent, paths, registry)?;
    let mut extracted = Vec::with_capacity(matched_boxes.len());

    for matched in matched_boxes {
        extracted.push(read_matched_bytes(reader, &matched.info, range)?);
    }

    Ok(extracted)
}

fn decode_payload<R>(
    reader: &mut R,
    matched: &MatchedBox,
    registry: &BoxRegistry,
) -> Result<Box<dyn DynCodecBox>, ExtractError>
where
    R: Read + Seek,
{
    matched.info.seek_to_payload(reader)?;
    let payload_size = matched.info.payload_size()?;
    let (payload, _) = unmarshal_any_with_context(
        reader,
        payload_size,
        matched.info.box_type(),
        registry,
        matched.info.lookup_context(),
        None,
    )
    .map_err(|source| ExtractError::PayloadDecode {
        path: matched.path.clone(),
        box_type: matched.info.box_type(),
        offset: matched.info.offset(),
        source,
    })?;
    Ok(payload)
}

fn read_matched_bytes<R>(
    reader: &mut R,
    info: &BoxInfo,
    range: ExtractedByteRange,
) -> Result<Vec<u8>, ExtractError>
where
    R: Read + Seek,
{
    let len = match range {
        ExtractedByteRange::FullBox => {
            info.seek_to_start(reader)?;
            info.size()
        }
        ExtractedByteRange::Payload => {
            info.seek_to_payload(reader)?;
            info.payload_size()?
        }
    };
    read_exact_bytes(reader, len)
}

fn read_exact_bytes<R>(reader: &mut R, len: u64) -> Result<Vec<u8>, ExtractError>
where
    R: Read,
{
    let mut bytes = usize::try_from(len)
        .map(Vec::with_capacity)
        .unwrap_or_else(|_| Vec::new());

    // `Read::read_to_end` on a `Take` reader does not error on an early underlying EOF, so the
    // copied byte count must be checked explicitly to preserve exact-byte semantics.
    let mut limited = reader.take(len);
    let copied = limited.read_to_end(&mut bytes)? as u64;
    if copied != len {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
    }

    Ok(bytes)
}

fn validate_paths(paths: &[BoxPath]) -> Result<(), ExtractError> {
    if paths.iter().any(BoxPath::is_empty) {
        return Err(ExtractError::EmptyPath);
    }

    Ok(())
}

fn match_paths(paths: &[BoxPath], current: &BoxPath) -> PathMatch {
    paths
        .iter()
        .fold(PathMatch::default(), |mut matched, path| {
            let next = current.compare_with(path);
            matched.forward_match |= next.forward_match;
            matched.exact_match |= next.exact_match;
            matched
        })
}

/// Errors raised while extracting path-matched boxes.
#[derive(Debug)]
pub enum ExtractError {
    /// An I/O operation failed while reading or seeking.
    Io(io::Error),
    /// Box header metadata was invalid or truncated.
    Header(HeaderError),
    /// Payload decode failed before a more specific matched-box context was available.
    Codec(CodecError),
    /// Structure walking failed before a specific extraction match could be reported.
    Walk(WalkError),
    /// One of the requested paths was empty.
    EmptyPath,
    /// A matched payload failed to decode with contextual path metadata.
    PayloadDecode {
        /// Matched path that was being decoded when the failure happened.
        path: BoxPath,
        /// Concrete box type at that matched path.
        box_type: FourCc,
        /// File offset of the matched box header.
        offset: u64,
        /// Underlying decode failure.
        source: CodecError,
    },
    /// A matched payload decoded successfully but did not match the requested concrete type.
    UnexpectedPayloadType {
        /// Matched path whose payload downcast failed.
        path: BoxPath,
        /// Concrete box type at that matched path.
        box_type: FourCc,
        /// File offset of the matched box header.
        offset: u64,
        /// Fully qualified Rust type name requested by the caller.
        expected_type: &'static str,
    },
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Header(error) => error.fmt(f),
            Self::Codec(error) => error.fmt(f),
            Self::Walk(error) => error.fmt(f),
            Self::EmptyPath => f.write_str("box path must not be empty"),
            Self::PayloadDecode {
                path,
                box_type,
                offset,
                source,
            } => write!(
                f,
                "failed to decode payload at {path} (type={box_type}, offset={offset}): {source}"
            ),
            Self::UnexpectedPayloadType {
                path,
                box_type,
                offset,
                expected_type,
            } => write!(
                f,
                "unexpected decoded payload type at {path} (type={box_type}, offset={offset}): expected {expected_type}"
            ),
        }
    }
}

impl Error for ExtractError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::Walk(error) => Some(error),
            Self::PayloadDecode { source, .. } => Some(source),
            Self::EmptyPath | Self::UnexpectedPayloadType { .. } => None,
        }
    }
}

impl From<io::Error> for ExtractError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<HeaderError> for ExtractError {
    fn from(value: HeaderError) -> Self {
        Self::Header(value)
    }
}

impl From<CodecError> for ExtractError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}

impl From<WalkError> for ExtractError {
    fn from(value: WalkError) -> Self {
        Self::Walk(value)
    }
}
