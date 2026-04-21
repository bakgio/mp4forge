//! Four-character box identifier support.

use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::str::FromStr;

/// Four-byte identifier used by MP4 boxes and related structures.
#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct FourCc([u8; 4]);

impl FourCc {
    /// Wildcard-style identifier used by path matching and "any type" handling.
    pub const ANY: Self = Self([0x00, 0x00, 0x00, 0x00]);

    /// Creates an identifier from its raw bytes.
    pub const fn from_bytes(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }

    /// Creates an identifier from a big-endian `u32`.
    pub const fn from_u32(value: u32) -> Self {
        Self(value.to_be_bytes())
    }

    /// Borrows the raw four-byte identifier.
    pub const fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }

    /// Returns the raw four-byte identifier by value.
    pub const fn into_bytes(self) -> [u8; 4] {
        self.0
    }

    /// Returns `true` when the identifiers are equal or either side is the wildcard marker.
    pub fn matches(self, other: Self) -> bool {
        self == Self::ANY || other == Self::ANY || self.0 == other.0
    }

    fn is_printable(self) -> bool {
        self.0.iter().all(|byte| matches!(byte, 0x20..=0x7e | 0xa9))
    }
}

impl From<[u8; 4]> for FourCc {
    fn from(value: [u8; 4]) -> Self {
        Self::from_bytes(value)
    }
}

impl From<u32> for FourCc {
    fn from(value: u32) -> Self {
        Self::from_u32(value)
    }
}

impl TryFrom<&str> for FourCc {
    type Error = ParseFourCcError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let bytes = value.as_bytes();
        if bytes.len() != 4 {
            return Err(ParseFourCcError { len: bytes.len() });
        }

        Ok(Self([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

impl FromStr for FourCc {
    type Err = ParseFourCcError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

impl fmt::Display for FourCc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_printable() {
            for byte in self.0 {
                if byte == 0xa9 {
                    f.write_str("(c)")?;
                } else {
                    f.write_char(char::from(byte))?;
                }
            }
            return Ok(());
        }

        write!(
            f,
            "0x{:02x}{:02x}{:02x}{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
}

impl fmt::Debug for FourCc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FourCc(\"{self}\")")
    }
}

/// Error returned when a string does not contain exactly four bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseFourCcError {
    len: usize,
}

impl ParseFourCcError {
    /// Returns the invalid byte length that triggered the parse failure.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` when the caller attempted to parse an empty string.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl fmt::Display for ParseFourCcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fourcc values must be exactly 4 bytes, got {}", self.len)
    }
}

impl Error for ParseFourCcError {}
