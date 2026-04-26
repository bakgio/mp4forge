//! Path-based typed payload rewrite helpers built on the writer layer.
//!
//! These helpers preserve the existing low-level writer flow for advanced use cases while offering
//! a small typed API for common "find payloads at this path and mutate them" rewrite operations,
//! including byte-slice wrappers for in-memory rewrite flows.

use std::any::type_name;
use std::error::Error;
use std::fmt;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use crate::FourCc;
#[cfg(feature = "async")]
use crate::async_io::{AsyncReadSeek, AsyncWriteSeek};
use crate::boxes::iso14496_12::{
    Ftyp, VisualSampleEntry, split_box_children_with_optional_trailing_bytes,
};
use crate::boxes::metadata::Keys;
use crate::boxes::{BoxLookupContext, BoxRegistry, default_registry};
use crate::codec::{CodecBox, CodecError, marshal_dyn, unmarshal, unmarshal_any_with_context};
use crate::header::{BoxInfo, HeaderError, SMALL_HEADER_SIZE};
use crate::walk::{BoxPath, PathMatch};
use crate::writer::{Writer, WriterError};
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

const FTYP: FourCc = FourCc::from_bytes(*b"ftyp");
const KEYS: FourCc = FourCc::from_bytes(*b"keys");
const QT_BRAND: FourCc = FourCc::from_bytes(*b"qt  ");

/// Rewrites every payload at `path` by downcasting it to `T` and applying `edit`.
///
/// The edit closure runs once per matched box in depth-first order. The returned count is the
/// number of payloads that were successfully rewritten. Unmatched boxes are copied through to the
/// output verbatim.
pub fn rewrite_box_as<R, W, T, F>(
    reader: &mut R,
    writer: W,
    path: BoxPath,
    edit: F,
) -> Result<usize, RewriteError>
where
    R: Read + Seek,
    W: Write + Seek,
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    let paths = [path];
    rewrite_boxes_as(reader, writer, &paths, edit)
}

/// Rewrites every payload that matches any path in `paths` by downcasting it to `T` and applying
/// `edit`.
///
/// Every matched payload must decode to `T`, otherwise
/// [`RewriteError::UnexpectedPayloadType`] is returned with the matched path and offset. Unmatched
/// boxes are copied through to the output verbatim.
pub fn rewrite_boxes_as<R, W, T, F>(
    reader: &mut R,
    writer: W,
    paths: &[BoxPath],
    edit: F,
) -> Result<usize, RewriteError>
where
    R: Read + Seek,
    W: Write + Seek,
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    let registry = default_registry();
    rewrite_boxes_as_with_registry(reader, writer, paths, &registry, edit)
}

/// Rewrites every payload at `path` in an in-memory MP4 byte slice and returns the rewritten
/// bytes.
///
/// This is equivalent to calling [`rewrite_box_as`] with `Cursor<&[u8]>` input and `Vec<u8>`
/// output storage. The edit closure runs once per matched box in depth-first order, and unmatched
/// boxes are copied through verbatim.
pub fn rewrite_box_as_bytes<T, F>(
    input: &[u8],
    path: BoxPath,
    edit: F,
) -> Result<Vec<u8>, RewriteError>
where
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    let paths = [path];
    rewrite_boxes_as_bytes::<T, _>(input, &paths, edit)
}

/// Rewrites every payload that matches any path in `paths` in an in-memory MP4 byte slice and
/// returns the rewritten bytes.
///
/// This is equivalent to calling [`rewrite_boxes_as`] with `Cursor<&[u8]>` input and `Vec<u8>`
/// output storage. Every matched payload must decode to `T`, otherwise
/// [`RewriteError::UnexpectedPayloadType`] is returned with the matched path and offset.
pub fn rewrite_boxes_as_bytes<T, F>(
    input: &[u8],
    paths: &[BoxPath],
    edit: F,
) -> Result<Vec<u8>, RewriteError>
where
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    let mut reader = Cursor::new(input);
    let mut writer = Cursor::new(Vec::with_capacity(input.len()));
    rewrite_boxes_as(&mut reader, &mut writer, paths, edit)?;
    Ok(writer.into_inner())
}

/// Rewrites every payload that matches any path in `paths` using `registry`, downcasts each match
/// to `T`, and applies `edit`.
///
/// Paths are evaluated from the file root. Subtrees that cannot possibly match are copied without
/// decoding so unrelated bytes remain untouched. The returned count reports how many payloads were
/// edited.
pub fn rewrite_boxes_as_with_registry<R, W, T, F>(
    reader: &mut R,
    writer: W,
    paths: &[BoxPath],
    registry: &BoxRegistry,
    mut edit: F,
) -> Result<usize, RewriteError>
where
    R: Read + Seek,
    W: Write + Seek,
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    validate_paths(paths)?;
    reader.seek(SeekFrom::Start(0))?;

    let mut writer = Writer::new(writer);
    if paths.is_empty() {
        io::copy(reader, &mut writer)?;
        return Ok(0);
    }

    let mut rewritten_count = 0;
    let mut plan = RewritePlan {
        paths,
        edit: &mut edit,
        rewritten_count: &mut rewritten_count,
    };
    rewrite_sequence::<R, W, T, F>(
        reader,
        &mut writer,
        registry,
        &mut plan,
        RewriteFrame::root(),
    )?;
    Ok(rewritten_count)
}

/// Rewrites every payload at `path` through the additive Tokio-based async library surface by
/// downcasting it to `T` and applying `edit`.
///
/// The edit closure runs once per matched box in depth-first order. The returned count is the
/// number of payloads that were successfully rewritten. Unmatched boxes are copied through to the
/// output verbatim.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn rewrite_box_as_async<R, W, T, F>(
    reader: &mut R,
    writer: W,
    path: BoxPath,
    edit: F,
) -> Result<usize, RewriteError>
where
    R: AsyncReadSeek,
    W: AsyncWriteSeek,
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    let paths = [path];
    rewrite_boxes_as_async(reader, writer, &paths, edit).await
}

/// Rewrites every payload that matches any path in `paths` through the additive Tokio-based async
/// library surface by downcasting it to `T` and applying `edit`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn rewrite_boxes_as_async<R, W, T, F>(
    reader: &mut R,
    writer: W,
    paths: &[BoxPath],
    edit: F,
) -> Result<usize, RewriteError>
where
    R: AsyncReadSeek,
    W: AsyncWriteSeek,
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    let registry = default_registry();
    rewrite_boxes_as_with_registry_async(reader, writer, paths, &registry, edit).await
}

/// Rewrites every payload at `path` in an in-memory MP4 byte slice through the additive
/// Tokio-based async library surface and returns the rewritten bytes.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn rewrite_box_as_bytes_async<T, F>(
    input: &[u8],
    path: BoxPath,
    edit: F,
) -> Result<Vec<u8>, RewriteError>
where
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    let paths = [path];
    rewrite_boxes_as_bytes_async::<T, _>(input, &paths, edit).await
}

/// Rewrites every payload that matches any path in `paths` in an in-memory MP4 byte slice through
/// the additive Tokio-based async library surface and returns the rewritten bytes.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn rewrite_boxes_as_bytes_async<T, F>(
    input: &[u8],
    paths: &[BoxPath],
    edit: F,
) -> Result<Vec<u8>, RewriteError>
where
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    let mut reader = Cursor::new(input);
    let mut writer = Cursor::new(Vec::with_capacity(input.len()));
    rewrite_boxes_as_async(&mut reader, &mut writer, paths, edit).await?;
    Ok(writer.into_inner())
}

/// Rewrites every payload that matches any path in `paths` through the additive Tokio-based async
/// library surface using `registry`, downcasts each match to `T`, and applies `edit`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn rewrite_boxes_as_with_registry_async<R, W, T, F>(
    reader: &mut R,
    writer: W,
    paths: &[BoxPath],
    registry: &BoxRegistry,
    mut edit: F,
) -> Result<usize, RewriteError>
where
    R: AsyncReadSeek,
    W: AsyncWriteSeek,
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    validate_paths(paths)?;
    reader.seek(SeekFrom::Start(0)).await?;

    let mut writer = Writer::new(writer);
    if paths.is_empty() {
        tokio::io::copy(reader, &mut writer).await?;
        return Ok(0);
    }

    let mut rewritten_count = 0;
    let mut plan = RewritePlan {
        paths,
        edit: &mut edit,
        rewritten_count: &mut rewritten_count,
    };
    rewrite_sequence_async::<R, W, T, F>(
        reader,
        &mut writer,
        registry,
        &mut plan,
        RewriteFrame::root(),
    )
    .await?;
    Ok(rewritten_count)
}

#[derive(Clone)]
struct RewriteFrame {
    remaining_size: u64,
    is_root: bool,
    path: BoxPath,
    sibling_context: BoxLookupContext,
}

impl RewriteFrame {
    const fn root() -> Self {
        Self {
            remaining_size: 0,
            is_root: true,
            path: BoxPath::empty(),
            sibling_context: BoxLookupContext::new(),
        }
    }

    fn child(remaining_size: u64, path: BoxPath, sibling_context: BoxLookupContext) -> Self {
        Self {
            remaining_size,
            is_root: false,
            path,
            sibling_context,
        }
    }
}

struct RewritePlan<'a, F> {
    paths: &'a [BoxPath],
    edit: &'a mut F,
    rewritten_count: &'a mut usize,
}

fn rewrite_sequence<R, W, T, F>(
    reader: &mut R,
    writer: &mut Writer<W>,
    registry: &BoxRegistry,
    plan: &mut RewritePlan<'_, F>,
    mut frame: RewriteFrame,
) -> Result<(), RewriteError>
where
    R: Read + Seek,
    W: Write + Seek,
    T: CodecBox + 'static,
    F: FnMut(&mut T),
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
            return Err(RewriteError::TooLargeBoxSize {
                box_type: info.box_type(),
                size: info.size(),
                available_size: frame.remaining_size,
            });
        }
        if !frame.is_root {
            frame.remaining_size -= info.size();
        }

        info.set_lookup_context(frame.sibling_context);
        inspect_context_carriers(reader, &mut info, &frame.path)?;
        process_box::<R, W, T, F>(reader, writer, registry, plan, &frame, &info)?;

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
        return Err(RewriteError::UnexpectedEof);
    }

    Ok(())
}

#[cfg(feature = "async")]
async fn rewrite_sequence_async<R, W, T, F>(
    reader: &mut R,
    writer: &mut Writer<W>,
    registry: &BoxRegistry,
    plan: &mut RewritePlan<'_, F>,
    mut frame: RewriteFrame,
) -> Result<(), RewriteError>
where
    R: AsyncReadSeek,
    W: AsyncWriteSeek,
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    loop {
        if !frame.is_root && frame.remaining_size < SMALL_HEADER_SIZE {
            break;
        }

        let start = reader.stream_position().await?;
        let mut info = match BoxInfo::read_async(reader).await {
            Ok(info) => info,
            Err(HeaderError::Io(error))
                if frame.is_root && clean_root_eof_async(reader, start, &error).await? =>
            {
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };

        if !frame.is_root && info.size() > frame.remaining_size {
            return Err(RewriteError::TooLargeBoxSize {
                box_type: info.box_type(),
                size: info.size(),
                available_size: frame.remaining_size,
            });
        }
        if !frame.is_root {
            frame.remaining_size -= info.size();
        }

        info.set_lookup_context(frame.sibling_context);
        inspect_context_carriers_async(reader, &mut info, &frame.path).await?;
        process_box_async::<R, W, T, F>(reader, writer, registry, plan, &frame, &info).await?;

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
        return Err(RewriteError::UnexpectedEof);
    }

    Ok(())
}

fn process_box<R, W, T, F>(
    reader: &mut R,
    writer: &mut Writer<W>,
    registry: &BoxRegistry,
    plan: &mut RewritePlan<'_, F>,
    frame: &RewriteFrame,
    info: &BoxInfo,
) -> Result<(), RewriteError>
where
    R: Read + Seek,
    W: Write + Seek,
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    let current_path = child_path(&frame.path, info.box_type());
    let path_match = match_paths(plan.paths, &current_path);
    if !path_match.forward_match && !path_match.exact_match {
        writer.copy_box(reader, info)?;
        return Ok(());
    }

    info.seek_to_payload(reader)?;
    let payload_size = info.payload_size()?;
    let (mut payload, payload_read) = unmarshal_any_with_context(
        reader,
        payload_size,
        info.box_type(),
        registry,
        info.lookup_context(),
        None,
    )
    .map_err(|source| RewriteError::PayloadDecode {
        path: current_path.clone(),
        box_type: info.box_type(),
        offset: info.offset(),
        source,
    })?;

    if path_match.exact_match {
        let typed = payload.as_any_mut().downcast_mut::<T>().ok_or_else(|| {
            RewriteError::UnexpectedPayloadType {
                path: current_path.clone(),
                box_type: info.box_type(),
                offset: info.offset(),
                expected_type: type_name::<T>(),
            }
        })?;
        (plan.edit)(typed);
        *plan.rewritten_count += 1;
    }

    let placeholder = BoxInfo::new(info.box_type(), info.header_size())
        .with_header_size(info.header_size())
        .with_lookup_context(info.lookup_context())
        .with_extend_to_eof(info.extend_to_eof());
    writer.start_box(placeholder)?;
    marshal_dyn(&mut *writer, payload.as_ref(), None).map_err(|source| {
        RewriteError::PayloadEncode {
            path: current_path.clone(),
            box_type: info.box_type(),
            offset: info.offset(),
            source,
        }
    })?;

    let children_offset = info.offset() + info.header_size() + payload_read;
    let (children_size, trailing_bytes) = if payload.as_any().is::<VisualSampleEntry>() {
        visual_sample_entry_children_layout(
            reader,
            children_offset,
            payload_size.saturating_sub(payload_read),
        )?
    } else {
        (payload_size.saturating_sub(payload_read), Vec::new())
    };
    reader.seek(SeekFrom::Start(children_offset))?;
    rewrite_sequence::<R, W, T, F>(
        reader,
        writer,
        registry,
        plan,
        RewriteFrame::child(
            children_size,
            current_path,
            info.lookup_context().enter(info.box_type()),
        ),
    )?;
    if !trailing_bytes.is_empty() {
        writer.write_all(&trailing_bytes)?;
    }
    info.seek_to_end(reader)?;
    writer.end_box()?;
    Ok(())
}

#[cfg(feature = "async")]
async fn process_box_async<R, W, T, F>(
    reader: &mut R,
    writer: &mut Writer<W>,
    registry: &BoxRegistry,
    plan: &mut RewritePlan<'_, F>,
    frame: &RewriteFrame,
    info: &BoxInfo,
) -> Result<(), RewriteError>
where
    R: AsyncReadSeek,
    W: AsyncWriteSeek,
    T: CodecBox + 'static,
    F: FnMut(&mut T),
{
    let current_path = child_path(&frame.path, info.box_type());
    let path_match = match_paths(plan.paths, &current_path);
    if !path_match.forward_match && !path_match.exact_match {
        writer.copy_box_async(reader, info).await?;
        return Ok(());
    }

    reader
        .seek(SeekFrom::Start(info.offset() + info.header_size()))
        .await?;
    let payload_size = info.payload_size()?;
    let mut payload_bytes = Vec::with_capacity(payload_size.try_into().unwrap_or(0));
    let mut payload_reader = (&mut *reader).take(payload_size);
    let payload_read = payload_reader.read_to_end(&mut payload_bytes).await? as u64;
    if payload_read != payload_size {
        return Err(RewriteError::UnexpectedEof);
    }
    let (encoded_payload, payload_read, is_visual_sample_entry) = {
        let (mut payload, payload_read) = unmarshal_any_with_context(
            &mut Cursor::new(payload_bytes.as_slice()),
            payload_size,
            info.box_type(),
            registry,
            info.lookup_context(),
            None,
        )
        .map_err(|source| RewriteError::PayloadDecode {
            path: current_path.clone(),
            box_type: info.box_type(),
            offset: info.offset(),
            source,
        })?;

        if path_match.exact_match {
            let typed = payload.as_any_mut().downcast_mut::<T>().ok_or_else(|| {
                RewriteError::UnexpectedPayloadType {
                    path: current_path.clone(),
                    box_type: info.box_type(),
                    offset: info.offset(),
                    expected_type: type_name::<T>(),
                }
            })?;
            (plan.edit)(typed);
            *plan.rewritten_count += 1;
        }

        let is_visual_sample_entry = payload.as_any().is::<VisualSampleEntry>();
        let mut encoded_payload = Vec::new();
        marshal_dyn(&mut encoded_payload, payload.as_ref(), None).map_err(|source| {
            RewriteError::PayloadEncode {
                path: current_path.clone(),
                box_type: info.box_type(),
                offset: info.offset(),
                source,
            }
        })?;
        (encoded_payload, payload_read, is_visual_sample_entry)
    };

    let placeholder = BoxInfo::new(info.box_type(), info.header_size())
        .with_header_size(info.header_size())
        .with_lookup_context(info.lookup_context())
        .with_extend_to_eof(info.extend_to_eof());
    writer.start_box_async(placeholder).await?;
    writer.write_all(&encoded_payload).await?;

    let children_offset = info.offset() + info.header_size() + payload_read;
    let (children_size, trailing_bytes) = if is_visual_sample_entry {
        visual_sample_entry_children_layout_async(
            reader,
            children_offset,
            payload_size.saturating_sub(payload_read),
        )
        .await?
    } else {
        (payload_size.saturating_sub(payload_read), Vec::new())
    };
    reader.seek(SeekFrom::Start(children_offset)).await?;
    Box::pin(rewrite_sequence_async::<R, W, T, F>(
        reader,
        writer,
        registry,
        plan,
        RewriteFrame::child(
            children_size,
            current_path,
            info.lookup_context().enter(info.box_type()),
        ),
    ))
    .await?;
    if !trailing_bytes.is_empty() {
        writer.write_all(&trailing_bytes).await?;
    }
    reader
        .seek(SeekFrom::Start(info.offset() + info.size()))
        .await?;
    writer.end_box_async().await?;
    Ok(())
}

fn inspect_context_carriers<R>(
    reader: &mut R,
    info: &mut BoxInfo,
    path: &BoxPath,
) -> Result<(), RewriteError>
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
) -> Result<(), RewriteError>
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

fn visual_sample_entry_children_layout<R>(
    reader: &mut R,
    extension_offset: u64,
    extension_size: u64,
) -> Result<(u64, Vec<u8>), RewriteError>
where
    R: Read + Seek,
{
    let checkpoint = reader.stream_position()?;
    reader.seek(SeekFrom::Start(extension_offset))?;
    let bytes = read_extension_bytes(reader, extension_size)?;
    reader.seek(SeekFrom::Start(checkpoint))?;

    let child_len = split_box_children_with_optional_trailing_bytes(&bytes);
    Ok((child_len as u64, bytes[child_len..].to_vec()))
}

#[cfg(feature = "async")]
async fn visual_sample_entry_children_layout_async<R>(
    reader: &mut R,
    extension_offset: u64,
    extension_size: u64,
) -> Result<(u64, Vec<u8>), RewriteError>
where
    R: AsyncReadSeek,
{
    let checkpoint = reader.stream_position().await?;
    reader.seek(SeekFrom::Start(extension_offset)).await?;
    let bytes = read_extension_bytes_async(reader, extension_size).await?;
    reader.seek(SeekFrom::Start(checkpoint)).await?;

    let child_len = split_box_children_with_optional_trailing_bytes(&bytes);
    Ok((child_len as u64, bytes[child_len..].to_vec()))
}

fn read_extension_bytes<R>(reader: &mut R, extension_size: u64) -> Result<Vec<u8>, RewriteError>
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

#[cfg(feature = "async")]
async fn read_extension_bytes_async<R>(
    reader: &mut R,
    extension_size: u64,
) -> Result<Vec<u8>, RewriteError>
where
    R: AsyncReadSeek,
{
    let extension_len = usize::try_from(extension_size).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "payload extension is too large")
    })?;
    let mut bytes = vec![0; extension_len];
    reader.read_exact(&mut bytes).await?;
    Ok(bytes)
}

fn decode_box<R, B>(reader: &mut R, info: &BoxInfo) -> Result<B, RewriteError>
where
    R: Read + Seek,
    B: Default + CodecBox,
{
    info.seek_to_payload(reader)?;
    let mut decoded = B::default();
    unmarshal(reader, info.payload_size()?, &mut decoded, None)?;
    info.seek_to_payload(reader)?;
    Ok(decoded)
}

#[cfg(feature = "async")]
async fn decode_box_async<R, B>(reader: &mut R, info: &BoxInfo) -> Result<B, RewriteError>
where
    R: AsyncReadSeek,
    B: Default + CodecBox + Send,
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

fn validate_paths(paths: &[BoxPath]) -> Result<(), RewriteError> {
    if paths.iter().any(BoxPath::is_empty) {
        return Err(RewriteError::EmptyPath);
    }

    Ok(())
}

fn child_path(path: &BoxPath, box_type: FourCc) -> BoxPath {
    path.iter()
        .copied()
        .chain(std::iter::once(box_type))
        .collect()
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

/// Errors raised while rewriting path-matched payloads.
#[derive(Debug)]
pub enum RewriteError {
    /// An I/O operation failed while reading, seeking, or writing.
    Io(io::Error),
    /// Box header metadata was invalid or truncated.
    Header(HeaderError),
    /// Payload codec work failed before a more specific matched-box context was available.
    Codec(CodecError),
    /// Low-level writer state became invalid.
    Writer(WriterError),
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
    /// A matched payload failed to encode after the edit closure mutated it.
    PayloadEncode {
        /// Matched path that was being re-encoded when the failure happened.
        path: BoxPath,
        /// Concrete box type at that matched path.
        box_type: FourCc,
        /// File offset of the matched box header.
        offset: u64,
        /// Underlying encode failure.
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
    /// A child box claimed more bytes than remain in its parent container.
    TooLargeBoxSize {
        /// Concrete box type whose declared size was invalid in the current container.
        box_type: FourCc,
        /// Declared child box size.
        size: u64,
        /// Remaining bytes available in the parent container.
        available_size: u64,
    },
    /// A non-QuickTime container ended before all advertised child bytes were consumed.
    UnexpectedEof,
}

impl fmt::Display for RewriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Header(error) => error.fmt(f),
            Self::Codec(error) => error.fmt(f),
            Self::Writer(error) => error.fmt(f),
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
            Self::PayloadEncode {
                path,
                box_type,
                offset,
                source,
            } => write!(
                f,
                "failed to encode payload at {path} (type={box_type}, offset={offset}): {source}"
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
            Self::TooLargeBoxSize {
                box_type,
                size,
                available_size,
            } => write!(
                f,
                "too large box size: type={box_type}, size={size}, actualBufSize={available_size}"
            ),
            Self::UnexpectedEof => f.write_str("unexpected EOF"),
        }
    }
}

impl Error for RewriteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::Writer(error) => Some(error),
            Self::PayloadDecode { source, .. } | Self::PayloadEncode { source, .. } => Some(source),
            Self::EmptyPath
            | Self::UnexpectedPayloadType { .. }
            | Self::TooLargeBoxSize { .. }
            | Self::UnexpectedEof => None,
        }
    }
}

impl From<io::Error> for RewriteError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<HeaderError> for RewriteError {
    fn from(value: HeaderError) -> Self {
        Self::Header(value)
    }
}

impl From<CodecError> for RewriteError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}

impl From<WriterError> for RewriteError {
    fn from(value: WriterError) -> Self {
        Self::Writer(value)
    }
}
