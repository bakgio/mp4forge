//! Bit-level reader and writer helpers used by the descriptor codec.

use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};

#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

#[cfg(feature = "async")]
use crate::async_io::{AsyncRead, AsyncSeek, AsyncWrite};

/// Error text returned when byte-oriented access is attempted on an unaligned stream.
pub const INVALID_ALIGNMENT_MESSAGE: &str = "invalid alignment";
/// Error text returned when a caller requests more bits than the provided buffer holds.
pub const INVALID_BIT_WIDTH_MESSAGE: &str = "bit width exceeds input buffer";

/// Reads arbitrary-width bit slices while preserving byte-alignment state.
#[derive(Debug)]
pub struct BitReader<R> {
    inner: R,
    octet: u8,
    remaining_bits: u8,
}

impl<R> BitReader<R> {
    /// Creates a bit reader around an existing byte reader.
    pub const fn new(inner: R) -> Self {
        Self {
            inner,
            octet: 0,
            remaining_bits: 0,
        }
    }

    /// Returns `true` when the next read starts on a byte boundary.
    pub const fn is_aligned(&self) -> bool {
        self.remaining_bits == 0
    }
}

impl<R: Read> BitReader<R> {
    /// Reads `width` bits and packs them into a big-endian byte vector.
    pub fn read_bits(&mut self, width: usize) -> io::Result<Vec<u8>> {
        let byte_len = width.div_ceil(8);
        let bit_offset = (byte_len * 8) - width;
        let mut data = vec![0_u8; byte_len];

        for index in 0..width {
            if self.read_bit()? {
                let bit_index = bit_offset + index;
                let byte_index = bit_index / 8;
                let within_byte = 7 - (bit_index % 8);
                data[byte_index] |= 1 << within_byte;
            }
        }

        Ok(data)
    }

    /// Reads a single bit from the stream.
    pub fn read_bit(&mut self) -> io::Result<bool> {
        if self.remaining_bits == 0 {
            let mut buf = [0_u8; 1];
            let read = self.inner.read(&mut buf)?;
            if read == 0 {
                return Err(io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "failed to fill whole buffer",
                ));
            }
            self.octet = buf[0];
            self.remaining_bits = 8;
        }

        self.remaining_bits -= 1;
        Ok((self.octet >> self.remaining_bits) & 0x01 != 0)
    }
}

impl<R: Read> Read for BitReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.is_aligned() {
            return Err(invalid_alignment());
        }
        self.inner.read(buf)
    }
}

impl<R: Read + Seek> Seek for BitReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        if matches!(pos, SeekFrom::Current(_)) && !self.is_aligned() {
            return Err(invalid_alignment());
        }

        let next = self.inner.seek(pos)?;
        self.remaining_bits = 0;
        Ok(next)
    }
}

/// Async bit-level reader used by the additive Tokio-based codec surface.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
#[derive(Debug)]
pub struct AsyncBitReader<R> {
    inner: R,
    octet: u8,
    remaining_bits: u8,
}

#[cfg(feature = "async")]
impl<R> AsyncBitReader<R> {
    /// Creates an async bit reader around an existing async byte reader.
    pub const fn new(inner: R) -> Self {
        Self {
            inner,
            octet: 0,
            remaining_bits: 0,
        }
    }

    /// Returns `true` when the next read starts on a byte boundary.
    pub const fn is_aligned(&self) -> bool {
        self.remaining_bits == 0
    }
}

#[cfg(feature = "async")]
impl<R: AsyncRead + Unpin> AsyncBitReader<R> {
    /// Reads `width` bits and packs them into a big-endian byte vector.
    pub async fn read_bits(&mut self, width: usize) -> io::Result<Vec<u8>> {
        let byte_len = width.div_ceil(8);
        let bit_offset = (byte_len * 8) - width;
        let mut data = vec![0_u8; byte_len];

        for index in 0..width {
            if self.read_bit().await? {
                let bit_index = bit_offset + index;
                let byte_index = bit_index / 8;
                let within_byte = 7 - (bit_index % 8);
                data[byte_index] |= 1 << within_byte;
            }
        }

        Ok(data)
    }

    /// Reads a single bit from the stream.
    pub async fn read_bit(&mut self) -> io::Result<bool> {
        if self.remaining_bits == 0 {
            let mut buf = [0_u8; 1];
            self.inner.read_exact(&mut buf).await?;
            self.octet = buf[0];
            self.remaining_bits = 8;
        }

        self.remaining_bits -= 1;
        Ok((self.octet >> self.remaining_bits) & 0x01 != 0)
    }

    /// Reads aligned bytes into `buf`.
    pub async fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        if !self.is_aligned() {
            return Err(invalid_alignment());
        }
        self.inner.read_exact(buf).await.map(|_| ())
    }
}

#[cfg(feature = "async")]
impl<R: AsyncRead + AsyncSeek + Unpin> AsyncBitReader<R> {
    /// Returns the current byte position.
    pub async fn stream_position(&mut self) -> io::Result<u64> {
        if !self.is_aligned() {
            return Err(invalid_alignment());
        }
        self.inner.stream_position().await
    }

    /// Seeks to `pos` and clears any pending bit alignment state.
    pub async fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        if matches!(pos, SeekFrom::Current(_)) && !self.is_aligned() {
            return Err(invalid_alignment());
        }

        let next = self.inner.seek(pos).await?;
        self.remaining_bits = 0;
        Ok(next)
    }
}

/// Writes arbitrary-width bit slices while preserving byte-alignment state.
#[derive(Debug)]
pub struct BitWriter<W> {
    inner: W,
    octet: u8,
    written_bits: u8,
}

impl<W> BitWriter<W> {
    /// Creates a bit writer around an existing byte writer.
    pub const fn new(inner: W) -> Self {
        Self {
            inner,
            octet: 0,
            written_bits: 0,
        }
    }

    /// Returns `true` when the next write starts on a byte boundary.
    pub const fn is_aligned(&self) -> bool {
        self.written_bits == 0
    }

    /// Returns the wrapped writer once all pending bits have been flushed.
    pub fn into_inner(self) -> io::Result<W> {
        if !self.is_aligned() {
            return Err(invalid_alignment());
        }

        Ok(self.inner)
    }
}

impl<W: Write> BitWriter<W> {
    /// Writes the least-significant `width` bits from `data` to the stream.
    pub fn write_bits(&mut self, data: &[u8], width: usize) -> io::Result<()> {
        let total_bits = data.len() * 8;
        if width > total_bits {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                INVALID_BIT_WIDTH_MESSAGE,
            ));
        }

        for index in (total_bits - width)..total_bits {
            let byte_index = index / 8;
            let within_byte = 7 - (index % 8);
            self.write_bit((data[byte_index] >> within_byte) & 0x01 != 0)?;
        }

        Ok(())
    }

    /// Writes a single bit to the stream.
    pub fn write_bit(&mut self, bit: bool) -> io::Result<()> {
        if bit {
            self.octet |= 1 << (7 - self.written_bits);
        }
        self.written_bits += 1;

        if self.written_bits == 8 {
            self.inner.write_all(&[self.octet])?;
            self.octet = 0;
            self.written_bits = 0;
        }

        Ok(())
    }
}

impl<W: Write> Write for BitWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.is_aligned() {
            return Err(invalid_alignment());
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Async bit-level writer used by the additive Tokio-based codec surface.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
#[derive(Debug)]
pub struct AsyncBitWriter<W> {
    inner: W,
    octet: u8,
    written_bits: u8,
}

#[cfg(feature = "async")]
impl<W> AsyncBitWriter<W> {
    /// Creates an async bit writer around an existing async byte writer.
    pub const fn new(inner: W) -> Self {
        Self {
            inner,
            octet: 0,
            written_bits: 0,
        }
    }

    /// Returns `true` when the next write starts on a byte boundary.
    pub const fn is_aligned(&self) -> bool {
        self.written_bits == 0
    }
}

#[cfg(feature = "async")]
impl<W: AsyncWrite + Unpin> AsyncBitWriter<W> {
    /// Writes the least-significant `width` bits from `data` to the stream.
    pub async fn write_bits(&mut self, data: &[u8], width: usize) -> io::Result<()> {
        let total_bits = data.len() * 8;
        if width > total_bits {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                INVALID_BIT_WIDTH_MESSAGE,
            ));
        }

        for index in (total_bits - width)..total_bits {
            let byte_index = index / 8;
            let within_byte = 7 - (index % 8);
            self.write_bit((data[byte_index] >> within_byte) & 0x01 != 0)
                .await?;
        }

        Ok(())
    }

    /// Writes a single bit to the stream.
    pub async fn write_bit(&mut self, bit: bool) -> io::Result<()> {
        if bit {
            self.octet |= 1 << (7 - self.written_bits);
        }
        self.written_bits += 1;

        if self.written_bits == 8 {
            self.inner.write_all(&[self.octet]).await?;
            self.octet = 0;
            self.written_bits = 0;
        }

        Ok(())
    }

    /// Writes aligned bytes from `buf`.
    pub async fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        if !self.is_aligned() {
            return Err(invalid_alignment());
        }
        self.inner.write_all(buf).await
    }

    /// Flushes the wrapped writer.
    pub async fn flush(&mut self) -> io::Result<()> {
        self.inner.flush().await
    }

    /// Returns the wrapped writer once all pending bits have been flushed.
    pub fn into_inner(self) -> io::Result<W> {
        if !self.is_aligned() {
            return Err(invalid_alignment());
        }

        Ok(self.inner)
    }
}

fn invalid_alignment() -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, INVALID_ALIGNMENT_MESSAGE)
}
