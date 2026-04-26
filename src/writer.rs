//! Box-writing helpers with header backfill and raw-copy support.

use std::error::Error;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};
#[cfg(feature = "async")]
use std::pin::Pin;
#[cfg(feature = "async")]
use std::task::{Context, Poll};

use crate::FourCc;
#[cfg(feature = "async")]
use crate::async_io::{AsyncReadSeek, AsyncWriteSeek};
use crate::header::{BoxInfo, HeaderError, SMALL_HEADER_SIZE};
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite};

/// Stateful MP4 writer that can backfill container sizes after payload bytes are written.
///
/// This wrapper is designed for rewrite-style flows that need to stream payload bytes, nest
/// container boxes, and then patch final sizes back into previously written headers.
pub struct Writer<W> {
    writer: W,
    box_stack: Vec<BoxInfo>,
}

impl<W> Writer<W> {
    /// Wraps `writer` with box-start and box-end tracking.
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            box_stack: Vec::new(),
        }
    }

    /// Returns a shared view of the underlying writer.
    pub const fn get_ref(&self) -> &W {
        &self.writer
    }

    /// Returns a mutable view of the underlying writer.
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.writer
    }

    /// Consumes the wrapper and returns the underlying writer.
    pub fn into_inner(self) -> W {
        self.writer
    }
}

impl<W> Writer<W>
where
    W: Write + Seek,
{
    /// Starts a new box using `box_type` and an empty small-header placeholder.
    ///
    /// The final size is written later by [`Writer::end_box`].
    pub fn start_box_type(&mut self, box_type: FourCc) -> Result<BoxInfo, WriterError> {
        self.start_box(BoxInfo::new(box_type, SMALL_HEADER_SIZE))
    }

    /// Writes `info` as the next box header and pushes it onto the open-box stack.
    ///
    /// Callers typically pass either a small-header placeholder or a header copied from an
    /// existing box when preserving layout details.
    pub fn start_box(&mut self, info: BoxInfo) -> Result<BoxInfo, WriterError> {
        let written = info.write(&mut self.writer)?;
        self.box_stack.push(written);
        Ok(written)
    }

    /// Rewrites the most recently opened box header with its final size.
    ///
    /// The returned [`BoxInfo`] reflects the finalized on-disk size after the rewrite completes.
    pub fn end_box(&mut self) -> Result<BoxInfo, WriterError> {
        let Some(started) = self.box_stack.pop() else {
            return Err(WriterError::NoOpenBox);
        };

        let end = self.writer.stream_position()?;
        if end < started.offset() {
            return Err(WriterError::InvalidBoxSpan {
                box_type: started.box_type(),
                offset: started.offset(),
                end,
            });
        }

        let final_size = end - started.offset();
        let rewritten = BoxInfo::new(started.box_type(), final_size)
            .with_offset(started.offset())
            .with_header_size(started.header_size())
            .with_lookup_context(started.lookup_context());

        started.seek_to_start(&mut self.writer)?;
        let rewritten = rewritten.write(&mut self.writer)?;
        if rewritten.header_size() != started.header_size() {
            return Err(WriterError::HeaderSizeChanged {
                box_type: started.box_type(),
                original_header_size: started.header_size(),
                rewritten_header_size: rewritten.header_size(),
            });
        }

        self.writer.seek(SeekFrom::Start(end))?;
        Ok(rewritten)
    }

    /// Copies the exact byte range described by `info` into the current output position.
    pub fn copy_box<R>(&mut self, reader: &mut R, info: &BoxInfo) -> Result<(), WriterError>
    where
        R: Read + Seek,
    {
        info.seek_to_start(reader)?;
        let mut limited = reader.take(info.size());
        let copied = io::copy(&mut limited, self)?;
        if copied != info.size() {
            return Err(WriterError::IncompleteCopy {
                expected_size: info.size(),
                actual_size: copied,
            });
        }

        Ok(())
    }
}

#[cfg(feature = "async")]
impl<W> Writer<W>
where
    W: AsyncWriteSeek,
{
    /// Starts a new box through the additive Tokio-based async library surface using `box_type`
    /// and an empty small-header placeholder.
    ///
    /// The final size is written later by [`Writer::end_box_async`].
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub async fn start_box_type_async(&mut self, box_type: FourCc) -> Result<BoxInfo, WriterError> {
        self.start_box_async(BoxInfo::new(box_type, SMALL_HEADER_SIZE))
            .await
    }

    /// Writes `info` as the next box header through the additive Tokio-based async library
    /// surface and pushes it onto the open-box stack.
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub async fn start_box_async(&mut self, info: BoxInfo) -> Result<BoxInfo, WriterError> {
        let written = info.write_async(&mut self.writer).await?;
        self.box_stack.push(written);
        Ok(written)
    }

    /// Rewrites the most recently opened box header with its final size through the additive
    /// Tokio-based async library surface.
    ///
    /// The returned [`BoxInfo`] reflects the finalized on-disk size after the rewrite completes.
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub async fn end_box_async(&mut self) -> Result<BoxInfo, WriterError> {
        let Some(started) = self.box_stack.pop() else {
            return Err(WriterError::NoOpenBox);
        };

        let end = self.writer.stream_position().await?;
        if end < started.offset() {
            return Err(WriterError::InvalidBoxSpan {
                box_type: started.box_type(),
                offset: started.offset(),
                end,
            });
        }

        let final_size = end - started.offset();
        let rewritten = BoxInfo::new(started.box_type(), final_size)
            .with_offset(started.offset())
            .with_header_size(started.header_size())
            .with_lookup_context(started.lookup_context());

        self.writer.seek(SeekFrom::Start(started.offset())).await?;
        let rewritten = rewritten.write_async(&mut self.writer).await?;
        if rewritten.header_size() != started.header_size() {
            return Err(WriterError::HeaderSizeChanged {
                box_type: started.box_type(),
                original_header_size: started.header_size(),
                rewritten_header_size: rewritten.header_size(),
            });
        }

        self.writer.seek(SeekFrom::Start(end)).await?;
        Ok(rewritten)
    }

    /// Copies the exact byte range described by `info` into the current output position through
    /// the additive Tokio-based async library surface.
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub async fn copy_box_async<R>(
        &mut self,
        reader: &mut R,
        info: &BoxInfo,
    ) -> Result<(), WriterError>
    where
        R: AsyncReadSeek,
    {
        info.seek_to_start_async(reader).await?;
        let mut limited = (&mut *reader).take(info.size());
        let copied = tokio::io::copy(&mut limited, self).await?;
        if copied != info.size() {
            return Err(WriterError::IncompleteCopy {
                expected_size: info.size(),
                actual_size: copied,
            });
        }

        Ok(())
    }
}

impl<W> Write for Writer<W>
where
    W: Write,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(feature = "async")]
impl<W> AsyncWrite for Writer<W>
where
    W: AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        Pin::new(&mut this.writer).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        Pin::new(&mut this.writer).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        Pin::new(&mut this.writer).poll_shutdown(cx)
    }
}

#[cfg(feature = "async")]
impl<W> AsyncSeek for Writer<W>
where
    W: AsyncSeek + Unpin,
{
    fn start_seek(self: Pin<&mut Self>, position: SeekFrom) -> io::Result<()> {
        let this = self.get_mut();
        Pin::new(&mut this.writer).start_seek(position)
    }

    fn poll_complete(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        let this = self.get_mut();
        Pin::new(&mut this.writer).poll_complete(cx)
    }
}

impl<W> Seek for Writer<W>
where
    W: Seek,
{
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.writer.seek(pos)
    }
}

/// Errors raised while writing or copying MP4 boxes.
#[derive(Debug)]
pub enum WriterError {
    /// An I/O operation failed while reading, seeking, or writing.
    Io(io::Error),
    /// Box header metadata was invalid or could not be encoded.
    Header(HeaderError),
    /// [`Writer::end_box`] was called with no corresponding open box.
    NoOpenBox,
    /// The current writer position moved before the recorded start offset of the open box.
    InvalidBoxSpan {
        /// Concrete box type being closed.
        box_type: FourCc,
        /// Recorded start offset of the open box.
        offset: u64,
        /// Current writer position observed during close.
        end: u64,
    },
    /// Re-encoding the finalized header changed its encoded width.
    HeaderSizeChanged {
        /// Concrete box type being closed.
        box_type: FourCc,
        /// Header width used when the box was opened.
        original_header_size: u64,
        /// Header width produced by the finalized size.
        rewritten_header_size: u64,
    },
    /// A raw-copy operation ended before all requested bytes were copied.
    IncompleteCopy {
        /// Number of bytes that should have been copied.
        expected_size: u64,
        /// Number of bytes that were actually copied.
        actual_size: u64,
    },
}

impl fmt::Display for WriterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Header(error) => error.fmt(f),
            Self::NoOpenBox => f.write_str("no open box to end"),
            Self::InvalidBoxSpan {
                box_type,
                offset,
                end,
            } => write!(
                f,
                "box end position is before box start: type={box_type}, start={offset}, end={end}"
            ),
            Self::HeaderSizeChanged {
                box_type,
                original_header_size,
                rewritten_header_size,
            } => write!(
                f,
                "header size changed while closing {box_type}: started={original_header_size}, ended={rewritten_header_size}"
            ),
            Self::IncompleteCopy {
                expected_size,
                actual_size,
            } => write!(
                f,
                "failed to copy box: expected {expected_size} bytes, copied {actual_size}"
            ),
        }
    }
}

impl Error for WriterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::NoOpenBox
            | Self::InvalidBoxSpan { .. }
            | Self::HeaderSizeChanged { .. }
            | Self::IncompleteCopy { .. } => None,
        }
    }
}

impl From<io::Error> for WriterError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<HeaderError> for WriterError {
    fn from(value: HeaderError) -> Self {
        Self::Header(value)
    }
}
