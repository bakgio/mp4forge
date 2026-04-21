//! Path-based box extraction helpers built on the structure walker.

use std::error::Error;
use std::fmt;
use std::io::{self, Read, Seek};

use crate::BoxInfo;
use crate::FourCc;
use crate::boxes::{BoxRegistry, default_registry};
use crate::codec::{CodecError, DynCodecBox, unmarshal_any_with_context};
use crate::header::HeaderError;
use crate::walk::{
    BoxPath, PathMatch, WalkControl, WalkError, WalkHandle, walk_structure_from_box_with_registry,
    walk_structure_with_registry,
};

/// Header metadata paired with a decoded runtime box payload.
pub struct ExtractedBox {
    /// Header metadata captured during the structure walk.
    pub info: BoxInfo,
    /// Decoded runtime-erased payload for the extracted box.
    pub payload: Box<dyn DynCodecBox>,
}

/// Extracts every box that matches `path`.
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

/// Extracts every box that matches any path in `paths`.
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

/// Extracts every box that matches any path in `paths` using `registry`.
pub fn extract_boxes_with_registry<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
    registry: &BoxRegistry,
) -> Result<Vec<BoxInfo>, ExtractError>
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
            matches.push(*handle.info());
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

/// Extracts every box that matches any path in `paths`, then decodes the payloads with `registry`.
pub fn extract_boxes_with_payload_with_registry<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    paths: &[BoxPath],
    registry: &BoxRegistry,
) -> Result<Vec<ExtractedBox>, ExtractError>
where
    R: Read + Seek,
{
    let infos = extract_boxes_with_registry(reader, parent, paths, registry)?;
    let mut matches = Vec::with_capacity(infos.len());

    for info in infos {
        info.seek_to_payload(reader)?;
        let (payload, _) = unmarshal_any_with_context(
            reader,
            info.payload_size()?,
            info.box_type(),
            registry,
            info.lookup_context(),
            None,
        )?;
        matches.push(ExtractedBox { info, payload });
    }

    Ok(matches)
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
    Io(io::Error),
    Header(HeaderError),
    Codec(CodecError),
    Walk(WalkError),
    EmptyPath,
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Header(error) => error.fmt(f),
            Self::Codec(error) => error.fmt(f),
            Self::Walk(error) => error.fmt(f),
            Self::EmptyPath => f.write_str("box path must not be empty"),
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
            Self::EmptyPath => None,
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
