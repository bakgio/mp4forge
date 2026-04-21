//! Box-writing helpers with header backfill and raw-copy support.

use std::error::Error;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::FourCc;
use crate::header::{BoxInfo, HeaderError, SMALL_HEADER_SIZE};

/// Stateful MP4 writer that can backfill container sizes after payload bytes are written.
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

    /// Returns a shared reference to the underlying writer.
    pub const fn get_ref(&self) -> &W {
        &self.writer
    }

    /// Returns a mutable reference to the underlying writer.
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
    pub fn start_box_type(&mut self, box_type: FourCc) -> Result<BoxInfo, WriterError> {
        self.start_box(BoxInfo::new(box_type, SMALL_HEADER_SIZE))
    }

    /// Writes `info` as the next box header and pushes it onto the open-box stack.
    pub fn start_box(&mut self, info: BoxInfo) -> Result<BoxInfo, WriterError> {
        let written = info.write(&mut self.writer)?;
        self.box_stack.push(written);
        Ok(written)
    }

    /// Rewrites the most recently opened box header with its final size.
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
    Io(io::Error),
    Header(HeaderError),
    NoOpenBox,
    InvalidBoxSpan {
        box_type: FourCc,
        offset: u64,
        end: u64,
    },
    HeaderSizeChanged {
        box_type: FourCc,
        original_header_size: u64,
        rewritten_header_size: u64,
    },
    IncompleteCopy {
        expected_size: u64,
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
