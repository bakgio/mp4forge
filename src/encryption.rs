//! Resolved common-encryption metadata helpers built on typed MP4 boxes.
//!
//! These helpers keep the existing low-level box model unchanged while providing a small semantic
//! layer for callers that need the effective sample-encryption parameters for a decoded `senc`
//! payload.

use std::error::Error;
use std::fmt;

use crate::FourCc;
use crate::boxes::iso14496_12::{Saiz, Sbgp, SeigEntry, SeigEntryL, Sgpd};
use crate::boxes::iso23001_7::{
    SENC_USE_SUBSAMPLE_ENCRYPTION, Senc, SencSample, SencSubsample, Tenc,
};
use crate::codec::ImmutableBox;

const SEIG_GROUPING_TYPE: FourCc = FourCc::from_bytes(*b"seig");
const SEIG_GROUPING_TYPE_U32: u32 = u32::from_be_bytes(*b"seig");
const FRAGMENT_LOCAL_DESCRIPTION_INDEX_BASE: u32 = 1 << 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SampleGroupDescriptionRef {
    group_description_index: u32,
    description_index: u32,
    fragment_local: bool,
}

struct ResolvedEncryptionDefaults<'a> {
    source: ResolvedSampleEncryptionSource,
    is_protected: bool,
    crypt_byte_block: u8,
    skip_byte_block: u8,
    per_sample_iv_size: Option<u8>,
    constant_iv: Option<&'a [u8]>,
    kid: [u8; 16],
}

struct DefaultSourceConfig<'a> {
    source_name: &'static str,
    is_protected: u8,
    crypt_byte_block: u8,
    skip_byte_block: u8,
    per_sample_iv_size: u8,
    constant_iv_size: u8,
    constant_iv: &'a [u8],
    kid: [u8; 16],
    source: ResolvedSampleEncryptionSource,
}

enum SeigEntries<'a> {
    Fixed(&'a [SeigEntry]),
    LengthPrefixed(&'a [SeigEntryL]),
}

impl<'a> SeigEntries<'a> {
    fn get(&self, index: usize) -> Option<&'a SeigEntry> {
        match self {
            Self::Fixed(entries) => entries.get(index),
            Self::LengthPrefixed(entries) => entries.get(index).map(|entry| &entry.seig_entry),
        }
    }
}

/// Optional typed context used to resolve the effective encryption parameters for one `senc`.
///
/// Supply whichever typed boxes are already available at the call site. The resolver prefers an
/// explicit `seig` description selected through `sbgp`, then falls back to track-level `tenc`
/// defaults for samples that are not covered by a sample-group override.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SampleEncryptionContext<'a> {
    /// Track-level encryption defaults from the protected sample entry.
    pub tenc: Option<&'a Tenc>,
    /// Typed `sgpd(seig)` description entries available for the current scope.
    pub sgpd: Option<&'a Sgpd>,
    /// Typed `sbgp(seig)` sample-to-group mapping for the current `senc`.
    pub sbgp: Option<&'a Sbgp>,
    /// Optional auxiliary-size box used to validate each resolved sample-info record length.
    pub saiz: Option<&'a Saiz>,
}

/// Resolved semantic view of one decoded `senc` payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSampleEncryptionInfo<'a> {
    /// Whether the source `senc` payload includes subsample-encryption counts and ranges.
    pub uses_subsample_encryption: bool,
    /// Per-sample resolved encryption metadata in sample order.
    pub samples: Vec<ResolvedSampleEncryptionSample<'a>>,
}

/// Resolved semantic view of one sample's encryption metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSampleEncryptionSample<'a> {
    /// One-based sample index inside the resolved `senc` payload.
    pub sample_index: u32,
    /// Source that supplied the effective encryption defaults for this sample.
    pub metadata_source: ResolvedSampleEncryptionSource,
    /// Whether the resolved defaults mark the sample as protected.
    pub is_protected: bool,
    /// Number of encrypted 16-byte blocks in each protection pattern cycle.
    pub crypt_byte_block: u8,
    /// Number of skipped 16-byte blocks in each protection pattern cycle.
    pub skip_byte_block: u8,
    /// Per-sample IV size when the sample carries an inline IV.
    pub per_sample_iv_size: Option<u8>,
    /// Per-sample IV bytes read directly from the `senc` sample record.
    pub initialization_vector: &'a [u8],
    /// Constant IV bytes inherited from the resolved defaults when inline IV bytes are absent.
    pub constant_iv: Option<&'a [u8]>,
    /// Effective key identifier for the sample.
    pub kid: [u8; 16],
    /// Decoded subsample-encryption records from the `senc` sample entry.
    pub subsamples: &'a [SencSubsample],
    /// Resolved auxiliary-information record length for this sample.
    pub auxiliary_info_size: u32,
}

impl<'a> ResolvedSampleEncryptionSample<'a> {
    /// Returns the effective IV bytes for the sample, preferring inline `senc` IV bytes when they
    /// are present and otherwise falling back to the resolved constant IV.
    pub fn effective_initialization_vector(&self) -> &'a [u8] {
        if !self.initialization_vector.is_empty() {
            self.initialization_vector
        } else {
            self.constant_iv.unwrap_or(&[])
        }
    }
}

/// Source that supplied the effective encryption defaults for one resolved sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedSampleEncryptionSource {
    /// Track-level defaults were taken from the active `tenc`.
    TrackEncryptionBox,
    /// Sample-level defaults were taken from a `seig` description selected by `sbgp`.
    SampleGroupDescription {
        /// Raw `group_description_index` value carried by `sbgp`.
        group_description_index: u32,
        /// One-based typed `seig` description index resolved inside the supplied `sgpd`.
        description_index: u32,
        /// Whether the resolved description index used fragment-local numbering.
        fragment_local: bool,
    },
}

/// Errors raised while resolving effective sample-encryption metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveSampleEncryptionError {
    /// The source `senc` version is not supported by the typed box model.
    UnsupportedSencVersion {
        /// Unsupported full-box version from `senc`.
        version: u8,
    },
    /// The source `senc` flags include unsupported bits.
    UnsupportedSencFlags {
        /// Raw `senc` flag bits.
        flags: u32,
    },
    /// The declared `senc` sample count does not match the decoded sample records.
    SampleCountMismatch {
        /// Declared sample count from `senc`.
        declared: u32,
        /// Actual decoded sample-record count.
        actual: usize,
    },
    /// One of the supplied typed context boxes is internally inconsistent for resolution.
    InvalidConfiguration {
        /// Box or helper component whose configuration was invalid.
        source: &'static str,
        /// Human-readable explanation of the rejected configuration.
        reason: &'static str,
    },
    /// The supplied `sbgp` does not describe `seig` sample groups.
    InvalidSbgpGroupingType {
        /// Raw grouping type from `sbgp`.
        actual: u32,
    },
    /// The supplied `sgpd` does not describe `seig` sample groups.
    InvalidSgpdGroupingType {
        /// Grouping type from `sgpd`.
        actual: FourCc,
    },
    /// `sbgp` covers more samples than the resolved `senc` contains.
    SampleGroupCoverageExceeded {
        /// Declared sample count from `senc`.
        sample_count: u32,
        /// Number of samples that the `sbgp` mapping attempted to cover.
        covered_sample_count: u64,
    },
    /// A fragment-local `group_description_index` encoded an invalid zero-based entry.
    InvalidFragmentLocalDescriptionIndex {
        /// Raw `group_description_index` from `sbgp`.
        group_description_index: u32,
    },
    /// No `tenc` defaults were available for a sample that was not covered by `sbgp(seig)`.
    MissingTrackEncryptionDefaults {
        /// One-based sample index that needed a `tenc` fallback.
        sample_index: u32,
    },
    /// An explicit `sbgp` description entry could not be resolved from the supplied `sgpd`.
    MissingSampleGroupDescription {
        /// One-based sample index whose override could not be resolved.
        sample_index: u32,
        /// Raw `group_description_index` from `sbgp`.
        group_description_index: u32,
        /// One-based typed description index expected inside `sgpd`.
        description_index: u32,
        /// Whether the missing description used fragment-local numbering.
        fragment_local: bool,
    },
    /// The sample's inline IV length does not match the resolved per-sample IV size.
    SampleInitializationVectorSizeMismatch {
        /// One-based sample index whose inline IV length was invalid.
        sample_index: u32,
        /// Expected inline IV size from the resolved defaults.
        expected: usize,
        /// Actual inline IV size stored in the decoded `senc` sample.
        actual: usize,
    },
    /// The sample carried inline IV bytes even though the resolved defaults use a constant IV.
    UnexpectedSampleInitializationVector {
        /// One-based sample index whose inline IV bytes were unexpected.
        sample_index: u32,
        /// Actual inline IV size stored in the decoded `senc` sample.
        actual: usize,
    },
    /// The supplied `saiz` box is internally inconsistent for the current `senc`.
    InvalidSaizLayout {
        /// Human-readable explanation of the rejected `saiz` layout.
        reason: &'static str,
    },
    /// The resolved auxiliary-information length does not match `saiz` for one sample.
    SaizSampleInfoSizeMismatch {
        /// One-based sample index whose resolved size mismatched `saiz`.
        sample_index: u32,
        /// Resolved auxiliary-information size computed from `senc`.
        expected: u32,
        /// Actual sample-info size read from `saiz`.
        actual: u32,
    },
    /// The resolved auxiliary-information size exceeded what the helper can represent.
    SampleInfoSizeOverflow {
        /// One-based sample index whose resolved size overflowed.
        sample_index: u32,
    },
}

impl fmt::Display for ResolveSampleEncryptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSencVersion { version } => {
                write!(f, "unsupported senc version {version}")
            }
            Self::UnsupportedSencFlags { flags } => {
                write!(f, "unsupported senc flags 0x{flags:06x}")
            }
            Self::SampleCountMismatch { declared, actual } => write!(
                f,
                "senc sample count mismatch: declared {declared} sample(s) but decoded {actual}"
            ),
            Self::InvalidConfiguration { source, reason } => {
                write!(f, "invalid {source} configuration: {reason}")
            }
            Self::InvalidSbgpGroupingType { actual } => write!(
                f,
                "sbgp grouping type must be \"seig\", found 0x{actual:08x}"
            ),
            Self::InvalidSgpdGroupingType { actual } => {
                write!(f, "sgpd grouping type must be \"seig\", found {actual}")
            }
            Self::SampleGroupCoverageExceeded {
                sample_count,
                covered_sample_count,
            } => write!(
                f,
                "sbgp sample coverage exceeds senc sample count: covered {covered_sample_count} sample(s) for senc sample count {sample_count}"
            ),
            Self::InvalidFragmentLocalDescriptionIndex {
                group_description_index,
            } => write!(
                f,
                "fragment-local sbgp group description index {group_description_index} resolves to description 0"
            ),
            Self::MissingTrackEncryptionDefaults { sample_index } => write!(
                f,
                "sample {sample_index} is not covered by sgpd(seig) and no tenc fallback was supplied"
            ),
            Self::MissingSampleGroupDescription {
                sample_index,
                group_description_index,
                description_index,
                fragment_local,
            } => write!(
                f,
                "sample {sample_index} uses sbgp group description index {group_description_index} but sgpd is missing description {description_index} (fragment_local={fragment_local})"
            ),
            Self::SampleInitializationVectorSizeMismatch {
                sample_index,
                expected,
                actual,
            } => write!(
                f,
                "sample {sample_index} has inline IV size {actual} but resolved defaults require {expected}"
            ),
            Self::UnexpectedSampleInitializationVector {
                sample_index,
                actual,
            } => write!(
                f,
                "sample {sample_index} has unexpected inline IV bytes in constant-IV mode ({actual} byte(s))"
            ),
            Self::InvalidSaizLayout { reason } => write!(f, "invalid saiz layout: {reason}"),
            Self::SaizSampleInfoSizeMismatch {
                sample_index,
                expected,
                actual,
            } => write!(
                f,
                "sample {sample_index} resolved auxiliary info size {expected} does not match saiz size {actual}"
            ),
            Self::SampleInfoSizeOverflow { sample_index } => write!(
                f,
                "sample {sample_index} auxiliary info size is too large to represent"
            ),
        }
    }
}

impl Error for ResolveSampleEncryptionError {}

/// Resolves the effective encryption metadata for every sample carried by `senc`.
///
/// The resolver prefers `sgpd(seig)` entries selected by `sbgp`, then falls back to `tenc`
/// defaults for samples that have no explicit group-description override. When `saiz` is present,
/// the resolved auxiliary-information length for every sample is validated against the declared
/// size table.
pub fn resolve_sample_encryption<'a>(
    senc: &'a Senc,
    context: SampleEncryptionContext<'a>,
) -> Result<ResolvedSampleEncryptionInfo<'a>, ResolveSampleEncryptionError> {
    if senc.version() != 0 {
        return Err(ResolveSampleEncryptionError::UnsupportedSencVersion {
            version: senc.version(),
        });
    }

    if senc.flags() & !SENC_USE_SUBSAMPLE_ENCRYPTION != 0 {
        return Err(ResolveSampleEncryptionError::UnsupportedSencFlags {
            flags: senc.flags(),
        });
    }

    if usize::try_from(senc.sample_count).ok() != Some(senc.samples.len()) {
        return Err(ResolveSampleEncryptionError::SampleCountMismatch {
            declared: senc.sample_count,
            actual: senc.samples.len(),
        });
    }

    validate_saiz_layout(context.saiz, senc.sample_count)?;
    let sample_group_refs =
        resolve_sample_group_refs(senc.sample_count, &senc.samples, context.sbgp)?;
    let uses_subsample_encryption = senc.uses_subsample_encryption();

    let mut samples = Vec::with_capacity(senc.samples.len());
    for (sample_offset, sample) in senc.samples.iter().enumerate() {
        let sample_index = (sample_offset as u32) + 1;
        let defaults = match sample_group_refs[sample_offset] {
            Some(group_ref) => resolve_seig_defaults(sample_index, context.sgpd, group_ref)?,
            None => resolve_tenc_defaults(sample_index, context.tenc)?,
        };

        validate_sample_initialization_vector(sample_index, sample, &defaults)?;
        let auxiliary_info_size =
            resolved_sample_info_size(sample_index, sample, uses_subsample_encryption)?;
        validate_saiz_sample_info_size(context.saiz, sample_index, auxiliary_info_size)?;

        samples.push(ResolvedSampleEncryptionSample {
            sample_index,
            metadata_source: defaults.source,
            is_protected: defaults.is_protected,
            crypt_byte_block: defaults.crypt_byte_block,
            skip_byte_block: defaults.skip_byte_block,
            per_sample_iv_size: defaults.per_sample_iv_size,
            initialization_vector: &sample.initialization_vector,
            constant_iv: defaults.constant_iv,
            kid: defaults.kid,
            subsamples: &sample.subsamples,
            auxiliary_info_size,
        });
    }

    Ok(ResolvedSampleEncryptionInfo {
        uses_subsample_encryption,
        samples,
    })
}

fn validate_saiz_layout(
    saiz: Option<&Saiz>,
    sample_count: u32,
) -> Result<(), ResolveSampleEncryptionError> {
    let Some(saiz) = saiz else {
        return Ok(());
    };

    if saiz.sample_count != sample_count {
        return Err(ResolveSampleEncryptionError::InvalidSaizLayout {
            reason: "sample count does not match senc",
        });
    }

    if saiz.default_sample_info_size == 0
        && usize::try_from(saiz.sample_count).ok() != Some(saiz.sample_info_size.len())
    {
        return Err(ResolveSampleEncryptionError::InvalidSaizLayout {
            reason: "per-sample size table length does not match the declared sample count",
        });
    }

    Ok(())
}

fn resolve_sample_group_refs(
    sample_count: u32,
    samples: &[SencSample],
    sbgp: Option<&Sbgp>,
) -> Result<Vec<Option<SampleGroupDescriptionRef>>, ResolveSampleEncryptionError> {
    let mut group_refs = vec![None; samples.len()];
    let Some(sbgp) = sbgp else {
        return Ok(group_refs);
    };

    if sbgp.grouping_type != SEIG_GROUPING_TYPE_U32 {
        return Err(ResolveSampleEncryptionError::InvalidSbgpGroupingType {
            actual: sbgp.grouping_type,
        });
    }

    let mut cursor = 0usize;
    for entry in &sbgp.entries {
        let entry_sample_count =
            usize::try_from(entry.sample_count).unwrap_or(group_refs.len().saturating_add(1));
        let next = cursor.saturating_add(entry_sample_count);
        if next > group_refs.len() {
            return Err(ResolveSampleEncryptionError::SampleGroupCoverageExceeded {
                sample_count,
                covered_sample_count: next as u64,
            });
        }

        let normalized = normalize_group_description_index(entry.group_description_index)?;
        for slot in &mut group_refs[cursor..next] {
            *slot = normalized;
        }
        cursor = next;
    }

    Ok(group_refs)
}

fn normalize_group_description_index(
    group_description_index: u32,
) -> Result<Option<SampleGroupDescriptionRef>, ResolveSampleEncryptionError> {
    if group_description_index == 0 {
        return Ok(None);
    }

    if group_description_index >= FRAGMENT_LOCAL_DESCRIPTION_INDEX_BASE {
        let description_index = group_description_index - FRAGMENT_LOCAL_DESCRIPTION_INDEX_BASE;
        if description_index == 0 {
            return Err(
                ResolveSampleEncryptionError::InvalidFragmentLocalDescriptionIndex {
                    group_description_index,
                },
            );
        }
        return Ok(Some(SampleGroupDescriptionRef {
            group_description_index,
            description_index,
            fragment_local: true,
        }));
    }

    Ok(Some(SampleGroupDescriptionRef {
        group_description_index,
        description_index: group_description_index,
        fragment_local: false,
    }))
}

fn resolve_tenc_defaults<'a>(
    sample_index: u32,
    tenc: Option<&'a Tenc>,
) -> Result<ResolvedEncryptionDefaults<'a>, ResolveSampleEncryptionError> {
    let Some(tenc) = tenc else {
        return Err(ResolveSampleEncryptionError::MissingTrackEncryptionDefaults { sample_index });
    };

    resolve_defaults(DefaultSourceConfig {
        source_name: "tenc",
        is_protected: tenc.default_is_protected,
        crypt_byte_block: tenc.default_crypt_byte_block,
        skip_byte_block: tenc.default_skip_byte_block,
        per_sample_iv_size: tenc.default_per_sample_iv_size,
        constant_iv_size: tenc.default_constant_iv_size,
        constant_iv: &tenc.default_constant_iv,
        kid: tenc.default_kid,
        source: ResolvedSampleEncryptionSource::TrackEncryptionBox,
    })
}

fn resolve_seig_defaults<'a>(
    sample_index: u32,
    sgpd: Option<&'a Sgpd>,
    group_ref: SampleGroupDescriptionRef,
) -> Result<ResolvedEncryptionDefaults<'a>, ResolveSampleEncryptionError> {
    let Some(sgpd) = sgpd else {
        return Err(
            ResolveSampleEncryptionError::MissingSampleGroupDescription {
                sample_index,
                group_description_index: group_ref.group_description_index,
                description_index: group_ref.description_index,
                fragment_local: group_ref.fragment_local,
            },
        );
    };

    if sgpd.grouping_type != SEIG_GROUPING_TYPE {
        return Err(ResolveSampleEncryptionError::InvalidSgpdGroupingType {
            actual: sgpd.grouping_type,
        });
    }

    let description_offset =
        usize::try_from(group_ref.description_index.saturating_sub(1)).unwrap_or(usize::MAX);
    let entry = typed_seig_entries(sgpd)?.get(description_offset).ok_or(
        ResolveSampleEncryptionError::MissingSampleGroupDescription {
            sample_index,
            group_description_index: group_ref.group_description_index,
            description_index: group_ref.description_index,
            fragment_local: group_ref.fragment_local,
        },
    )?;

    resolve_defaults(DefaultSourceConfig {
        source_name: "sgpd(seig)",
        is_protected: entry.is_protected,
        crypt_byte_block: entry.crypt_byte_block,
        skip_byte_block: entry.skip_byte_block,
        per_sample_iv_size: entry.per_sample_iv_size,
        constant_iv_size: entry.constant_iv_size,
        constant_iv: &entry.constant_iv,
        kid: entry.kid,
        source: ResolvedSampleEncryptionSource::SampleGroupDescription {
            group_description_index: group_ref.group_description_index,
            description_index: group_ref.description_index,
            fragment_local: group_ref.fragment_local,
        },
    })
}

fn typed_seig_entries<'a>(sgpd: &'a Sgpd) -> Result<SeigEntries<'a>, ResolveSampleEncryptionError> {
    match (sgpd.seig_entries.is_empty(), sgpd.seig_entries_l.is_empty()) {
        (false, false) => Err(ResolveSampleEncryptionError::InvalidConfiguration {
            source: "sgpd",
            reason: "typed seig entries are populated in both fixed-length and length-prefixed storage",
        }),
        (false, true) => Ok(SeigEntries::Fixed(&sgpd.seig_entries)),
        (true, false) => Ok(SeigEntries::LengthPrefixed(&sgpd.seig_entries_l)),
        (true, true) if sgpd.entry_count == 0 => Ok(SeigEntries::Fixed(&[])),
        (true, true) => Err(ResolveSampleEncryptionError::InvalidConfiguration {
            source: "sgpd",
            reason: "typed seig entries are not populated for the declared entry count",
        }),
    }
}

fn resolve_defaults<'a>(
    config: DefaultSourceConfig<'a>,
) -> Result<ResolvedEncryptionDefaults<'a>, ResolveSampleEncryptionError> {
    let is_protected = match config.is_protected {
        0 => false,
        1 => true,
        _ => {
            return Err(ResolveSampleEncryptionError::InvalidConfiguration {
                source: config.source_name,
                reason: "IsProtected must be either 0 or 1",
            });
        }
    };

    let has_constant_iv = config.constant_iv_size != 0 || !config.constant_iv.is_empty();
    if !is_protected {
        if config.per_sample_iv_size != 0 {
            return Err(ResolveSampleEncryptionError::InvalidConfiguration {
                source: config.source_name,
                reason: "unprotected samples must not declare a per-sample IV size",
            });
        }
        if has_constant_iv {
            return Err(ResolveSampleEncryptionError::InvalidConfiguration {
                source: config.source_name,
                reason: "unprotected samples must not declare a constant IV",
            });
        }

        return Ok(ResolvedEncryptionDefaults {
            source: config.source,
            is_protected,
            crypt_byte_block: config.crypt_byte_block,
            skip_byte_block: config.skip_byte_block,
            per_sample_iv_size: None,
            constant_iv: None,
            kid: config.kid,
        });
    }

    if config.per_sample_iv_size != 0 {
        if has_constant_iv {
            return Err(ResolveSampleEncryptionError::InvalidConfiguration {
                source: config.source_name,
                reason: "per-sample IV mode must not also declare a constant IV",
            });
        }

        return Ok(ResolvedEncryptionDefaults {
            source: config.source,
            is_protected,
            crypt_byte_block: config.crypt_byte_block,
            skip_byte_block: config.skip_byte_block,
            per_sample_iv_size: Some(config.per_sample_iv_size),
            constant_iv: None,
            kid: config.kid,
        });
    }

    if usize::from(config.constant_iv_size) != config.constant_iv.len() {
        return Err(ResolveSampleEncryptionError::InvalidConfiguration {
            source: config.source_name,
            reason: "constant IV length does not match the declared constant IV size",
        });
    }
    if config.constant_iv.is_empty() {
        return Err(ResolveSampleEncryptionError::InvalidConfiguration {
            source: config.source_name,
            reason: "protected samples with no per-sample IV size must declare a constant IV",
        });
    }

    Ok(ResolvedEncryptionDefaults {
        source: config.source,
        is_protected,
        crypt_byte_block: config.crypt_byte_block,
        skip_byte_block: config.skip_byte_block,
        per_sample_iv_size: None,
        constant_iv: Some(config.constant_iv),
        kid: config.kid,
    })
}

fn validate_sample_initialization_vector(
    sample_index: u32,
    sample: &SencSample,
    defaults: &ResolvedEncryptionDefaults<'_>,
) -> Result<(), ResolveSampleEncryptionError> {
    let actual = sample.initialization_vector.len();
    match defaults.per_sample_iv_size {
        Some(expected) if actual == usize::from(expected) => Ok(()),
        Some(expected) => Err(
            ResolveSampleEncryptionError::SampleInitializationVectorSizeMismatch {
                sample_index,
                expected: usize::from(expected),
                actual,
            },
        ),
        None if actual == 0 => Ok(()),
        None => Err(
            ResolveSampleEncryptionError::UnexpectedSampleInitializationVector {
                sample_index,
                actual,
            },
        ),
    }
}

fn resolved_sample_info_size(
    sample_index: u32,
    sample: &SencSample,
    uses_subsample_encryption: bool,
) -> Result<u32, ResolveSampleEncryptionError> {
    let mut size = u32::try_from(sample.initialization_vector.len())
        .map_err(|_| ResolveSampleEncryptionError::SampleInfoSizeOverflow { sample_index })?;

    if uses_subsample_encryption {
        size = size
            .checked_add(2)
            .ok_or(ResolveSampleEncryptionError::SampleInfoSizeOverflow { sample_index })?;
        let subsample_count = u32::try_from(sample.subsamples.len())
            .map_err(|_| ResolveSampleEncryptionError::SampleInfoSizeOverflow { sample_index })?;
        let subsample_bytes = subsample_count
            .checked_mul(6)
            .ok_or(ResolveSampleEncryptionError::SampleInfoSizeOverflow { sample_index })?;
        size = size
            .checked_add(subsample_bytes)
            .ok_or(ResolveSampleEncryptionError::SampleInfoSizeOverflow { sample_index })?;
    }

    Ok(size)
}

fn validate_saiz_sample_info_size(
    saiz: Option<&Saiz>,
    sample_index: u32,
    expected: u32,
) -> Result<(), ResolveSampleEncryptionError> {
    let Some(saiz) = saiz else {
        return Ok(());
    };

    let actual = if saiz.default_sample_info_size != 0 {
        u32::from(saiz.default_sample_info_size)
    } else {
        let offset = usize::try_from(sample_index - 1).unwrap_or(usize::MAX);
        let Some(size) = saiz.sample_info_size.get(offset) else {
            return Err(ResolveSampleEncryptionError::InvalidSaizLayout {
                reason: "per-sample size table is shorter than the declared sample count",
            });
        };
        u32::from(*size)
    };

    if actual != expected {
        return Err(ResolveSampleEncryptionError::SaizSampleInfoSizeMismatch {
            sample_index,
            expected,
            actual,
        });
    }

    Ok(())
}
