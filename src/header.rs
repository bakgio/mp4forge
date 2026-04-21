//! MP4 box header parsing, encoding, and seek helpers.

use std::error::Error;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::boxes::BoxLookupContext;
use crate::fourcc::FourCc;

/// Byte width of a standard 32-bit MP4 box header.
pub const SMALL_HEADER_SIZE: u64 = 8;
/// Byte width of an MP4 box header that carries a 64-bit size.
pub const LARGE_HEADER_SIZE: u64 = 16;

/// Physical header layout used by a serialized box header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderForm {
    /// Four-byte size followed by the four-byte type.
    Small,
    /// Size marker `1` followed by the four-byte type and 64-bit size.
    Large,
    /// Size marker `0`, meaning the box extends to the end of the available stream.
    ExtendToEof,
}

impl HeaderForm {
    /// Returns the number of bytes occupied by this header form.
    pub const fn header_size(self) -> u64 {
        match self {
            Self::Small | Self::ExtendToEof => SMALL_HEADER_SIZE,
            Self::Large => LARGE_HEADER_SIZE,
        }
    }
}

/// Parsed or to-be-written MP4 box header metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxInfo {
    offset: u64,
    size: u64,
    header_size: u64,
    box_type: FourCc,
    extend_to_eof: bool,
    lookup_context: BoxLookupContext,
}

impl BoxInfo {
    /// Creates header metadata for a box with the given type and total size.
    pub const fn new(box_type: FourCc, size: u64) -> Self {
        Self {
            offset: 0,
            size,
            header_size: SMALL_HEADER_SIZE,
            box_type,
            extend_to_eof: false,
            lookup_context: BoxLookupContext::new(),
        }
    }

    /// Returns a copy of the header info with a new absolute offset.
    pub const fn with_offset(mut self, offset: u64) -> Self {
        self.offset = offset;
        self
    }

    /// Returns a copy of the header info with a new encoded header size.
    pub const fn with_header_size(mut self, header_size: u64) -> Self {
        self.header_size = header_size;
        self
    }

    /// Returns a copy of the header info with an updated extend-to-EOF flag.
    pub const fn with_extend_to_eof(mut self, extend_to_eof: bool) -> Self {
        self.extend_to_eof = extend_to_eof;
        self
    }

    /// Returns a copy of the header info with the supplied lookup context.
    pub const fn with_lookup_context(mut self, lookup_context: BoxLookupContext) -> Self {
        self.lookup_context = lookup_context;
        self
    }

    /// Returns the absolute byte offset of the box header.
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the full box size, including the header.
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Returns the encoded header size in bytes.
    pub const fn header_size(&self) -> u64 {
        self.header_size
    }

    /// Returns the four-character type stored in the header.
    pub const fn box_type(&self) -> FourCc {
        self.box_type
    }

    /// Returns `true` when the header extends the box to the end of the stream.
    pub const fn extend_to_eof(&self) -> bool {
        self.extend_to_eof
    }

    /// Returns the registry lookup context that applies while decoding this box.
    pub const fn lookup_context(&self) -> BoxLookupContext {
        self.lookup_context
    }

    /// Returns the declared payload size after subtracting the encoded header width.
    pub fn payload_size(&self) -> Result<u64, HeaderError> {
        self.size
            .checked_sub(self.header_size)
            .ok_or(HeaderError::SizeUnderflow {
                size: self.size,
                header_size: self.header_size,
            })
    }

    /// Returns the serialized header form implied by the stored metadata.
    pub const fn header_form(&self) -> HeaderForm {
        if self.extend_to_eof {
            HeaderForm::ExtendToEof
        } else if self.size <= u32::MAX as u64 && self.header_size != LARGE_HEADER_SIZE {
            HeaderForm::Small
        } else {
            HeaderForm::Large
        }
    }

    /// Encodes the current header into its on-disk byte representation.
    pub fn encode(&self) -> Vec<u8> {
        match self.header_form() {
            HeaderForm::Small => {
                let mut data = Vec::with_capacity(SMALL_HEADER_SIZE as usize);
                data.extend_from_slice(&(self.size as u32).to_be_bytes());
                data.extend_from_slice(self.box_type.as_bytes());
                data
            }
            HeaderForm::Large => {
                let mut data = Vec::with_capacity(LARGE_HEADER_SIZE as usize);
                data.extend_from_slice(&1_u32.to_be_bytes());
                data.extend_from_slice(self.box_type.as_bytes());
                data.extend_from_slice(&self.size.to_be_bytes());
                data
            }
            HeaderForm::ExtendToEof => {
                let mut data = Vec::with_capacity(SMALL_HEADER_SIZE as usize);
                data.extend_from_slice(&0_u32.to_be_bytes());
                data.extend_from_slice(self.box_type.as_bytes());
                data
            }
        }
    }

    /// Writes the header and returns normalized metadata that reflects the written form.
    pub fn write<W>(&self, writer: &mut W) -> Result<Self, HeaderError>
    where
        W: Write + Seek,
    {
        let offset = writer.stream_position()?;
        let encoded = self.encode();
        writer.write_all(&encoded)?;

        let prior_payload =
            self.size
                .checked_sub(self.header_size)
                .ok_or(HeaderError::SizeUnderflow {
                    size: self.size,
                    header_size: self.header_size,
                })?;

        Ok(Self {
            offset,
            size: prior_payload + encoded.len() as u64,
            header_size: encoded.len() as u64,
            box_type: self.box_type,
            extend_to_eof: self.extend_to_eof,
            lookup_context: self.lookup_context,
        })
    }

    /// Reads a header from the current stream position.
    pub fn read<R>(reader: &mut R) -> Result<Self, HeaderError>
    where
        R: Read + Seek,
    {
        let offset = reader.stream_position()?;

        let mut small_header = [0_u8; SMALL_HEADER_SIZE as usize];
        reader.read_exact(&mut small_header)?;

        let size = u32::from_be_bytes([
            small_header[0],
            small_header[1],
            small_header[2],
            small_header[3],
        ]) as u64;
        let box_type = FourCc::from_bytes([
            small_header[4],
            small_header[5],
            small_header[6],
            small_header[7],
        ]);

        let mut info = Self::new(box_type, size).with_offset(offset);

        if size == 0 {
            // Extend-to-EOF boxes are normalized to their effective stream length.
            let end = reader.seek(SeekFrom::End(0))?;
            info.size = end - offset;
            info.extend_to_eof = true;
            info.seek_to_payload(reader)?;
        } else if size == 1 {
            // Size marker `1` switches the header into its 64-bit form.
            let mut large_size = [0_u8; 8];
            reader.read_exact(&mut large_size)?;
            info.header_size = LARGE_HEADER_SIZE;
            info.size = u64::from_be_bytes(large_size);
        }

        if info.size == 0 {
            return Err(HeaderError::InvalidSize);
        }

        if info.size < info.header_size {
            return Err(HeaderError::SizeUnderflow {
                size: info.size,
                header_size: info.header_size,
            });
        }

        Ok(info)
    }

    pub(crate) fn set_lookup_context(&mut self, lookup_context: BoxLookupContext) {
        self.lookup_context = lookup_context;
    }

    /// Seeks to the beginning of the box header.
    pub fn seek_to_start<S: Seek>(&self, seeker: &mut S) -> io::Result<u64> {
        seeker.seek(SeekFrom::Start(self.offset))
    }

    /// Seeks to the start of the box payload.
    pub fn seek_to_payload<S: Seek>(&self, seeker: &mut S) -> io::Result<u64> {
        seeker.seek(SeekFrom::Start(self.offset + self.header_size))
    }

    /// Seeks to the byte immediately after the end of the box.
    pub fn seek_to_end<S: Seek>(&self, seeker: &mut S) -> io::Result<u64> {
        seeker.seek(SeekFrom::Start(self.offset + self.size))
    }
}

/// Errors raised while parsing or writing box headers.
#[derive(Debug)]
pub enum HeaderError {
    Io(io::Error),
    InvalidSize,
    SizeUnderflow { size: u64, header_size: u64 },
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::InvalidSize => f.write_str("invalid size"),
            Self::SizeUnderflow { size, header_size } => {
                write!(
                    f,
                    "declared box size {size} is smaller than header size {header_size}"
                )
            }
        }
    }
}

impl Error for HeaderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::InvalidSize | Self::SizeUnderflow { .. } => None,
        }
    }
}

impl From<io::Error> for HeaderError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
