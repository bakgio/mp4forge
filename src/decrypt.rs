//! Feature-gated synchronous decryption types and helpers.
//!
//! This module exposes the additive public shape for the native decryption rollout without
//! changing the current default build. The initial synchronous path targets the Common Encryption
//! family first, then extends through additive broader protected-format branches that compose with
//! the crate's existing synchronous and Tokio-based async library architecture. The in-memory
//! decrypt entry points stay on the synchronous path, while the additive async surface later
//! composes on top for file-backed decrypt workflows.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::Cursor;
use std::io::Seek;
use std::path::Path;

use aes::Aes128;
use aes::cipher::{Block, BlockDecrypt, BlockEncrypt, KeyInit};
#[cfg(feature = "async")]
use tokio::fs as tokio_fs;

use crate::BoxInfo;
use crate::FourCc;
use crate::boxes::isma_cryp::{Isfm, Islt};
use crate::boxes::iso14496_12::{
    Co64, Frma, Ftyp, Mfro, Mpod, Saio, Saiz, Sbgp, Schm, Sgpd, Sidx, Stco, Stsc, Stsz,
    TFHD_BASE_DATA_OFFSET_PRESENT, TFHD_DEFAULT_BASE_IS_MOOF, TFHD_DEFAULT_SAMPLE_SIZE_PRESENT,
    TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT, TRUN_DATA_OFFSET_PRESENT, TRUN_SAMPLE_SIZE_PRESENT,
    Tfhd, Tfra, Tkhd, Trex, Trun, UUID_SAMPLE_ENCRYPTION, Uuid, UuidPayload,
};
use crate::boxes::iso14496_14::{DescriptorCommand, Iods, parse_descriptor_commands};
use crate::boxes::iso23001_7::{Senc, Tenc, decode_senc_payload_with_iv_size};
use crate::boxes::marlin::{
    MARLIN_BRAND_MGSV, MARLIN_IPMPS_TYPE_MGSV, MarlinShortSchm, MarlinStyp,
};
use crate::boxes::oma_dcf::{
    Grpi, OHDR_ENCRYPTION_METHOD_AES_CBC, OHDR_ENCRYPTION_METHOD_AES_CTR,
    OHDR_ENCRYPTION_METHOD_NULL, OHDR_PADDING_SCHEME_NONE, OHDR_PADDING_SCHEME_RFC_2630, Odaf,
    Odda, Odhe, Ohdr,
};
use crate::codec::{ImmutableBox, MutableBox, marshal, unmarshal};
use crate::encryption::{
    ResolveSampleEncryptionError, ResolvedSampleEncryptionSample, SampleEncryptionContext,
    resolve_sample_encryption,
};
use crate::extract::{ExtractError, extract_box, extract_box_as, extract_box_payload_bytes};
use crate::sidx::{
    TopLevelSidxPlan, TopLevelSidxPlanAction, TopLevelSidxPlanOptions,
    apply_top_level_sidx_plan_bytes, plan_top_level_sidx_update_bytes,
};
use crate::walk::BoxPath;

const CENC: FourCc = FourCc::from_bytes(*b"cenc");
const CENS: FourCc = FourCc::from_bytes(*b"cens");
const CBC1: FourCc = FourCc::from_bytes(*b"cbc1");
const CBCS: FourCc = FourCc::from_bytes(*b"cbcs");
const ENCV: FourCc = FourCc::from_bytes(*b"encv");
const ENCA: FourCc = FourCc::from_bytes(*b"enca");
const FREE: FourCc = FourCc::from_bytes(*b"free");
const FTYP: FourCc = FourCc::from_bytes(*b"ftyp");
const IODS: FourCc = FourCc::from_bytes(*b"iods");
const MDAT: FourCc = FourCc::from_bytes(*b"mdat");
const MFRA: FourCc = FourCc::from_bytes(*b"mfra");
const MFRO: FourCc = FourCc::from_bytes(*b"mfro");
const MOOF: FourCc = FourCc::from_bytes(*b"moof");
const MOOV: FourCc = FourCc::from_bytes(*b"moov");
const MVEX: FourCc = FourCc::from_bytes(*b"mvex");
const ODCF: FourCc = FourCc::from_bytes(*b"odcf");
const ODAF: FourCc = FourCc::from_bytes(*b"odaf");
const ODDA: FourCc = FourCc::from_bytes(*b"odda");
const ODHE: FourCc = FourCc::from_bytes(*b"odhe");
const OHDR: FourCc = FourCc::from_bytes(*b"ohdr");
const ODKM: FourCc = FourCc::from_bytes(*b"odkm");
const ODRM: FourCc = FourCc::from_bytes(*b"odrm");
const OPF2: FourCc = FourCc::from_bytes(*b"opf2");
const GRPI: FourCc = FourCc::from_bytes(*b"grpi");
const PIFF: FourCc = FourCc::from_bytes(*b"piff");
const SBGP: FourCc = FourCc::from_bytes(*b"sbgp");
const SGPD: FourCc = FourCc::from_bytes(*b"sgpd");
const SAIO: FourCc = FourCc::from_bytes(*b"saio");
const SAIZ: FourCc = FourCc::from_bytes(*b"saiz");
const SENC: FourCc = FourCc::from_bytes(*b"senc");
const SINF: FourCc = FourCc::from_bytes(*b"sinf");
const SCHI: FourCc = FourCc::from_bytes(*b"schi");
const SCHM: FourCc = FourCc::from_bytes(*b"schm");
const GKEY: FourCc = FourCc::from_bytes(*b"gkey");
const STBL: FourCc = FourCc::from_bytes(*b"stbl");
const STCO: FourCc = FourCc::from_bytes(*b"stco");
const STSC: FourCc = FourCc::from_bytes(*b"stsc");
const STSD: FourCc = FourCc::from_bytes(*b"stsd");
const STSZ: FourCc = FourCc::from_bytes(*b"stsz");
const TKHD: FourCc = FourCc::from_bytes(*b"tkhd");
const TRAF: FourCc = FourCc::from_bytes(*b"traf");
const TRAK: FourCc = FourCc::from_bytes(*b"trak");
const TENC: FourCc = FourCc::from_bytes(*b"tenc");
const TFHD: FourCc = FourCc::from_bytes(*b"tfhd");
const TFRA: FourCc = FourCc::from_bytes(*b"tfra");
const TREX: FourCc = FourCc::from_bytes(*b"trex");
const TRUN: FourCc = FourCc::from_bytes(*b"trun");
const UUID: FourCc = FourCc::from_bytes(*b"uuid");
const FRMA: FourCc = FourCc::from_bytes(*b"frma");
const MDIA: FourCc = FourCc::from_bytes(*b"mdia");
const MINF: FourCc = FourCc::from_bytes(*b"minf");
const SEIG: FourCc = FourCc::from_bytes(*b"seig");
const IAEC: FourCc = FourCc::from_bytes(*b"iAEC");

const PIFF_TRACK_ENCRYPTION_USER_TYPE: [u8; 16] = [
    0x89, 0x74, 0xdb, 0xce, 0x7b, 0xe7, 0x4c, 0x51, 0x84, 0xf9, 0x71, 0x48, 0xf9, 0x88, 0x25, 0x54,
];

/// Native Common Encryption scheme types targeted by the first decryption implementation phase.
pub const NATIVE_COMMON_ENCRYPTION_SCHEME_TYPES: [FourCc; 4] = [CENC, CENS, CBC1, CBCS];

/// MP4-family decryption format groups covered by the full decryption roadmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecryptionFormatFamily {
    /// The Common Encryption family, including `cenc`, `cens`, `cbc1`, and `cbcs`.
    CommonEncryption,
    /// OMA DCF protected MP4-family content.
    OmaDcf,
    /// Marlin IPMP protected MP4-family content.
    MarlinIpmp,
    /// PIFF-triggered compatibility behavior for protected fragmented content.
    PiffCompatibility,
    /// Generic protected MP4-family fallback behavior when a more specific family does not apply.
    StandardProtected,
}

/// Broader MP4-family decryption groups that extend beyond the native Common Encryption core.
pub const BROADER_MP4_DECRYPTION_FAMILIES: [DecryptionFormatFamily; 4] = [
    DecryptionFormatFamily::OmaDcf,
    DecryptionFormatFamily::MarlinIpmp,
    DecryptionFormatFamily::PiffCompatibility,
    DecryptionFormatFamily::StandardProtected,
];

/// Full MP4-family decryption groups that the roadmap keeps in scope.
pub const FULL_MP4_DECRYPTION_FAMILIES: [DecryptionFormatFamily; 5] = [
    DecryptionFormatFamily::CommonEncryption,
    DecryptionFormatFamily::OmaDcf,
    DecryptionFormatFamily::MarlinIpmp,
    DecryptionFormatFamily::PiffCompatibility,
    DecryptionFormatFamily::StandardProtected,
];

/// Native Common Encryption scheme variants supported by the first decryption core landing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeCommonEncryptionScheme {
    /// AES-CTR full-sample Common Encryption.
    Cenc,
    /// AES-CTR Common Encryption with pattern metadata when present.
    Cens,
    /// AES-CBC full-block Common Encryption.
    Cbc1,
    /// AES-CBC Common Encryption with pattern metadata when present.
    Cbcs,
}

impl NativeCommonEncryptionScheme {
    /// Returns the four-character scheme type for this native variant.
    pub const fn scheme_type(self) -> FourCc {
        match self {
            Self::Cenc => CENC,
            Self::Cens => CENS,
            Self::Cbc1 => CBC1,
            Self::Cbcs => CBCS,
        }
    }

    /// Resolves one native Common Encryption variant from a four-character scheme type.
    pub fn from_scheme_type(scheme_type: FourCc) -> Option<Self> {
        match scheme_type {
            CENC => Some(Self::Cenc),
            CENS => Some(Self::Cens),
            CBC1 => Some(Self::Cbc1),
            CBCS => Some(Self::Cbcs),
            _ => None,
        }
    }

    const fn uses_cbc(self) -> bool {
        matches!(self, Self::Cbc1 | Self::Cbcs)
    }

    const fn resets_iv_at_each_subsample(self) -> bool {
        matches!(self, Self::Cbcs)
    }
}

/// Identifies a decryption key either by decimal track ID or by 128-bit KID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecryptionKeyId {
    /// A decimal track identifier.
    TrackId(u32),
    /// A 128-bit key identifier.
    Kid([u8; 16]),
}

impl DecryptionKeyId {
    /// Parses one key identifier in the supported decryption syntax.
    ///
    /// The accepted forms are:
    ///
    /// - a decimal track ID such as `1`
    /// - a 32-character hexadecimal KID such as `00112233445566778899aabbccddeeff`
    pub fn from_spec(input: &str) -> Result<Self, ParseDecryptionKeyError> {
        if input.len() == 32 {
            return Ok(Self::Kid(parse_hex_16("key id", input)?));
        }

        let track_id =
            input
                .parse::<u32>()
                .map_err(|_| ParseDecryptionKeyError::InvalidTrackId {
                    input: input.to_owned(),
                })?;
        Ok(Self::TrackId(track_id))
    }
}

/// One decryption key entry addressed either by decimal track ID or by KID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DecryptionKey {
    id: DecryptionKeyId,
    key: [u8; 16],
}

impl DecryptionKey {
    /// Parses one decryption key entry from the supported `ID:KEY` syntax.
    pub fn from_spec(input: &str) -> Result<Self, ParseDecryptionKeyError> {
        let (id_text, key_text) =
            input
                .split_once(':')
                .ok_or_else(|| ParseDecryptionKeyError::InvalidSpec {
                    input: input.to_owned(),
                    reason: "expected <id>:<key>",
                })?;

        Ok(Self {
            id: DecryptionKeyId::from_spec(id_text)?,
            key: parse_hex_16("content key", key_text)?,
        })
    }

    /// Creates a decryption key addressed by decimal track ID.
    pub fn track(track_id: u32, key: [u8; 16]) -> Self {
        Self {
            id: DecryptionKeyId::TrackId(track_id),
            key,
        }
    }

    /// Creates a decryption key addressed by 128-bit KID.
    pub fn kid(kid: [u8; 16], key: [u8; 16]) -> Self {
        Self {
            id: DecryptionKeyId::Kid(kid),
            key,
        }
    }

    /// Returns the identifier used to select this key.
    pub fn id(&self) -> DecryptionKeyId {
        self.id
    }

    /// Returns the raw 16-byte content key.
    pub fn key_bytes(&self) -> [u8; 16] {
        self.key
    }

    /// Formats this key entry back into `ID:KEY` syntax.
    pub fn to_spec(&self) -> String {
        match self.id {
            DecryptionKeyId::TrackId(track_id) => {
                format!("{track_id}:{}", encode_hex(self.key))
            }
            DecryptionKeyId::Kid(kid) => format!("{}:{}", encode_hex(kid), encode_hex(self.key)),
        }
    }
}

/// Coarse decryption progress phases shared by the sync and async decryption paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecryptProgressPhase {
    /// Opening the encrypted input source.
    OpenInput,
    /// Opening the decrypted output target.
    OpenOutput,
    /// Opening the optional fragments-info input.
    OpenFragmentsInfo,
    /// Inspecting the file structure and resolving the active decrypt path.
    InspectStructure,
    /// Transforming encrypted samples into decrypted output.
    ProcessSamples,
    /// Finalizing the rewritten decrypted output.
    FinalizeOutput,
}

/// A snapshot of decryption progress for one sync or async operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DecryptProgress {
    /// Current phase of the decryption operation.
    pub phase: DecryptProgressPhase,
    /// Completed work units for the current phase.
    pub completed: u64,
    /// Total work units for the current phase when they are known.
    pub total: Option<u64>,
}

impl DecryptProgress {
    /// Creates one progress snapshot.
    pub const fn new(phase: DecryptProgressPhase, completed: u64, total: Option<u64>) -> Self {
        Self {
            phase,
            completed,
            total,
        }
    }
}

/// Additive synchronous decryption options for the native decryption path.
///
/// The same option shape is intended to stay reusable by later async and CLI layers. Keys may be
/// supplied repeatedly, addressed either by decimal track ID or by 128-bit KID. When decrypting a
/// standalone media segment, callers can also supply the matching initialization-segment bytes
/// through `fragments_info`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecryptOptions {
    keys: Vec<DecryptionKey>,
    fragments_info: Option<Vec<u8>>,
}

impl DecryptOptions {
    /// Creates an empty option set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the configured decryption keys in lookup order.
    pub fn keys(&self) -> &[DecryptionKey] {
        &self.keys
    }

    /// Adds one already-parsed decryption key to this option set.
    pub fn add_key(&mut self, key: DecryptionKey) -> &mut Self {
        self.keys.push(key);
        self
    }

    /// Adds one already-parsed decryption key and returns the updated option set.
    pub fn with_key(mut self, key: DecryptionKey) -> Self {
        self.add_key(key);
        self
    }

    /// Parses and adds one `ID:KEY` entry to this option set.
    pub fn add_key_spec(&mut self, input: &str) -> Result<&mut Self, ParseDecryptionKeyError> {
        self.keys.push(DecryptionKey::from_spec(input)?);
        Ok(self)
    }

    /// Parses and adds one `ID:KEY` entry, returning the updated option set.
    pub fn with_key_spec(mut self, input: &str) -> Result<Self, ParseDecryptionKeyError> {
        self.add_key_spec(input)?;
        Ok(self)
    }

    /// Returns the optional initialization-segment bytes used for standalone media segments.
    pub fn fragments_info_bytes(&self) -> Option<&[u8]> {
        self.fragments_info.as_deref()
    }

    /// Stores initialization-segment bytes for later standalone media-segment decryption.
    pub fn set_fragments_info_bytes(&mut self, fragments_info: impl AsRef<[u8]>) -> &mut Self {
        self.fragments_info = Some(fragments_info.as_ref().to_vec());
        self
    }

    /// Stores initialization-segment bytes and returns the updated option set.
    pub fn with_fragments_info_bytes(mut self, fragments_info: impl AsRef<[u8]>) -> Self {
        self.set_fragments_info_bytes(fragments_info);
        self
    }

    /// Clears any previously stored initialization-segment bytes.
    pub fn clear_fragments_info_bytes(&mut self) -> &mut Self {
        self.fragments_info = None;
        self
    }
}

/// Errors raised while parsing decryption key input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseDecryptionKeyError {
    /// The outer `ID:KEY` form was malformed.
    InvalidSpec {
        /// Original user input.
        input: String,
        /// Human-readable reason for rejection.
        reason: &'static str,
    },
    /// The track-ID portion was not a valid unsigned decimal integer.
    InvalidTrackId {
        /// Original user input for the track ID field.
        input: String,
    },
    /// A fixed-length hexadecimal field had the wrong number of characters.
    InvalidHexLength {
        /// Field name used in the error message.
        field: &'static str,
        /// Actual character length of the field.
        actual: usize,
    },
    /// A hexadecimal field contained a non-hexadecimal character.
    InvalidHexDigit {
        /// Field name used in the error message.
        field: &'static str,
        /// Zero-based byte index of the rejected nibble pair.
        index: usize,
        /// Rejected character value.
        value: char,
    },
}

impl fmt::Display for ParseDecryptionKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSpec { input, reason } => {
                write!(f, "invalid decryption key spec {input:?}: {reason}")
            }
            Self::InvalidTrackId { input } => {
                write!(
                    f,
                    "invalid track id {input:?}: expected an unsigned decimal integer"
                )
            }
            Self::InvalidHexLength { field, actual } => write!(
                f,
                "invalid {field}: expected 32 hexadecimal characters but found {actual}"
            ),
            Self::InvalidHexDigit {
                field,
                index,
                value,
            } => write!(
                f,
                "invalid {field}: character {value:?} at byte index {index} is not hexadecimal"
            ),
        }
    }
}

impl Error for ParseDecryptionKeyError {}

/// Errors raised by the high-level synchronous decryption API.
#[derive(Debug)]
pub enum DecryptError {
    /// File-backed decrypt I/O failed.
    Io(std::io::Error),
    /// The native decrypt-and-rewrite path rejected the current input or transform state.
    Rewrite(DecryptRewriteError),
    /// Standalone media-segment decrypt requires matching initialization-segment bytes.
    MissingFragmentsInfo,
    /// The input does not match one of the currently supported synchronous decrypt layouts.
    InvalidInput {
        /// Human-readable explanation of the rejected input shape.
        reason: String,
    },
}

impl fmt::Display for DecryptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Rewrite(error) => error.fmt(f),
            Self::MissingFragmentsInfo => write!(
                f,
                "standalone media-segment decrypt requires matching fragments-info bytes"
            ),
            Self::InvalidInput { reason } => write!(f, "unsupported decrypt input: {reason}"),
        }
    }
}

impl Error for DecryptError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Rewrite(error) => Some(error),
            Self::MissingFragmentsInfo | Self::InvalidInput { .. } => None,
        }
    }
}

impl From<std::io::Error> for DecryptError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ExtractError> for DecryptError {
    fn from(value: ExtractError) -> Self {
        Self::Rewrite(value.into())
    }
}

impl From<DecryptRewriteError> for DecryptError {
    fn from(value: DecryptRewriteError) -> Self {
        Self::Rewrite(value)
    }
}

/// Errors raised by the native Common Encryption sample-transform core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommonEncryptionDecryptError {
    /// The caller requested a scheme type outside the current native Common Encryption set.
    UnsupportedNativeSchemeType {
        /// Raw scheme type that could not be mapped to the native core.
        scheme_type: FourCc,
    },
    /// No key matched the current sample's track ID or KID.
    MissingDecryptionKey {
        /// Optional track ID supplied by the higher-level caller for key lookup precedence.
        track_id: Option<u32>,
        /// Effective sample KID resolved from typed encryption defaults.
        kid: [u8; 16],
    },
    /// A protected sample did not resolve any usable IV bytes.
    MissingInitializationVector {
        /// Native scheme that required the IV.
        scheme: NativeCommonEncryptionScheme,
    },
    /// A protected sample resolved an IV size that the native scheme does not accept.
    InvalidInitializationVectorSize {
        /// Native scheme that rejected the IV.
        scheme: NativeCommonEncryptionScheme,
        /// Actual resolved IV byte count.
        actual: usize,
        /// Human-readable allowed size set for the scheme.
        expected: &'static str,
    },
    /// One subsample declared more bytes than remain in the encrypted sample payload.
    InvalidProtectedRegion {
        /// Bytes left in the encrypted sample when the region was validated.
        remaining: usize,
        /// Clear bytes declared for the failing subsample region.
        clear_bytes: usize,
        /// Protected bytes declared for the failing subsample region.
        protected_bytes: usize,
    },
    /// A subsample region declared a protected-byte count that does not fit on this platform.
    ProtectedByteCountOverflow {
        /// Original 32-bit protected-byte count from the resolved sample metadata.
        protected_bytes: u32,
    },
}

impl fmt::Display for CommonEncryptionDecryptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedNativeSchemeType { scheme_type } => {
                write!(
                    f,
                    "unsupported native Common Encryption scheme type {scheme_type}"
                )
            }
            Self::MissingDecryptionKey { track_id, kid } => match track_id {
                Some(track_id) => write!(
                    f,
                    "missing decryption key for track {track_id} or KID {}",
                    encode_hex(*kid)
                ),
                None => write!(f, "missing decryption key for KID {}", encode_hex(*kid)),
            },
            Self::MissingInitializationVector { scheme } => {
                write!(
                    f,
                    "protected {scheme:?} sample is missing its effective initialization vector"
                )
            }
            Self::InvalidInitializationVectorSize {
                scheme,
                actual,
                expected,
            } => write!(
                f,
                "{scheme:?} requires {expected} initialization vector bytes but resolved {actual}"
            ),
            Self::InvalidProtectedRegion {
                remaining,
                clear_bytes,
                protected_bytes,
            } => write!(
                f,
                "subsample region exceeds the encrypted sample bounds: remaining={remaining}, clear={clear_bytes}, protected={protected_bytes}"
            ),
            Self::ProtectedByteCountOverflow { protected_bytes } => write!(
                f,
                "protected subsample byte count {protected_bytes} does not fit in usize"
            ),
        }
    }
}

impl Error for CommonEncryptionDecryptError {}

/// Errors raised while rewriting decrypted MP4 output for the native Common Encryption path.
#[derive(Debug)]
pub enum DecryptRewriteError {
    /// Typed extraction failed while analyzing the current input layout.
    Extract(ExtractError),
    /// Resolved sample-encryption defaults were inconsistent for the current track fragment.
    Resolve(ResolveSampleEncryptionError),
    /// Sample-level native Common Encryption transform work failed.
    Decrypt(CommonEncryptionDecryptError),
    /// The current encrypted layout is not one of the supported native rewrite shapes.
    InvalidLayout {
        /// Human-readable explanation of the rejected layout.
        reason: String,
    },
    /// A keyed protected track uses a scheme type outside the current native rewrite set.
    UnsupportedTrackSchemeType {
        /// Track identifier from `tkhd` or `tfhd`.
        track_id: u32,
        /// Raw `schm` scheme type that could not be mapped to the native rewrite path.
        scheme_type: FourCc,
    },
    /// A computed sample byte range did not fit within any root `mdat` payload.
    SampleDataRangeNotFound {
        /// Track identifier from `tkhd` or `tfhd`.
        track_id: u32,
        /// One-based sample index inside the active fragment track run.
        sample_index: u32,
        /// Absolute byte offset that the rewrite path attempted to read.
        absolute_offset: u64,
        /// Sample byte length requested at that offset.
        sample_size: u32,
    },
}

impl fmt::Display for DecryptRewriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Extract(error) => error.fmt(f),
            Self::Resolve(error) => error.fmt(f),
            Self::Decrypt(error) => error.fmt(f),
            Self::InvalidLayout { reason } => {
                write!(f, "unsupported native decrypt layout: {reason}")
            }
            Self::UnsupportedTrackSchemeType {
                track_id,
                scheme_type,
            } => write!(
                f,
                "track {track_id} uses unsupported native decrypt scheme type {scheme_type}"
            ),
            Self::SampleDataRangeNotFound {
                track_id,
                sample_index,
                absolute_offset,
                sample_size,
            } => write!(
                f,
                "sample {sample_index} for track {track_id} points outside root media data: offset={absolute_offset}, size={sample_size}"
            ),
        }
    }
}

impl Error for DecryptRewriteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Extract(error) => Some(error),
            Self::Resolve(error) => Some(error),
            Self::Decrypt(error) => Some(error),
            Self::InvalidLayout { .. }
            | Self::UnsupportedTrackSchemeType { .. }
            | Self::SampleDataRangeNotFound { .. } => None,
        }
    }
}

impl From<ExtractError> for DecryptRewriteError {
    fn from(value: ExtractError) -> Self {
        Self::Extract(value)
    }
}

impl From<ResolveSampleEncryptionError> for DecryptRewriteError {
    fn from(value: ResolveSampleEncryptionError) -> Self {
        Self::Resolve(value)
    }
}

impl From<CommonEncryptionDecryptError> for DecryptRewriteError {
    fn from(value: CommonEncryptionDecryptError) -> Self {
        Self::Decrypt(value)
    }
}

/// Selects the content key for one resolved sample using the native precedence rules.
///
/// The native path first looks for a track-ID match when one is supplied, then falls back to the
/// sample's effective KID.
pub fn select_decryption_key(
    keys: &[DecryptionKey],
    track_id: Option<u32>,
    sample: &ResolvedSampleEncryptionSample<'_>,
) -> Result<[u8; 16], CommonEncryptionDecryptError> {
    if let Some(track_id) = track_id
        && let Some(key) = keys.iter().find_map(|entry| match entry.id {
            DecryptionKeyId::TrackId(candidate) if candidate == track_id => Some(entry.key),
            _ => None,
        })
    {
        return Ok(key);
    }

    if let Some(key) = keys.iter().find_map(|entry| match entry.id {
        DecryptionKeyId::Kid(candidate) if candidate == sample.kid => Some(entry.key),
        _ => None,
    }) {
        return Ok(key);
    }

    Err(CommonEncryptionDecryptError::MissingDecryptionKey {
        track_id,
        kid: sample.kid,
    })
}

/// Decrypts one resolved Common Encryption sample using the supplied native scheme and content key.
///
/// This primitive core is isolated from file traversal and rewrite policy so it can be reused by
/// both the later sync and async file-backed entry points.
pub fn decrypt_common_encryption_sample(
    scheme: NativeCommonEncryptionScheme,
    content_key: [u8; 16],
    sample: &ResolvedSampleEncryptionSample<'_>,
    encrypted_sample: &[u8],
) -> Result<Vec<u8>, CommonEncryptionDecryptError> {
    if !sample.is_protected {
        return Ok(encrypted_sample.to_vec());
    }

    let iv = effective_initialization_vector(scheme, sample)?;
    let mut transformer = SampleTransformer::new(
        scheme,
        Aes128::new(&content_key.into()),
        iv,
        sample.crypt_byte_block,
        sample.skip_byte_block,
    );

    let mut output = vec![0_u8; encrypted_sample.len()];
    if sample.subsamples.is_empty() {
        transformer.transform_region(encrypted_sample, &mut output)?;
        return Ok(output);
    }

    let mut cursor = 0usize;
    for subsample in sample.subsamples {
        let clear_bytes = usize::from(subsample.bytes_of_clear_data);
        let protected_bytes = usize::try_from(subsample.bytes_of_protected_data).map_err(|_| {
            CommonEncryptionDecryptError::ProtectedByteCountOverflow {
                protected_bytes: subsample.bytes_of_protected_data,
            }
        })?;
        let region_len = clear_bytes.checked_add(protected_bytes).ok_or(
            CommonEncryptionDecryptError::InvalidProtectedRegion {
                remaining: encrypted_sample.len().saturating_sub(cursor),
                clear_bytes,
                protected_bytes,
            },
        )?;
        if encrypted_sample.len().saturating_sub(cursor) < region_len {
            return Err(CommonEncryptionDecryptError::InvalidProtectedRegion {
                remaining: encrypted_sample.len().saturating_sub(cursor),
                clear_bytes,
                protected_bytes,
            });
        }

        output[cursor..cursor + clear_bytes]
            .copy_from_slice(&encrypted_sample[cursor..cursor + clear_bytes]);
        cursor += clear_bytes;

        if protected_bytes != 0 {
            if scheme.resets_iv_at_each_subsample() {
                transformer.reset_for_subsample();
            }
            transformer.transform_region(
                &encrypted_sample[cursor..cursor + protected_bytes],
                &mut output[cursor..cursor + protected_bytes],
            )?;
            cursor += protected_bytes;
        }
    }

    output[cursor..].copy_from_slice(&encrypted_sample[cursor..]);
    Ok(output)
}

/// Resolves the content key and decrypts one resolved Common Encryption sample in one step.
pub fn decrypt_common_encryption_sample_with_keys(
    scheme: NativeCommonEncryptionScheme,
    track_id: Option<u32>,
    keys: &[DecryptionKey],
    sample: &ResolvedSampleEncryptionSample<'_>,
    encrypted_sample: &[u8],
) -> Result<Vec<u8>, CommonEncryptionDecryptError> {
    let content_key = select_decryption_key(keys, track_id, sample)?;
    decrypt_common_encryption_sample(scheme, content_key, sample, encrypted_sample)
}

/// Resolves a native scheme from a raw four-character code, then decrypts one sample with the
/// selected content key.
pub fn decrypt_common_encryption_sample_by_scheme_type_with_keys(
    scheme_type: FourCc,
    track_id: Option<u32>,
    keys: &[DecryptionKey],
    sample: &ResolvedSampleEncryptionSample<'_>,
    encrypted_sample: &[u8],
) -> Result<Vec<u8>, CommonEncryptionDecryptError> {
    let scheme = NativeCommonEncryptionScheme::from_scheme_type(scheme_type)
        .ok_or(CommonEncryptionDecryptError::UnsupportedNativeSchemeType { scheme_type })?;
    decrypt_common_encryption_sample_with_keys(scheme, track_id, keys, sample, encrypted_sample)
}

fn decrypt_sample_for_active_track(
    active: &ActiveTrackDecryption<'_>,
    sample: &ResolvedSampleEncryptionSample<'_>,
    encrypted_sample: &[u8],
) -> Result<Vec<u8>, CommonEncryptionDecryptError> {
    if active.sample_entry.scheme_type == PIFF {
        return Ok(encrypted_sample.to_vec());
    }

    decrypt_common_encryption_sample(active.scheme, active.key, sample, encrypted_sample)
}

/// Rewrites one encrypted initialization segment into a clear variant for the currently keyed
/// native Common Encryption tracks.
///
/// This helper rebuilds the affected sample-entry hierarchy into the canonical clear layout for
/// tracks with matching keys. PIFF-triggered compatibility tracks intentionally keep their
/// protected sample-entry structure so the output matches the established PIFF decrypt behavior.
/// Tracks without matching keys remain untouched so callers can perform partial decrypt workflows
/// without forcing the entire init segment to fail.
pub fn decrypt_common_encryption_init_bytes(
    init_segment: &[u8],
    keys: &[DecryptionKey],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let context = analyze_init_segment(init_segment)?;
    let rebuilt_moov = rebuild_common_encryption_moov(init_segment, &context, keys)?;
    let root_boxes = read_root_box_infos(init_segment)?;
    let mut output = Vec::with_capacity(init_segment.len());
    for info in root_boxes {
        if info.box_type() == MOOV {
            output.extend_from_slice(&rebuilt_moov);
        } else {
            output.extend_from_slice(slice_box_bytes(init_segment, info)?);
        }
    }
    Ok(output)
}

fn decrypt_common_encryption_init_bytes_legacy(
    init_segment: &[u8],
    keys: &[DecryptionKey],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let context = analyze_init_segment(init_segment)?;
    let mut output = init_segment.to_vec();
    for track in &context.tracks {
        for sample_entry in &track.protected_sample_entries {
            if resolve_key_for_sample_entry(track, sample_entry, keys)?.is_none()
                || sample_entry.scheme_type == PIFF
            {
                continue;
            }
            patch_sample_entry_type(
                &mut output,
                sample_entry.sample_entry_info,
                sample_entry.original_format,
            )?;
            replace_box_with_free(&mut output, sample_entry.sinf_info)?;
        }
    }
    Ok(output)
}

/// Rewrites one encrypted media segment into a clear variant using the keyed native Common
/// Encryption track definitions resolved from `init_segment`.
///
/// Tracks without matching keys remain untouched. The supported native rewrite path expects the
/// fragment sample metadata to be carried by typed `senc` boxes plus the existing typed protection
/// helpers. PIFF-triggered compatibility tracks keep their fragment protection boxes in place so
/// the decrypted output remains byte-compatible with the established PIFF decrypt behavior.
pub fn decrypt_common_encryption_media_segment_bytes(
    init_segment: &[u8],
    media_segment: &[u8],
    keys: &[DecryptionKey],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let context = analyze_init_segment(init_segment)?;
    decrypt_media_bytes_with_context(media_segment, &context, keys)
}

/// Rewrites one encrypted fragmented MP4 file into a clear variant for the currently keyed native
/// Common Encryption tracks.
///
/// This helper supports the common single-file layout where the movie box and one or more
/// fragments appear in the same byte stream. Tracks without matching keys remain untouched.
/// PIFF-triggered compatibility tracks preserve their protected movie and fragment structure and
/// keep their retained reference payload bytes unchanged.
pub fn decrypt_common_encryption_file_bytes(
    input: &[u8],
    keys: &[DecryptionKey],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let context = analyze_init_segment(input)?;
    if let Some(output) = try_rebuild_common_encryption_file_bytes(input, &context, keys)? {
        return refresh_fragmented_top_level_sidx(output);
    }

    let mut output = decrypt_common_encryption_init_bytes_legacy(input, keys)?;
    decrypt_media_bytes_in_place_legacy(input, &mut output, &context, keys)?;
    refresh_fragmented_top_level_sidx(output)
}

/// Decrypts one encrypted byte slice through the additive synchronous library surface.
///
/// Supported inputs are:
///
/// - an init segment containing `moov`
/// - a standalone media segment containing `moof`, when `options` also carries matching
///   initialization-segment bytes through `fragments_info`
/// - a single fragmented file containing both `moov` and one or more `moof` boxes
/// - a non-fragmented movie file containing `moov`, `mdat`, and the currently supported OMA DCF
///   protected sample-entry layout
/// - a top-level OMA DCF atom file containing one or more root `odrm` boxes
pub fn decrypt_bytes(input: &[u8], options: &DecryptOptions) -> Result<Vec<u8>, DecryptError> {
    decrypt_bytes_with_optional_progress(input, options, None::<fn(DecryptProgress)>)
}

/// Decrypts one encrypted byte slice and reports coarse synchronous progress snapshots.
pub fn decrypt_bytes_with_progress<F>(
    input: &[u8],
    options: &DecryptOptions,
    progress: F,
) -> Result<Vec<u8>, DecryptError>
where
    F: FnMut(DecryptProgress),
{
    decrypt_bytes_with_optional_progress(input, options, Some(progress))
}

/// Decrypts one encrypted file path into a clear output file through the additive synchronous
/// library surface.
pub fn decrypt_file(
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    options: &DecryptOptions,
) -> Result<(), DecryptError> {
    decrypt_file_with_optional_progress(
        input_path.as_ref(),
        output_path.as_ref(),
        options,
        None::<fn(DecryptProgress)>,
    )
}

/// Decrypts one encrypted file path into a clear output file and reports coarse synchronous
/// progress snapshots.
pub fn decrypt_file_with_progress<F>(
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    options: &DecryptOptions,
    progress: F,
) -> Result<(), DecryptError>
where
    F: FnMut(DecryptProgress),
{
    decrypt_file_with_optional_progress(
        input_path.as_ref(),
        output_path.as_ref(),
        options,
        Some(progress),
    )
}

/// Decrypts one encrypted file path into a clear output file through the additive Tokio-based
/// async library surface.
///
/// The async decrypt rollout stays file-backed for now. Pure in-memory decrypt entry points remain
/// on the synchronous path because the native transform core itself does not perform asynchronous
/// I/O. The supported file-backed layouts are the same as the synchronous path, including
/// top-level OMA DCF atom files and the currently supported protected-sample-entry OMA DCF movie
/// layout.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn decrypt_file_async(
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    options: &DecryptOptions,
) -> Result<(), DecryptError> {
    decrypt_file_with_optional_progress_async(
        input_path.as_ref(),
        output_path.as_ref(),
        options,
        None::<fn(DecryptProgress)>,
    )
    .await
}

/// Decrypts one encrypted file path into a clear output file through the additive Tokio-based
/// async library surface and reports coarse progress snapshots.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn decrypt_file_with_progress_async<F>(
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    options: &DecryptOptions,
    progress: F,
) -> Result<(), DecryptError>
where
    F: FnMut(DecryptProgress) + Send,
{
    decrypt_file_with_optional_progress_async(
        input_path.as_ref(),
        output_path.as_ref(),
        options,
        Some(progress),
    )
    .await
}

#[derive(Clone)]
struct InitDecryptContext {
    moov_info: BoxInfo,
    tracks: Vec<ProtectedTrackState>,
}

#[derive(Clone)]
struct ProtectedTrackState {
    track_id: u32,
    trak_info: BoxInfo,
    mdia_info: BoxInfo,
    minf_info: BoxInfo,
    stbl_info: BoxInfo,
    stsd_info: BoxInfo,
    protected_sample_entries: Vec<ProtectedSampleEntryState>,
    trex: Option<Trex>,
}

#[derive(Clone)]
struct ProtectedSampleEntryState {
    sample_description_index: u32,
    sample_entry_info: BoxInfo,
    original_format: FourCc,
    scheme_type: FourCc,
    sinf_info: BoxInfo,
    tenc: Tenc,
    piff_protection_mode: Option<u8>,
}

#[derive(Clone)]
struct OmaProtectedMovieContext {
    ftyp_info: Option<BoxInfo>,
    moov_info: BoxInfo,
    tracks: Vec<OmaProtectedMovieTrackState>,
    other_tracks: Vec<MovieChunkTrackState>,
    mdat_infos: Vec<BoxInfo>,
}

#[derive(Clone)]
struct OmaProtectedMovieTrackState {
    track_id: u32,
    trak_info: BoxInfo,
    mdia_info: BoxInfo,
    minf_info: BoxInfo,
    stbl_info: BoxInfo,
    stsd_info: BoxInfo,
    sample_entry_info: BoxInfo,
    original_format: FourCc,
    sinf_info: BoxInfo,
    stsz_info: BoxInfo,
    stsz: Stsz,
    stsc: Stsc,
    chunk_offsets: ChunkOffsetBoxState,
    sample_sizes: Vec<u32>,
    odaf: Odaf,
    ohdr: Ohdr,
}

#[derive(Clone)]
struct IaecProtectedMovieContext {
    ftyp_info: Option<BoxInfo>,
    moov_info: BoxInfo,
    tracks: Vec<IaecProtectedMovieTrackState>,
    other_tracks: Vec<MovieChunkTrackState>,
    mdat_infos: Vec<BoxInfo>,
}

#[derive(Clone)]
struct IaecProtectedMovieTrackState {
    track_id: u32,
    trak_info: BoxInfo,
    mdia_info: BoxInfo,
    minf_info: BoxInfo,
    stbl_info: BoxInfo,
    stsd_info: BoxInfo,
    sample_entry_info: BoxInfo,
    original_format: FourCc,
    sinf_info: BoxInfo,
    stsz_info: BoxInfo,
    stsz: Stsz,
    stsc: Stsc,
    chunk_offsets: ChunkOffsetBoxState,
    sample_sizes: Vec<u32>,
    isfm: Isfm,
    islt: Option<Islt>,
}

#[derive(Clone)]
struct MovieChunkTrackState {
    track_id: u32,
    trak_info: BoxInfo,
    mdia_info: BoxInfo,
    minf_info: BoxInfo,
    stbl_info: BoxInfo,
    stsc: Stsc,
    chunk_offsets: ChunkOffsetBoxState,
    sample_sizes: Vec<u32>,
}

type TrackRelativeChunkOffsets = BTreeMap<u32, Vec<u64>>;
type RebuiltMovieSampleSizes = BTreeMap<u32, Vec<u64>>;
type RebuiltMoviePayload = (Vec<u8>, RebuiltMovieSampleSizes, TrackRelativeChunkOffsets);

#[derive(Clone, Copy)]
struct MovieRootRewriteContext<'a> {
    input: &'a [u8],
    ftyp_info: Option<BoxInfo>,
    moov_info: BoxInfo,
    mdat_infos: &'a [BoxInfo],
}

#[derive(Clone)]
struct MarlinMovieContext {
    ftyp_info: BoxInfo,
    ftyp: Ftyp,
    moov_info: BoxInfo,
    iods_info: BoxInfo,
    od_track_info: BoxInfo,
    mdat_infos: Vec<BoxInfo>,
    tracks: Vec<MarlinMovieTrackState>,
}

#[derive(Clone)]
struct MarlinMovieTrackState {
    track_id: u32,
    trak_info: BoxInfo,
    mdia_info: BoxInfo,
    minf_info: BoxInfo,
    stbl_info: BoxInfo,
    stsz_info: BoxInfo,
    stsz: Stsz,
    stsc: Stsc,
    chunk_offsets: ChunkOffsetBoxState,
    sample_sizes: Vec<u32>,
    marlin: Option<MarlinTrackProtection>,
}

#[derive(Clone)]
enum ChunkOffsetBoxState {
    Stco { info: BoxInfo, box_value: Stco },
    Co64 { info: BoxInfo, box_value: Co64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarlinTrackKeyMode {
    Track,
    Group,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MarlinTrackProtection {
    key_mode: MarlinTrackKeyMode,
    stream_type: Option<String>,
    wrapped_group_key: Option<Vec<u8>>,
}

#[derive(Clone, Copy)]
struct MovieTrackPayloadPlan<'a> {
    track_id: u32,
    stsc: &'a Stsc,
    chunk_offsets: &'a ChunkOffsetBoxState,
    sample_sizes: &'a [u32],
}

struct MovieTrackRewritePlan {
    track_id: u32,
    trak_info: BoxInfo,
    mdia_info: BoxInfo,
    minf_info: BoxInfo,
    stbl_info: BoxInfo,
    chunk_offsets: ChunkOffsetBoxState,
    stsd_replacement: Option<(u64, Vec<u8>)>,
    stsz_replacement: Option<(u64, Vec<u8>)>,
}

#[derive(Clone, Copy)]
struct ActiveTrackDecryption<'a> {
    track: &'a ProtectedTrackState,
    sample_entry: &'a ProtectedSampleEntryState,
    scheme: NativeCommonEncryptionScheme,
    key: [u8; 16],
}

#[derive(Clone, Copy)]
struct MediaDataRange {
    start: u64,
    end: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecryptInputLayout {
    InitSegment,
    MediaSegment,
    FragmentedFile,
    MarlinIpmpFile,
    OmaDcfProtectedMovieFile,
    IaecProtectedMovieFile,
    OmaDcfAtomFile,
}

struct ProgressReporter<F> {
    callback: Option<F>,
}

impl<F> ProgressReporter<F>
where
    F: FnMut(DecryptProgress),
{
    fn new(callback: Option<F>) -> Self {
        Self { callback }
    }

    fn report(&mut self, phase: DecryptProgressPhase, completed: u64, total: Option<u64>) {
        if let Some(callback) = self.callback.as_mut() {
            callback(DecryptProgress::new(phase, completed, total));
        }
    }
}

fn decrypt_bytes_with_optional_progress<F>(
    input: &[u8],
    options: &DecryptOptions,
    progress: Option<F>,
) -> Result<Vec<u8>, DecryptError>
where
    F: FnMut(DecryptProgress),
{
    let mut reporter = ProgressReporter::new(progress);
    let output = decrypt_input_bytes(input, options, &mut reporter)?;
    reporter.report(DecryptProgressPhase::FinalizeOutput, 0, Some(1));
    reporter.report(DecryptProgressPhase::FinalizeOutput, 1, Some(1));
    Ok(output)
}

fn decrypt_file_with_optional_progress<F>(
    input_path: &Path,
    output_path: &Path,
    options: &DecryptOptions,
    progress: Option<F>,
) -> Result<(), DecryptError>
where
    F: FnMut(DecryptProgress),
{
    let mut reporter = ProgressReporter::new(progress);
    reporter.report(DecryptProgressPhase::OpenInput, 0, Some(1));
    let input = fs::read(input_path)?;
    reporter.report(DecryptProgressPhase::OpenInput, 1, Some(1));

    let output = decrypt_input_bytes(&input, options, &mut reporter)?;

    reporter.report(DecryptProgressPhase::OpenOutput, 0, Some(1));
    fs::write(output_path, output)?;
    reporter.report(DecryptProgressPhase::OpenOutput, 1, Some(1));
    reporter.report(DecryptProgressPhase::FinalizeOutput, 0, Some(1));
    reporter.report(DecryptProgressPhase::FinalizeOutput, 1, Some(1));
    Ok(())
}

#[cfg(feature = "async")]
async fn decrypt_file_with_optional_progress_async<F>(
    input_path: &Path,
    output_path: &Path,
    options: &DecryptOptions,
    progress: Option<F>,
) -> Result<(), DecryptError>
where
    F: FnMut(DecryptProgress) + Send,
{
    let mut reporter = ProgressReporter::new(progress);
    reporter.report(DecryptProgressPhase::OpenInput, 0, Some(1));
    let input = tokio_fs::read(input_path).await?;
    reporter.report(DecryptProgressPhase::OpenInput, 1, Some(1));

    let output = decrypt_input_bytes(&input, options, &mut reporter)?;

    reporter.report(DecryptProgressPhase::OpenOutput, 0, Some(1));
    tokio_fs::write(output_path, output).await?;
    reporter.report(DecryptProgressPhase::OpenOutput, 1, Some(1));
    reporter.report(DecryptProgressPhase::FinalizeOutput, 0, Some(1));
    reporter.report(DecryptProgressPhase::FinalizeOutput, 1, Some(1));
    Ok(())
}

fn decrypt_input_bytes<F>(
    input: &[u8],
    options: &DecryptOptions,
    reporter: &mut ProgressReporter<F>,
) -> Result<Vec<u8>, DecryptError>
where
    F: FnMut(DecryptProgress),
{
    reporter.report(DecryptProgressPhase::InspectStructure, 0, Some(1));
    let layout = classify_decrypt_input(input)?;
    reporter.report(DecryptProgressPhase::InspectStructure, 1, Some(1));
    match layout {
        DecryptInputLayout::InitSegment => {
            reporter.report(DecryptProgressPhase::ProcessSamples, 0, Some(1));
            let output = decrypt_common_encryption_init_bytes(input, options.keys())?;
            reporter.report(DecryptProgressPhase::ProcessSamples, 1, Some(1));
            Ok(output)
        }
        DecryptInputLayout::MediaSegment => {
            reporter.report(DecryptProgressPhase::OpenFragmentsInfo, 0, Some(1));
            let fragments_info = options
                .fragments_info_bytes()
                .ok_or(DecryptError::MissingFragmentsInfo)?;
            reporter.report(DecryptProgressPhase::OpenFragmentsInfo, 1, Some(1));
            reporter.report(DecryptProgressPhase::ProcessSamples, 0, Some(1));
            let output = decrypt_common_encryption_media_segment_bytes(
                fragments_info,
                input,
                options.keys(),
            )?;
            reporter.report(DecryptProgressPhase::ProcessSamples, 1, Some(1));
            Ok(output)
        }
        DecryptInputLayout::FragmentedFile => {
            reporter.report(DecryptProgressPhase::ProcessSamples, 0, Some(1));
            let output = decrypt_common_encryption_file_bytes(input, options.keys())?;
            reporter.report(DecryptProgressPhase::ProcessSamples, 1, Some(1));
            Ok(output)
        }
        DecryptInputLayout::MarlinIpmpFile => {
            reporter.report(DecryptProgressPhase::ProcessSamples, 0, Some(1));
            let output = decrypt_marlin_movie_file_bytes(input, options.keys())?;
            reporter.report(DecryptProgressPhase::ProcessSamples, 1, Some(1));
            Ok(output)
        }
        DecryptInputLayout::OmaDcfProtectedMovieFile => {
            reporter.report(DecryptProgressPhase::ProcessSamples, 0, Some(1));
            let output = decrypt_oma_dcf_movie_file_bytes(input, options.keys())?;
            reporter.report(DecryptProgressPhase::ProcessSamples, 1, Some(1));
            Ok(output)
        }
        DecryptInputLayout::IaecProtectedMovieFile => {
            reporter.report(DecryptProgressPhase::ProcessSamples, 0, Some(1));
            let output = decrypt_iaec_movie_file_bytes(input, options.keys())?;
            reporter.report(DecryptProgressPhase::ProcessSamples, 1, Some(1));
            Ok(output)
        }
        DecryptInputLayout::OmaDcfAtomFile => {
            reporter.report(DecryptProgressPhase::ProcessSamples, 0, Some(1));
            let output = decrypt_oma_dcf_atom_file_bytes(input, options.keys())?;
            reporter.report(DecryptProgressPhase::ProcessSamples, 1, Some(1));
            Ok(output)
        }
    }
}

fn classify_decrypt_input(input: &[u8]) -> Result<DecryptInputLayout, DecryptError> {
    let mut reader = Cursor::new(input);
    let has_moov = !extract_box(&mut reader, None, BoxPath::from([MOOV]))?.is_empty();
    let mut reader = Cursor::new(input);
    let has_moof = !extract_box(&mut reader, None, BoxPath::from([MOOF]))?.is_empty();
    let mut reader = Cursor::new(input);
    let has_mdat = !extract_box(&mut reader, None, BoxPath::from([MDAT]))?.is_empty();
    let mut reader = Cursor::new(input);
    let has_odrm = !extract_box(&mut reader, None, BoxPath::from([ODRM]))?.is_empty();
    let mut reader = Cursor::new(input);
    let ftyp = extract_box_as::<_, Ftyp>(&mut reader, None, BoxPath::from([FTYP]))?;
    let is_marlin_ipmp_movie = ftyp.iter().any(|entry| {
        entry.major_brand == MARLIN_BRAND_MGSV
            || entry.compatible_brands.contains(&MARLIN_BRAND_MGSV)
    });
    let is_oma_dcf_atom_file = has_odrm
        && ftyp
            .iter()
            .any(|entry| entry.major_brand == ODCF || entry.compatible_brands.contains(&ODCF));
    let protected_movie_layout =
        if has_moov && has_mdat && !has_moof && !is_oma_dcf_atom_file && is_marlin_ipmp_movie {
            Some(DecryptInputLayout::MarlinIpmpFile)
        } else if has_moov && has_mdat && !has_moof && !is_oma_dcf_atom_file {
            detect_non_fragmented_protected_movie_layout(input)?
        } else {
            None
        };

    match (
        has_moov,
        has_moof,
        has_mdat,
        is_oma_dcf_atom_file,
        protected_movie_layout,
    ) {
        (false, false, _, true, _) => Ok(DecryptInputLayout::OmaDcfAtomFile),
        (true, true, _, false, _) => Ok(DecryptInputLayout::FragmentedFile),
        (true, false, true, false, Some(DecryptInputLayout::MarlinIpmpFile)) => {
            Ok(DecryptInputLayout::MarlinIpmpFile)
        }
        (true, false, true, false, Some(DecryptInputLayout::OmaDcfProtectedMovieFile)) => {
            Ok(DecryptInputLayout::OmaDcfProtectedMovieFile)
        }
        (true, false, true, false, Some(DecryptInputLayout::IaecProtectedMovieFile)) => {
            Ok(DecryptInputLayout::IaecProtectedMovieFile)
        }
        (true, false, false, false, _) => Ok(DecryptInputLayout::InitSegment),
        (false, true, _, false, _) => Ok(DecryptInputLayout::MediaSegment),
        (false, false, false, false, _) => Err(DecryptError::InvalidInput {
            reason: "expected a moov box, a moof box, both, or a root OMA DCF atom file"
                .to_owned(),
        }),
        (_, _, _, true, _) => Err(DecryptError::InvalidInput {
            reason:
                "root OMA DCF atom files are expected to carry odrm without moov or moof at the top level"
                    .to_owned(),
        }),
        (true, false, true, false, None) => Err(DecryptError::InvalidInput {
            reason:
                "non-fragmented movie files are only supported for the current Marlin IPMP, OMA DCF, or IAEC protected layouts"
                    .to_owned(),
        }),
        _ => Err(DecryptError::InvalidInput {
            reason: "input does not match one of the currently supported decrypt layouts"
                .to_owned(),
        }),
    }
}

fn detect_non_fragmented_protected_movie_layout(
    input: &[u8],
) -> Result<Option<DecryptInputLayout>, DecryptError> {
    if contains_oma_dcf_protected_sample_entries(input)? {
        return Ok(Some(DecryptInputLayout::OmaDcfProtectedMovieFile));
    }
    if contains_iaec_protected_sample_entries(input)? {
        return Ok(Some(DecryptInputLayout::IaecProtectedMovieFile));
    }
    Ok(None)
}

fn contains_oma_dcf_protected_sample_entries(input: &[u8]) -> Result<bool, DecryptError> {
    let mut reader = Cursor::new(input);
    let odkm_infos = extract_box(
        &mut reader,
        None,
        BoxPath::from([
            MOOV,
            TRAK,
            MDIA,
            MINF,
            STBL,
            STSD,
            FourCc::ANY,
            SINF,
            SCHI,
            ODKM,
        ]),
    )?;
    if !odkm_infos.is_empty() {
        return Ok(true);
    }

    let mut reader = Cursor::new(input);
    let schm_boxes = extract_box_as::<_, Schm>(
        &mut reader,
        None,
        BoxPath::from([MOOV, TRAK, MDIA, MINF, STBL, STSD, FourCc::ANY, SINF, SCHM]),
    )?;
    Ok(schm_boxes.iter().any(|entry| entry.scheme_type == ODKM))
}

fn contains_iaec_protected_sample_entries(input: &[u8]) -> Result<bool, DecryptError> {
    let mut reader = Cursor::new(input);
    let scheme_boxes = extract_box_as::<_, Schm>(
        &mut reader,
        None,
        BoxPath::from([MOOV, TRAK, MDIA, MINF, STBL, STSD, FourCc::ANY, SINF, SCHM]),
    )?;
    Ok(scheme_boxes.iter().any(|entry| entry.scheme_type == IAEC))
}

fn decrypt_oma_dcf_atom_file_bytes(
    input: &[u8],
    keys: &[DecryptionKey],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let root_boxes = read_root_box_infos(input)?;
    let mut output = Vec::with_capacity(input.len());
    let mut odrm_index = 0_u32;

    for info in root_boxes {
        if info.box_type() != ODRM {
            output.extend_from_slice(slice_box_bytes(input, info)?);
            continue;
        }

        odrm_index = odrm_index
            .checked_add(1)
            .ok_or_else(|| invalid_layout("OMA DCF atom index overflowed u32".to_string()))?;
        let key = keys.iter().find_map(|entry| match entry.id() {
            DecryptionKeyId::TrackId(candidate) if candidate == odrm_index => {
                Some(entry.key_bytes())
            }
            _ => None,
        });

        if let Some(key) = key {
            output.extend_from_slice(&rewrite_oma_dcf_atom_box(input, info, key)?);
        } else {
            output.extend_from_slice(slice_box_bytes(input, info)?);
        }
    }

    Ok(output)
}

fn rewrite_oma_dcf_atom_box(
    input: &[u8],
    odrm_info: BoxInfo,
    key: [u8; 16],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let odrm_info = normalize_oma_dcf_atom_root_info(input, odrm_info)?;
    let mut reader = Cursor::new(input);
    let odhe =
        extract_single_as::<_, Odhe>(&mut reader, Some(&odrm_info), BoxPath::from([ODHE]), "odhe")?;
    let mut reader = Cursor::new(input);
    let odhe_info =
        extract_single_info(&mut reader, Some(&odrm_info), BoxPath::from([ODHE]), "odhe")?;
    let mut reader = Cursor::new(input);
    let ohdr =
        extract_single_as::<_, Ohdr>(&mut reader, Some(&odhe_info), BoxPath::from([OHDR]), "ohdr")?;
    let mut reader = Cursor::new(input);
    let ohdr_info =
        extract_single_info(&mut reader, Some(&odhe_info), BoxPath::from([OHDR]), "ohdr")?;
    let mut reader = Cursor::new(input);
    let odda =
        extract_single_as::<_, Odda>(&mut reader, Some(&odrm_info), BoxPath::from([ODDA]), "odda")?;
    let odda_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(&mut reader, Some(&odrm_info), BoxPath::from([ODDA]), "odda")?
    };
    let grpi = {
        let mut reader = Cursor::new(input);
        extract_optional_single_as::<_, Grpi>(
            &mut reader,
            Some(&ohdr_info),
            BoxPath::from([GRPI]),
            "grpi",
        )?
    };

    if ohdr.encryption_method == OHDR_ENCRYPTION_METHOD_NULL {
        return Ok(slice_box_bytes(input, odrm_info)?.to_vec());
    }

    let content_key = unwrap_oma_dcf_group_key(&ohdr, grpi.as_ref(), key)?;
    let clear_payload = decrypt_oma_dcf_atom_payload(&ohdr, &odda, content_key)?;
    let mut patched_ohdr = ohdr.clone();
    patched_ohdr.encryption_method = OHDR_ENCRYPTION_METHOD_NULL;
    patched_ohdr.padding_scheme = OHDR_PADDING_SCHEME_NONE;

    let mut patched_odda = odda.clone();
    patched_odda.encrypted_payload = clear_payload;

    let rebuilt_odhe = rebuild_oma_dcf_odhe(input, odhe, odhe_info, patched_ohdr, ohdr_info)?;
    let rebuilt_odda =
        encode_box_with_children_and_header_size(&patched_odda, &[], odda_info.header_size())?;

    let mut reader = Cursor::new(input);
    let child_infos = extract_box(&mut reader, Some(&odrm_info), BoxPath::from([FourCc::ANY]))?;
    let mut odrm_children = Vec::new();
    for child_info in child_infos {
        match child_info.box_type() {
            ODHE => odrm_children.extend_from_slice(&rebuilt_odhe),
            ODDA => odrm_children.extend_from_slice(&rebuilt_odda),
            _ => odrm_children.extend_from_slice(slice_box_bytes(input, child_info)?),
        }
    }

    rebuild_oma_dcf_odrm(input, odrm_info, &odrm_children)
}

fn rebuild_oma_dcf_odhe(
    input: &[u8],
    odhe: Odhe,
    odhe_info: BoxInfo,
    patched_ohdr: Ohdr,
    ohdr_info: BoxInfo,
) -> Result<Vec<u8>, DecryptRewriteError> {
    let rebuilt_ohdr = rebuild_oma_dcf_ohdr(input, patched_ohdr, ohdr_info)?;
    let mut reader = Cursor::new(input);
    let child_infos = extract_box(&mut reader, Some(&odhe_info), BoxPath::from([FourCc::ANY]))?;
    let mut odhe_children = Vec::new();
    for child_info in child_infos {
        match child_info.box_type() {
            OHDR => odhe_children.extend_from_slice(&rebuilt_ohdr),
            _ => odhe_children.extend_from_slice(slice_box_bytes(input, child_info)?),
        }
    }
    encode_box_with_children(&odhe, &odhe_children)
}

fn rebuild_oma_dcf_ohdr(
    input: &[u8],
    ohdr: Ohdr,
    ohdr_info: BoxInfo,
) -> Result<Vec<u8>, DecryptRewriteError> {
    let mut reader = Cursor::new(input);
    let child_infos = extract_box(&mut reader, Some(&ohdr_info), BoxPath::from([FourCc::ANY]))?;
    let mut ohdr_children = Vec::new();
    for child_info in child_infos {
        ohdr_children.extend_from_slice(slice_box_bytes(input, child_info)?);
    }
    encode_box_with_children(&ohdr, &ohdr_children)
}

fn normalize_oma_dcf_atom_root_info(
    input: &[u8],
    odrm_info: BoxInfo,
) -> Result<BoxInfo, DecryptRewriteError> {
    let generic_header_size = raw_header_size(input, odrm_info)?;
    let header_size = if generic_header_size == 16 {
        let version_flags_offset = odrm_info
            .offset()
            .checked_add(generic_header_size)
            .ok_or_else(|| {
                invalid_layout("OMA DCF atom root header offset overflowed u64".to_owned())
            })?;
        let child_header_offset = version_flags_offset.checked_add(4).ok_or_else(|| {
            invalid_layout("OMA DCF atom root child offset overflowed u64".to_owned())
        })?;
        let version_flags_offset = usize::try_from(version_flags_offset).map_err(|_| {
            invalid_layout("OMA DCF atom root header offset does not fit in usize".to_owned())
        })?;
        let child_header_offset = usize::try_from(child_header_offset).map_err(|_| {
            invalid_layout("OMA DCF atom root child offset does not fit in usize".to_owned())
        })?;
        let has_full_box_prefix = input
            .get(version_flags_offset..version_flags_offset + 4)
            .is_some_and(|prefix| prefix == [0, 0, 0, 0])
            && input
                .get(child_header_offset + 4..child_header_offset + 8)
                .is_some_and(|box_type| box_type == ODHE.as_bytes());
        if has_full_box_prefix {
            20
        } else {
            generic_header_size
        }
    } else {
        generic_header_size
    };

    Ok(odrm_info.with_header_size(header_size))
}

fn rebuild_oma_dcf_odrm(
    input: &[u8],
    odrm_info: BoxInfo,
    children: &[u8],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let generic_header_size = raw_header_size(input, odrm_info)?;
    let generic_header_size = usize::try_from(generic_header_size).map_err(|_| {
        invalid_layout("OMA DCF atom root header size does not fit in usize".to_owned())
    })?;
    let full_header_size = usize::try_from(odrm_info.header_size()).map_err(|_| {
        invalid_layout("OMA DCF atom root normalized header size does not fit in usize".to_owned())
    })?;
    let header_extra = input
        .get(
            usize::try_from(odrm_info.offset()).map_err(|_| {
                invalid_layout("OMA DCF atom root offset does not fit in usize".to_owned())
            })? + generic_header_size
                ..usize::try_from(odrm_info.offset()).map_err(|_| {
                    invalid_layout("OMA DCF atom root offset does not fit in usize".to_owned())
                })? + full_header_size,
        )
        .ok_or_else(|| {
            invalid_layout("OMA DCF atom root header bytes are outside the input buffer".to_owned())
        })?;
    let mut payload = Vec::with_capacity(header_extra.len() + children.len());
    payload.extend_from_slice(header_extra);
    payload.extend_from_slice(children);
    encode_raw_box_with_header_size(
        ODRM,
        &payload,
        u64::try_from(generic_header_size).unwrap_or(8),
    )
}

fn decrypt_oma_dcf_atom_payload(
    ohdr: &Ohdr,
    odda: &Odda,
    key: [u8; 16],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let plaintext_length = usize::try_from(ohdr.plaintext_length)
        .map_err(|_| invalid_layout("OMA DCF plaintext length does not fit in usize".to_owned()))?;

    match ohdr.encryption_method {
        OHDR_ENCRYPTION_METHOD_NULL => Ok(odda.encrypted_payload.clone()),
        OHDR_ENCRYPTION_METHOD_AES_CBC => {
            if ohdr.padding_scheme != OHDR_PADDING_SCHEME_RFC_2630 {
                return Err(invalid_layout(
                    "OMA DCF AES-CBC atom payloads require RFC 2630 padding".to_owned(),
                ));
            }
            decrypt_oma_dcf_cbc_payload(&odda.encrypted_payload, key, plaintext_length)
        }
        OHDR_ENCRYPTION_METHOD_AES_CTR => {
            if ohdr.padding_scheme != OHDR_PADDING_SCHEME_NONE {
                return Err(invalid_layout(
                    "OMA DCF AES-CTR atom payloads require no padding".to_owned(),
                ));
            }
            decrypt_oma_dcf_ctr_payload(&odda.encrypted_payload, key, plaintext_length)
        }
        other => Err(invalid_layout(format!(
            "OMA DCF atom payload uses unsupported encryption method {other}"
        ))),
    }
}

fn unwrap_oma_dcf_group_key(
    ohdr: &Ohdr,
    grpi: Option<&Grpi>,
    key: [u8; 16],
) -> Result<[u8; 16], DecryptRewriteError> {
    let Some(grpi) = grpi else {
        return Ok(key);
    };

    if grpi.group_key.len() < 32 {
        return Err(invalid_layout(
            "OMA DCF group-key-wrapped content keys must include a 16-byte IV plus wrapped key bytes"
                .to_owned(),
        ));
    }

    let unwrapped = match ohdr.encryption_method {
        OHDR_ENCRYPTION_METHOD_AES_CBC => decrypt_oma_dcf_cbc_payload(&grpi.group_key, key, 16)?,
        OHDR_ENCRYPTION_METHOD_AES_CTR => decrypt_oma_dcf_ctr_payload(&grpi.group_key, key, 16)?,
        OHDR_ENCRYPTION_METHOD_NULL => return Ok(key),
        other => {
            return Err(invalid_layout(format!(
                "OMA DCF group-key unwrap uses unsupported encryption method {other}"
            )));
        }
    };

    unwrapped.try_into().map_err(|_| {
        invalid_layout("OMA DCF group-key unwrap did not yield one 16-byte content key".to_owned())
    })
}

fn decrypt_oma_dcf_cbc_payload(
    payload: &[u8],
    key: [u8; 16],
    plaintext_length: usize,
) -> Result<Vec<u8>, DecryptRewriteError> {
    if payload.len() < 32 || !payload.len().is_multiple_of(16) {
        return Err(invalid_layout(
            "OMA DCF AES-CBC atom payload must include a 16-byte IV plus block-aligned ciphertext"
                .to_owned(),
        ));
    }

    let iv = &payload[..16];
    let ciphertext = &payload[16..];
    let cipher = Aes128::new(&key.into());
    let mut previous = [0_u8; 16];
    previous.copy_from_slice(iv);
    let mut decrypted = Vec::with_capacity(ciphertext.len());

    for chunk in ciphertext.chunks_exact(16) {
        let mut block = Block::<Aes128>::default();
        block.copy_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        for (index, value) in block.iter_mut().enumerate() {
            *value ^= previous[index];
        }
        decrypted.extend_from_slice(&block);
        previous.copy_from_slice(chunk);
    }

    let unpadded = remove_rfc_2630_padding(&decrypted)?;
    if unpadded.len() != plaintext_length {
        return Err(invalid_layout(format!(
            "OMA DCF AES-CBC plaintext length mismatch: header declared {plaintext_length} bytes but decrypted {}",
            unpadded.len()
        )));
    }
    Ok(unpadded)
}

fn decrypt_oma_dcf_ctr_payload(
    payload: &[u8],
    key: [u8; 16],
    plaintext_length: usize,
) -> Result<Vec<u8>, DecryptRewriteError> {
    if payload.len() < 16 {
        return Err(invalid_layout(
            "OMA DCF AES-CTR atom payload must include a 16-byte IV".to_owned(),
        ));
    }

    let mut counter = [0_u8; 16];
    counter.copy_from_slice(&payload[..16]);
    let ciphertext = &payload[16..];
    let cipher = Aes128::new(&key.into());
    let mut output = vec![0_u8; ciphertext.len()];

    for (index, chunk) in ciphertext.chunks(16).enumerate() {
        let mut keystream = Block::<Aes128>::default();
        keystream.copy_from_slice(&counter);
        cipher.encrypt_block(&mut keystream);
        let start = index * 16;
        for (offset, byte) in chunk.iter().enumerate() {
            output[start + offset] = byte ^ keystream[offset];
        }
        increment_counter_be(&mut counter);
    }

    if output.len() != plaintext_length {
        return Err(invalid_layout(format!(
            "OMA DCF AES-CTR plaintext length mismatch: header declared {plaintext_length} bytes but decrypted {}",
            output.len()
        )));
    }
    Ok(output)
}

fn remove_rfc_2630_padding(bytes: &[u8]) -> Result<Vec<u8>, DecryptRewriteError> {
    let Some(&padding_size) = bytes.last() else {
        return Err(invalid_layout(
            "OMA DCF AES-CBC payload cannot be empty after decryption".to_owned(),
        ));
    };
    let padding_size = usize::from(padding_size);
    if padding_size == 0 || padding_size > 16 || padding_size > bytes.len() {
        return Err(invalid_layout(
            "OMA DCF AES-CBC payload has invalid RFC 2630 padding".to_owned(),
        ));
    }
    if !bytes[bytes.len() - padding_size..]
        .iter()
        .all(|byte| usize::from(*byte) == padding_size)
    {
        return Err(invalid_layout(
            "OMA DCF AES-CBC payload has inconsistent RFC 2630 padding bytes".to_owned(),
        ));
    }
    Ok(bytes[..bytes.len() - padding_size].to_vec())
}

fn increment_counter_be(counter: &mut [u8; 16]) {
    for byte in counter.iter_mut().rev() {
        let (value, carry) = byte.overflowing_add(1);
        *byte = value;
        if !carry {
            break;
        }
    }
}

fn read_root_box_infos(input: &[u8]) -> Result<Vec<BoxInfo>, DecryptRewriteError> {
    let mut reader = Cursor::new(input);
    let mut root_boxes = Vec::new();
    loop {
        let position = reader.stream_position().map_err(|error| {
            invalid_layout(format!("failed to read root-box position: {error}"))
        })?;
        if usize::try_from(position)
            .ok()
            .is_some_and(|offset| offset >= input.len())
        {
            break;
        }

        let info = BoxInfo::read(&mut reader)
            .map_err(|error| invalid_layout(format!("failed to read root box header: {error}")))?;
        info.seek_to_end(&mut reader)
            .map_err(|error| invalid_layout(format!("failed to skip past root box: {error}")))?;
        root_boxes.push(info);
    }
    Ok(root_boxes)
}

fn slice_box_bytes(input: &[u8], info: BoxInfo) -> Result<&[u8], DecryptRewriteError> {
    let start = usize::try_from(info.offset())
        .map_err(|_| invalid_layout("box offset does not fit in usize".to_owned()))?;
    let end = usize::try_from(info.offset() + info.size())
        .map_err(|_| invalid_layout("box end does not fit in usize".to_owned()))?;
    input.get(start..end).ok_or_else(|| {
        invalid_layout(format!(
            "box bytes for {} are outside the available input buffer",
            info.box_type()
        ))
    })
}

fn encode_raw_box(box_type: FourCc, payload: &[u8]) -> Result<Vec<u8>, DecryptRewriteError> {
    encode_raw_box_with_header_size(box_type, payload, 8)
}

fn encode_raw_box_with_header_size(
    box_type: FourCc,
    payload: &[u8],
    header_size: u64,
) -> Result<Vec<u8>, DecryptRewriteError> {
    let size = header_size
        .checked_add(u64::try_from(payload.len()).map_err(|_| {
            invalid_layout("encoded box payload length does not fit in u64".to_owned())
        })?)
        .ok_or_else(|| invalid_layout("encoded box size overflowed u64".to_owned()))?;
    let info = BoxInfo::new(box_type, size).with_header_size(header_size);
    let mut bytes = info.encode();
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

fn encode_box_with_children<T>(
    box_value: &T,
    children: &[u8],
) -> Result<Vec<u8>, DecryptRewriteError>
where
    T: crate::codec::CodecBox + ImmutableBox,
{
    encode_box_with_children_and_header_size(box_value, children, 8)
}

fn encode_box_with_children_and_header_size<T>(
    box_value: &T,
    children: &[u8],
    header_size: u64,
) -> Result<Vec<u8>, DecryptRewriteError>
where
    T: crate::codec::CodecBox + ImmutableBox,
{
    let mut payload = Vec::new();
    marshal(&mut payload, box_value, None).map_err(|error| {
        invalid_layout(format!(
            "failed to encode {} payload: {error}",
            box_value.box_type()
        ))
    })?;
    payload.extend_from_slice(children);
    encode_raw_box_with_header_size(box_value.box_type(), &payload, header_size)
}

fn raw_header_size(input: &[u8], info: BoxInfo) -> Result<u64, DecryptRewriteError> {
    let offset = usize::try_from(info.offset())
        .map_err(|_| invalid_layout("box offset does not fit in usize".to_owned()))?;
    let size_field = input.get(offset..offset + 4).ok_or_else(|| {
        invalid_layout("box header bytes are outside the input buffer".to_owned())
    })?;
    let size_field = u32::from_be_bytes(size_field.try_into().unwrap());
    Ok(if size_field == 1 { 16 } else { 8 })
}

fn decrypt_marlin_movie_file_bytes(
    input: &[u8],
    keys: &[DecryptionKey],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let context = analyze_marlin_movie_file(input)?;
    let root_boxes = read_root_box_infos(input)?;
    let mdat_ranges = media_data_ranges_from_infos(&context.mdat_infos);
    let mut track_keys = BTreeMap::new();
    for track in &context.tracks {
        let Some(protection) = track.marlin.as_ref() else {
            continue;
        };
        if let Some(track_key) = resolve_marlin_track_key(track.track_id, protection, keys)? {
            track_keys.insert(track.track_id, track_key);
        }
    }

    let payload_tracks = context
        .tracks
        .iter()
        .map(|track| MovieTrackPayloadPlan {
            track_id: track.track_id,
            stsc: &track.stsc,
            chunk_offsets: &track.chunk_offsets,
            sample_sizes: &track.sample_sizes,
        })
        .collect::<Vec<_>>();
    let (clear_payload, clear_sizes_by_track, relative_chunk_offsets) = rebuild_movie_payload(
        input,
        &mdat_ranges,
        &payload_tracks,
        |track_id, _sample_index, _absolute_offset, _sample_size, sample_bytes| {
            if let Some(key) = track_keys.get(&track_id).copied() {
                decrypt_marlin_sample_payload(sample_bytes, key)
            } else {
                Ok(sample_bytes.to_vec())
            }
        },
    )?;

    let mut track_plans = Vec::new();
    for track in &context.tracks {
        let clear_sizes = clear_sizes_by_track.get(&track.track_id).ok_or_else(|| {
            invalid_layout(format!(
                "missing clear sample sizes for Marlin track {}",
                track.track_id
            ))
        })?;
        track_plans.push(MovieTrackRewritePlan {
            track_id: track.track_id,
            trak_info: track.trak_info,
            mdia_info: track.mdia_info,
            minf_info: track.minf_info,
            stbl_info: track.stbl_info,
            chunk_offsets: track.chunk_offsets.clone(),
            stsd_replacement: None,
            stsz_replacement: Some((
                track.stsz_info.offset(),
                build_patched_stsz_bytes(&track.stsz, clear_sizes, "Marlin")?,
            )),
        });
    }

    let placeholder_offsets = track_plans
        .iter()
        .map(|plan| (plan.track_id, chunk_offsets_values(&plan.chunk_offsets)))
        .collect::<TrackRelativeChunkOffsets>();
    let moov_placeholder = build_marlin_moov_with_track_replacements(
        input,
        &context,
        &track_plans,
        &placeholder_offsets,
    )?;
    let clear_mdat = encode_raw_box(MDAT, &clear_payload)?;
    let clear_mdat_header_size =
        u64::try_from(clear_mdat.len().saturating_sub(clear_payload.len())).map_err(|_| {
            invalid_layout("clear Marlin mdat header size does not fit in u64".to_owned())
        })?;
    let mdat_payload_start = compute_single_mdat_payload_offset(
        input,
        &root_boxes,
        Some(context.ftyp_info),
        context.moov_info,
        Some(&encode_box_with_children(
            &build_clear_marlin_ftyp(&context.ftyp),
            &[],
        )?),
        &moov_placeholder,
        clear_mdat_header_size,
    )?;

    let absolute_offsets = relative_chunk_offsets
        .iter()
        .map(|(track_id, offsets)| {
            let absolute = offsets
                .iter()
                .map(|offset| {
                    mdat_payload_start.checked_add(*offset).ok_or_else(|| {
                        invalid_layout("clear Marlin chunk offset overflowed u64".to_owned())
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((*track_id, absolute))
        })
        .collect::<Result<TrackRelativeChunkOffsets, DecryptRewriteError>>()?;
    let clear_moov = build_marlin_moov_with_track_replacements(
        input,
        &context,
        &track_plans,
        &absolute_offsets,
    )?;
    let clear_ftyp = encode_box_with_children(&build_clear_marlin_ftyp(&context.ftyp), &[])?;

    rebuild_root_boxes_with_single_mdat(
        input,
        &root_boxes,
        Some(context.ftyp_info),
        context.moov_info,
        Some(&clear_ftyp),
        &clear_moov,
        &clear_mdat,
    )
}

fn build_clear_marlin_ftyp(ftyp: &Ftyp) -> Ftyp {
    let mp42 = FourCc::from_bytes(*b"mp42");
    let mut clear = ftyp.clone();
    clear.major_brand = mp42;
    clear.minor_version = 1;
    for brand in &mut clear.compatible_brands {
        if *brand == MARLIN_BRAND_MGSV {
            *brand = mp42;
        }
    }
    clear
}

fn build_marlin_moov_with_track_replacements(
    input: &[u8],
    context: &MarlinMovieContext,
    track_plans: &[MovieTrackRewritePlan],
    chunk_offsets_by_track: &TrackRelativeChunkOffsets,
) -> Result<Vec<u8>, DecryptRewriteError> {
    let mut moov_replacements = BTreeMap::from([
        (context.iods_info.offset(), None),
        (context.od_track_info.offset(), None),
    ]);
    for plan in track_plans {
        let new_offsets = chunk_offsets_by_track
            .get(&plan.track_id)
            .cloned()
            .ok_or_else(|| {
                invalid_layout(format!(
                    "missing rewritten chunk offsets for Marlin track {}",
                    plan.track_id
                ))
            })?;
        let mut stbl_replacements = BTreeMap::new();
        stbl_replacements.insert(
            chunk_offset_box_offset(&plan.chunk_offsets),
            Some(build_patched_chunk_offset_box_bytes(
                &plan.chunk_offsets,
                &new_offsets,
            )?),
        );
        if let Some((offset, bytes)) = &plan.stsz_replacement {
            stbl_replacements.insert(*offset, Some(bytes.clone()));
        }
        let trak_bytes = rebuild_track_with_stbl_replacements(
            input,
            plan.trak_info,
            plan.mdia_info,
            plan.minf_info,
            plan.stbl_info,
            &stbl_replacements,
        )?;
        moov_replacements.insert(plan.trak_info.offset(), Some(trak_bytes));
    }
    rebuild_box_with_child_replacements(input, context.moov_info, &moov_replacements, None)
}

fn analyze_marlin_movie_file(input: &[u8]) -> Result<MarlinMovieContext, DecryptRewriteError> {
    let root_boxes = read_root_box_infos(input)?;
    let ftyp_info = root_boxes
        .iter()
        .copied()
        .find(|info| info.box_type() == FTYP)
        .ok_or_else(|| {
            invalid_layout("expected one root ftyp box in the Marlin movie file".to_owned())
        })?;
    let moov_info = root_boxes
        .iter()
        .copied()
        .find(|info| info.box_type() == MOOV)
        .ok_or_else(|| {
            invalid_layout("expected one root moov box in the Marlin movie file".to_owned())
        })?;
    let mdat_infos = root_boxes
        .iter()
        .copied()
        .filter(|info| info.box_type() == MDAT)
        .collect::<Vec<_>>();
    if mdat_infos.is_empty() {
        return Err(invalid_layout(
            "expected at least one root mdat box in the Marlin movie file".to_owned(),
        ));
    }

    let mut reader = Cursor::new(input);
    let ftyp = extract_single_as::<_, Ftyp>(&mut reader, None, BoxPath::from([FTYP]), "ftyp")?;
    if ftyp.major_brand != MARLIN_BRAND_MGSV && !ftyp.compatible_brands.contains(&MARLIN_BRAND_MGSV)
    {
        return Err(invalid_layout(
            "the current Marlin movie path expects the MGSV file-type brand".to_owned(),
        ));
    }

    let iods_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(&mut reader, None, BoxPath::from([MOOV, IODS]), "iods")?
    };
    let iods = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Iods>(&mut reader, None, BoxPath::from([MOOV, IODS]), "iods")?
    };
    let initial_object_descriptor = iods.initial_object_descriptor().ok_or_else(|| {
        invalid_layout(
            "the current Marlin movie path expects one initial object descriptor in iods"
                .to_owned(),
        )
    })?;
    let od_track_id = initial_object_descriptor
        .sub_descriptors
        .iter()
        .find_map(|descriptor| descriptor.es_id_inc_descriptor())
        .map(|descriptor| descriptor.track_id)
        .ok_or_else(|| {
            invalid_layout(
                "the current Marlin movie path expects iods to carry one ES-ID-increment descriptor"
                    .to_owned(),
            )
        })?;

    let mut reader = Cursor::new(input);
    let trak_infos = extract_box(&mut reader, None, BoxPath::from([MOOV, TRAK]))?;
    let mut od_track_info = None;
    for trak_info in &trak_infos {
        let mut reader = Cursor::new(input);
        let tkhd = extract_single_as::<_, Tkhd>(
            &mut reader,
            Some(trak_info),
            BoxPath::from([TKHD]),
            "trak/tkhd",
        )?;
        if tkhd.track_id == od_track_id {
            od_track_info = Some(*trak_info);
            break;
        }
    }
    let od_track_info = od_track_info.ok_or_else(|| {
        invalid_layout(format!(
            "expected one Marlin object-descriptor track with track id {od_track_id}"
        ))
    })?;

    let mdat_ranges = media_data_ranges_from_infos(&mdat_infos);
    let marlin_tracks = analyze_marlin_od_track(input, &od_track_info, &mdat_ranges)?;
    if marlin_tracks.is_empty() {
        return Err(invalid_layout(
            "the current Marlin movie path found no carried track protection entries in the OD track"
                .to_owned(),
        ));
    }

    let mut tracks = Vec::new();
    for trak_info in trak_infos {
        if trak_info.offset() == od_track_info.offset() {
            continue;
        }
        tracks.push(analyze_marlin_movie_track(
            input,
            &trak_info,
            &marlin_tracks,
        )?);
    }

    Ok(MarlinMovieContext {
        ftyp_info,
        ftyp,
        moov_info,
        iods_info,
        od_track_info,
        mdat_infos,
        tracks,
    })
}

fn analyze_marlin_od_track(
    input: &[u8],
    od_track_info: &BoxInfo,
    mdat_ranges: &[MediaDataRange],
) -> Result<BTreeMap<u32, MarlinTrackProtection>, DecryptRewriteError> {
    let od_track_id = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Tkhd>(
            &mut reader,
            Some(od_track_info),
            BoxPath::from([TKHD]),
            "trak/tkhd",
        )?
        .track_id
    };
    let mpod = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Mpod>(
            &mut reader,
            Some(od_track_info),
            BoxPath::from([FourCc::from_bytes(*b"tref"), FourCc::from_bytes(*b"mpod")]),
            "mpod",
        )?
    };
    if mpod.track_ids.is_empty() {
        return Err(invalid_layout(
            "the current Marlin OD track expects one or more mpod track references".to_owned(),
        ));
    }

    let stsz = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Stsz>(
            &mut reader,
            Some(od_track_info),
            BoxPath::from([MDIA, MINF, STBL, STSZ]),
            "stsz",
        )?
    };
    let od_sample_sizes = sample_sizes_from_stsz(&stsz)?;
    if od_sample_sizes.is_empty() {
        return Err(invalid_layout(format!(
            "the current Marlin OD track path expects at least one OD sample but found {}",
            od_sample_sizes.len()
        )));
    }

    let stsc = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Stsc>(
            &mut reader,
            Some(od_track_info),
            BoxPath::from([MDIA, MINF, STBL, STSC]),
            "stsc",
        )?
    };
    let chunk_offsets = {
        let mut reader = Cursor::new(input);
        let stco = extract_optional_single_as::<_, Stco>(
            &mut reader,
            Some(od_track_info),
            BoxPath::from([MDIA, MINF, STBL, STCO]),
            "stco",
        )?;
        let mut reader = Cursor::new(input);
        let co64 = extract_optional_single_as::<_, Co64>(
            &mut reader,
            Some(od_track_info),
            BoxPath::from([MDIA, MINF, STBL, FourCc::from_bytes(*b"co64")]),
            "co64",
        )?;
        let mut reader = Cursor::new(input);
        let stco_info = extract_box(
            &mut reader,
            Some(od_track_info),
            BoxPath::from([MDIA, MINF, STBL, STCO]),
        )?;
        let mut reader = Cursor::new(input);
        let co64_info = extract_box(
            &mut reader,
            Some(od_track_info),
            BoxPath::from([MDIA, MINF, STBL, FourCc::from_bytes(*b"co64")]),
        )?;
        match (stco, co64) {
            (Some(_), Some(_)) => {
                return Err(invalid_layout(
                    "the current Marlin OD track path does not support both stco and co64"
                        .to_owned(),
                ));
            }
            (Some(stco), None) => {
                let [info] = stco_info.as_slice() else {
                    return Err(invalid_layout(format!(
                        "expected exactly one stco box for the Marlin OD track but found {}",
                        stco_info.len()
                    )));
                };
                ChunkOffsetBoxState::Stco {
                    info: *info,
                    box_value: stco,
                }
            }
            (None, Some(co64)) => {
                let [info] = co64_info.as_slice() else {
                    return Err(invalid_layout(format!(
                        "expected exactly one co64 box for the Marlin OD track but found {}",
                        co64_info.len()
                    )));
                };
                ChunkOffsetBoxState::Co64 {
                    info: *info,
                    box_value: co64,
                }
            }
            (None, None) => {
                return Err(invalid_layout(
                    "the current Marlin OD track path expects stco or co64".to_owned(),
                ));
            }
        }
    };
    let od_chunks = compute_track_chunks(od_track_id, &stsc, &chunk_offsets, &od_sample_sizes)?;
    let (sample_offset, sample_size) = od_chunks
        .iter()
        .find_map(|chunk| chunk.sample_sizes.first().map(|size| (chunk.offset, *size)))
        .ok_or_else(|| {
            invalid_layout(
                "the current Marlin OD track path could not resolve the first OD sample".to_owned(),
            )
        })?;

    let sample_bytes = read_sample_range(input, mdat_ranges, sample_offset, sample_size).ok_or(
        DecryptRewriteError::SampleDataRangeNotFound {
            track_id: od_track_id,
            sample_index: 1,
            absolute_offset: sample_offset,
            sample_size,
        },
    )?;
    let commands = parse_descriptor_commands(sample_bytes).map_err(|error| {
        invalid_layout(format!(
            "failed to parse Marlin OD track command stream: {error}"
        ))
    })?;
    let object_update = commands
        .iter()
        .find_map(|command| match command {
            DescriptorCommand::DescriptorUpdate(update) if update.tag == 0x01 => Some(update),
            _ => None,
        })
        .ok_or_else(|| {
            invalid_layout(
                "the current Marlin OD track path expects one object-descriptor-update command"
                    .to_owned(),
            )
        })?;
    let ipmp_update = commands
        .iter()
        .find_map(|command| match command {
            DescriptorCommand::DescriptorUpdate(update) if update.tag == 0x05 => Some(update),
            _ => None,
        })
        .ok_or_else(|| {
            invalid_layout(
                "the current Marlin OD track path expects one IPMP-descriptor-update command"
                    .to_owned(),
            )
        })?;

    let mut tracks = BTreeMap::new();
    for descriptor in &object_update.descriptors {
        let Some(object_descriptor) = descriptor.object_descriptor() else {
            continue;
        };
        let Some(es_id_ref) = object_descriptor
            .sub_descriptors
            .iter()
            .find_map(|descriptor| descriptor.es_id_ref_descriptor())
        else {
            continue;
        };
        let ref_index = usize::from(es_id_ref.ref_index);
        if ref_index == 0 || ref_index > mpod.track_ids.len() {
            continue;
        }
        let track_id = mpod.track_ids[ref_index - 1];
        let Some(pointer) = object_descriptor
            .sub_descriptors
            .iter()
            .find_map(|descriptor| descriptor.ipmp_descriptor_pointer())
        else {
            continue;
        };
        let Some(ipmp_descriptor) = ipmp_update.descriptors.iter().find_map(|descriptor| {
            let ipmp_descriptor = descriptor.ipmp_descriptor()?;
            (ipmp_descriptor.ipmps_type == MARLIN_IPMPS_TYPE_MGSV
                && ipmp_descriptor.descriptor_id == pointer.descriptor_id)
                .then_some(ipmp_descriptor)
        }) else {
            continue;
        };
        let Some(protection) = parse_marlin_track_protection(&ipmp_descriptor.data)? else {
            continue;
        };
        tracks.insert(track_id, protection);
    }

    Ok(tracks)
}

fn parse_marlin_track_protection(
    bytes: &[u8],
) -> Result<Option<MarlinTrackProtection>, DecryptRewriteError> {
    let carried_atoms = read_root_box_infos(bytes)?;
    for atom_info in carried_atoms {
        if atom_info.box_type() != SINF {
            continue;
        }
        let atom_bytes = slice_box_bytes(bytes, atom_info)?;
        if let Some(protection) = parse_marlin_sinf(atom_bytes)? {
            return Ok(Some(protection));
        }
    }
    Ok(None)
}

fn parse_marlin_sinf(bytes: &[u8]) -> Result<Option<MarlinTrackProtection>, DecryptRewriteError> {
    let payload = bytes.get(8..).ok_or_else(|| {
        invalid_layout("Marlin sinf bytes are shorter than their box header".to_owned())
    })?;
    let child_infos = read_root_box_infos(payload)?;
    let satr_type = FourCc::from_bytes(*b"satr");
    let styp_type = FourCc::from_bytes(*b"styp");

    let mut scheme = None;
    let mut stream_type = None;
    let mut wrapped_group_key = None;
    for child_info in child_infos {
        match child_info.box_type() {
            SCHM => {
                let child_bytes = slice_box_bytes(payload, child_info)?;
                let versioned_payload = child_bytes
                    .get(usize::try_from(child_info.header_size()).unwrap_or(8)..)
                    .ok_or_else(|| {
                        invalid_layout("Marlin schm atom is shorter than expected".to_owned())
                    })?;
                let short_payload = versioned_payload.get(4..).ok_or_else(|| {
                    invalid_layout("Marlin schm atom is missing its short-form payload".to_owned())
                })?;
                scheme = Some(
                    MarlinShortSchm::parse_payload(short_payload).map_err(|error| {
                        invalid_layout(format!(
                            "failed to parse Marlin short-form schm payload: {error}"
                        ))
                    })?,
                );
            }
            SCHI => {
                let schi_bytes = slice_box_bytes(payload, child_info)?;
                let schi_payload = schi_bytes
                    .get(usize::try_from(child_info.header_size()).unwrap_or(8)..)
                    .ok_or_else(|| {
                        invalid_layout("Marlin schi atom is shorter than expected".to_owned())
                    })?;
                let schi_children = read_root_box_infos(schi_payload)?;
                for schi_child in schi_children {
                    match schi_child.box_type() {
                        GKEY => {
                            let gkey_bytes = slice_box_bytes(schi_payload, schi_child)?;
                            let gkey_payload = gkey_bytes
                                .get(usize::try_from(schi_child.header_size()).unwrap_or(8)..)
                                .ok_or_else(|| {
                                    invalid_layout(
                                        "Marlin gkey atom is shorter than expected".to_owned(),
                                    )
                                })?;
                            wrapped_group_key = Some(gkey_payload.to_vec());
                        }
                        box_type if box_type == satr_type => {
                            let satr_bytes = slice_box_bytes(schi_payload, schi_child)?;
                            let satr_payload = satr_bytes
                                .get(usize::try_from(schi_child.header_size()).unwrap_or(8)..)
                                .ok_or_else(|| {
                                    invalid_layout(
                                        "Marlin satr atom is shorter than expected".to_owned(),
                                    )
                                })?;
                            let satr_children = read_root_box_infos(satr_payload)?;
                            for satr_child in satr_children {
                                if satr_child.box_type() != styp_type {
                                    continue;
                                }
                                let styp_bytes = slice_box_bytes(satr_payload, satr_child)?;
                                let styp_payload = styp_bytes
                                    .get(usize::try_from(satr_child.header_size()).unwrap_or(8)..)
                                    .ok_or_else(|| {
                                        invalid_layout(
                                            "Marlin styp atom is shorter than expected".to_owned(),
                                        )
                                    })?;
                                stream_type = Some(
                                    MarlinStyp::parse_payload(styp_payload)
                                        .map_err(|error| {
                                            invalid_layout(format!(
                                                "failed to parse Marlin styp payload: {error}"
                                            ))
                                        })?
                                        .value,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let Some(scheme) = scheme else {
        return Ok(None);
    };
    let key_mode = if scheme.uses_track_key() {
        MarlinTrackKeyMode::Track
    } else if scheme.uses_group_key() {
        MarlinTrackKeyMode::Group
    } else {
        return Ok(None);
    };

    Ok(Some(MarlinTrackProtection {
        key_mode,
        stream_type,
        wrapped_group_key,
    }))
}

fn analyze_marlin_movie_track(
    input: &[u8],
    trak_info: &BoxInfo,
    marlin_tracks: &BTreeMap<u32, MarlinTrackProtection>,
) -> Result<MarlinMovieTrackState, DecryptRewriteError> {
    let mut reader = Cursor::new(input);
    let tkhd = extract_single_as::<_, Tkhd>(
        &mut reader,
        Some(trak_info),
        BoxPath::from([TKHD]),
        "trak/tkhd",
    )?;
    let mdia_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(&mut reader, Some(trak_info), BoxPath::from([MDIA]), "mdia")?
    };
    let minf_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF]),
            "minf",
        )?
    };
    let stbl_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL]),
            "stbl",
        )?
    };

    let stsz = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Stsz>(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, STSZ]),
            "stsz",
        )?
    };
    let stsz_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, STSZ]),
            "stsz",
        )?
    };
    let sample_sizes = sample_sizes_from_stsz(&stsz)?;

    let stsc = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Stsc>(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, STSC]),
            "stsc",
        )?
    };
    let chunk_offsets = {
        let mut reader = Cursor::new(input);
        let stco = extract_optional_single_as::<_, Stco>(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, STCO]),
            "stco",
        )?;
        let mut reader = Cursor::new(input);
        let co64 = extract_optional_single_as::<_, Co64>(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, FourCc::from_bytes(*b"co64")]),
            "co64",
        )?;
        let mut reader = Cursor::new(input);
        let stco_info = extract_box(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, STCO]),
        )?;
        let mut reader = Cursor::new(input);
        let co64_info = extract_box(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, FourCc::from_bytes(*b"co64")]),
        )?;
        match (stco, co64) {
            (Some(_), Some(_)) => {
                return Err(invalid_layout(format!(
                    "track {} has both stco and co64 chunk-offset boxes",
                    tkhd.track_id
                )));
            }
            (Some(stco), None) => {
                let [info] = stco_info.as_slice() else {
                    return Err(invalid_layout(format!(
                        "expected exactly one stco box for track {} but found {}",
                        tkhd.track_id,
                        stco_info.len()
                    )));
                };
                ChunkOffsetBoxState::Stco {
                    info: *info,
                    box_value: stco,
                }
            }
            (None, Some(co64)) => {
                let [info] = co64_info.as_slice() else {
                    return Err(invalid_layout(format!(
                        "expected exactly one co64 box for track {} but found {}",
                        tkhd.track_id,
                        co64_info.len()
                    )));
                };
                ChunkOffsetBoxState::Co64 {
                    info: *info,
                    box_value: co64,
                }
            }
            (None, None) => {
                return Err(invalid_layout(format!(
                    "track {} is missing stco or co64 chunk offsets",
                    tkhd.track_id
                )));
            }
        }
    };

    Ok(MarlinMovieTrackState {
        track_id: tkhd.track_id,
        trak_info: *trak_info,
        mdia_info,
        minf_info,
        stbl_info,
        stsz_info,
        stsz,
        stsc,
        chunk_offsets,
        sample_sizes,
        marlin: marlin_tracks.get(&tkhd.track_id).cloned(),
    })
}

#[derive(Clone)]
struct TrackChunkLayout {
    offset: u64,
    sample_sizes: Vec<u32>,
}

fn compute_chunk_sample_counts(
    stsc: &Stsc,
    chunk_count: usize,
    sample_count: usize,
    track_id: u32,
) -> Result<Vec<u32>, DecryptRewriteError> {
    if chunk_count == 0 {
        return Ok(Vec::new());
    }
    if stsc.entries.is_empty() {
        return Err(invalid_layout(format!(
            "track {} is missing stsc entries for its {} chunk(s)",
            track_id, chunk_count
        )));
    }

    let mut counts = Vec::with_capacity(chunk_count);
    for (index, entry) in stsc.entries.iter().enumerate() {
        if entry.first_chunk == 0 {
            return Err(invalid_layout(format!(
                "track {} has an stsc entry with first_chunk 0",
                track_id
            )));
        }
        if entry.sample_description_index != 1 {
            return Err(invalid_layout(format!(
                "track {} uses unsupported stsc sample-description index {}",
                track_id, entry.sample_description_index
            )));
        }
        let next_first_chunk = stsc
            .entries
            .get(index + 1)
            .map(|entry| entry.first_chunk)
            .unwrap_or(u32::try_from(chunk_count + 1).map_err(|_| {
                invalid_layout("chunk-count sentinel does not fit in u32".to_owned())
            })?);
        if next_first_chunk <= entry.first_chunk {
            return Err(invalid_layout(format!(
                "track {} has descending or duplicated stsc first_chunk values",
                track_id
            )));
        }

        for _ in entry.first_chunk..next_first_chunk {
            counts.push(entry.samples_per_chunk);
        }
    }

    if counts.len() != chunk_count {
        return Err(invalid_layout(format!(
            "track {} resolved {} chunk mappings from stsc but has {} chunk offset(s)",
            track_id,
            counts.len(),
            chunk_count
        )));
    }
    let resolved_sample_count = counts.iter().try_fold(0usize, |total, count| {
        total
            .checked_add(usize::try_from(*count).map_err(|_| {
                invalid_layout("stsc samples-per-chunk value does not fit in usize".to_owned())
            })?)
            .ok_or_else(|| {
                invalid_layout("resolved chunk sample count overflowed usize".to_owned())
            })
    })?;
    if resolved_sample_count != sample_count {
        return Err(invalid_layout(format!(
            "track {} resolved {} samples from stsc but stsz reports {}",
            track_id, resolved_sample_count, sample_count
        )));
    }

    Ok(counts)
}

fn chunk_offsets_values(chunk_offsets: &ChunkOffsetBoxState) -> Vec<u64> {
    match chunk_offsets {
        ChunkOffsetBoxState::Stco { box_value, .. } => box_value.chunk_offset.to_vec(),
        ChunkOffsetBoxState::Co64 { box_value, .. } => box_value.chunk_offset.clone(),
    }
}

fn compute_track_chunks(
    track_id: u32,
    stsc: &Stsc,
    chunk_offsets: &ChunkOffsetBoxState,
    sample_sizes: &[u32],
) -> Result<Vec<TrackChunkLayout>, DecryptRewriteError> {
    let chunk_offsets = chunk_offsets_values(chunk_offsets);
    let chunk_sample_counts =
        compute_chunk_sample_counts(stsc, chunk_offsets.len(), sample_sizes.len(), track_id)?;

    let mut sample_index = 0usize;
    let mut chunks = Vec::with_capacity(chunk_offsets.len());
    for (offset, sample_count) in chunk_offsets.into_iter().zip(chunk_sample_counts) {
        let sample_count = usize::try_from(sample_count)
            .map_err(|_| invalid_layout("chunk sample count does not fit in usize".to_owned()))?;
        let end = sample_index
            .checked_add(sample_count)
            .ok_or_else(|| invalid_layout("track sample cursor overflowed usize".to_owned()))?;
        let Some(sample_sizes) = sample_sizes.get(sample_index..end) else {
            return Err(invalid_layout(format!(
                "track {} chunk layout exceeds the available sample-size table",
                track_id
            )));
        };
        chunks.push(TrackChunkLayout {
            offset,
            sample_sizes: sample_sizes.to_vec(),
        });
        sample_index = end;
    }
    if sample_index != sample_sizes.len() {
        return Err(invalid_layout(format!(
            "track {} chunk layout left {} sample-size entries unused",
            track_id,
            sample_sizes.len() - sample_index
        )));
    }

    Ok(chunks)
}

fn resolve_marlin_track_key(
    track_id: u32,
    protection: &MarlinTrackProtection,
    keys: &[DecryptionKey],
) -> Result<Option<[u8; 16]>, DecryptRewriteError> {
    match protection.key_mode {
        MarlinTrackKeyMode::Track => Ok(keys.iter().find_map(|entry| match entry.id() {
            DecryptionKeyId::TrackId(candidate) if candidate == track_id => Some(entry.key_bytes()),
            _ => None,
        })),
        MarlinTrackKeyMode::Group => {
            let Some(group_key) = keys.iter().find_map(|entry| match entry.id() {
                DecryptionKeyId::TrackId(0) => Some(entry.key_bytes()),
                _ => None,
            }) else {
                return Ok(None);
            };
            let wrapped_key = protection.wrapped_group_key.as_ref().ok_or_else(|| {
                invalid_layout(format!(
                    "Marlin group-key track {} is missing its wrapped gkey payload",
                    track_id
                ))
            })?;
            Ok(Some(unwrap_marlin_group_key(group_key, wrapped_key)?))
        }
    }
}

fn unwrap_marlin_group_key(
    group_key: [u8; 16],
    wrapped_key: &[u8],
) -> Result<[u8; 16], DecryptRewriteError> {
    if wrapped_key.len() < 24 || !wrapped_key.len().is_multiple_of(8) {
        return Err(invalid_layout(
            "Marlin group-key unwrap expects a wrapped key payload of at least 24 bytes and a multiple of 8"
                .to_owned(),
        ));
    }

    let n = wrapped_key.len() / 8 - 1;
    let mut a = wrapped_key[..8].try_into().unwrap();
    let mut r = wrapped_key[8..]
        .chunks_exact(8)
        .map(|chunk| chunk.try_into().unwrap())
        .collect::<Vec<[u8; 8]>>();
    let aes = Aes128::new(&group_key.into());

    for j in (0..=5usize).rev() {
        for i in (1..=n).rev() {
            let t = u64::try_from(n * j + i).map_err(|_| {
                invalid_layout("Marlin group-key unwrap round index overflowed u64".to_owned())
            })?;
            let mut block = Block::<Aes128>::default();
            let mut a_value = u64::from_be_bytes(a);
            a_value ^= t;
            block[..8].copy_from_slice(&a_value.to_be_bytes());
            block[8..].copy_from_slice(&r[i - 1]);
            aes.decrypt_block(&mut block);
            a.copy_from_slice(&block[..8]);
            r[i - 1].copy_from_slice(&block[8..16]);
        }
    }

    if a != [0xA6; 8] {
        return Err(invalid_layout(
            "Marlin group-key unwrap failed its AES key-wrap integrity check".to_owned(),
        ));
    }

    let mut clear = Vec::with_capacity(r.len() * 8);
    for chunk in r {
        clear.extend_from_slice(&chunk);
    }
    let clear = <[u8; 16]>::try_from(clear.as_slice()).map_err(|_| {
        invalid_layout("Marlin group-key unwrap did not yield one 16-byte track key".to_owned())
    })?;
    Ok(clear)
}

fn decrypt_marlin_sample_payload(
    payload: &[u8],
    key: [u8; 16],
) -> Result<Vec<u8>, DecryptRewriteError> {
    decrypt_oma_dcf_cbc_sample_payload(payload, key)
}

fn decrypt_oma_dcf_movie_file_bytes(
    input: &[u8],
    keys: &[DecryptionKey],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let context = analyze_oma_dcf_movie_file(input)?;
    let protected_by_track = context
        .tracks
        .iter()
        .map(|track| (track.track_id, track))
        .collect::<BTreeMap<_, _>>();
    let track_keys = keys
        .iter()
        .filter_map(|entry| match entry.id() {
            DecryptionKeyId::TrackId(track_id) => Some((track_id, entry.key_bytes())),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mdat_ranges = media_data_ranges_from_infos(&context.mdat_infos);

    let mut payload_tracks = context
        .tracks
        .iter()
        .map(|track| MovieTrackPayloadPlan {
            track_id: track.track_id,
            stsc: &track.stsc,
            chunk_offsets: &track.chunk_offsets,
            sample_sizes: &track.sample_sizes,
        })
        .collect::<Vec<_>>();
    payload_tracks.extend(
        context
            .other_tracks
            .iter()
            .map(|track| MovieTrackPayloadPlan {
                track_id: track.track_id,
                stsc: &track.stsc,
                chunk_offsets: &track.chunk_offsets,
                sample_sizes: &track.sample_sizes,
            }),
    );

    let (clear_payload, clear_sample_sizes, track_chunk_offsets) = rebuild_movie_payload(
        input,
        &mdat_ranges,
        &payload_tracks,
        |track_id, _sample_index, _absolute_offset, _sample_size, sample_bytes| {
            let Some(track) = protected_by_track.get(&track_id) else {
                return Ok(sample_bytes.to_vec());
            };
            let Some(key) = track_keys.get(&track_id).copied() else {
                return Ok(sample_bytes.to_vec());
            };
            decrypt_oma_dcf_sample_entry_payload(&track.odaf, &track.ohdr, key, sample_bytes)
        },
    )?;

    let mut track_plans = Vec::new();
    for track in &context.tracks {
        let stsd_replacement = if track_keys.contains_key(&track.track_id) {
            Some((
                track.stsd_info.offset(),
                rebuild_box_with_child_replacements(
                    input,
                    track.stsd_info,
                    &BTreeMap::from([(
                        track.sample_entry_info.offset(),
                        Some(build_clear_sample_entry_bytes(
                            input,
                            track.sample_entry_info,
                            track.original_format,
                            track.sinf_info,
                        )?),
                    )]),
                    None,
                )?,
            ))
        } else {
            None
        };
        let stsz_replacement = if track_keys.contains_key(&track.track_id) {
            Some((
                track.stsz_info.offset(),
                build_patched_stsz_bytes(
                    &track.stsz,
                    clear_sample_sizes.get(&track.track_id).ok_or_else(|| {
                        invalid_layout(format!(
                            "missing rebuilt sample sizes for OMA DCF track {}",
                            track.track_id
                        ))
                    })?,
                    "OMA DCF",
                )?,
            ))
        } else {
            None
        };
        track_plans.push(MovieTrackRewritePlan {
            track_id: track.track_id,
            trak_info: track.trak_info,
            mdia_info: track.mdia_info,
            minf_info: track.minf_info,
            stbl_info: track.stbl_info,
            chunk_offsets: track.chunk_offsets.clone(),
            stsd_replacement,
            stsz_replacement,
        });
    }
    track_plans.extend(
        context
            .other_tracks
            .iter()
            .map(|track| MovieTrackRewritePlan {
                track_id: track.track_id,
                trak_info: track.trak_info,
                mdia_info: track.mdia_info,
                minf_info: track.minf_info,
                stbl_info: track.stbl_info,
                chunk_offsets: track.chunk_offsets.clone(),
                stsd_replacement: None,
                stsz_replacement: None,
            }),
    );

    rebuild_movie_file_with_track_plans(
        MovieRootRewriteContext {
            input,
            ftyp_info: context.ftyp_info,
            moov_info: context.moov_info,
            mdat_infos: &context.mdat_infos,
        },
        &track_plans,
        &track_chunk_offsets,
        &clear_payload,
        build_patched_oma_clear_ftyp_bytes(input, context.ftyp_info)?,
    )
}

fn analyze_movie_chunk_track(
    input: &[u8],
    trak_info: &BoxInfo,
) -> Result<MovieChunkTrackState, DecryptRewriteError> {
    let mut reader = Cursor::new(input);
    let tkhd = extract_single_as::<_, Tkhd>(
        &mut reader,
        Some(trak_info),
        BoxPath::from([TKHD]),
        "trak/tkhd",
    )?;
    let mdia_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(&mut reader, Some(trak_info), BoxPath::from([MDIA]), "mdia")?
    };
    let minf_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF]),
            "minf",
        )?
    };
    let stbl_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL]),
            "stbl",
        )?
    };
    let stsz = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Stsz>(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, STSZ]),
            "stsz",
        )?
    };
    if stsz.sample_count == 0 {
        return Err(invalid_layout(format!(
            "track {} has no samples to decrypt in stsz",
            tkhd.track_id
        )));
    }
    let sample_sizes = sample_sizes_from_stsz(&stsz)?;
    let stsc = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Stsc>(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, STSC]),
            "stsc",
        )?
    };
    let stco = {
        let mut reader = Cursor::new(input);
        extract_optional_single_as::<_, Stco>(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, STCO]),
            "stco",
        )?
    };
    let co64 = {
        let mut reader = Cursor::new(input);
        extract_optional_single_as::<_, Co64>(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, FourCc::from_bytes(*b"co64")]),
            "co64",
        )?
    };
    let chunk_offsets = match (stco, co64) {
        (Some(_), Some(_)) => {
            return Err(invalid_layout(format!(
                "track {} has both stco and co64 chunk-offset boxes",
                tkhd.track_id
            )));
        }
        (Some(stco), None) => {
            let info = {
                let mut reader = Cursor::new(input);
                extract_single_info(
                    &mut reader,
                    Some(trak_info),
                    BoxPath::from([MDIA, MINF, STBL, STCO]),
                    "stco",
                )?
            };
            ChunkOffsetBoxState::Stco {
                info,
                box_value: stco,
            }
        }
        (None, Some(co64)) => {
            let info = {
                let mut reader = Cursor::new(input);
                extract_single_info(
                    &mut reader,
                    Some(trak_info),
                    BoxPath::from([MDIA, MINF, STBL, FourCc::from_bytes(*b"co64")]),
                    "co64",
                )?
            };
            ChunkOffsetBoxState::Co64 {
                info,
                box_value: co64,
            }
        }
        (None, None) => {
            return Err(invalid_layout(format!(
                "track {} is missing stco or co64 chunk offsets",
                tkhd.track_id
            )));
        }
    };

    let _ = compute_track_chunks(tkhd.track_id, &stsc, &chunk_offsets, &sample_sizes)?;

    Ok(MovieChunkTrackState {
        track_id: tkhd.track_id,
        trak_info: *trak_info,
        mdia_info,
        minf_info,
        stbl_info,
        stsc,
        chunk_offsets,
        sample_sizes,
    })
}

fn analyze_oma_dcf_movie_file(
    input: &[u8],
) -> Result<OmaProtectedMovieContext, DecryptRewriteError> {
    let root_boxes = read_root_box_infos(input)?;
    let ftyp_info = root_boxes
        .iter()
        .copied()
        .find(|info| info.box_type() == FTYP);
    let Some(moov_info) = root_boxes
        .iter()
        .copied()
        .find(|info| info.box_type() == MOOV)
    else {
        return Err(invalid_layout(
            "expected one root moov box in the protected movie file".to_owned(),
        ));
    };
    let mdat_infos = root_boxes
        .iter()
        .copied()
        .filter(|info| info.box_type() == MDAT)
        .collect::<Vec<_>>();
    if mdat_infos.is_empty() {
        return Err(invalid_layout(
            "expected at least one root mdat box in the protected movie file".to_owned(),
        ));
    }
    let mut reader = Cursor::new(input);
    let traks = extract_box(&mut reader, None, BoxPath::from([MOOV, TRAK]))?;
    let mut protected_tracks = Vec::new();
    let mut other_tracks = Vec::new();
    for trak_info in traks {
        if let Some(track) = analyze_oma_dcf_movie_track(input, &trak_info)? {
            protected_tracks.push(track);
        } else {
            other_tracks.push(analyze_movie_chunk_track(input, &trak_info)?);
        }
    }

    if protected_tracks.is_empty() {
        return Err(invalid_layout(
            "expected at least one OMA DCF protected sample-entry track in the movie file"
                .to_owned(),
        ));
    }

    Ok(OmaProtectedMovieContext {
        ftyp_info,
        moov_info,
        tracks: protected_tracks,
        other_tracks,
        mdat_infos,
    })
}

fn analyze_oma_dcf_movie_track(
    input: &[u8],
    trak_info: &BoxInfo,
) -> Result<Option<OmaProtectedMovieTrackState>, DecryptRewriteError> {
    let track_layout = analyze_movie_chunk_track(input, trak_info)?;
    let stsd_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, STSD]),
            "stsd",
        )?
    };

    let mut reader = Cursor::new(input);
    let encv_infos = extract_box(
        &mut reader,
        Some(trak_info),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCV]),
    )?;
    let mut reader = Cursor::new(input);
    let enca_infos = extract_box(
        &mut reader,
        Some(trak_info),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCA]),
    )?;
    let (sample_entry_info, sample_entry_type) =
        match (encv_infos.as_slice(), enca_infos.as_slice()) {
            ([], []) => return Ok(None),
            ([info], []) => (*info, ENCV),
            ([], [info]) => (*info, ENCA),
            _ => {
                return Err(invalid_layout(format!(
                    "track {} has an unsupported protected sample-entry count",
                    track_layout.track_id
                )));
            }
        };

    let protected_prefix = BoxPath::from([MDIA, MINF, STBL, STSD, sample_entry_type]);
    let protected_sinf_prefix = child_path(&protected_prefix, SINF);
    let original_format = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Frma>(
            &mut reader,
            Some(trak_info),
            child_path(&protected_sinf_prefix, FRMA),
            "frma",
        )?
        .data_format
    };
    let sinf_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            protected_sinf_prefix.clone(),
            "sinf",
        )?
    };
    let schm = {
        let mut reader = Cursor::new(input);
        extract_optional_single_as::<_, Schm>(
            &mut reader,
            Some(trak_info),
            child_path(&protected_sinf_prefix, SCHM),
            "schm",
        )?
    };
    let odkm_prefix = child_path(&child_path(&protected_sinf_prefix, SCHI), ODKM);
    let odkm_info = {
        let mut reader = Cursor::new(input);
        let mut infos = extract_box(&mut reader, Some(trak_info), odkm_prefix.clone())?;
        if infos.len() > 1 {
            return Err(invalid_layout(format!(
                "expected at most one odkm box for track {} but found {}",
                track_layout.track_id,
                infos.len()
            )));
        }
        infos.pop()
    };

    let is_oma = match schm {
        Some(schm) => schm.scheme_type == ODKM,
        None => odkm_info.is_some(),
    };
    if !is_oma {
        return Ok(None);
    }

    let odaf = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Odaf>(
            &mut reader,
            Some(trak_info),
            child_path(&odkm_prefix, ODAF),
            "odaf",
        )?
    };
    if odaf.key_indicator_length != 0 {
        return Err(invalid_layout(format!(
            "track {} uses unsupported OMA DCF key-indicator length {}",
            track_layout.track_id, odaf.key_indicator_length
        )));
    }
    if odaf.iv_length > 16 {
        return Err(invalid_layout(format!(
            "track {} uses unsupported OMA DCF IV length {}",
            track_layout.track_id, odaf.iv_length
        )));
    }

    let ohdr = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Ohdr>(
            &mut reader,
            Some(trak_info),
            child_path(&odkm_prefix, OHDR),
            "ohdr",
        )?
    };
    let ohdr_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            child_path(&odkm_prefix, OHDR),
            "ohdr",
        )?
    };
    let mut reader = Cursor::new(input);
    let grpi_children = extract_box(&mut reader, Some(&ohdr_info), BoxPath::from([GRPI]))?;
    if !grpi_children.is_empty() {
        return Err(invalid_layout(
            "group-key-wrapped OMA DCF protected sample entries are not supported yet".to_owned(),
        ));
    }

    Ok(Some(OmaProtectedMovieTrackState {
        track_id: track_layout.track_id,
        trak_info: track_layout.trak_info,
        mdia_info: track_layout.mdia_info,
        minf_info: track_layout.minf_info,
        stbl_info: track_layout.stbl_info,
        stsd_info,
        sample_entry_info,
        original_format,
        sinf_info,
        stsz_info: {
            let mut reader = Cursor::new(input);
            extract_single_info(
                &mut reader,
                Some(trak_info),
                BoxPath::from([MDIA, MINF, STBL, STSZ]),
                "stsz",
            )?
        },
        stsz: {
            let mut reader = Cursor::new(input);
            extract_single_as::<_, Stsz>(
                &mut reader,
                Some(trak_info),
                BoxPath::from([MDIA, MINF, STBL, STSZ]),
                "stsz",
            )?
        },
        stsc: track_layout.stsc,
        chunk_offsets: track_layout.chunk_offsets,
        sample_sizes: track_layout.sample_sizes,
        odaf,
        ohdr,
    }))
}

fn sample_sizes_from_stsz(stsz: &Stsz) -> Result<Vec<u32>, DecryptRewriteError> {
    if stsz.sample_size != 0 {
        return Ok(vec![stsz.sample_size; stsz.sample_count as usize]);
    }

    if stsz.entry_size.len() != stsz.sample_count as usize {
        return Err(invalid_layout(format!(
            "stsz entry-size count {} does not match sample_count {}",
            stsz.entry_size.len(),
            stsz.sample_count
        )));
    }
    stsz.entry_size
        .iter()
        .copied()
        .map(|size| {
            u32::try_from(size).map_err(|_| {
                invalid_layout("protected movie sample size does not fit in u32".to_owned())
            })
        })
        .collect()
}

fn build_clear_sample_entry_bytes(
    input: &[u8],
    sample_entry_info: BoxInfo,
    original_format: FourCc,
    sinf_info: BoxInfo,
) -> Result<Vec<u8>, DecryptRewriteError> {
    let mut child_replacements = BTreeMap::new();
    child_replacements.insert(sinf_info.offset(), None);
    let mut rebuilt =
        rebuild_box_with_child_replacements(input, sample_entry_info, &child_replacements, None)?;
    patch_box_type_bytes(&mut rebuilt, original_format)?;
    Ok(rebuilt)
}

fn build_patched_stsz_bytes(
    stsz: &Stsz,
    clear_sample_sizes: &[u64],
    label: &str,
) -> Result<Vec<u8>, DecryptRewriteError> {
    let mut patched_stsz = stsz.clone();
    patched_stsz.sample_count = u32::try_from(clear_sample_sizes.len())
        .map_err(|_| invalid_layout(format!("{label} sample count does not fit in u32")))?;
    if patched_stsz.sample_size == 0 {
        patched_stsz.entry_size = clear_sample_sizes.to_vec();
    } else if let Some(&uniform_size) = clear_sample_sizes.first() {
        if !clear_sample_sizes.iter().all(|&size| size == uniform_size) {
            return Err(invalid_layout(format!(
                "fixed-size {label} sample tables require all decrypted samples to have the same size"
            )));
        }
        patched_stsz.sample_size = u32::try_from(uniform_size)
            .map_err(|_| invalid_layout(format!("{label} sample size does not fit in u32")))?;
        patched_stsz.entry_size.clear();
    } else {
        patched_stsz.sample_size = 0;
        patched_stsz.entry_size.clear();
    }
    encode_box_with_children(&patched_stsz, &[])
}

fn build_patched_chunk_offset_box_bytes(
    chunk_offsets: &ChunkOffsetBoxState,
    new_offsets: &[u64],
) -> Result<Vec<u8>, DecryptRewriteError> {
    match chunk_offsets {
        ChunkOffsetBoxState::Stco { box_value, .. } => {
            let mut patched = box_value.clone();
            patched.chunk_offset = new_offsets.to_vec();
            encode_box_with_children(&patched, &[])
        }
        ChunkOffsetBoxState::Co64 { box_value, .. } => {
            let mut patched = box_value.clone();
            patched.chunk_offset = new_offsets.to_vec();
            encode_box_with_children(&patched, &[])
        }
    }
}

fn build_patched_oma_clear_ftyp_bytes(
    input: &[u8],
    ftyp_info: Option<BoxInfo>,
) -> Result<Option<Vec<u8>>, DecryptRewriteError> {
    let Some(_ftyp_info) = ftyp_info else {
        return Ok(None);
    };
    let mut reader = Cursor::new(input);
    let mut ftyp = extract_single_as::<_, Ftyp>(&mut reader, None, BoxPath::from([FTYP]), "ftyp")?;
    ftyp.compatible_brands.retain(|brand| *brand != OPF2);
    Ok(Some(encode_box_with_children(&ftyp, &[])?))
}

fn media_data_ranges_from_infos(mdat_infos: &[BoxInfo]) -> Vec<MediaDataRange> {
    mdat_infos
        .iter()
        .map(|info| MediaDataRange {
            start: info.offset() + info.header_size(),
            end: info.offset() + info.size(),
        })
        .collect()
}

fn build_movie_moov_with_track_replacements(
    input: &[u8],
    moov_info: BoxInfo,
    track_plans: &[MovieTrackRewritePlan],
    chunk_offsets_by_track: &TrackRelativeChunkOffsets,
) -> Result<Vec<u8>, DecryptRewriteError> {
    let mut moov_replacements = BTreeMap::new();
    for plan in track_plans {
        let new_offsets = chunk_offsets_by_track
            .get(&plan.track_id)
            .cloned()
            .ok_or_else(|| {
                invalid_layout(format!(
                    "missing rewritten chunk offsets for movie track {}",
                    plan.track_id
                ))
            })?;
        let mut stbl_replacements = BTreeMap::new();
        stbl_replacements.insert(
            chunk_offset_box_offset(&plan.chunk_offsets),
            Some(build_patched_chunk_offset_box_bytes(
                &plan.chunk_offsets,
                &new_offsets,
            )?),
        );
        if let Some((offset, bytes)) = &plan.stsd_replacement {
            stbl_replacements.insert(*offset, Some(bytes.clone()));
        }
        if let Some((offset, bytes)) = &plan.stsz_replacement {
            stbl_replacements.insert(*offset, Some(bytes.clone()));
        }
        let trak_bytes = rebuild_track_with_stbl_replacements(
            input,
            plan.trak_info,
            plan.mdia_info,
            plan.minf_info,
            plan.stbl_info,
            &stbl_replacements,
        )?;
        moov_replacements.insert(plan.trak_info.offset(), Some(trak_bytes));
    }
    rebuild_box_with_child_replacements(input, moov_info, &moov_replacements, None)
}

fn compute_single_mdat_payload_offset(
    input: &[u8],
    root_boxes: &[BoxInfo],
    ftyp_info: Option<BoxInfo>,
    moov_info: BoxInfo,
    patched_ftyp_bytes: Option<&[u8]>,
    moov_bytes: &[u8],
    mdat_header_size: u64,
) -> Result<u64, DecryptRewriteError> {
    let mut offset = 0_u64;
    for info in root_boxes {
        if info.box_type() == MDAT {
            continue;
        }
        let size = if Some(*info) == ftyp_info {
            patched_ftyp_bytes
                .map(|bytes| bytes.len() as u64)
                .unwrap_or(info.size())
        } else if info.offset() == moov_info.offset() {
            u64::try_from(moov_bytes.len()).map_err(|_| {
                invalid_layout("replacement moov size does not fit in u64".to_owned())
            })?
        } else {
            u64::try_from(slice_box_bytes(input, *info)?.len())
                .map_err(|_| invalid_layout("root box size does not fit in u64".to_owned()))?
        };
        offset = offset
            .checked_add(size)
            .ok_or_else(|| invalid_layout("root box offset overflowed u64".to_owned()))?;
    }
    offset
        .checked_add(mdat_header_size)
        .ok_or_else(|| invalid_layout("clear mdat payload offset overflowed u64".to_owned()))
}

fn rebuild_root_boxes_with_single_mdat(
    input: &[u8],
    root_boxes: &[BoxInfo],
    ftyp_info: Option<BoxInfo>,
    moov_info: BoxInfo,
    patched_ftyp_bytes: Option<&[u8]>,
    moov_bytes: &[u8],
    mdat_bytes: &[u8],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let mut output = Vec::new();
    for info in root_boxes {
        if info.box_type() == MDAT {
            continue;
        }
        if Some(*info) == ftyp_info {
            if let Some(bytes) = patched_ftyp_bytes {
                output.extend_from_slice(bytes);
            } else {
                output.extend_from_slice(slice_box_bytes(input, *info)?);
            }
        } else if info.offset() == moov_info.offset() {
            output.extend_from_slice(moov_bytes);
        } else {
            output.extend_from_slice(slice_box_bytes(input, *info)?);
        }
    }
    output.extend_from_slice(mdat_bytes);
    Ok(output)
}

fn rebuild_movie_payload<F>(
    input: &[u8],
    mdat_ranges: &[MediaDataRange],
    tracks: &[MovieTrackPayloadPlan<'_>],
    mut process_sample: F,
) -> Result<RebuiltMoviePayload, DecryptRewriteError>
where
    F: FnMut(u32, u32, u64, u32, &[u8]) -> Result<Vec<u8>, DecryptRewriteError>,
{
    let mut all_chunks = Vec::new();
    let mut sample_indices = BTreeMap::new();
    let mut rebuilt_sample_sizes = BTreeMap::<u32, Vec<u64>>::new();
    let mut relative_offsets = BTreeMap::<u32, Vec<u64>>::new();
    for track in tracks {
        sample_indices.insert(track.track_id, 0_u32);
        rebuilt_sample_sizes.insert(track.track_id, Vec::new());
        relative_offsets.insert(track.track_id, Vec::new());
        for chunk in compute_track_chunks(
            track.track_id,
            track.stsc,
            track.chunk_offsets,
            track.sample_sizes,
        )? {
            all_chunks.push((track.track_id, chunk));
        }
    }
    all_chunks.sort_by_key(|(_, chunk)| chunk.offset);

    let mut payload = Vec::new();
    let mut previous_chunk_end = None;
    for (track_id, chunk) in all_chunks {
        let chunk_size = sum_chunk_size(&chunk.sample_sizes)?;
        if let Some(previous_chunk_end) = previous_chunk_end
            && chunk.offset < previous_chunk_end
        {
            return Err(invalid_layout(format!(
                "track {track_id} has overlapping chunk ranges in the protected movie layout"
            )));
        }
        previous_chunk_end = Some(
            chunk
                .offset
                .checked_add(chunk_size)
                .ok_or_else(|| invalid_layout("movie chunk end overflowed u64".to_owned()))?,
        );

        relative_offsets
            .get_mut(&track_id)
            .unwrap()
            .push(u64::try_from(payload.len()).map_err(|_| {
                invalid_layout("rebuilt mdat payload length does not fit in u64".to_owned())
            })?);

        let mut sample_offset = chunk.offset;
        for sample_size in chunk.sample_sizes {
            let sample_index = sample_indices.get_mut(&track_id).ok_or_else(|| {
                invalid_layout(format!(
                    "missing sample index state for movie track {}",
                    track_id
                ))
            })?;
            *sample_index = sample_index
                .checked_add(1)
                .ok_or_else(|| invalid_layout("movie sample index overflowed u32".to_owned()))?;
            let sample_bytes = read_sample_range(input, mdat_ranges, sample_offset, sample_size)
                .ok_or(DecryptRewriteError::SampleDataRangeNotFound {
                    track_id,
                    sample_index: *sample_index,
                    absolute_offset: sample_offset,
                    sample_size,
                })?;
            let rebuilt = process_sample(
                track_id,
                *sample_index,
                sample_offset,
                sample_size,
                sample_bytes,
            )?;
            rebuilt_sample_sizes.get_mut(&track_id).unwrap().push(
                u64::try_from(rebuilt.len()).map_err(|_| {
                    invalid_layout("rebuilt movie sample size does not fit in u64".to_owned())
                })?,
            );
            payload.extend_from_slice(&rebuilt);
            sample_offset = sample_offset
                .checked_add(u64::from(sample_size))
                .ok_or_else(|| invalid_layout("movie sample offset overflowed u64".to_owned()))?;
        }
    }

    Ok((payload, rebuilt_sample_sizes, relative_offsets))
}

fn rebuild_movie_file_with_track_plans(
    root: MovieRootRewriteContext<'_>,
    track_plans: &[MovieTrackRewritePlan],
    relative_chunk_offsets: &TrackRelativeChunkOffsets,
    clear_payload: &[u8],
    patched_ftyp_bytes: Option<Vec<u8>>,
) -> Result<Vec<u8>, DecryptRewriteError> {
    if root.mdat_infos.is_empty() {
        return Err(invalid_layout(
            "expected at least one root mdat box in the protected movie file".to_owned(),
        ));
    }

    let root_boxes = read_root_box_infos(root.input)?;
    let placeholder_offsets = track_plans
        .iter()
        .map(|plan| (plan.track_id, chunk_offsets_values(&plan.chunk_offsets)))
        .collect::<TrackRelativeChunkOffsets>();
    let moov_placeholder = build_movie_moov_with_track_replacements(
        root.input,
        root.moov_info,
        track_plans,
        &placeholder_offsets,
    )?;

    let mdat_bytes = encode_raw_box(MDAT, clear_payload)?;
    let mdat_header_size = u64::try_from(mdat_bytes.len().saturating_sub(clear_payload.len()))
        .map_err(|_| invalid_layout("clear mdat header size does not fit in u64".to_owned()))?;
    let mdat_payload_offset = compute_single_mdat_payload_offset(
        root.input,
        &root_boxes,
        root.ftyp_info,
        root.moov_info,
        patched_ftyp_bytes.as_deref(),
        &moov_placeholder,
        mdat_header_size,
    )?;
    let absolute_offsets = relative_chunk_offsets
        .iter()
        .map(|(track_id, offsets)| {
            let absolute = offsets
                .iter()
                .map(|offset| {
                    mdat_payload_offset.checked_add(*offset).ok_or_else(|| {
                        invalid_layout("patched movie chunk offset overflowed u64".to_owned())
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((*track_id, absolute))
        })
        .collect::<Result<TrackRelativeChunkOffsets, DecryptRewriteError>>()?;
    let moov_final = build_movie_moov_with_track_replacements(
        root.input,
        root.moov_info,
        track_plans,
        &absolute_offsets,
    )?;
    rebuild_root_boxes_with_single_mdat(
        root.input,
        &root_boxes,
        root.ftyp_info,
        root.moov_info,
        patched_ftyp_bytes.as_deref(),
        &moov_final,
        &mdat_bytes,
    )
}

#[allow(clippy::too_many_arguments)]
fn rebuild_track_with_stbl_replacements(
    input: &[u8],
    trak_info: BoxInfo,
    mdia_info: BoxInfo,
    minf_info: BoxInfo,
    stbl_info: BoxInfo,
    stbl_replacements: &BTreeMap<u64, Option<Vec<u8>>>,
) -> Result<Vec<u8>, DecryptRewriteError> {
    let stbl = rebuild_box_with_child_replacements(input, stbl_info, stbl_replacements, None)?;

    let mut minf_replacements = BTreeMap::new();
    minf_replacements.insert(stbl_info.offset(), Some(stbl));
    let minf = rebuild_box_with_child_replacements(input, minf_info, &minf_replacements, None)?;

    let mut mdia_replacements = BTreeMap::new();
    mdia_replacements.insert(minf_info.offset(), Some(minf));
    let mdia = rebuild_box_with_child_replacements(input, mdia_info, &mdia_replacements, None)?;

    let mut trak_replacements = BTreeMap::new();
    trak_replacements.insert(mdia_info.offset(), Some(mdia));
    rebuild_box_with_child_replacements(input, trak_info, &trak_replacements, None)
}

fn sum_chunk_size(sample_sizes: &[u32]) -> Result<u64, DecryptRewriteError> {
    sample_sizes.iter().try_fold(0_u64, |total, size| {
        total
            .checked_add(u64::from(*size))
            .ok_or_else(|| invalid_layout("chunk byte size overflowed u64".to_owned()))
    })
}
fn chunk_offset_box_offset(chunk_offsets: &ChunkOffsetBoxState) -> u64 {
    match chunk_offsets {
        ChunkOffsetBoxState::Stco { info, .. } | ChunkOffsetBoxState::Co64 { info, .. } => {
            info.offset()
        }
    }
}

fn rebuild_box_with_child_replacements(
    input: &[u8],
    parent_info: BoxInfo,
    child_replacements: &BTreeMap<u64, Option<Vec<u8>>>,
    override_type: Option<FourCc>,
) -> Result<Vec<u8>, DecryptRewriteError> {
    let parent_bytes = slice_box_bytes(input, parent_info)?;
    let header_size = usize::try_from(parent_info.header_size())
        .map_err(|_| invalid_layout("box header size does not fit in usize".to_owned()))?;
    let mut reader = Cursor::new(input);
    let child_infos = extract_box(
        &mut reader,
        Some(&parent_info),
        BoxPath::from([FourCc::ANY]),
    )?;

    let mut payload = Vec::with_capacity(parent_bytes.len().saturating_sub(header_size));
    let mut cursor = header_size;
    for child_info in child_infos {
        let relative_start = usize::try_from(child_info.offset() - parent_info.offset())
            .map_err(|_| invalid_layout("child offset does not fit in usize".to_owned()))?;
        let relative_end =
            usize::try_from(child_info.offset() + child_info.size() - parent_info.offset())
                .map_err(|_| invalid_layout("child end does not fit in usize".to_owned()))?;
        if relative_start < cursor || relative_end > parent_bytes.len() {
            return Err(invalid_layout(format!(
                "child {} lies outside the available parent payload while rebuilding {}",
                child_info.box_type(),
                parent_info.box_type()
            )));
        }
        payload.extend_from_slice(&parent_bytes[cursor..relative_start]);
        match child_replacements.get(&child_info.offset()) {
            Some(Some(replacement)) => payload.extend_from_slice(replacement),
            Some(None) => {}
            None => payload.extend_from_slice(&parent_bytes[relative_start..relative_end]),
        }
        cursor = relative_end;
    }
    payload.extend_from_slice(&parent_bytes[cursor..]);

    let box_type = override_type.unwrap_or(parent_info.box_type());
    let total_size = u64::try_from(header_size)
        .ok()
        .and_then(|header| header.checked_add(u64::try_from(payload.len()).ok()?))
        .ok_or_else(|| invalid_layout("rebuilt box size overflowed u64".to_owned()))?;
    let mut rebuilt = BoxInfo::new(box_type, total_size)
        .with_header_size(parent_info.header_size())
        .encode();
    rebuilt.extend_from_slice(&payload);
    Ok(rebuilt)
}

fn decrypt_oma_dcf_sample_entry_payload(
    odaf: &Odaf,
    ohdr: &Ohdr,
    key: [u8; 16],
    sample_bytes: &[u8],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let mut payload = sample_bytes;
    let is_encrypted = if odaf.selective_encryption {
        let Some((&flag, rest)) = payload.split_first() else {
            return Err(invalid_layout(
                "selectively encrypted OMA DCF sample is missing its encryption flag".to_owned(),
            ));
        };
        payload = rest;
        (flag & 0x80) != 0
    } else {
        true
    };

    if !is_encrypted || ohdr.encryption_method == OHDR_ENCRYPTION_METHOD_NULL {
        return Ok(payload.to_vec());
    }

    let iv_length = usize::from(odaf.iv_length);
    if iv_length == 0 || payload.len() < iv_length {
        return Err(invalid_layout(
            "encrypted OMA DCF sample is missing its initialization vector".to_owned(),
        ));
    }

    match ohdr.encryption_method {
        OHDR_ENCRYPTION_METHOD_AES_CBC => {
            if iv_length != 16 {
                return Err(invalid_layout(
                    "OMA DCF CBC sample decrypt requires a 16-byte initialization vector"
                        .to_owned(),
                ));
            }
            if ohdr.padding_scheme != OHDR_PADDING_SCHEME_RFC_2630 {
                return Err(invalid_layout(
                    "OMA DCF CBC sample decrypt requires RFC 2630 padding".to_owned(),
                ));
            }
            decrypt_oma_dcf_cbc_sample_payload(payload, key)
        }
        OHDR_ENCRYPTION_METHOD_AES_CTR => {
            if ohdr.padding_scheme != OHDR_PADDING_SCHEME_NONE {
                return Err(invalid_layout(
                    "OMA DCF CTR sample decrypt requires the no-padding scheme".to_owned(),
                ));
            }
            decrypt_oma_dcf_ctr_sample_payload(payload, key, iv_length)
        }
        method => Err(invalid_layout(format!(
            "unsupported OMA DCF sample encryption method {method}"
        ))),
    }
}

fn decrypt_oma_dcf_cbc_sample_payload(
    payload: &[u8],
    key: [u8; 16],
) -> Result<Vec<u8>, DecryptRewriteError> {
    if payload.len() < 32 || !(payload.len() - 16).is_multiple_of(16) {
        return Err(invalid_layout(
            "OMA DCF CBC sample payload has an invalid IV or ciphertext length".to_owned(),
        ));
    }

    let mut previous = [0_u8; 16];
    previous.copy_from_slice(&payload[..16]);
    let ciphertext = &payload[16..];
    let aes = Aes128::new(&key.into());
    let mut plaintext = Vec::with_capacity(ciphertext.len());
    for chunk in ciphertext.chunks_exact(16) {
        let mut block = Block::<Aes128>::default();
        block.copy_from_slice(chunk);
        let encrypted = block;
        aes.decrypt_block(&mut block);
        for index in 0..16 {
            block[index] ^= previous[index];
        }
        plaintext.extend_from_slice(&block);
        previous.copy_from_slice(&encrypted);
    }
    remove_rfc_2630_padding(&plaintext)
}

fn decrypt_oma_dcf_ctr_sample_payload(
    payload: &[u8],
    key: [u8; 16],
    iv_length: usize,
) -> Result<Vec<u8>, DecryptRewriteError> {
    if payload.len() < iv_length {
        return Err(invalid_layout(
            "OMA DCF CTR sample payload is shorter than its initialization vector".to_owned(),
        ));
    }

    let mut counter = [0_u8; 16];
    counter[16 - iv_length..].copy_from_slice(&payload[..iv_length]);
    let ciphertext = &payload[iv_length..];
    let aes = Aes128::new(&key.into());
    let mut output = vec![0_u8; ciphertext.len()];
    let mut cursor = 0usize;
    while cursor < ciphertext.len() {
        let mut stream_block = Block::<Aes128>::default();
        stream_block.copy_from_slice(&counter);
        aes.encrypt_block(&mut stream_block);
        let chunk_len = 16.min(ciphertext.len() - cursor);
        for index in 0..chunk_len {
            output[cursor + index] = ciphertext[cursor + index] ^ stream_block[index];
        }
        cursor += chunk_len;
        increment_counter_suffix_be(&mut counter, iv_length);
    }
    Ok(output)
}

fn increment_counter_suffix_be(counter: &mut [u8; 16], counter_bytes: usize) {
    for byte in counter[16 - counter_bytes..].iter_mut().rev() {
        *byte = byte.wrapping_add(1);
        if *byte != 0 {
            break;
        }
    }
}

fn decrypt_iaec_movie_file_bytes(
    input: &[u8],
    keys: &[DecryptionKey],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let context = analyze_iaec_movie_file(input)?;
    let protected_by_track = context
        .tracks
        .iter()
        .map(|track| (track.track_id, track))
        .collect::<BTreeMap<_, _>>();
    let track_keys = keys
        .iter()
        .filter_map(|entry| match entry.id() {
            DecryptionKeyId::TrackId(track_id) => Some((track_id, entry.key_bytes())),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mdat_ranges = media_data_ranges_from_infos(&context.mdat_infos);

    let mut payload_tracks = context
        .tracks
        .iter()
        .map(|track| MovieTrackPayloadPlan {
            track_id: track.track_id,
            stsc: &track.stsc,
            chunk_offsets: &track.chunk_offsets,
            sample_sizes: &track.sample_sizes,
        })
        .collect::<Vec<_>>();
    payload_tracks.extend(
        context
            .other_tracks
            .iter()
            .map(|track| MovieTrackPayloadPlan {
                track_id: track.track_id,
                stsc: &track.stsc,
                chunk_offsets: &track.chunk_offsets,
                sample_sizes: &track.sample_sizes,
            }),
    );

    let (clear_payload, clear_sample_sizes, track_chunk_offsets) = rebuild_movie_payload(
        input,
        &mdat_ranges,
        &payload_tracks,
        |track_id, _sample_index, _absolute_offset, _sample_size, sample_bytes| {
            let Some(track) = protected_by_track.get(&track_id) else {
                return Ok(sample_bytes.to_vec());
            };
            let Some(key) = track_keys.get(&track_id).copied() else {
                return Ok(sample_bytes.to_vec());
            };
            decrypt_iaec_sample_entry_payload(&track.isfm, track.islt.as_ref(), key, sample_bytes)
        },
    )?;

    let mut track_plans = Vec::new();
    for track in &context.tracks {
        let stsd_replacement = if track_keys.contains_key(&track.track_id) {
            Some((
                track.stsd_info.offset(),
                rebuild_box_with_child_replacements(
                    input,
                    track.stsd_info,
                    &BTreeMap::from([(
                        track.sample_entry_info.offset(),
                        Some(build_clear_sample_entry_bytes(
                            input,
                            track.sample_entry_info,
                            track.original_format,
                            track.sinf_info,
                        )?),
                    )]),
                    None,
                )?,
            ))
        } else {
            None
        };
        let stsz_replacement = if track_keys.contains_key(&track.track_id) {
            Some((
                track.stsz_info.offset(),
                build_patched_stsz_bytes(
                    &track.stsz,
                    clear_sample_sizes.get(&track.track_id).ok_or_else(|| {
                        invalid_layout(format!(
                            "missing rebuilt sample sizes for IAEC track {}",
                            track.track_id
                        ))
                    })?,
                    "IAEC",
                )?,
            ))
        } else {
            None
        };
        track_plans.push(MovieTrackRewritePlan {
            track_id: track.track_id,
            trak_info: track.trak_info,
            mdia_info: track.mdia_info,
            minf_info: track.minf_info,
            stbl_info: track.stbl_info,
            chunk_offsets: track.chunk_offsets.clone(),
            stsd_replacement,
            stsz_replacement,
        });
    }
    track_plans.extend(
        context
            .other_tracks
            .iter()
            .map(|track| MovieTrackRewritePlan {
                track_id: track.track_id,
                trak_info: track.trak_info,
                mdia_info: track.mdia_info,
                minf_info: track.minf_info,
                stbl_info: track.stbl_info,
                chunk_offsets: track.chunk_offsets.clone(),
                stsd_replacement: None,
                stsz_replacement: None,
            }),
    );

    rebuild_movie_file_with_track_plans(
        MovieRootRewriteContext {
            input,
            ftyp_info: context.ftyp_info,
            moov_info: context.moov_info,
            mdat_infos: &context.mdat_infos,
        },
        &track_plans,
        &track_chunk_offsets,
        &clear_payload,
        None,
    )
}

fn analyze_iaec_movie_file(input: &[u8]) -> Result<IaecProtectedMovieContext, DecryptRewriteError> {
    let root_boxes = read_root_box_infos(input)?;
    let ftyp_info = root_boxes
        .iter()
        .copied()
        .find(|info| info.box_type() == FTYP);
    let Some(moov_info) = root_boxes
        .iter()
        .copied()
        .find(|info| info.box_type() == MOOV)
    else {
        return Err(invalid_layout(
            "expected one root moov box in the protected movie file".to_owned(),
        ));
    };
    let mdat_infos = root_boxes
        .iter()
        .copied()
        .filter(|info| info.box_type() == MDAT)
        .collect::<Vec<_>>();
    if mdat_infos.is_empty() {
        return Err(invalid_layout(
            "expected at least one root mdat box in the protected movie file".to_owned(),
        ));
    }

    let mut reader = Cursor::new(input);
    let traks = extract_box(&mut reader, None, BoxPath::from([MOOV, TRAK]))?;
    let mut protected_tracks = Vec::new();
    let mut other_tracks = Vec::new();
    for trak_info in traks {
        if let Some(track) = analyze_iaec_movie_track(input, &trak_info)? {
            protected_tracks.push(track);
        } else {
            other_tracks.push(analyze_movie_chunk_track(input, &trak_info)?);
        }
    }

    if protected_tracks.is_empty() {
        return Err(invalid_layout(
            "expected at least one IAEC protected sample-entry track in the movie file".to_owned(),
        ));
    }

    Ok(IaecProtectedMovieContext {
        ftyp_info,
        moov_info,
        tracks: protected_tracks,
        other_tracks,
        mdat_infos,
    })
}

fn analyze_iaec_movie_track(
    input: &[u8],
    trak_info: &BoxInfo,
) -> Result<Option<IaecProtectedMovieTrackState>, DecryptRewriteError> {
    let track_layout = analyze_movie_chunk_track(input, trak_info)?;
    let stsd_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, STSD]),
            "stsd",
        )?
    };

    let mut reader = Cursor::new(input);
    let encv_infos = extract_box(
        &mut reader,
        Some(trak_info),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCV]),
    )?;
    let mut reader = Cursor::new(input);
    let enca_infos = extract_box(
        &mut reader,
        Some(trak_info),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCA]),
    )?;
    let (sample_entry_info, sample_entry_type) =
        match (encv_infos.as_slice(), enca_infos.as_slice()) {
            ([], []) => return Ok(None),
            ([info], []) => (*info, ENCV),
            ([], [info]) => (*info, ENCA),
            _ => {
                return Err(invalid_layout(format!(
                    "track {} has an unsupported protected sample-entry count",
                    track_layout.track_id
                )));
            }
        };

    let protected_prefix = BoxPath::from([MDIA, MINF, STBL, STSD, sample_entry_type]);
    let protected_sinf_prefix = child_path(&protected_prefix, SINF);
    let original_format = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Frma>(
            &mut reader,
            Some(trak_info),
            child_path(&protected_sinf_prefix, FRMA),
            "frma",
        )?
        .data_format
    };
    let sinf_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            protected_sinf_prefix.clone(),
            "sinf",
        )?
    };
    let schm = {
        let mut reader = Cursor::new(input);
        extract_optional_single_as::<_, Schm>(
            &mut reader,
            Some(trak_info),
            child_path(&protected_sinf_prefix, SCHM),
            "schm",
        )?
    };
    let is_iaec = matches!(schm, Some(schm) if schm.scheme_type == IAEC);
    if !is_iaec {
        return Ok(None);
    }

    let schi_prefix = child_path(&protected_sinf_prefix, SCHI);
    let isfm = {
        let mut reader = Cursor::new(input);
        extract_single_as::<_, Isfm>(
            &mut reader,
            Some(trak_info),
            child_path(&schi_prefix, FourCc::from_bytes(*b"iSFM")),
            "iSFM",
        )?
    };
    if isfm.iv_length > 8 {
        return Err(invalid_layout(format!(
            "track {} uses unsupported IAEC IV length {}",
            track_layout.track_id, isfm.iv_length
        )));
    }
    let islt = {
        let mut reader = Cursor::new(input);
        extract_optional_single_as::<_, Islt>(
            &mut reader,
            Some(trak_info),
            child_path(&schi_prefix, FourCc::from_bytes(*b"iSLT")),
            "iSLT",
        )?
    };

    Ok(Some(IaecProtectedMovieTrackState {
        track_id: track_layout.track_id,
        trak_info: track_layout.trak_info,
        mdia_info: track_layout.mdia_info,
        minf_info: track_layout.minf_info,
        stbl_info: track_layout.stbl_info,
        stsd_info,
        sample_entry_info,
        original_format,
        sinf_info,
        stsz_info: {
            let mut reader = Cursor::new(input);
            extract_single_info(
                &mut reader,
                Some(trak_info),
                BoxPath::from([MDIA, MINF, STBL, STSZ]),
                "stsz",
            )?
        },
        stsz: {
            let mut reader = Cursor::new(input);
            extract_single_as::<_, Stsz>(
                &mut reader,
                Some(trak_info),
                BoxPath::from([MDIA, MINF, STBL, STSZ]),
                "stsz",
            )?
        },
        stsc: track_layout.stsc,
        chunk_offsets: track_layout.chunk_offsets,
        sample_sizes: track_layout.sample_sizes,
        isfm,
        islt,
    }))
}

fn decrypt_iaec_sample_entry_payload(
    isfm: &Isfm,
    islt: Option<&Islt>,
    key: [u8; 16],
    sample_bytes: &[u8],
) -> Result<Vec<u8>, DecryptRewriteError> {
    if sample_bytes.is_empty() {
        return Err(invalid_layout(
            "IAEC sample payload must not be empty".to_owned(),
        ));
    }

    let selective_header_len = if isfm.selective_encryption { 1 } else { 0 };
    let mut payload_start = 0usize;
    let is_encrypted = if isfm.selective_encryption {
        payload_start = 1;
        (sample_bytes[0] & 0x80) != 0
    } else {
        true
    };

    let header_size = selective_header_len
        + if is_encrypted {
            usize::from(isfm.iv_length) + usize::from(isfm.key_indicator_length)
        } else {
            0
        };
    if header_size > sample_bytes.len() {
        return Err(invalid_layout(
            "IAEC sample payload is shorter than its declared header".to_owned(),
        ));
    }

    if !is_encrypted {
        return Ok(sample_bytes[selective_header_len..].to_vec());
    }

    let iv_end = payload_start + usize::from(isfm.iv_length);
    let iv_bytes = &sample_bytes[payload_start..iv_end];
    payload_start = iv_end;

    let mut indicator_cursor = payload_start;
    let mut remaining_indicator_bytes = usize::from(isfm.key_indicator_length);
    while remaining_indicator_bytes > 4 {
        remaining_indicator_bytes -= 1;
        indicator_cursor += 1;
    }
    let mut key_indicator = 0u32;
    for byte in &sample_bytes[indicator_cursor..indicator_cursor + remaining_indicator_bytes] {
        key_indicator = (key_indicator << 8) | u32::from(*byte);
    }
    if key_indicator != 0 {
        return Err(invalid_layout(format!(
            "IAEC key indicators other than 0 are not supported yet (resolved {key_indicator})"
        )));
    }

    let payload = &sample_bytes[header_size..];
    let salt = islt.map(|entry| entry.salt).unwrap_or([0u8; 8]);
    decrypt_iaec_payload(payload, key, salt, iv_bytes)
}

fn decrypt_iaec_payload(
    payload: &[u8],
    key: [u8; 16],
    salt: [u8; 8],
    iv_bytes: &[u8],
) -> Result<Vec<u8>, DecryptRewriteError> {
    if iv_bytes.len() > 8 {
        return Err(invalid_layout(
            "IAEC currently supports IV lengths up to 8 bytes".to_owned(),
        ));
    }

    let aes = Aes128::new(&key.into());
    let mut byte_stream_offset_bytes = [0u8; 8];
    byte_stream_offset_bytes[8 - iv_bytes.len()..].copy_from_slice(iv_bytes);
    let mut byte_stream_offset = u64::from_be_bytes(byte_stream_offset_bytes);

    let mut output = vec![0u8; payload.len()];
    let mut cursor = 0usize;
    if !payload.is_empty() && !byte_stream_offset.is_multiple_of(16) {
        let offset = usize::try_from(byte_stream_offset % 16).unwrap();
        let counter_block = iaec_counter_block(salt, byte_stream_offset / 16);
        let mut keystream_block = Block::<Aes128>::default();
        keystream_block.copy_from_slice(&counter_block);
        aes.encrypt_block(&mut keystream_block);
        let chunk_len = (16 - offset).min(payload.len());
        for index in 0..chunk_len {
            output[index] = payload[index] ^ keystream_block[offset + index];
        }
        cursor += chunk_len;
        byte_stream_offset += chunk_len as u64;
    }

    while cursor < payload.len() {
        let mut counter_block = Block::<Aes128>::default();
        counter_block.copy_from_slice(&iaec_counter_block(salt, byte_stream_offset / 16));
        aes.encrypt_block(&mut counter_block);
        let chunk_len = 16.min(payload.len() - cursor);
        for index in 0..chunk_len {
            output[cursor + index] = payload[cursor + index] ^ counter_block[index];
        }
        cursor += chunk_len;
        byte_stream_offset += chunk_len as u64;
    }

    Ok(output)
}

fn iaec_counter_block(salt: [u8; 8], block_offset: u64) -> [u8; 16] {
    let mut counter = [0u8; 16];
    counter[..8].copy_from_slice(&salt);
    counter[8..].copy_from_slice(&block_offset.to_be_bytes());
    counter
}

fn analyze_init_segment(input: &[u8]) -> Result<InitDecryptContext, DecryptRewriteError> {
    let mut reader = Cursor::new(input);
    let moovs = extract_box(&mut reader, None, BoxPath::from([MOOV]))?;
    if moovs.len() != 1 {
        return Err(invalid_layout(format!(
            "expected exactly one moov box but found {}",
            moovs.len()
        )));
    }

    let mut reader = Cursor::new(input);
    let trexes = extract_box_as::<_, Trex>(&mut reader, None, BoxPath::from([MOOV, MVEX, TREX]))?;
    let trex_by_track = trexes
        .into_iter()
        .map(|trex| (trex.track_id, trex))
        .collect::<BTreeMap<_, _>>();

    let mut reader = Cursor::new(input);
    let traks = extract_box(&mut reader, None, BoxPath::from([MOOV, TRAK]))?;
    let mut tracks = Vec::new();
    for trak in traks {
        if let Some(track) = analyze_protected_track(input, &trak, &trex_by_track)? {
            tracks.push(track);
        }
    }

    Ok(InitDecryptContext {
        moov_info: moovs[0],
        tracks,
    })
}

fn analyze_protected_track(
    input: &[u8],
    trak_info: &BoxInfo,
    trex_by_track: &BTreeMap<u32, Trex>,
) -> Result<Option<ProtectedTrackState>, DecryptRewriteError> {
    let mut reader = Cursor::new(input);
    let tkhd = extract_single_as::<_, Tkhd>(
        &mut reader,
        Some(trak_info),
        BoxPath::from([TKHD]),
        "trak/tkhd",
    )?;

    let mdia_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(&mut reader, Some(trak_info), BoxPath::from([MDIA]), "mdia")?
    };
    let minf_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF]),
            "minf",
        )?
    };
    let stbl_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL]),
            "stbl",
        )?
    };
    let stsd_info = {
        let mut reader = Cursor::new(input);
        extract_single_info(
            &mut reader,
            Some(trak_info),
            BoxPath::from([MDIA, MINF, STBL, STSD]),
            "stsd",
        )?
    };
    let protected_sample_entries =
        analyze_protected_sample_entries(input, tkhd.track_id, stsd_info)?;
    if protected_sample_entries.is_empty() {
        return Ok(None);
    }

    Ok(Some(ProtectedTrackState {
        track_id: tkhd.track_id,
        trak_info: *trak_info,
        mdia_info,
        minf_info,
        stbl_info,
        stsd_info,
        protected_sample_entries,
        trex: trex_by_track.get(&tkhd.track_id).cloned(),
    }))
}

fn analyze_protected_sample_entries(
    input: &[u8],
    track_id: u32,
    stsd_info: BoxInfo,
) -> Result<Vec<ProtectedSampleEntryState>, DecryptRewriteError> {
    let mut reader = Cursor::new(input);
    let sample_entry_infos =
        extract_box(&mut reader, Some(&stsd_info), BoxPath::from([FourCc::ANY]))?;
    let mut protected_sample_entries = Vec::new();

    for (index, sample_entry_info) in sample_entry_infos.iter().copied().enumerate() {
        let sample_entry_type = sample_entry_info.box_type();
        if sample_entry_type != ENCV && sample_entry_type != ENCA {
            continue;
        }

        let sample_description_index = u32::try_from(index + 1).map_err(|_| {
            invalid_layout(format!(
                "track {track_id} sample-description index does not fit in u32"
            ))
        })?;
        let original_format = {
            let mut reader = Cursor::new(input);
            extract_single_as::<_, Frma>(
                &mut reader,
                Some(&sample_entry_info),
                BoxPath::from([SINF, FRMA]),
                "frma",
            )?
            .data_format
        };
        let scheme_type = {
            let mut reader = Cursor::new(input);
            extract_single_as::<_, Schm>(
                &mut reader,
                Some(&sample_entry_info),
                BoxPath::from([SINF, SCHM]),
                "schm",
            )?
            .scheme_type
        };
        let sinf_info = {
            let mut reader = Cursor::new(input);
            extract_single_info(
                &mut reader,
                Some(&sample_entry_info),
                BoxPath::from([SINF]),
                "sinf",
            )?
        };
        let (tenc, piff_protection_mode) = extract_track_encryption_box(input, &sample_entry_info)?;

        protected_sample_entries.push(ProtectedSampleEntryState {
            sample_description_index,
            sample_entry_info,
            original_format,
            scheme_type,
            sinf_info,
            tenc,
            piff_protection_mode,
        });
    }

    if protected_sample_entries.len() > 1 {
        let incompatible_types = protected_sample_entries.iter().any(|entry| {
            entry.sample_entry_info.box_type()
                != protected_sample_entries[0].sample_entry_info.box_type()
        });
        if incompatible_types {
            return Err(invalid_layout(format!(
                "track {track_id} mixes incompatible protected sample-entry types under one stsd"
            )));
        }
    }

    Ok(protected_sample_entries)
}

fn extract_track_encryption_box(
    input: &[u8],
    sample_entry_info: &BoxInfo,
) -> Result<(Tenc, Option<u8>), DecryptRewriteError> {
    let mut reader = Cursor::new(input);
    if let Some(tenc) = extract_optional_single_as::<_, Tenc>(
        &mut reader,
        Some(sample_entry_info),
        BoxPath::from([SINF, SCHI, TENC]),
        "tenc",
    )? {
        return Ok((tenc, None));
    }

    let mut reader = Cursor::new(input);
    let uuid_boxes = extract_box_as::<_, Uuid>(
        &mut reader,
        Some(sample_entry_info),
        BoxPath::from([SINF, SCHI, UUID]),
    )?;
    let mut matches = uuid_boxes
        .into_iter()
        .filter(|uuid| uuid.user_type == PIFF_TRACK_ENCRYPTION_USER_TYPE);

    let Some(uuid_box) = matches.next() else {
        return Err(invalid_layout(
            "expected one track encryption box under the protected sample entry".to_owned(),
        ));
    };
    if matches.next().is_some() {
        return Err(invalid_layout(
            "expected at most one PIFF UUID track encryption box under the protected sample entry"
                .to_owned(),
        ));
    }

    decode_piff_track_encryption(uuid_box)
}

fn decode_piff_track_encryption(uuid: Uuid) -> Result<(Tenc, Option<u8>), DecryptRewriteError> {
    let UuidPayload::Raw(payload) = uuid.payload else {
        return Err(invalid_layout(
            "expected raw PIFF UUID track-encryption payload bytes".to_owned(),
        ));
    };
    if payload.len() < 24 {
        return Err(invalid_layout(
            "PIFF UUID track-encryption payload is too short".to_owned(),
        ));
    }

    let version = payload[0];
    if version != 0 {
        return Err(invalid_layout(format!(
            "PIFF UUID track-encryption payload version {version} is not supported"
        )));
    }
    let flags = u32::from_be_bytes([0, payload[1], payload[2], payload[3]]);
    let reserved = payload[4];
    let second_reserved = payload[5];
    if second_reserved != 0 {
        return Err(invalid_layout(
            "PIFF UUID track-encryption payload reserved byte must be zero".to_owned(),
        ));
    }

    let default_is_protected = payload[6];
    let default_per_sample_iv_size = payload[7];
    let default_kid = payload[8..24].try_into().unwrap();

    let mut tenc = Tenc::default();
    tenc.set_version(version);
    tenc.set_flags(flags);
    tenc.reserved = reserved;
    tenc.default_is_protected = if default_is_protected == 0 { 0 } else { 1 };
    tenc.default_per_sample_iv_size = default_per_sample_iv_size;
    tenc.default_kid = default_kid;

    let mut cursor = 24usize;
    if default_per_sample_iv_size == 0 {
        let Some(&constant_iv_size) = payload.get(cursor) else {
            return Err(invalid_layout(
                "PIFF UUID track-encryption payload is missing its constant IV size".to_owned(),
            ));
        };
        cursor += 1;
        let end = cursor + usize::from(constant_iv_size);
        if end > payload.len() {
            return Err(invalid_layout(
                "PIFF UUID track-encryption payload constant IV is truncated".to_owned(),
            ));
        }
        tenc.default_constant_iv_size = constant_iv_size;
        tenc.default_constant_iv = payload[cursor..end].to_vec();
        cursor = end;
    }

    if cursor != payload.len() {
        return Err(invalid_layout(
            "PIFF UUID track-encryption payload has unexpected trailing bytes".to_owned(),
        ));
    }

    Ok((tenc, Some(default_is_protected)))
}

#[derive(Clone)]
struct DirectChildEdit {
    child_info: BoxInfo,
    replacement: Option<Vec<u8>>,
}

fn relative_box_range(
    parent: BoxInfo,
    child: BoxInfo,
) -> Result<(usize, usize), DecryptRewriteError> {
    let start = child
        .offset()
        .checked_sub(parent.offset())
        .ok_or_else(|| invalid_layout("child box starts before its parent".to_owned()))?;
    let end = start
        .checked_add(child.size())
        .ok_or_else(|| invalid_layout("child box end overflowed u64".to_owned()))?;
    let start = usize::try_from(start)
        .map_err(|_| invalid_layout("relative child offset does not fit in usize".to_owned()))?;
    let end = usize::try_from(end)
        .map_err(|_| invalid_layout("relative child end does not fit in usize".to_owned()))?;
    Ok((start, end))
}

fn rebuild_box_with_child_edits(
    input: &[u8],
    parent: BoxInfo,
    edits: &[DirectChildEdit],
) -> Result<Vec<u8>, DecryptRewriteError> {
    if edits.is_empty() {
        return Ok(slice_box_bytes(input, parent)?.to_vec());
    }

    let parent_bytes = slice_box_bytes(input, parent)?;
    let header_size = usize::try_from(parent.header_size())
        .map_err(|_| invalid_layout("box header size does not fit in usize".to_owned()))?;
    if header_size > parent_bytes.len() {
        return Err(invalid_layout(format!(
            "{} header size exceeds the available parent bytes",
            parent.box_type()
        )));
    }

    let mut sorted_edits = edits.to_vec();
    sorted_edits.sort_by_key(|edit| edit.child_info.offset());

    let mut payload = Vec::new();
    let mut cursor = header_size;
    for edit in &sorted_edits {
        let (start, end) = relative_box_range(parent, edit.child_info)?;
        if start < cursor || end > parent_bytes.len() {
            return Err(invalid_layout(format!(
                "child edit for {} is not aligned within {}",
                edit.child_info.box_type(),
                parent.box_type()
            )));
        }
        payload.extend_from_slice(&parent_bytes[cursor..start]);
        if let Some(replacement) = &edit.replacement {
            payload.extend_from_slice(replacement);
        }
        cursor = end;
    }
    payload.extend_from_slice(&parent_bytes[cursor..]);

    let mut rebuilt = BoxInfo::new(
        parent.box_type(),
        parent
            .header_size()
            .checked_add(u64::try_from(payload.len()).map_err(|_| {
                invalid_layout("rebuilt box payload length does not fit in u64".to_owned())
            })?)
            .ok_or_else(|| invalid_layout("rebuilt box size overflowed u64".to_owned()))?,
    )
    .with_header_size(parent.header_size())
    .encode();
    rebuilt.extend_from_slice(&payload);
    Ok(rebuilt)
}

fn patch_box_type_bytes(bytes: &mut [u8], box_type: FourCc) -> Result<(), DecryptRewriteError> {
    if bytes.len() < 8 {
        return Err(invalid_layout(
            "box bytes are shorter than the standard box header".to_owned(),
        ));
    }
    bytes[4..8].copy_from_slice(box_type.as_bytes());
    Ok(())
}

fn build_common_encryption_track_replacement(
    input: &[u8],
    track: &ProtectedTrackState,
    keys: &[DecryptionKey],
) -> Result<Option<Vec<u8>>, DecryptRewriteError> {
    let mut sample_entry_replacements = BTreeMap::new();
    for sample_entry in &track.protected_sample_entries {
        if resolve_key_for_sample_entry(track, sample_entry, keys)?.is_none()
            || sample_entry.scheme_type == PIFF
        {
            continue;
        }
        sample_entry_replacements.insert(
            sample_entry.sample_entry_info.offset(),
            Some(build_clear_sample_entry_bytes(
                input,
                sample_entry.sample_entry_info,
                sample_entry.original_format,
                sample_entry.sinf_info,
            )?),
        );
    }
    if sample_entry_replacements.is_empty() {
        return Ok(None);
    }

    let stsd_bytes = rebuild_box_with_child_replacements(
        input,
        track.stsd_info,
        &sample_entry_replacements,
        None,
    )?;
    let stbl_bytes = rebuild_box_with_child_edits(
        input,
        track.stbl_info,
        &[DirectChildEdit {
            child_info: track.stsd_info,
            replacement: Some(stsd_bytes),
        }],
    )?;
    let minf_bytes = rebuild_box_with_child_edits(
        input,
        track.minf_info,
        &[DirectChildEdit {
            child_info: track.stbl_info,
            replacement: Some(stbl_bytes),
        }],
    )?;
    let mdia_bytes = rebuild_box_with_child_edits(
        input,
        track.mdia_info,
        &[DirectChildEdit {
            child_info: track.minf_info,
            replacement: Some(minf_bytes),
        }],
    )?;
    rebuild_box_with_child_edits(
        input,
        track.trak_info,
        &[DirectChildEdit {
            child_info: track.mdia_info,
            replacement: Some(mdia_bytes),
        }],
    )
    .map(Some)
}

fn rebuild_common_encryption_moov(
    input: &[u8],
    context: &InitDecryptContext,
    keys: &[DecryptionKey],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let mut track_edits = Vec::new();
    for track in &context.tracks {
        if let Some(replacement) = build_common_encryption_track_replacement(input, track, keys)? {
            track_edits.push(DirectChildEdit {
                child_info: track.trak_info,
                replacement: Some(replacement),
            });
        }
    }
    rebuild_box_with_child_edits(input, context.moov_info, &track_edits)
}

#[derive(Clone)]
struct TrafRewritePlan {
    moof_info: BoxInfo,
    traf_info: BoxInfo,
    tfhd_flags: u32,
    trun_infos: Vec<BoxInfo>,
    truns: Vec<Trun>,
    remove_infos: Vec<BoxInfo>,
}

fn decrypt_media_bytes_with_context(
    media_segment: &[u8],
    context: &InitDecryptContext,
    keys: &[DecryptionKey],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let mut decrypted = media_segment.to_vec();
    if let Some(moof_replacements) =
        build_common_encryption_fragment_replacements(media_segment, &mut decrypted, context, keys)?
    {
        return rebuild_common_encryption_root_bytes(
            media_segment,
            &decrypted,
            None,
            &moof_replacements,
            &BTreeMap::new(),
        );
    }

    decrypt_media_bytes_with_context_legacy(media_segment, context, keys)
}

fn decrypt_media_bytes_with_context_legacy(
    media_segment: &[u8],
    context: &InitDecryptContext,
    keys: &[DecryptionKey],
) -> Result<Vec<u8>, DecryptRewriteError> {
    let mut output = media_segment.to_vec();
    decrypt_media_bytes_in_place_legacy(media_segment, &mut output, context, keys)?;
    Ok(output)
}

fn try_rebuild_common_encryption_file_bytes(
    input: &[u8],
    context: &InitDecryptContext,
    keys: &[DecryptionKey],
) -> Result<Option<Vec<u8>>, DecryptRewriteError> {
    let mut decrypted = input.to_vec();
    let Some(moof_replacements) =
        build_common_encryption_fragment_replacements(input, &mut decrypted, context, keys)?
    else {
        return Ok(None);
    };

    let rebuilt_moov = rebuild_common_encryption_moov(input, context, keys)?;
    let mfra_replacements = build_common_encryption_mfra_replacements(
        input,
        &decrypted,
        Some((context.moov_info.offset(), rebuilt_moov.as_slice())),
        &moof_replacements,
    )?;
    Ok(Some(rebuild_common_encryption_root_bytes(
        input,
        &decrypted,
        Some((context.moov_info.offset(), rebuilt_moov)),
        &moof_replacements,
        &mfra_replacements,
    )?))
}

fn rebuild_common_encryption_root_bytes(
    input: &[u8],
    decrypted: &[u8],
    moov_replacement: Option<(u64, Vec<u8>)>,
    moof_replacements: &BTreeMap<u64, Vec<u8>>,
    extra_root_replacements: &BTreeMap<u64, Vec<u8>>,
) -> Result<Vec<u8>, DecryptRewriteError> {
    let root_boxes = read_root_box_infos(input)?;
    let mut output = Vec::with_capacity(decrypted.len());
    for info in root_boxes {
        if let Some((moov_offset, replacement)) = &moov_replacement
            && info.offset() == *moov_offset
        {
            output.extend_from_slice(replacement);
            continue;
        }
        if let Some(replacement) = extra_root_replacements.get(&info.offset()) {
            output.extend_from_slice(replacement);
            continue;
        }
        if let Some(replacement) = moof_replacements.get(&info.offset()) {
            output.extend_from_slice(replacement);
            continue;
        }
        if info.box_type() == MDAT {
            output.extend_from_slice(slice_box_bytes(decrypted, info)?);
            continue;
        }
        output.extend_from_slice(slice_box_bytes(input, info)?);
    }
    Ok(output)
}

fn refresh_fragmented_top_level_sidx(bytes: Vec<u8>) -> Result<Vec<u8>, DecryptRewriteError> {
    let Some(mut plan) =
        plan_top_level_sidx_update_bytes(&bytes, TopLevelSidxPlanOptions::default()).map_err(
            |error| {
                invalid_layout(format!(
                    "failed to refresh top-level sidx after decrypt rewrite: {error}"
                ))
            },
        )?
    else {
        return Ok(bytes);
    };
    preserve_existing_top_level_sidx_version(&bytes, &mut plan)?;

    apply_top_level_sidx_plan_bytes(&bytes, &plan).map_err(|error| {
        invalid_layout(format!(
            "failed to apply refreshed top-level sidx after decrypt rewrite: {error}"
        ))
    })
}

fn preserve_existing_top_level_sidx_version(
    bytes: &[u8],
    plan: &mut TopLevelSidxPlan,
) -> Result<(), DecryptRewriteError> {
    let existing = match &plan.action {
        TopLevelSidxPlanAction::Replace { existing } => existing,
        TopLevelSidxPlanAction::Insert => return Ok(()),
    };
    let existing_sidx = decode_existing_top_level_sidx(bytes, existing.info)?;
    if existing_sidx.version() != 0 {
        return Ok(());
    }

    let earliest_presentation_time = plan.sidx.earliest_presentation_time();
    if earliest_presentation_time > u64::from(u32::MAX) {
        return Ok(());
    }

    plan.sidx.set_version(0);
    plan.sidx.set_flags(existing_sidx.flags());
    plan.sidx.earliest_presentation_time_v0 =
        u32::try_from(earliest_presentation_time).map_err(|_| {
            invalid_layout(
                "top-level sidx earliest presentation time does not fit version 0".to_owned(),
            )
        })?;
    plan.sidx.first_offset_v0 = 0;
    Ok(())
}

fn decode_existing_top_level_sidx(
    bytes: &[u8],
    info: BoxInfo,
) -> Result<Sidx, DecryptRewriteError> {
    let box_bytes = slice_box_bytes(bytes, info)?;
    let header_size = usize::try_from(info.header_size()).map_err(|_| {
        invalid_layout("existing top-level sidx header size does not fit usize".to_owned())
    })?;
    let payload = box_bytes.get(header_size..).ok_or_else(|| {
        invalid_layout(
            "existing top-level sidx payload does not fit within the input bytes".to_owned(),
        )
    })?;
    let mut decoded = Sidx::default();
    unmarshal(
        &mut Cursor::new(payload),
        info.payload_size().map_err(|error| {
            invalid_layout(format!(
                "failed to read existing top-level sidx payload size before refresh: {error}"
            ))
        })?,
        &mut decoded,
        None,
    )
    .map_err(|error| {
        invalid_layout(format!(
            "failed to decode existing top-level sidx before refresh: {error}"
        ))
    })?;
    Ok(decoded)
}

fn build_common_encryption_fragment_replacements(
    input: &[u8],
    decrypted: &mut [u8],
    context: &InitDecryptContext,
    keys: &[DecryptionKey],
) -> Result<Option<BTreeMap<u64, Vec<u8>>>, DecryptRewriteError> {
    let track_by_id = context
        .tracks
        .iter()
        .map(|track| (track.track_id, track))
        .collect::<BTreeMap<_, _>>();

    let root_boxes = read_root_box_infos(input)?;
    let mdat_ranges = root_boxes
        .iter()
        .copied()
        .filter(|info| info.box_type() == MDAT)
        .map(|info| MediaDataRange {
            start: info.offset() + info.header_size(),
            end: info.offset() + info.size(),
        })
        .collect::<Vec<_>>();
    let moofs = root_boxes
        .iter()
        .copied()
        .filter(|info| info.box_type() == MOOF)
        .collect::<Vec<_>>();

    let mut reader = Cursor::new(input);
    let trafs = extract_box(&mut reader, None, BoxPath::from([MOOF, TRAF]))?;
    let mut plans = Vec::new();
    for traf_info in trafs {
        let Some(moof_info) = moofs
            .iter()
            .copied()
            .find(|moof| contains_box(*moof, traf_info))
        else {
            return Err(invalid_layout(format!(
                "traf at offset {} is not contained by any moof",
                traf_info.offset()
            )));
        };

        let mut reader = Cursor::new(input);
        let tfhd = extract_single_as::<_, Tfhd>(
            &mut reader,
            Some(&traf_info),
            BoxPath::from([TFHD]),
            "tfhd",
        )?;

        let mut reader = Cursor::new(input);
        let truns =
            extract_box_as::<_, Trun>(&mut reader, Some(&traf_info), BoxPath::from([TRUN]))?;
        let mut reader = Cursor::new(input);
        let trun_infos = extract_box(&mut reader, Some(&traf_info), BoxPath::from([TRUN]))?;
        if truns.is_empty() || truns.len() != trun_infos.len() {
            return Err(invalid_layout(format!(
                "track {} requires one or more aligned trun boxes",
                tfhd.track_id
            )));
        }

        let mut remove_infos = Vec::new();
        if let Some(track) = track_by_id.get(&tfhd.track_id).copied() {
            let sample_description_index = resolve_fragment_sample_description_index(track, &tfhd)?;
            if let Some(active) =
                activate_track_sample_entry(track, sample_description_index, keys)?
            {
                let (senc, senc_info) = extract_fragment_sample_encryption_box(
                    input,
                    &traf_info,
                    &active.sample_entry.tenc,
                )?;

                let mut reader = Cursor::new(input);
                let saiz = extract_optional_single_as::<_, Saiz>(
                    &mut reader,
                    Some(&traf_info),
                    BoxPath::from([SAIZ]),
                    "saiz",
                )?;
                let mut reader = Cursor::new(input);
                let saio = extract_optional_single_as::<_, Saio>(
                    &mut reader,
                    Some(&traf_info),
                    BoxPath::from([SAIO]),
                    "saio",
                )?;
                let mut reader = Cursor::new(input);
                let sgpd_entries = extract_box_as::<_, Sgpd>(
                    &mut reader,
                    Some(&traf_info),
                    BoxPath::from([SGPD]),
                )?;
                let mut reader = Cursor::new(input);
                let sgpd_infos = extract_box(&mut reader, Some(&traf_info), BoxPath::from([SGPD]))?;
                let mut reader = Cursor::new(input);
                let sbgp_entries = extract_box_as::<_, Sbgp>(
                    &mut reader,
                    Some(&traf_info),
                    BoxPath::from([SBGP]),
                )?;
                let mut reader = Cursor::new(input);
                let sbgp_infos = extract_box(&mut reader, Some(&traf_info), BoxPath::from([SBGP]))?;

                let sgpd = select_seig_sgpd(&sgpd_entries);
                let sbgp = select_seig_sbgp(&sbgp_entries);
                let resolved = resolve_sample_encryption(
                    &senc,
                    SampleEncryptionContext {
                        tenc: Some(&active.sample_entry.tenc),
                        sgpd,
                        sbgp,
                        saiz: saiz.as_ref(),
                    },
                )?;

                let sample_spans = compute_sample_spans(
                    &tfhd,
                    active.track.trex.as_ref(),
                    moof_info.offset(),
                    &truns,
                    &trun_infos,
                )?;
                if sample_spans.len() != resolved.samples.len() {
                    return Err(invalid_layout(format!(
                        "track {} resolved {} encrypted sample records but {} sample span(s)",
                        active.track.track_id,
                        resolved.samples.len(),
                        sample_spans.len()
                    )));
                }

                for (sample, span) in resolved.samples.iter().zip(sample_spans.iter()) {
                    let encrypted = read_sample_range(input, &mdat_ranges, span.offset, span.size)
                        .ok_or(DecryptRewriteError::SampleDataRangeNotFound {
                            track_id: active.track.track_id,
                            sample_index: sample.sample_index,
                            absolute_offset: span.offset,
                            sample_size: span.size,
                        })?;
                    let clear = decrypt_sample_for_active_track(&active, sample, encrypted)?;
                    write_sample_range(decrypted, &mdat_ranges, span.offset, &clear).ok_or(
                        DecryptRewriteError::SampleDataRangeNotFound {
                            track_id: active.track.track_id,
                            sample_index: sample.sample_index,
                            absolute_offset: span.offset,
                            sample_size: span.size,
                        },
                    )?;
                }

                if active.sample_entry.scheme_type == PIFF {
                    plans.push(TrafRewritePlan {
                        moof_info,
                        traf_info,
                        tfhd_flags: tfhd.flags(),
                        trun_infos,
                        truns,
                        remove_infos,
                    });
                    continue;
                }

                remove_infos.push(senc_info);
                if let Some(saiz_info) =
                    extract_optional_single_info_from_infos(&traf_info, SAIZ, input)?
                {
                    remove_infos.push(saiz_info);
                }
                if let Some(saio_info) =
                    extract_optional_single_info_from_infos(&traf_info, SAIO, input)?
                    && saio.as_ref().is_none_or(|saio| {
                        saio.aux_info_type == FourCc::ANY
                            || saio.aux_info_type == active.sample_entry.scheme_type
                    })
                {
                    remove_infos.push(saio_info);
                }
                for (entry, info) in sbgp_entries.iter().zip(sbgp_infos.iter().copied()) {
                    if entry.grouping_type == u32::from_be_bytes(*b"seig") {
                        remove_infos.push(info);
                    }
                }
                for (entry, info) in sgpd_entries.iter().zip(sgpd_infos.iter().copied()) {
                    if entry.grouping_type == SEIG {
                        remove_infos.push(info);
                    }
                }
            }
        }

        plans.push(TrafRewritePlan {
            moof_info,
            traf_info,
            tfhd_flags: tfhd.flags(),
            trun_infos,
            truns,
            remove_infos,
        });
    }

    let mut moof_replacements = BTreeMap::new();
    for moof_info in &moofs {
        let moof_plans = plans
            .iter()
            .filter(|plan| plan.moof_info.offset() == moof_info.offset())
            .collect::<Vec<_>>();
        if moof_plans.is_empty() {
            continue;
        }

        let removed_in_moof = moof_plans
            .iter()
            .flat_map(|plan| plan.remove_infos.iter())
            .try_fold(0_u64, |acc, info| {
                acc.checked_add(info.size()).ok_or_else(|| {
                    invalid_layout("removed fragment metadata size overflowed u64".to_owned())
                })
            })?;

        if removed_in_moof != 0
            && moof_plans.iter().any(|plan| {
                plan.tfhd_flags & TFHD_BASE_DATA_OFFSET_PRESENT != 0
                    || plan
                        .truns
                        .iter()
                        .any(|trun| trun.flags() & TRUN_DATA_OFFSET_PRESENT == 0)
            })
        {
            return Ok(None);
        }

        let mut traf_edits = Vec::new();
        for plan in moof_plans {
            let mut child_edits = Vec::new();
            for (trun_info, trun) in plan.trun_infos.iter().copied().zip(plan.truns.iter()) {
                let mut patched_trun = trun.clone();
                if removed_in_moof != 0 {
                    let removed = i64::try_from(removed_in_moof).map_err(|_| {
                        invalid_layout(
                            "removed fragment metadata size does not fit in i64".to_owned(),
                        )
                    })?;
                    let patched = i64::from(trun.data_offset)
                        .checked_sub(removed)
                        .ok_or_else(|| {
                            invalid_layout("patched trun data offset overflowed i64".to_owned())
                        })?;
                    patched_trun.data_offset = i32::try_from(patched).map_err(|_| {
                        invalid_layout(format!(
                            "patched trun data offset for traf at {} does not fit in i32",
                            plan.traf_info.offset()
                        ))
                    })?;
                }
                child_edits.push(DirectChildEdit {
                    child_info: trun_info,
                    replacement: Some(encode_box_with_children(&patched_trun, &[])?),
                });
            }
            child_edits.extend(
                plan.remove_infos
                    .iter()
                    .copied()
                    .map(|info| DirectChildEdit {
                        child_info: info,
                        replacement: None,
                    }),
            );

            let rebuilt_traf = rebuild_box_with_child_edits(input, plan.traf_info, &child_edits)?;
            if rebuilt_traf != slice_box_bytes(input, plan.traf_info)? {
                traf_edits.push(DirectChildEdit {
                    child_info: plan.traf_info,
                    replacement: Some(rebuilt_traf),
                });
            }
        }

        if !traf_edits.is_empty() {
            moof_replacements.insert(
                moof_info.offset(),
                rebuild_box_with_child_edits(input, *moof_info, &traf_edits)?,
            );
        }
    }

    Ok(Some(moof_replacements))
}

fn build_common_encryption_mfra_replacements(
    input: &[u8],
    decrypted: &[u8],
    moov_replacement: Option<(u64, &[u8])>,
    moof_replacements: &BTreeMap<u64, Vec<u8>>,
) -> Result<BTreeMap<u64, Vec<u8>>, DecryptRewriteError> {
    let root_boxes = read_root_box_infos(input)?;
    let mfra_infos = root_boxes
        .iter()
        .copied()
        .filter(|info| info.box_type() == MFRA)
        .collect::<Vec<_>>();
    if mfra_infos.is_empty() {
        return Ok(BTreeMap::new());
    }

    let rewritten_offsets = compute_rewritten_root_offsets(
        input,
        decrypted,
        &root_boxes,
        moov_replacement,
        moof_replacements,
        &BTreeMap::new(),
    )?;

    let mut replacements = BTreeMap::new();
    for mfra_info in mfra_infos {
        let mut reader = Cursor::new(input);
        let tfra_boxes =
            extract_box_as::<_, Tfra>(&mut reader, Some(&mfra_info), BoxPath::from([TFRA]))?;
        let mut reader = Cursor::new(input);
        let tfra_infos = extract_box(&mut reader, Some(&mfra_info), BoxPath::from([TFRA]))?;
        if tfra_boxes.len() != tfra_infos.len() {
            return Err(invalid_layout(
                "expected aligned tfra boxes inside mfra for Common Encryption rewrite".to_owned(),
            ));
        }

        let mut child_edits = Vec::new();
        for (tfra_info, tfra_box) in tfra_infos.iter().copied().zip(tfra_boxes) {
            let mut patched_tfra = tfra_box.clone();
            let version = patched_tfra.version();
            let mut changed = false;
            for entry in &mut patched_tfra.entries {
                let original_moof_offset = if version == 0 {
                    u64::from(entry.moof_offset_v0)
                } else {
                    entry.moof_offset_v1
                };
                let Some(&rewritten_moof_offset) = rewritten_offsets.get(&original_moof_offset)
                else {
                    continue;
                };

                if version == 0 {
                    let rewritten_moof_offset =
                        u32::try_from(rewritten_moof_offset).map_err(|_| {
                            invalid_layout(
                                "rewritten tfra moof offset does not fit in u32".to_owned(),
                            )
                        })?;
                    if entry.moof_offset_v0 != rewritten_moof_offset {
                        entry.moof_offset_v0 = rewritten_moof_offset;
                        changed = true;
                    }
                } else if entry.moof_offset_v1 != rewritten_moof_offset {
                    entry.moof_offset_v1 = rewritten_moof_offset;
                    changed = true;
                }
            }
            if changed {
                child_edits.push(DirectChildEdit {
                    child_info: tfra_info,
                    replacement: Some(encode_box_with_children(&patched_tfra, &[])?),
                });
            }
        }

        let mut rebuilt_mfra = rebuild_box_with_child_edits(input, mfra_info, &child_edits)?;
        if let Some(mfro_info) = extract_optional_single_info_from_infos(&mfra_info, MFRO, input)? {
            let mut reader = Cursor::new(input);
            let Some(mut mfro) = extract_optional_single_as::<_, Mfro>(
                &mut reader,
                Some(&mfra_info),
                BoxPath::from([MFRO]),
                "mfro",
            )?
            else {
                return Err(invalid_layout(
                    "expected mfro to decode when its box info is present".to_owned(),
                ));
            };
            mfro.size = u32::try_from(rebuilt_mfra.len()).map_err(|_| {
                invalid_layout("rewritten mfra size does not fit in u32".to_owned())
            })?;
            let mfro_replacement = encode_box_with_children(&mfro, &[])?;
            rebuilt_mfra = rebuild_box_with_child_edits(
                input,
                mfra_info,
                &[
                    child_edits,
                    vec![DirectChildEdit {
                        child_info: mfro_info,
                        replacement: Some(mfro_replacement),
                    }],
                ]
                .concat(),
            )?;
        }

        if rebuilt_mfra != slice_box_bytes(input, mfra_info)? {
            replacements.insert(mfra_info.offset(), rebuilt_mfra);
        }
    }

    Ok(replacements)
}

fn compute_rewritten_root_offsets(
    input: &[u8],
    decrypted: &[u8],
    root_boxes: &[BoxInfo],
    moov_replacement: Option<(u64, &[u8])>,
    moof_replacements: &BTreeMap<u64, Vec<u8>>,
    extra_root_replacements: &BTreeMap<u64, Vec<u8>>,
) -> Result<BTreeMap<u64, u64>, DecryptRewriteError> {
    let mut next_offset = 0_u64;
    let mut offsets = BTreeMap::new();
    for info in root_boxes {
        offsets.insert(info.offset(), next_offset);
        next_offset = next_offset
            .checked_add(rewritten_root_box_size(
                input,
                decrypted,
                *info,
                moov_replacement,
                moof_replacements,
                extra_root_replacements,
            )?)
            .ok_or_else(|| invalid_layout("rewritten root offset overflowed u64".to_owned()))?;
    }
    Ok(offsets)
}

fn rewritten_root_box_size(
    input: &[u8],
    decrypted: &[u8],
    info: BoxInfo,
    moov_replacement: Option<(u64, &[u8])>,
    moof_replacements: &BTreeMap<u64, Vec<u8>>,
    extra_root_replacements: &BTreeMap<u64, Vec<u8>>,
) -> Result<u64, DecryptRewriteError> {
    if let Some((moov_offset, replacement)) = moov_replacement
        && info.offset() == moov_offset
    {
        return u64::try_from(replacement.len())
            .map_err(|_| invalid_layout("rebuilt moov size does not fit in u64".to_owned()));
    }
    if let Some(replacement) = extra_root_replacements.get(&info.offset()) {
        return u64::try_from(replacement.len()).map_err(|_| {
            invalid_layout("rewritten root replacement size does not fit in u64".to_owned())
        });
    }
    if let Some(replacement) = moof_replacements.get(&info.offset()) {
        return u64::try_from(replacement.len())
            .map_err(|_| invalid_layout("rebuilt moof size does not fit in u64".to_owned()));
    }
    if info.box_type() == MDAT {
        return u64::try_from(slice_box_bytes(decrypted, info)?.len())
            .map_err(|_| invalid_layout("rewritten mdat size does not fit in u64".to_owned()));
    }
    u64::try_from(slice_box_bytes(input, info)?.len())
        .map_err(|_| invalid_layout("root box size does not fit in u64".to_owned()))
}

fn decrypt_media_bytes_in_place_legacy(
    input: &[u8],
    output: &mut [u8],
    context: &InitDecryptContext,
    keys: &[DecryptionKey],
) -> Result<(), DecryptRewriteError> {
    let track_by_id = context
        .tracks
        .iter()
        .map(|track| (track.track_id, track))
        .collect::<BTreeMap<_, _>>();

    let mut reader = Cursor::new(input);
    let mdat_infos = extract_box(&mut reader, None, BoxPath::from([MDAT]))?;
    let mdat_ranges = mdat_infos
        .into_iter()
        .map(|info| MediaDataRange {
            start: info.offset() + info.header_size(),
            end: info.offset() + info.size(),
        })
        .collect::<Vec<_>>();

    let mut reader = Cursor::new(input);
    let moofs = extract_box(&mut reader, None, BoxPath::from([MOOF]))?;
    let mut reader = Cursor::new(input);
    let trafs = extract_box(&mut reader, None, BoxPath::from([MOOF, TRAF]))?;
    for traf_info in trafs {
        let Some(moof_info) = moofs
            .iter()
            .copied()
            .find(|moof| contains_box(*moof, traf_info))
        else {
            return Err(invalid_layout(format!(
                "traf at offset {} is not contained by any moof",
                traf_info.offset()
            )));
        };

        let mut reader = Cursor::new(input);
        let tfhd = extract_single_as::<_, Tfhd>(
            &mut reader,
            Some(&traf_info),
            BoxPath::from([TFHD]),
            "tfhd",
        )?;
        let Some(track) = track_by_id.get(&tfhd.track_id).copied() else {
            continue;
        };
        let sample_description_index = resolve_fragment_sample_description_index(track, &tfhd)?;
        let Some(active) = activate_track_sample_entry(track, sample_description_index, keys)?
        else {
            continue;
        };

        let mut reader = Cursor::new(input);
        let truns =
            extract_box_as::<_, Trun>(&mut reader, Some(&traf_info), BoxPath::from([TRUN]))?;
        let mut reader = Cursor::new(input);
        let trun_infos = extract_box(&mut reader, Some(&traf_info), BoxPath::from([TRUN]))?;
        if truns.is_empty() || truns.len() != trun_infos.len() {
            return Err(invalid_layout(format!(
                "track {} requires one or more aligned trun boxes",
                active.track.track_id
            )));
        }

        let (senc, senc_info) =
            extract_fragment_sample_encryption_box(input, &traf_info, &active.sample_entry.tenc)?;

        let mut reader = Cursor::new(input);
        let saiz = extract_optional_single_as::<_, Saiz>(
            &mut reader,
            Some(&traf_info),
            BoxPath::from([SAIZ]),
            "saiz",
        )?;
        let mut reader = Cursor::new(input);
        let saio = extract_optional_single_as::<_, Saio>(
            &mut reader,
            Some(&traf_info),
            BoxPath::from([SAIO]),
            "saio",
        )?;
        let mut reader = Cursor::new(input);
        let sgpd_entries =
            extract_box_as::<_, Sgpd>(&mut reader, Some(&traf_info), BoxPath::from([SGPD]))?;
        let mut reader = Cursor::new(input);
        let sgpd_infos = extract_box(&mut reader, Some(&traf_info), BoxPath::from([SGPD]))?;
        let mut reader = Cursor::new(input);
        let sbgp_entries =
            extract_box_as::<_, Sbgp>(&mut reader, Some(&traf_info), BoxPath::from([SBGP]))?;
        let mut reader = Cursor::new(input);
        let sbgp_infos = extract_box(&mut reader, Some(&traf_info), BoxPath::from([SBGP]))?;

        let sgpd = select_seig_sgpd(&sgpd_entries);
        let sbgp = select_seig_sbgp(&sbgp_entries);
        let resolved = resolve_sample_encryption(
            &senc,
            SampleEncryptionContext {
                tenc: Some(&active.sample_entry.tenc),
                sgpd,
                sbgp,
                saiz: saiz.as_ref(),
            },
        )?;

        let sample_spans = compute_sample_spans(
            &tfhd,
            active.track.trex.as_ref(),
            moof_info.offset(),
            &truns,
            &trun_infos,
        )?;
        if sample_spans.len() != resolved.samples.len() {
            return Err(invalid_layout(format!(
                "track {} resolved {} encrypted sample records but {} sample span(s)",
                active.track.track_id,
                resolved.samples.len(),
                sample_spans.len()
            )));
        }

        for (sample, span) in resolved.samples.iter().zip(sample_spans.iter()) {
            let encrypted = read_sample_range(input, &mdat_ranges, span.offset, span.size).ok_or(
                DecryptRewriteError::SampleDataRangeNotFound {
                    track_id: active.track.track_id,
                    sample_index: sample.sample_index,
                    absolute_offset: span.offset,
                    sample_size: span.size,
                },
            )?;
            let decrypted = decrypt_sample_for_active_track(&active, sample, encrypted)?;
            write_sample_range(output, &mdat_ranges, span.offset, &decrypted).ok_or(
                DecryptRewriteError::SampleDataRangeNotFound {
                    track_id: active.track.track_id,
                    sample_index: sample.sample_index,
                    absolute_offset: span.offset,
                    sample_size: span.size,
                },
            )?;
        }

        if active.sample_entry.scheme_type == PIFF {
            continue;
        }

        replace_box_with_free(output, senc_info)?;
        if let Some(saiz_info) = extract_optional_single_info_from_infos(&traf_info, SAIZ, input)? {
            replace_box_with_free(output, saiz_info)?;
        }
        if let Some(saio_info) = extract_optional_single_info_from_infos(&traf_info, SAIO, input)?
            && saio.as_ref().is_none_or(|saio| {
                saio.aux_info_type == FourCc::ANY
                    || saio.aux_info_type == active.sample_entry.scheme_type
            })
        {
            replace_box_with_free(output, saio_info)?;
        }
        for (entry, info) in sbgp_entries.iter().zip(sbgp_infos.iter().copied()) {
            if entry.grouping_type == u32::from_be_bytes(*b"seig") {
                replace_box_with_free(output, info)?;
            }
        }
        for (entry, info) in sgpd_entries.iter().zip(sgpd_infos.iter().copied()) {
            if entry.grouping_type == SEIG {
                replace_box_with_free(output, info)?;
            }
        }
    }

    Ok(())
}

#[derive(Clone, Copy)]
struct SampleSpan {
    offset: u64,
    size: u32,
}

fn compute_sample_spans(
    tfhd: &Tfhd,
    trex: Option<&Trex>,
    moof_offset: u64,
    truns: &[Trun],
    trun_infos: &[BoxInfo],
) -> Result<Vec<SampleSpan>, DecryptRewriteError> {
    let base_data_offset = if tfhd.flags() & TFHD_BASE_DATA_OFFSET_PRESENT != 0 {
        tfhd.base_data_offset
    } else {
        moof_offset
    };
    let mut sample_spans = Vec::new();
    let mut next_offset = None::<u64>;
    for (trun, trun_info) in truns.iter().zip(trun_infos.iter()) {
        let mut current_offset = if trun.flags() & TRUN_DATA_OFFSET_PRESENT != 0 {
            let absolute = i128::from(base_data_offset) + i128::from(trun.data_offset);
            if absolute < 0 || absolute > i128::from(u64::MAX) {
                return Err(invalid_layout(format!(
                    "trun at offset {} computed an invalid data offset",
                    trun_info.offset()
                )));
            }
            absolute as u64
        } else if let Some(next_offset) = next_offset {
            next_offset
        } else if tfhd.flags() & TFHD_DEFAULT_BASE_IS_MOOF != 0 {
            moof_offset
        } else {
            base_data_offset
        };

        for sample_index in 0..usize::try_from(trun.sample_count).unwrap_or(0) {
            let sample_size = if trun.flags() & TRUN_SAMPLE_SIZE_PRESENT != 0 {
                trun.entries
                    .get(sample_index)
                    .map(|entry| entry.sample_size)
                    .ok_or_else(|| {
                        invalid_layout(format!(
                            "trun at offset {} is missing sample size entry {}",
                            trun_info.offset(),
                            sample_index + 1
                        ))
                    })?
            } else if tfhd.flags() & TFHD_DEFAULT_SAMPLE_SIZE_PRESENT != 0 {
                tfhd.default_sample_size
            } else if let Some(trex) = trex {
                trex.default_sample_size
            } else {
                return Err(invalid_layout(format!(
                    "track {} sample sizes require tfhd or trex defaults",
                    tfhd.track_id
                )));
            };

            sample_spans.push(SampleSpan {
                offset: current_offset,
                size: sample_size,
            });
            current_offset = current_offset
                .checked_add(u64::from(sample_size))
                .ok_or_else(|| invalid_layout("sample offset overflowed u64".to_string()))?;
        }
        next_offset = Some(current_offset);
    }

    Ok(sample_spans)
}

fn activate_track_sample_entry<'a>(
    track: &'a ProtectedTrackState,
    sample_description_index: u32,
    keys: &[DecryptionKey],
) -> Result<Option<ActiveTrackDecryption<'a>>, DecryptRewriteError> {
    let Some(sample_entry) = resolve_protected_sample_entry(track, sample_description_index)?
    else {
        return Ok(None);
    };
    let Some(key) = resolve_key_for_sample_entry(track, sample_entry, keys)? else {
        return Ok(None);
    };
    let scheme = resolve_sample_entry_scheme(track.track_id, sample_entry)?;

    Ok(Some(ActiveTrackDecryption {
        track,
        sample_entry,
        scheme,
        key,
    }))
}

fn resolve_protected_sample_entry(
    track: &ProtectedTrackState,
    sample_description_index: u32,
) -> Result<Option<&ProtectedSampleEntryState>, DecryptRewriteError> {
    if sample_description_index == 0 {
        return Err(invalid_layout(format!(
            "track {} uses invalid sample-description index 0",
            track.track_id
        )));
    }
    Ok(track
        .protected_sample_entries
        .iter()
        .find(|entry| entry.sample_description_index == sample_description_index))
}

fn resolve_fragment_sample_description_index(
    track: &ProtectedTrackState,
    tfhd: &Tfhd,
) -> Result<u32, DecryptRewriteError> {
    if tfhd.flags() & TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT != 0 {
        return Ok(tfhd.sample_description_index);
    }
    if let Some(trex) = track.trex.as_ref() {
        return Ok(trex.default_sample_description_index);
    }
    if track.protected_sample_entries.len() == 1 {
        return Ok(track.protected_sample_entries[0].sample_description_index);
    }

    Err(invalid_layout(format!(
        "track {} requires tfhd or trex sample-description defaults when multiple protected sample entries are present",
        track.track_id
    )))
}

fn resolve_key_for_sample_entry(
    track: &ProtectedTrackState,
    sample_entry: &ProtectedSampleEntryState,
    keys: &[DecryptionKey],
) -> Result<Option<[u8; 16]>, DecryptRewriteError> {
    if let Some(key) = keys.iter().find_map(|entry| match entry.id {
        DecryptionKeyId::Kid(candidate) if candidate == sample_entry.tenc.default_kid => {
            Some(entry.key)
        }
        _ => None,
    }) {
        return Ok(Some(key));
    }

    let track_keys = keys
        .iter()
        .filter_map(|entry| match entry.id {
            DecryptionKeyId::TrackId(candidate) if candidate == track.track_id => Some(entry.key),
            _ => None,
        })
        .collect::<Vec<_>>();
    let ordered_zero_kid_track_key =
        resolve_ordered_track_key_for_zero_kid_sample_entry(track, sample_entry, &track_keys);
    match track_keys.as_slice() {
        [] => Ok(None),
        [key] => Ok(Some(*key)),
        [first, ..] if track.protected_sample_entries.len() == 1 => Ok(Some(*first)),
        _ if ordered_zero_kid_track_key.is_some() => Ok(ordered_zero_kid_track_key),
        _ => Err(invalid_layout(format!(
            "track {} has multiple track-ID keys but sample-description {} needs per-entry key selection; use KID-addressed keys or provide one ordered track-ID key per zero-KID protected sample entry",
            track.track_id, sample_entry.sample_description_index
        ))),
    }
}

fn resolve_ordered_track_key_for_zero_kid_sample_entry(
    track: &ProtectedTrackState,
    sample_entry: &ProtectedSampleEntryState,
    track_keys: &[[u8; 16]],
) -> Option<[u8; 16]> {
    if sample_entry.tenc.default_kid != [0; 16] {
        return None;
    }

    let zero_kid_entries = track
        .protected_sample_entries
        .iter()
        .filter(|entry| entry.tenc.default_kid == [0; 16])
        .collect::<Vec<_>>();
    if zero_kid_entries.len() != track_keys.len() {
        return None;
    }

    zero_kid_entries
        .iter()
        .position(|entry| entry.sample_description_index == sample_entry.sample_description_index)
        .map(|ordinal| track_keys[ordinal])
}

fn resolve_sample_entry_scheme(
    track_id: u32,
    sample_entry: &ProtectedSampleEntryState,
) -> Result<NativeCommonEncryptionScheme, DecryptRewriteError> {
    if let Some(scheme) = NativeCommonEncryptionScheme::from_scheme_type(sample_entry.scheme_type) {
        return Ok(scheme);
    }
    if sample_entry.scheme_type == PIFF {
        return match sample_entry
            .piff_protection_mode
            .unwrap_or(sample_entry.tenc.default_is_protected)
        {
            1 => Ok(NativeCommonEncryptionScheme::Cenc),
            2 => Ok(NativeCommonEncryptionScheme::Cbc1),
            mode => Err(invalid_layout(format!(
                "track {} uses unsupported PIFF protection mode {}",
                track_id, mode
            ))),
        };
    }

    Err(DecryptRewriteError::UnsupportedTrackSchemeType {
        track_id,
        scheme_type: sample_entry.scheme_type,
    })
}

fn extract_fragment_sample_encryption_box(
    input: &[u8],
    traf_info: &BoxInfo,
    tenc: &Tenc,
) -> Result<(Senc, BoxInfo), DecryptRewriteError> {
    let mut reader = Cursor::new(input);
    let senc_infos = extract_box(&mut reader, Some(traf_info), BoxPath::from([SENC]))?;
    let mut reader = Cursor::new(input);
    let senc_payloads =
        extract_box_payload_bytes(&mut reader, Some(traf_info), BoxPath::from([SENC]))?;
    match (senc_payloads.len(), senc_infos.len()) {
        (1, 1) => {
            let senc = decode_senc_payload_with_iv_size(
                &senc_payloads[0],
                usize::from(tenc.default_per_sample_iv_size),
            )
            .map_err(|error| {
                invalid_layout(format!(
                    "failed to decode sample encryption box with the selected track defaults: {error}"
                ))
            })?;
            return Ok((senc, senc_infos[0]));
        }
        (0, 0) => {}
        _ => {
            return Err(invalid_layout(
                "expected aligned sample encryption boxes inside the track fragment".to_owned(),
            ));
        }
    }

    let mut reader = Cursor::new(input);
    let uuid_boxes =
        extract_box_as::<_, Uuid>(&mut reader, Some(traf_info), BoxPath::from([UUID]))?;
    let mut reader = Cursor::new(input);
    let uuid_infos = extract_box(&mut reader, Some(traf_info), BoxPath::from([UUID]))?;

    let mut match_index = None;
    let mut match_senc = None;
    for (index, uuid_box) in uuid_boxes.into_iter().enumerate() {
        if uuid_box.user_type != UUID_SAMPLE_ENCRYPTION {
            continue;
        }
        let UuidPayload::SampleEncryption(senc) = uuid_box.payload else {
            return Err(invalid_layout(
                "expected typed sample-encryption data in the PIFF UUID sample box".to_owned(),
            ));
        };
        if match_index.is_some() {
            return Err(invalid_layout(
                "expected at most one PIFF UUID sample-encryption box in each track fragment"
                    .to_owned(),
            ));
        }
        match_index = Some(index);
        match_senc = Some(senc);
    }

    match (match_index, match_senc) {
        (Some(index), Some(senc)) => Ok((senc, uuid_infos[index])),
        _ => Err(invalid_layout(
            "expected one sample encryption box inside the protected track fragment".to_owned(),
        )),
    }
}

fn select_seig_sgpd(entries: &[Sgpd]) -> Option<&Sgpd> {
    entries.iter().find(|entry| entry.grouping_type == SEIG)
}

fn select_seig_sbgp(entries: &[Sbgp]) -> Option<&Sbgp> {
    entries
        .iter()
        .find(|entry| entry.grouping_type == u32::from_be_bytes(*b"seig"))
}

fn patch_sample_entry_type(
    bytes: &mut [u8],
    sample_entry_info: BoxInfo,
    original_format: FourCc,
) -> Result<(), DecryptRewriteError> {
    let start = usize::try_from(sample_entry_info.offset())
        .map_err(|_| invalid_layout("sample entry offset does not fit in usize".to_string()))?;
    let type_offset = start
        .checked_add(4)
        .ok_or_else(|| invalid_layout("sample entry offset overflowed".to_string()))?;
    let end = type_offset
        .checked_add(4)
        .ok_or_else(|| invalid_layout("sample entry type offset overflowed".to_string()))?;
    if end > bytes.len() {
        return Err(invalid_layout(
            "sample entry type patch is out of range".to_string(),
        ));
    }
    bytes[type_offset..end].copy_from_slice(original_format.as_bytes());
    Ok(())
}

fn replace_box_with_free(bytes: &mut [u8], info: BoxInfo) -> Result<(), DecryptRewriteError> {
    let start = usize::try_from(info.offset())
        .map_err(|_| invalid_layout("box offset does not fit in usize".to_string()))?;
    let size = usize::try_from(info.size())
        .map_err(|_| invalid_layout("box size does not fit in usize".to_string()))?;
    let end = start
        .checked_add(size)
        .ok_or_else(|| invalid_layout("box end overflowed".to_string()))?;
    if end > bytes.len() {
        return Err(invalid_layout(format!(
            "box replacement for {} exceeds the available buffer",
            info.box_type()
        )));
    }

    let replacement = BoxInfo::new(FREE, info.size())
        .with_header_size(info.header_size())
        .encode();
    if replacement.len() as u64 != info.header_size() {
        return Err(invalid_layout(format!(
            "free replacement header size changed for {}",
            info.box_type()
        )));
    }

    bytes[start..start + replacement.len()].copy_from_slice(&replacement);
    bytes[start + replacement.len()..end].fill(0);
    Ok(())
}

fn read_sample_range<'a>(
    bytes: &'a [u8],
    ranges: &[MediaDataRange],
    absolute_offset: u64,
    sample_size: u32,
) -> Option<&'a [u8]> {
    let size = u64::from(sample_size);
    let end = absolute_offset.checked_add(size)?;
    let range = ranges
        .iter()
        .find(|range| absolute_offset >= range.start && end <= range.end)?;
    let start = usize::try_from(absolute_offset).ok()?;
    let end = usize::try_from(end).ok()?;
    if end > bytes.len() || absolute_offset < range.start || u64::try_from(end).ok()? > range.end {
        return None;
    }
    Some(&bytes[start..end])
}

fn write_sample_range(
    bytes: &mut [u8],
    ranges: &[MediaDataRange],
    absolute_offset: u64,
    sample: &[u8],
) -> Option<()> {
    let end = absolute_offset.checked_add(u64::try_from(sample.len()).ok()?)?;
    let range = ranges
        .iter()
        .find(|range| absolute_offset >= range.start && end <= range.end)?;
    let start = usize::try_from(absolute_offset).ok()?;
    let end = usize::try_from(end).ok()?;
    if end > bytes.len() || absolute_offset < range.start || u64::try_from(end).ok()? > range.end {
        return None;
    }
    bytes[start..end].copy_from_slice(sample);
    Some(())
}

fn contains_box(parent: BoxInfo, child: BoxInfo) -> bool {
    child.offset() >= parent.offset()
        && child.offset() + child.size() <= parent.offset() + parent.size()
}

fn extract_single_as<R, T>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    path: BoxPath,
    label: &'static str,
) -> Result<T, DecryptRewriteError>
where
    R: std::io::Read + std::io::Seek,
    T: crate::codec::CodecBox + Clone + 'static,
{
    let mut values = extract_box_as::<_, T>(reader, parent, path)?;
    if values.len() != 1 {
        return Err(invalid_layout(format!(
            "expected exactly one {label} box but found {}",
            values.len()
        )));
    }
    Ok(values.remove(0))
}

fn extract_optional_single_as<R, T>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    path: BoxPath,
    label: &'static str,
) -> Result<Option<T>, DecryptRewriteError>
where
    R: std::io::Read + std::io::Seek,
    T: crate::codec::CodecBox + Clone + 'static,
{
    let mut values = extract_box_as::<_, T>(reader, parent, path)?;
    if values.len() > 1 {
        return Err(invalid_layout(format!(
            "expected at most one {label} box but found {}",
            values.len()
        )));
    }
    Ok(values.pop())
}

fn extract_single_info<R>(
    reader: &mut R,
    parent: Option<&BoxInfo>,
    path: BoxPath,
    label: &'static str,
) -> Result<BoxInfo, DecryptRewriteError>
where
    R: std::io::Read + std::io::Seek,
{
    let mut infos = extract_box(reader, parent, path)?;
    if infos.len() != 1 {
        return Err(invalid_layout(format!(
            "expected exactly one {label} box but found {}",
            infos.len()
        )));
    }
    Ok(infos.remove(0))
}

fn extract_optional_single_info_from_infos(
    parent: &BoxInfo,
    box_type: FourCc,
    input: &[u8],
) -> Result<Option<BoxInfo>, DecryptRewriteError> {
    let mut reader = Cursor::new(input);
    let mut infos = extract_box(&mut reader, Some(parent), BoxPath::from([box_type]))?;
    if infos.len() > 1 {
        return Err(invalid_layout(format!(
            "expected at most one {} box but found {}",
            box_type,
            infos.len()
        )));
    }
    Ok(infos.pop())
}

fn child_path(path: &BoxPath, child: FourCc) -> BoxPath {
    path.iter().copied().chain(std::iter::once(child)).collect()
}

fn invalid_layout(reason: impl Into<String>) -> DecryptRewriteError {
    DecryptRewriteError::InvalidLayout {
        reason: reason.into(),
    }
}

fn parse_hex_16(field: &'static str, input: &str) -> Result<[u8; 16], ParseDecryptionKeyError> {
    if input.len() != 32 {
        return Err(ParseDecryptionKeyError::InvalidHexLength {
            field,
            actual: input.len(),
        });
    }

    let bytes = input.as_bytes();
    let mut output = [0_u8; 16];
    for (index, chunk) in bytes.chunks_exact(2).enumerate() {
        let high = decode_hex_nibble(field, index, chunk[0] as char)?;
        let low = decode_hex_nibble(field, index, chunk[1] as char)?;
        output[index] = (high << 4) | low;
    }

    Ok(output)
}

fn decode_hex_nibble(
    field: &'static str,
    index: usize,
    value: char,
) -> Result<u8, ParseDecryptionKeyError> {
    match value {
        '0'..='9' => Ok((value as u8) - b'0'),
        'a'..='f' => Ok((value as u8) - b'a' + 10),
        'A'..='F' => Ok((value as u8) - b'A' + 10),
        _ => Err(ParseDecryptionKeyError::InvalidHexDigit {
            field,
            index,
            value,
        }),
    }
}

fn encode_hex(bytes: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(32);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn effective_initialization_vector(
    scheme: NativeCommonEncryptionScheme,
    sample: &ResolvedSampleEncryptionSample<'_>,
) -> Result<[u8; 16], CommonEncryptionDecryptError> {
    let bytes = sample.effective_initialization_vector();
    if bytes.is_empty() {
        return Err(CommonEncryptionDecryptError::MissingInitializationVector { scheme });
    }

    let expected = if scheme.uses_cbc() {
        "exactly 16"
    } else {
        "8 or 16"
    };
    match (scheme.uses_cbc(), bytes.len()) {
        (true, 16) | (false, 8 | 16) => {}
        _ => {
            return Err(
                CommonEncryptionDecryptError::InvalidInitializationVectorSize {
                    scheme,
                    actual: bytes.len(),
                    expected,
                },
            );
        }
    }

    let mut iv = [0_u8; 16];
    iv[..bytes.len()].copy_from_slice(bytes);
    Ok(iv)
}

struct SampleTransformer {
    crypt_byte_block: u8,
    skip_byte_block: u8,
    pattern_stream_offset: u64,
    cipher: SampleCipher,
}

impl SampleTransformer {
    fn new(
        scheme: NativeCommonEncryptionScheme,
        aes: Aes128,
        iv: [u8; 16],
        crypt_byte_block: u8,
        skip_byte_block: u8,
    ) -> Self {
        Self {
            crypt_byte_block,
            skip_byte_block,
            pattern_stream_offset: 0,
            cipher: if scheme.uses_cbc() {
                SampleCipher::Cbc {
                    aes,
                    iv,
                    chain_block: iv,
                }
            } else {
                SampleCipher::Ctr {
                    aes,
                    iv,
                    encrypted_offset: 0,
                }
            },
        }
    }

    fn reset_for_subsample(&mut self) {
        self.pattern_stream_offset = 0;
        self.cipher.reset();
    }

    fn transform_region(
        &mut self,
        encrypted_region: &[u8],
        output_region: &mut [u8],
    ) -> Result<(), CommonEncryptionDecryptError> {
        if encrypted_region.len() != output_region.len() {
            return Err(CommonEncryptionDecryptError::InvalidProtectedRegion {
                remaining: encrypted_region.len(),
                clear_bytes: 0,
                protected_bytes: output_region.len(),
            });
        }
        if self.crypt_byte_block != 0 && self.skip_byte_block != 0 {
            self.transform_pattern_region(encrypted_region, output_region);
        } else {
            self.cipher
                .process_encrypted_chunk(encrypted_region, output_region);
        }
        Ok(())
    }

    fn transform_pattern_region(&mut self, encrypted_region: &[u8], output_region: &mut [u8]) {
        let pattern_span = usize::from(self.crypt_byte_block) + usize::from(self.skip_byte_block);
        let mut cursor = 0usize;
        while cursor < encrypted_region.len() {
            let block_position =
                usize::try_from(self.pattern_stream_offset / 16).unwrap_or(usize::MAX);
            let pattern_position = block_position % pattern_span;

            let mut crypt_size = 0usize;
            let mut skip_size = usize::from(self.skip_byte_block) * 16;
            if pattern_position < usize::from(self.crypt_byte_block) {
                crypt_size = (usize::from(self.crypt_byte_block) - pattern_position) * 16;
            } else {
                skip_size = (pattern_span - pattern_position) * 16;
            }

            let remain = encrypted_region.len() - cursor;
            if crypt_size > remain {
                crypt_size = 16 * (remain / 16);
                skip_size = remain - crypt_size;
            }
            if crypt_size + skip_size > remain {
                skip_size = remain - crypt_size;
            }

            if crypt_size != 0 {
                self.cipher.process_encrypted_chunk(
                    &encrypted_region[cursor..cursor + crypt_size],
                    &mut output_region[cursor..cursor + crypt_size],
                );
                cursor += crypt_size;
                self.pattern_stream_offset += crypt_size as u64;
            }

            if skip_size != 0 {
                output_region[cursor..cursor + skip_size]
                    .copy_from_slice(&encrypted_region[cursor..cursor + skip_size]);
                cursor += skip_size;
                self.pattern_stream_offset += skip_size as u64;
            }
        }
    }
}

enum SampleCipher {
    Ctr {
        aes: Aes128,
        iv: [u8; 16],
        encrypted_offset: u64,
    },
    Cbc {
        aes: Aes128,
        iv: [u8; 16],
        chain_block: [u8; 16],
    },
}

impl SampleCipher {
    fn reset(&mut self) {
        match self {
            Self::Ctr {
                encrypted_offset, ..
            } => *encrypted_offset = 0,
            Self::Cbc {
                iv, chain_block, ..
            } => *chain_block = *iv,
        }
    }

    fn process_encrypted_chunk(&mut self, input: &[u8], output: &mut [u8]) {
        match self {
            Self::Ctr {
                aes,
                iv,
                encrypted_offset,
            } => {
                let mut cursor = 0usize;
                while cursor < input.len() {
                    let block_offset = usize::try_from(*encrypted_offset % 16).unwrap();
                    let chunk_len = (16 - block_offset).min(input.len() - cursor);
                    let mut counter_block = compute_ctr_counter_block(iv, *encrypted_offset);
                    aes.encrypt_block(&mut counter_block);
                    for index in 0..chunk_len {
                        output[cursor + index] =
                            input[cursor + index] ^ counter_block[block_offset + index];
                    }
                    cursor += chunk_len;
                    *encrypted_offset += chunk_len as u64;
                }
            }
            Self::Cbc {
                aes, chain_block, ..
            } => {
                let full_blocks_len = input.len() - (input.len() % 16);
                let mut cursor = 0usize;
                while cursor < full_blocks_len {
                    let ciphertext = &input[cursor..cursor + 16];
                    let mut block = Block::<Aes128>::clone_from_slice(ciphertext);
                    aes.decrypt_block(&mut block);
                    for index in 0..16 {
                        output[cursor + index] = block[index] ^ chain_block[index];
                    }
                    chain_block.copy_from_slice(ciphertext);
                    cursor += 16;
                }
                output[full_blocks_len..].copy_from_slice(&input[full_blocks_len..]);
            }
        }
    }
}

fn compute_ctr_counter_block(iv: &[u8; 16], stream_offset: u64) -> Block<Aes128> {
    let counter_offset = stream_offset / 16;
    let counter_offset_bytes = counter_offset.to_be_bytes();
    let mut counter_block = Block::<Aes128>::default();

    let mut carry = 0u16;
    for index in 0..8 {
        let offset = 15 - index;
        let sum = u16::from(iv[offset]) + u16::from(counter_offset_bytes[7 - index]) + carry;
        counter_block[offset] = (sum & 0xff) as u8;
        carry = if sum >= 0x100 { 1 } else { 0 };
    }
    for index in 8..16 {
        let offset = 15 - index;
        counter_block[offset] = iv[offset];
    }

    counter_block
}
