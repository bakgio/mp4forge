//! File-summary helpers built on the extraction and box layers, with byte-slice convenience entry
//! points for in-memory probe flows.

use std::error::Error;
use std::fmt;
use std::io::{self, Cursor, Read, Seek, SeekFrom};

use crate::BoxInfo;
use crate::FourCc;
use crate::bitio::BitReader;
use crate::boxes::av1::AV1CodecConfiguration;
use crate::boxes::etsi_ts_102_366::Dac3;
use crate::boxes::iso14496_12::{
    AVCDecoderConfiguration, AudioSampleEntry, Btrt, Co64, Colr, Ctts, Fiel,
    HEVCDecoderConfiguration, Mvhd, Pasp, Stco, Stsc, Stsz, Stts, TextSubtitleSampleEntry, Tfdt,
    Tfhd, Tkhd, Trun, VisualSampleEntry, XMLSubtitleSampleEntry,
};
use crate::boxes::iso14496_12::{Frma, Hdlr, Schm};
use crate::boxes::iso14496_14::Esds;
use crate::boxes::iso14496_30::{WebVTTConfigurationBox, WebVTTSourceLabelBox};
use crate::boxes::iso23001_5::PcmC;
use crate::boxes::opus::DOps;
use crate::boxes::vp::VpCodecConfiguration;
use crate::codec::{CodecBox, CodecError, ImmutableBox, unmarshal};
use crate::extract::{ExtractError, ExtractedBox, extract_boxes, extract_boxes_with_payload};
use crate::header::HeaderError;
use crate::walk::BoxPath;

const FTYP: FourCc = FourCc::from_bytes(*b"ftyp");
const MOOV: FourCc = FourCc::from_bytes(*b"moov");
const MVHD: FourCc = FourCc::from_bytes(*b"mvhd");
const TRAK: FourCc = FourCc::from_bytes(*b"trak");
const MOOF: FourCc = FourCc::from_bytes(*b"moof");
const MDAT: FourCc = FourCc::from_bytes(*b"mdat");
const TKHD: FourCc = FourCc::from_bytes(*b"tkhd");
const EDTS: FourCc = FourCc::from_bytes(*b"edts");
const ELST: FourCc = FourCc::from_bytes(*b"elst");
const MDIA: FourCc = FourCc::from_bytes(*b"mdia");
const HDLR: FourCc = FourCc::from_bytes(*b"hdlr");
const MDHD: FourCc = FourCc::from_bytes(*b"mdhd");
const MINF: FourCc = FourCc::from_bytes(*b"minf");
const STBL: FourCc = FourCc::from_bytes(*b"stbl");
const STSD: FourCc = FourCc::from_bytes(*b"stsd");
const AVC1: FourCc = FourCc::from_bytes(*b"avc1");
const AVCC: FourCc = FourCc::from_bytes(*b"avcC");
const HEV1: FourCc = FourCc::from_bytes(*b"hev1");
const HVC1: FourCc = FourCc::from_bytes(*b"hvc1");
const HVCC: FourCc = FourCc::from_bytes(*b"hvcC");
const AV01: FourCc = FourCc::from_bytes(*b"av01");
const AV1C: FourCc = FourCc::from_bytes(*b"av1C");
const VP08: FourCc = FourCc::from_bytes(*b"vp08");
const VP09: FourCc = FourCc::from_bytes(*b"vp09");
const VPCC: FourCc = FourCc::from_bytes(*b"vpcC");
const ENCV: FourCc = FourCc::from_bytes(*b"encv");
const BTRT: FourCc = FourCc::from_bytes(*b"btrt");
const COLR: FourCc = FourCc::from_bytes(*b"colr");
const FIEL: FourCc = FourCc::from_bytes(*b"fiel");
const PASP: FourCc = FourCc::from_bytes(*b"pasp");
const MP4A: FourCc = FourCc::from_bytes(*b"mp4a");
const OPUS: FourCc = FourCc::from_bytes(*b"Opus");
const DOPS: FourCc = FourCc::from_bytes(*b"dOps");
const AC_3: FourCc = FourCc::from_bytes(*b"ac-3");
const DAC3: FourCc = FourCc::from_bytes(*b"dac3");
const IPCM: FourCc = FourCc::from_bytes(*b"ipcm");
const FPCM: FourCc = FourCc::from_bytes(*b"fpcm");
const PCMC: FourCc = FourCc::from_bytes(*b"pcmC");
const WAVE: FourCc = FourCc::from_bytes(*b"wave");
const ESDS: FourCc = FourCc::from_bytes(*b"esds");
const ENCA: FourCc = FourCc::from_bytes(*b"enca");
const STPP: FourCc = FourCc::from_bytes(*b"stpp");
const SBTT: FourCc = FourCc::from_bytes(*b"sbtt");
const WVTT: FourCc = FourCc::from_bytes(*b"wvtt");
const VTTC_CONFIG: FourCc = FourCc::from_bytes(*b"vttC");
const VLAB: FourCc = FourCc::from_bytes(*b"vlab");
const COLR_NCLX: FourCc = FourCc::from_bytes(*b"nclx");
const COLR_RICC: FourCc = FourCc::from_bytes(*b"rICC");
const COLR_PROF: FourCc = FourCc::from_bytes(*b"prof");
const SINF: FourCc = FourCc::from_bytes(*b"sinf");
const FRMA: FourCc = FourCc::from_bytes(*b"frma");
const SCHM: FourCc = FourCc::from_bytes(*b"schm");
const STCO: FourCc = FourCc::from_bytes(*b"stco");
const CO64: FourCc = FourCc::from_bytes(*b"co64");
const STTS: FourCc = FourCc::from_bytes(*b"stts");
const CTTS: FourCc = FourCc::from_bytes(*b"ctts");
const STSC: FourCc = FourCc::from_bytes(*b"stsc");
const STSZ: FourCc = FourCc::from_bytes(*b"stsz");
const TRAF: FourCc = FourCc::from_bytes(*b"traf");
const TFHD: FourCc = FourCc::from_bytes(*b"tfhd");
const TFDT: FourCc = FourCc::from_bytes(*b"tfdt");
const TRUN: FourCc = FourCc::from_bytes(*b"trun");

/// High-level summary of one MP4 file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeInfo {
    /// Major brand from the root `ftyp` box.
    pub major_brand: FourCc,
    /// Minor version from the root `ftyp` box.
    pub minor_version: u32,
    /// Compatible brands listed by the root `ftyp` box.
    pub compatible_brands: Vec<FourCc>,
    /// Whether the `moov` box appears before the first `mdat`.
    pub fast_start: bool,
    /// Movie timescale from `mvhd`.
    pub timescale: u32,
    /// Movie duration from `mvhd`.
    pub duration: u64,
    /// Per-track summaries extracted from `trak` boxes.
    pub tracks: Vec<TrackInfo>,
    /// Fragment summaries extracted from `moof` boxes.
    pub segments: Vec<SegmentInfo>,
}

impl Default for ProbeInfo {
    fn default() -> Self {
        Self {
            major_brand: FourCc::ANY,
            minor_version: 0,
            compatible_brands: Vec::new(),
            fast_start: false,
            timescale: 0,
            duration: 0,
            tracks: Vec::new(),
            segments: Vec::new(),
        }
    }
}

/// Additive controls for eager probe expansion.
///
/// The existing [`probe`], [`probe_detailed`], and [`probe_codec_detailed`] entry points continue
/// to use the full eager behavior. Callers that need a lighter-weight summary can opt into the
/// companion `*_with_options` entry points and disable the expensive expansions they do not need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeOptions {
    /// Whether to expand per-sample timing, composition-offset, and size data from `stts`, `ctts`,
    /// and `stsz`.
    pub expand_samples: bool,
    /// Whether to expand per-chunk offsets and sample counts from `stco`/`co64` and `stsc`.
    pub expand_chunks: bool,
    /// Whether to aggregate fragmented segment summaries from `moof` boxes.
    pub include_segments: bool,
}

impl ProbeOptions {
    /// Returns the existing eager probe behavior.
    pub const fn full() -> Self {
        Self {
            expand_samples: true,
            expand_chunks: true,
            include_segments: true,
        }
    }

    /// Returns a lighter-weight probe behavior for large-file inspection.
    pub const fn lightweight() -> Self {
        Self {
            expand_samples: false,
            expand_chunks: false,
            include_segments: false,
        }
    }
}

impl Default for ProbeOptions {
    fn default() -> Self {
        Self::full()
    }
}

/// Summary of one logical media track.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackInfo {
    /// Track identifier from `tkhd`.
    pub track_id: u32,
    /// Media timescale from `mdhd`.
    pub timescale: u32,
    /// Media duration from `mdhd`.
    pub duration: u64,
    /// High-level codec classification derived from the sample description.
    pub codec: TrackCodec,
    /// Whether the track uses encrypted sample entries.
    pub encrypted: bool,
    /// Edit-list entries when `elst` is present.
    pub edit_list: Vec<EditListEntry>,
    /// Expanded per-sample timing and size data.
    pub samples: Vec<SampleInfo>,
    /// Expanded chunk offsets and sample counts.
    pub chunks: Vec<ChunkInfo>,
    /// AVC configuration summary when the track is AVC-based.
    pub avc: Option<AvcDecoderConfigInfo>,
    /// AAC configuration summary when the track is MP4A-based.
    pub mp4a: Option<Mp4aInfo>,
}

/// Additive detailed probe summary that extends [`ProbeInfo`] without changing its public shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetailedProbeInfo {
    /// Major brand from the root `ftyp` box.
    pub major_brand: FourCc,
    /// Minor version from the root `ftyp` box.
    pub minor_version: u32,
    /// Compatible brands listed by the root `ftyp` box.
    pub compatible_brands: Vec<FourCc>,
    /// Whether the `moov` box appears before the first `mdat`.
    pub fast_start: bool,
    /// Movie timescale from `mvhd`.
    pub timescale: u32,
    /// Movie duration from `mvhd`.
    pub duration: u64,
    /// Per-track detailed summaries extracted from `trak` boxes.
    pub tracks: Vec<DetailedTrackInfo>,
    /// Fragment summaries extracted from `moof` boxes.
    pub segments: Vec<SegmentInfo>,
}

impl Default for DetailedProbeInfo {
    fn default() -> Self {
        Self {
            major_brand: FourCc::ANY,
            minor_version: 0,
            compatible_brands: Vec::new(),
            fast_start: false,
            timescale: 0,
            duration: 0,
            tracks: Vec::new(),
            segments: Vec::new(),
        }
    }
}

/// Additive per-track summary that extends [`TrackInfo`] with richer sample-entry details.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DetailedTrackInfo {
    /// Backwards-compatible coarse summary preserved from [`TrackInfo`].
    pub summary: TrackInfo,
    /// Normalized codec family derived from the sample entry or protected original format.
    pub codec_family: TrackCodecFamily,
    /// Handler type from `hdlr` when present.
    pub handler_type: Option<FourCc>,
    /// ISO-639-2 language code derived from `mdhd` when present.
    pub language: Option<String>,
    /// Sample-entry box type found under `stsd`, including encrypted wrappers such as `encv`.
    pub sample_entry_type: Option<FourCc>,
    /// Original-format sample-entry type from `frma` when the track is protected.
    pub original_format: Option<FourCc>,
    /// Protection-scheme summary from `schm` when the track is protected.
    pub protection_scheme: Option<ProtectionSchemeInfo>,
    /// Display width from the visual sample entry when present.
    pub display_width: Option<u16>,
    /// Display height from the visual sample entry when present.
    pub display_height: Option<u16>,
    /// Channel count from the audio sample entry when present.
    pub channel_count: Option<u16>,
    /// Integer sample rate from the audio sample entry when present.
    pub sample_rate: Option<u16>,
}

/// Additive detailed probe summary that extends [`DetailedProbeInfo`] with codec-specific
/// configuration details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodecDetailedProbeInfo {
    /// Major brand from the root `ftyp` box.
    pub major_brand: FourCc,
    /// Minor version from the root `ftyp` box.
    pub minor_version: u32,
    /// Compatible brands listed by the root `ftyp` box.
    pub compatible_brands: Vec<FourCc>,
    /// Whether the `moov` box appears before the first `mdat`.
    pub fast_start: bool,
    /// Movie timescale from `mvhd`.
    pub timescale: u32,
    /// Movie duration from `mvhd`.
    pub duration: u64,
    /// Per-track detailed summaries extracted from `trak` boxes.
    pub tracks: Vec<CodecDetailedTrackInfo>,
    /// Fragment summaries extracted from `moof` boxes.
    pub segments: Vec<SegmentInfo>,
}

impl Default for CodecDetailedProbeInfo {
    fn default() -> Self {
        Self {
            major_brand: FourCc::ANY,
            minor_version: 0,
            compatible_brands: Vec::new(),
            fast_start: false,
            timescale: 0,
            duration: 0,
            tracks: Vec::new(),
            segments: Vec::new(),
        }
    }
}

/// Additive per-track summary that extends [`DetailedTrackInfo`] with parsed codec-specific
/// configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodecDetailedTrackInfo {
    /// Backwards-compatible detailed track summary preserved from [`DetailedTrackInfo`].
    pub summary: DetailedTrackInfo,
    /// Parsed codec-specific configuration when it is available for the track family.
    pub codec_details: TrackCodecDetails,
}

/// Additive detailed probe summary that extends [`CodecDetailedProbeInfo`] with media
/// characteristics already parsed by the crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaCharacteristicsProbeInfo {
    /// Major brand from the root `ftyp` box.
    pub major_brand: FourCc,
    /// Minor version from the root `ftyp` box.
    pub minor_version: u32,
    /// Compatible brands listed by the root `ftyp` box.
    pub compatible_brands: Vec<FourCc>,
    /// Whether the `moov` box appears before the first `mdat`.
    pub fast_start: bool,
    /// Movie timescale from `mvhd`.
    pub timescale: u32,
    /// Movie duration from `mvhd`.
    pub duration: u64,
    /// Per-track detailed summaries extracted from `trak` boxes.
    pub tracks: Vec<MediaCharacteristicsTrackInfo>,
    /// Fragment summaries extracted from `moof` boxes.
    pub segments: Vec<SegmentInfo>,
}

impl Default for MediaCharacteristicsProbeInfo {
    fn default() -> Self {
        Self {
            major_brand: FourCc::ANY,
            minor_version: 0,
            compatible_brands: Vec::new(),
            fast_start: false,
            timescale: 0,
            duration: 0,
            tracks: Vec::new(),
            segments: Vec::new(),
        }
    }
}

/// Additive per-track summary that extends [`DetailedTrackInfo`] with parsed codec and media
/// characteristics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaCharacteristicsTrackInfo {
    /// Backwards-compatible detailed track summary preserved from [`DetailedTrackInfo`].
    pub summary: DetailedTrackInfo,
    /// Parsed codec-specific configuration when it is available for the track family.
    pub codec_details: TrackCodecDetails,
    /// Sample-entry media characteristics already parsed by the crate.
    pub media_characteristics: TrackMediaCharacteristics,
}

/// Media characteristics derived from sample-entry side boxes such as `btrt`, `colr`, `pasp`,
/// and `fiel`.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackMediaCharacteristics {
    /// Declared buffering and bitrate data from `btrt` when present.
    pub declared_bitrate: Option<DeclaredBitrateInfo>,
    /// Declared colorimetry data from `colr` when present.
    pub color: Option<ColorInfo>,
    /// Declared pixel aspect ratio from `pasp` when present.
    pub pixel_aspect_ratio: Option<PixelAspectRatioInfo>,
    /// Declared field-order hint from `fiel` when present.
    pub field_order: Option<FieldOrderInfo>,
}

/// Declared buffering and bitrate values parsed from `btrt`.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeclaredBitrateInfo {
    /// Decoder buffer size from `BufferSizeDB`.
    pub buffer_size_db: u32,
    /// Peak bitrate from `MaxBitrate`.
    pub max_bitrate: u32,
    /// Average bitrate from `AvgBitrate`.
    pub avg_bitrate: u32,
}

/// Declared color information parsed from `colr`.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorInfo {
    /// Active colour-type discriminator such as `nclx`, `rICC`, or `prof`.
    pub colour_type: FourCc,
    /// Colour-primaries code when `ColourType` is `nclx`.
    pub colour_primaries: Option<u16>,
    /// Transfer-characteristics code when `ColourType` is `nclx`.
    pub transfer_characteristics: Option<u16>,
    /// Matrix-coefficients code when `ColourType` is `nclx`.
    pub matrix_coefficients: Option<u16>,
    /// Full-range flag when `ColourType` is `nclx`.
    pub full_range: Option<bool>,
    /// Embedded ICC profile size when `ColourType` stores profile bytes.
    pub profile_size: Option<usize>,
    /// Opaque payload size for unrecognized colour types.
    pub unknown_size: Option<usize>,
}

impl Default for ColorInfo {
    fn default() -> Self {
        Self {
            colour_type: FourCc::ANY,
            colour_primaries: None,
            transfer_characteristics: None,
            matrix_coefficients: None,
            full_range: None,
            profile_size: None,
            unknown_size: None,
        }
    }
}

/// Declared pixel aspect ratio parsed from `pasp`.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PixelAspectRatioInfo {
    /// Horizontal spacing numerator from `HSpacing`.
    pub h_spacing: u32,
    /// Vertical spacing denominator from `VSpacing`.
    pub v_spacing: u32,
}

/// Declared field-order hint parsed from `fiel`.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldOrderInfo {
    /// Stored field count from `FieldCount`.
    pub field_count: u8,
    /// Stored field-ordering code from `FieldOrdering`.
    pub field_ordering: u8,
    /// Whether the hint indicates multiple interlaced fields.
    pub interlaced: bool,
}

/// Parsed codec-specific configuration for one recognized track family.
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(tag = "kind", content = "value", rename_all = "snake_case")
)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TrackCodecDetails {
    /// No codec-specific configuration was parsed for the track.
    #[default]
    Unknown,
    /// AVC decoder configuration parsed from `avcC`.
    Avc(AvcCodecDetails),
    /// HEVC decoder configuration parsed from `hvcC`.
    Hevc(HevcCodecDetails),
    /// AV1 decoder configuration parsed from `av1C`.
    Av1(Av1CodecDetails),
    /// VP8 decoder configuration parsed from `vpcC`.
    Vp8(VpCodecDetails),
    /// VP9 decoder configuration parsed from `vpcC`.
    Vp9(VpCodecDetails),
    /// MPEG-4 audio configuration parsed from `esds`.
    Mp4Audio(Mp4AudioCodecDetails),
    /// Opus decoder configuration parsed from `dOps`.
    Opus(OpusCodecDetails),
    /// AC-3 decoder configuration parsed from `dac3`.
    Ac3(Ac3CodecDetails),
    /// PCM configuration parsed from `pcmC`.
    Pcm(PcmCodecDetails),
    /// XML subtitle metadata parsed from `stpp`.
    XmlSubtitle(XmlSubtitleCodecDetails),
    /// Text subtitle metadata parsed from `sbtt`.
    TextSubtitle(TextSubtitleCodecDetails),
    /// WebVTT metadata parsed from `vttC` and `vlab`.
    WebVtt(WebVttCodecDetails),
}

/// Parsed AVC decoder configuration details.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AvcCodecDetails {
    /// AVC decoder configuration version.
    pub configuration_version: u8,
    /// AVC profile indication.
    pub profile: u8,
    /// AVC profile-compatibility byte.
    pub profile_compatibility: u8,
    /// AVC level indication.
    pub level: u8,
    /// Length-prefix width used for NAL units.
    pub length_size: u16,
    /// Chroma-format identifier when the high-profile extension fields are present.
    pub chroma_format: Option<u8>,
    /// Bit depth for luma samples when the high-profile extension fields are present.
    pub bit_depth_luma: Option<u8>,
    /// Bit depth for chroma samples when the high-profile extension fields are present.
    pub bit_depth_chroma: Option<u8>,
}

/// Parsed HEVC decoder configuration details.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HevcCodecDetails {
    /// HEVC decoder configuration version.
    pub configuration_version: u8,
    /// General profile space value.
    pub profile_space: u8,
    /// General tier flag.
    pub tier_flag: bool,
    /// General profile identifier.
    pub profile_idc: u8,
    /// Packed 32-bit compatibility mask derived from `general_profile_compatibility`.
    pub profile_compatibility_mask: u32,
    /// General constraint-indicator bytes.
    pub constraint_indicator: [u8; 6],
    /// General level identifier.
    pub level_idc: u8,
    /// Minimum spatial segmentation identifier.
    pub min_spatial_segmentation_idc: u16,
    /// Parallelism type.
    pub parallelism_type: u8,
    /// Chroma format identifier.
    pub chroma_format_idc: u8,
    /// Luma bit depth in bits.
    pub bit_depth_luma: u8,
    /// Chroma bit depth in bits.
    pub bit_depth_chroma: u8,
    /// Average frame rate from `hvcC`.
    pub avg_frame_rate: u16,
    /// Constant-frame-rate indicator.
    pub constant_frame_rate: u8,
    /// Number of temporal layers.
    pub num_temporal_layers: u8,
    /// Temporal-ID-nested indicator.
    pub temporal_id_nested: u8,
    /// Length-prefix width used for NAL units.
    pub length_size: u16,
}

/// Parsed AV1 decoder configuration details.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Av1CodecDetails {
    /// Sequence profile identifier.
    pub seq_profile: u8,
    /// Sequence level identifier.
    pub seq_level_idx_0: u8,
    /// Sequence tier identifier.
    pub seq_tier_0: u8,
    /// Decoded bit depth in bits.
    pub bit_depth: u8,
    /// Whether the sequence is monochrome.
    pub monochrome: bool,
    /// Horizontal chroma-subsampling flag.
    pub chroma_subsampling_x: u8,
    /// Vertical chroma-subsampling flag.
    pub chroma_subsampling_y: u8,
    /// Chroma sample-position code.
    pub chroma_sample_position: u8,
    /// Initial presentation-delay offset when the field is present.
    pub initial_presentation_delay_minus_one: Option<u8>,
}

/// Parsed VP8 or VP9 decoder configuration details.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VpCodecDetails {
    /// VP profile identifier.
    pub profile: u8,
    /// VP level identifier.
    pub level: u8,
    /// Decoded bit depth in bits.
    pub bit_depth: u8,
    /// Chroma-subsampling code.
    pub chroma_subsampling: u8,
    /// Whether the stream uses full-range luma values.
    pub full_range: bool,
    /// Color-primaries code.
    pub colour_primaries: u8,
    /// Transfer-characteristics code.
    pub transfer_characteristics: u8,
    /// Matrix-coefficients code.
    pub matrix_coefficients: u8,
    /// Codec-initialization-data size from `vpcC`.
    pub codec_initialization_data_size: u16,
}

/// Parsed MPEG-4 audio decoder configuration details.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mp4AudioCodecDetails {
    /// MPEG object-type indication from the decoder-config descriptor.
    pub object_type_indication: u8,
    /// AAC audio object type derived from the decoder-specific info payload.
    pub audio_object_type: u8,
    /// Channel count from the audio sample entry.
    pub channel_count: u16,
    /// Integer sample rate from the audio sample entry when present.
    pub sample_rate: Option<u16>,
}

/// Parsed Opus decoder configuration details.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpusCodecDetails {
    /// Output channel count from `dOps`.
    pub output_channel_count: u8,
    /// Decoder pre-skip from `dOps`.
    pub pre_skip: u16,
    /// Input sample rate from `dOps`.
    pub input_sample_rate: u32,
    /// Output gain from `dOps`.
    pub output_gain: i16,
    /// Channel-mapping-family identifier from `dOps`.
    pub channel_mapping_family: u8,
    /// Stream count when explicit channel mapping is present.
    pub stream_count: Option<u8>,
    /// Coupled-stream count when explicit channel mapping is present.
    pub coupled_count: Option<u8>,
    /// Channel-mapping table when explicit channel mapping is present.
    pub channel_mapping: Vec<u8>,
}

/// Parsed AC-3 decoder configuration details.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ac3CodecDetails {
    /// Sample-rate code from `dac3`.
    pub sample_rate_code: u8,
    /// Bit-stream identification from `dac3`.
    pub bit_stream_identification: u8,
    /// Bit-stream mode from `dac3`.
    pub bit_stream_mode: u8,
    /// Audio-coding-mode from `dac3`.
    pub audio_coding_mode: u8,
    /// Whether the bitstream carries an LFE channel.
    pub lfe_on: bool,
    /// Bit-rate code from `dac3`.
    pub bit_rate_code: u8,
}

/// Parsed PCM configuration details.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PcmCodecDetails {
    /// PCM format flags from `pcmC`.
    pub format_flags: u8,
    /// PCM sample size from `pcmC`.
    pub sample_size: u8,
}

/// Parsed XML subtitle sample-entry details.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XmlSubtitleCodecDetails {
    /// XML namespace string from `stpp`.
    pub namespace: String,
    /// XML schema-location string from `stpp`.
    pub schema_location: String,
    /// Auxiliary MIME types from `stpp`.
    pub auxiliary_mime_types: String,
}

/// Parsed text subtitle sample-entry details.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextSubtitleCodecDetails {
    /// Content-encoding label from `sbtt`.
    pub content_encoding: String,
    /// MIME format label from `sbtt`.
    pub mime_format: String,
}

/// Parsed WebVTT sample-entry details.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebVttCodecDetails {
    /// Configuration payload from `vttC` when present.
    pub config: Option<String>,
    /// Source-label payload from `vlab` when present.
    pub source_label: Option<String>,
}

#[derive(Default)]
struct TrackCodecConfigRefs<'a> {
    avcc: Option<&'a AVCDecoderConfiguration>,
    hvcc: Option<&'a HEVCDecoderConfiguration>,
    av1c: Option<&'a AV1CodecConfiguration>,
    vpcc: Option<&'a VpCodecConfiguration>,
    dops: Option<&'a DOps>,
    dac3: Option<&'a Dac3>,
    pcmc: Option<&'a PcmC>,
    xml_subtitle_sample_entry: Option<&'a XMLSubtitleSampleEntry>,
    text_subtitle_sample_entry: Option<&'a TextSubtitleSampleEntry>,
    webvtt_configuration: Option<&'a WebVTTConfigurationBox>,
    webvtt_source_label: Option<&'a WebVTTSourceLabelBox>,
}

#[derive(Default)]
struct TrackMediaCharacteristicRefs<'a> {
    btrt: Option<&'a Btrt>,
    colr: Option<&'a Colr>,
    pasp: Option<&'a Pasp>,
    fiel: Option<&'a Fiel>,
}

struct ParsedRichTrackInfo {
    summary: DetailedTrackInfo,
    codec_details: TrackCodecDetails,
    media_characteristics: TrackMediaCharacteristics,
}

/// Coarse codec classification used by the probe surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackCodec {
    /// No recognized sample entry was found.
    #[default]
    Unknown,
    /// AVC/H.264 video carried by `avc1` or `encv`.
    Avc1,
    /// MPEG-4 audio carried by `mp4a` or `enca`.
    Mp4a,
}

/// Normalized codec family derived from the sample entry or protected original format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackCodecFamily {
    /// No recognized codec family was derived.
    #[default]
    Unknown,
    /// AVC/H.264 video.
    Avc,
    /// HEVC/H.265 video.
    Hevc,
    /// AV1 video.
    Av1,
    /// VP8 video.
    Vp8,
    /// VP9 video.
    Vp9,
    /// MPEG-4 audio carried by `mp4a`.
    Mp4Audio,
    /// Opus audio.
    Opus,
    /// AC-3 audio.
    Ac3,
    /// PCM audio carried by `ipcm` or `fpcm`.
    Pcm,
    /// XML subtitle text carried by `stpp`.
    XmlSubtitle,
    /// Plain-text subtitle data carried by `sbtt`.
    TextSubtitle,
    /// WebVTT text carried by `wvtt`.
    WebVtt,
}

/// Protection-scheme summary derived from `schm`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectionSchemeInfo {
    /// Protection scheme type from `schm`.
    pub scheme_type: FourCc,
    /// Protection scheme version from `schm`.
    pub scheme_version: u32,
}

impl Default for ProtectionSchemeInfo {
    fn default() -> Self {
        Self {
            scheme_type: FourCc::ANY,
            scheme_version: 0,
        }
    }
}

/// One edit-list entry from `elst`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditListEntry {
    /// Media time selected by the edit entry.
    pub media_time: i64,
    /// Presentation duration covered by the edit entry.
    pub segment_duration: u64,
}

/// Expanded sample timing and size information.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleInfo {
    /// Sample size in bytes.
    pub size: u32,
    /// Decode-time delta in the track timescale.
    pub time_delta: u32,
    /// Composition-time offset in the track timescale.
    pub composition_time_offset: i64,
}

/// Expanded chunk placement information.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChunkInfo {
    /// File offset of the chunk payload.
    pub data_offset: u64,
    /// Number of samples stored in the chunk.
    pub samples_per_chunk: u32,
}

/// Summary of AVC decoder configuration attached to a video track.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AvcDecoderConfigInfo {
    /// AVC decoder configuration version.
    pub configuration_version: u8,
    /// AVC profile indication.
    pub profile: u8,
    /// AVC profile-compatibility byte.
    pub profile_compatibility: u8,
    /// AVC level indication.
    pub level: u8,
    /// Length-prefix width used for NAL units.
    pub length_size: u16,
    /// Display width from the visual sample entry.
    pub width: u16,
    /// Display height from the visual sample entry.
    pub height: u16,
}

/// AAC profile details derived from an `esds` descriptor stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AacProfileInfo {
    /// MPEG object type indication from the decoder-config descriptor.
    pub object_type_indication: u8,
    /// AAC audio object type derived from the decoder-specific info payload.
    pub audio_object_type: u8,
}

/// Summary of MP4A decoder configuration attached to an audio track.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mp4aInfo {
    /// MPEG object type indication from the decoder-config descriptor.
    pub object_type_indication: u8,
    /// AAC audio object type derived from the decoder-specific info payload.
    pub audio_object_type: u8,
    /// Channel count from the audio sample entry.
    pub channel_count: u16,
}

/// Summary of one fragmented-media segment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SegmentInfo {
    /// Track identifier from `tfhd`.
    pub track_id: u32,
    /// File offset of the owning `moof` box.
    pub moof_offset: u64,
    /// Base media decode time from `tfdt`.
    pub base_media_decode_time: u64,
    /// Default sample duration from `tfhd`.
    pub default_sample_duration: u32,
    /// Number of samples described by `trun`.
    pub sample_count: u32,
    /// Segment duration in the track timescale.
    pub duration: u32,
    /// Minimum composition-time offset observed across the run.
    pub composition_time_offset: i32,
    /// Total sample payload size in bytes.
    pub size: u32,
}

/// Probes a file and returns the backwards-compatible coarse movie, track, and fragment summary.
///
/// For richer sample-entry, handler, language, and protection metadata, use [`probe_detailed`].
pub fn probe<R>(reader: &mut R) -> Result<ProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    probe_with_options(reader, ProbeOptions::default())
}

/// Probes a file with additive expansion controls and returns the backwards-compatible coarse
/// movie, track, and fragment summary.
pub fn probe_with_options<R>(reader: &mut R, options: ProbeOptions) -> Result<ProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    Ok(strip_probe_details(probe_detailed_with_options(
        reader, options,
    )?))
}

/// Probes a file and returns an additive detailed movie, track, and fragment summary.
pub fn probe_detailed<R>(reader: &mut R) -> Result<DetailedProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    probe_detailed_with_options(reader, ProbeOptions::default())
}

/// Probes a file with additive expansion controls and returns the detailed movie, track, and
/// fragment summary.
pub fn probe_detailed_with_options<R>(
    reader: &mut R,
    options: ProbeOptions,
) -> Result<DetailedProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    Ok(strip_codec_details(probe_codec_detailed_with_options(
        reader, options,
    )?))
}

/// Probes a file and returns an additive detailed summary with parsed codec-specific
/// configuration when it is available.
pub fn probe_codec_detailed<R>(reader: &mut R) -> Result<CodecDetailedProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    probe_codec_detailed_with_options(reader, ProbeOptions::default())
}

/// Probes a file with additive expansion controls and returns the codec-detailed summary.
pub fn probe_codec_detailed_with_options<R>(
    reader: &mut R,
    options: ProbeOptions,
) -> Result<CodecDetailedProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    let paths = root_probe_box_paths(options);
    let infos = extract_boxes(reader, None, &paths)?;

    let mut summary = CodecDetailedProbeInfo::default();
    let mut mdat_appeared = false;

    for info in infos {
        match info.box_type() {
            FTYP => {
                let ftyp = read_payload_as::<_, crate::boxes::iso14496_12::Ftyp>(reader, &info)?;
                summary.major_brand = ftyp.major_brand;
                summary.minor_version = ftyp.minor_version;
                summary.compatible_brands = ftyp.compatible_brands;
            }
            MOOV => {
                summary.fast_start = !mdat_appeared;
            }
            MVHD => {
                let mvhd = read_payload_as::<_, Mvhd>(reader, &info)?;
                summary.timescale = mvhd.timescale;
                summary.duration = mvhd.duration();
            }
            TRAK => {
                summary
                    .tracks
                    .push(probe_trak_codec_detailed(reader, &info, options)?);
            }
            MOOF if options.include_segments => {
                summary.segments.push(probe_moof(reader, &info)?);
            }
            MDAT => {
                mdat_appeared = true;
            }
            _ => {}
        }
    }

    Ok(summary)
}

/// Probes a file and returns an additive summary with parsed codec and media characteristics.
pub fn probe_media_characteristics<R>(
    reader: &mut R,
) -> Result<MediaCharacteristicsProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    probe_media_characteristics_with_options(reader, ProbeOptions::default())
}

/// Probes a file with additive expansion controls and returns the media-characteristics summary.
pub fn probe_media_characteristics_with_options<R>(
    reader: &mut R,
    options: ProbeOptions,
) -> Result<MediaCharacteristicsProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    let paths = root_probe_box_paths(options);
    let infos = extract_boxes(reader, None, &paths)?;

    let mut summary = MediaCharacteristicsProbeInfo::default();
    let mut mdat_appeared = false;

    for info in infos {
        match info.box_type() {
            FTYP => {
                let ftyp = read_payload_as::<_, crate::boxes::iso14496_12::Ftyp>(reader, &info)?;
                summary.major_brand = ftyp.major_brand;
                summary.minor_version = ftyp.minor_version;
                summary.compatible_brands = ftyp.compatible_brands;
            }
            MOOV => {
                summary.fast_start = !mdat_appeared;
            }
            MVHD => {
                let mvhd = read_payload_as::<_, Mvhd>(reader, &info)?;
                summary.timescale = mvhd.timescale;
                summary.duration = mvhd.duration();
            }
            TRAK => {
                summary
                    .tracks
                    .push(probe_trak_media_characteristics(reader, &info, options)?);
            }
            MOOF if options.include_segments => {
                summary.segments.push(probe_moof(reader, &info)?);
            }
            MDAT => {
                mdat_appeared = true;
            }
            _ => {}
        }
    }

    Ok(summary)
}

/// Probes an in-memory MP4 byte slice and returns the coarse movie, track, and fragment
/// summary.
///
/// This is equivalent to calling [`probe`] with `Cursor<&[u8]>`.
pub fn probe_bytes(input: &[u8]) -> Result<ProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe(&mut reader)
}

/// Probes an in-memory MP4 byte slice with additive expansion controls and returns the coarse
/// movie, track, and fragment summary.
pub fn probe_bytes_with_options(
    input: &[u8],
    options: ProbeOptions,
) -> Result<ProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe_with_options(&mut reader, options)
}

/// Probes an in-memory MP4 byte slice and returns the additive detailed summary.
///
/// This is equivalent to calling [`probe_detailed`] with `Cursor<&[u8]>`.
pub fn probe_detailed_bytes(input: &[u8]) -> Result<DetailedProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe_detailed(&mut reader)
}

/// Probes an in-memory MP4 byte slice with additive expansion controls and returns the detailed
/// summary.
pub fn probe_detailed_bytes_with_options(
    input: &[u8],
    options: ProbeOptions,
) -> Result<DetailedProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe_detailed_with_options(&mut reader, options)
}

/// Probes an in-memory MP4 byte slice and returns the additive codec-detailed summary.
///
/// This is equivalent to calling [`probe_codec_detailed`] with `Cursor<&[u8]>`.
pub fn probe_codec_detailed_bytes(input: &[u8]) -> Result<CodecDetailedProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe_codec_detailed(&mut reader)
}

/// Probes an in-memory MP4 byte slice with additive expansion controls and returns the
/// codec-detailed summary.
pub fn probe_codec_detailed_bytes_with_options(
    input: &[u8],
    options: ProbeOptions,
) -> Result<CodecDetailedProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe_codec_detailed_with_options(&mut reader, options)
}

/// Probes an in-memory MP4 byte slice and returns the additive media-characteristics summary.
///
/// This is equivalent to calling [`probe_media_characteristics`] with `Cursor<&[u8]>`.
pub fn probe_media_characteristics_bytes(
    input: &[u8],
) -> Result<MediaCharacteristicsProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe_media_characteristics(&mut reader)
}

/// Probes an in-memory MP4 byte slice with additive expansion controls and returns the
/// media-characteristics summary.
pub fn probe_media_characteristics_bytes_with_options(
    input: &[u8],
    options: ProbeOptions,
) -> Result<MediaCharacteristicsProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe_media_characteristics_with_options(&mut reader, options)
}

/// Legacy fragmented-file probe entry point that currently aliases [`probe`].
pub fn probe_fra<R>(reader: &mut R) -> Result<ProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    probe(reader)
}

/// Legacy fragmented-file detailed probe entry point that currently aliases [`probe_detailed`].
pub fn probe_fra_detailed<R>(reader: &mut R) -> Result<DetailedProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    probe_detailed(reader)
}

/// Legacy fragmented-file codec-detailed probe entry point that currently aliases
/// [`probe_codec_detailed`].
pub fn probe_fra_codec_detailed<R>(reader: &mut R) -> Result<CodecDetailedProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    probe_codec_detailed(reader)
}

/// Legacy fragmented-file media-characteristics probe entry point that currently aliases
/// [`probe_media_characteristics`].
pub fn probe_fra_media_characteristics<R>(
    reader: &mut R,
) -> Result<MediaCharacteristicsProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    probe_media_characteristics(reader)
}

/// Legacy fragmented-file probe entry point for in-memory MP4 bytes.
///
/// This currently aliases [`probe_bytes`] for callers that already use the `probe_fra` naming.
pub fn probe_fra_bytes(input: &[u8]) -> Result<ProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe_fra(&mut reader)
}

/// Legacy fragmented-file detailed probe entry point for in-memory MP4 bytes.
///
/// This currently aliases [`probe_detailed_bytes`] for callers that already use the
/// `probe_fra` naming.
pub fn probe_fra_detailed_bytes(input: &[u8]) -> Result<DetailedProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe_fra_detailed(&mut reader)
}

/// Legacy fragmented-file codec-detailed probe entry point for in-memory MP4 bytes.
///
/// This currently aliases [`probe_codec_detailed_bytes`] for callers that already use the
/// `probe_fra` naming.
pub fn probe_fra_codec_detailed_bytes(input: &[u8]) -> Result<CodecDetailedProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe_fra_codec_detailed(&mut reader)
}

/// This currently aliases [`probe_media_characteristics_bytes`] for callers that already use the
/// fragmented-file helper naming.
pub fn probe_fra_media_characteristics_bytes(
    input: &[u8],
) -> Result<MediaCharacteristicsProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe_fra_media_characteristics(&mut reader)
}

/// Detects the AAC object profile exposed by an `esds` descriptor stream.
pub fn detect_aac_profile(esds: &Esds) -> Result<Option<AacProfileInfo>, ProbeError> {
    let Some(decoder_config) = esds.decoder_config_descriptor() else {
        return Ok(None);
    };
    if decoder_config.object_type_indication != 0x40 {
        return Ok(Some(AacProfileInfo {
            object_type_indication: decoder_config.object_type_indication,
            audio_object_type: 0,
        }));
    }

    let specific_info = esds
        .decoder_specific_info()
        .ok_or(ProbeError::MissingDescriptor(
            "decoder specific info descriptor",
        ))?;

    let mut reader = BitReader::new(Cursor::new(specific_info));
    let mut remaining_bits = specific_info.len() * 8;

    let (audio_object_type, read_bits) = get_audio_object_type(&mut reader)?;
    remaining_bits = remaining_bits.saturating_sub(read_bits);

    let sampling_frequency_index = read_bits_u8(&mut reader, 4)?;
    remaining_bits = remaining_bits.saturating_sub(4);
    if sampling_frequency_index == 0x0f {
        let _ = read_bits_u32(&mut reader, 24)?;
        remaining_bits = remaining_bits.saturating_sub(24);
    }

    if audio_object_type == 2 && remaining_bits >= 20 {
        let _ = read_bits_u8(&mut reader, 4)?;
        remaining_bits = remaining_bits.saturating_sub(4);
        let sync_extension_type = read_bits_u16(&mut reader, 11)?;
        remaining_bits = remaining_bits.saturating_sub(11);
        if sync_extension_type == 0x02b7 {
            let (ext_audio_object_type, _) = get_audio_object_type(&mut reader)?;
            if ext_audio_object_type == 5 || ext_audio_object_type == 22 {
                let sbr = read_bits_u8(&mut reader, 1)?;
                remaining_bits = remaining_bits.saturating_sub(1);
                if sbr != 0 {
                    if ext_audio_object_type == 5 {
                        let ext_sampling_frequency_index = read_bits_u8(&mut reader, 4)?;
                        remaining_bits = remaining_bits.saturating_sub(4);
                        if ext_sampling_frequency_index == 0x0f {
                            let _ = read_bits_u32(&mut reader, 24)?;
                            remaining_bits = remaining_bits.saturating_sub(24);
                        }
                        if remaining_bits >= 12 {
                            let sync_extension_type = read_bits_u16(&mut reader, 11)?;
                            if sync_extension_type == 0x0548 {
                                let ps = read_bits_u8(&mut reader, 1)?;
                                if ps != 0 {
                                    return Ok(Some(AacProfileInfo {
                                        object_type_indication: 0x40,
                                        audio_object_type: 29,
                                    }));
                                }
                            }
                        }
                    }

                    return Ok(Some(AacProfileInfo {
                        object_type_indication: 0x40,
                        audio_object_type: 5,
                    }));
                }
            }
        }
    }

    Ok(Some(AacProfileInfo {
        object_type_indication: 0x40,
        audio_object_type,
    }))
}

/// Finds sample indices whose AVC payload contains an IDR NAL unit.
pub fn find_idr_frames<R>(reader: &mut R, track: &TrackInfo) -> Result<Vec<usize>, ProbeError>
where
    R: Read + Seek,
{
    let Some(avc) = track.avc.as_ref() else {
        return Ok(Vec::new());
    };
    let length_size = u32::from(avc.length_size);

    let mut sample_index = 0usize;
    let mut indices = Vec::new();
    for chunk in &track.chunks {
        let end = sample_index.saturating_add(chunk.samples_per_chunk as usize);
        let mut data_offset = chunk.data_offset;
        while sample_index < end && sample_index < track.samples.len() {
            let sample = &track.samples[sample_index];
            if sample.size != 0 {
                let mut nal_offset = 0_u32;
                while nal_offset.saturating_add(length_size).saturating_add(1) <= sample.size {
                    reader.seek(SeekFrom::Start(data_offset + u64::from(nal_offset)))?;
                    let mut data = vec![0_u8; length_size as usize + 1];
                    reader.read_exact(&mut data)?;

                    let mut nal_length = 0_u32;
                    for byte in &data[..length_size as usize] {
                        nal_length = (nal_length << 8) | u32::from(*byte);
                    }
                    if data[length_size as usize] & 0x1f == 5 {
                        indices.push(sample_index);
                        break;
                    }

                    nal_offset = nal_offset
                        .saturating_add(length_size)
                        .saturating_add(nal_length);
                }
            }

            data_offset = data_offset.saturating_add(u64::from(sample.size));
            sample_index += 1;
        }
    }

    Ok(indices)
}

/// Returns the average bitrate implied by `samples` in the supplied timescale.
pub fn average_sample_bitrate(samples: &[SampleInfo], timescale: u32) -> u64 {
    let total_size = samples
        .iter()
        .map(|sample| u64::from(sample.size))
        .sum::<u64>();
    let total_duration = samples
        .iter()
        .map(|sample| u64::from(sample.time_delta))
        .sum::<u64>();
    if total_duration == 0 {
        return 0;
    }

    8 * total_size * u64::from(timescale) / total_duration
}

/// Returns the maximum rolling-window bitrate implied by `samples`.
pub fn max_sample_bitrate(samples: &[SampleInfo], timescale: u32, window_time_delta: u64) -> u64 {
    if window_time_delta == 0 || samples.is_empty() {
        return 0;
    }

    let mut max_bitrate = 0_u64;
    let mut size = 0_u64;
    let mut duration = 0_u64;
    let mut begin = 0usize;
    let mut end = 0usize;

    while end < samples.len() {
        while end < samples.len() {
            size += u64::from(samples[end].size);
            duration += u64::from(samples[end].time_delta);
            end += 1;
            if duration >= window_time_delta {
                break;
            }
        }

        if let Some(bitrate) = size
            .checked_mul(8)
            .and_then(|bits| bits.checked_mul(u64::from(timescale)))
            .and_then(|scaled_bits| scaled_bits.checked_div(duration))
        {
            max_bitrate = max_bitrate.max(bitrate);
        }

        while duration >= window_time_delta && begin < end {
            size -= u64::from(samples[begin].size);
            duration -= u64::from(samples[begin].time_delta);
            begin += 1;
        }
    }

    max_bitrate
}

/// Returns the average bitrate implied by `segments` for `track_id`.
pub fn average_segment_bitrate(segments: &[SegmentInfo], track_id: u32, timescale: u32) -> u64 {
    let mut total_size = 0_u64;
    let mut total_duration = 0_u64;
    for segment in segments {
        if segment.track_id == track_id {
            total_size += u64::from(segment.size);
            total_duration += u64::from(segment.duration);
        }
    }
    if total_duration == 0 {
        return 0;
    }

    8 * total_size * u64::from(timescale) / total_duration
}

/// Returns the maximum per-segment bitrate implied by `segments` for `track_id`.
pub fn max_segment_bitrate(segments: &[SegmentInfo], track_id: u32, timescale: u32) -> u64 {
    let mut max_bitrate = 0_u64;
    for segment in segments {
        if segment.track_id == track_id && segment.duration != 0 {
            let bitrate =
                8 * u64::from(segment.size) * u64::from(timescale) / u64::from(segment.duration);
            max_bitrate = max_bitrate.max(bitrate);
        }
    }
    max_bitrate
}

fn strip_probe_details(details: DetailedProbeInfo) -> ProbeInfo {
    ProbeInfo {
        major_brand: details.major_brand,
        minor_version: details.minor_version,
        compatible_brands: details.compatible_brands,
        fast_start: details.fast_start,
        timescale: details.timescale,
        duration: details.duration,
        tracks: details
            .tracks
            .into_iter()
            .map(|track| track.summary)
            .collect(),
        segments: details.segments,
    }
}

fn strip_codec_details(details: CodecDetailedProbeInfo) -> DetailedProbeInfo {
    DetailedProbeInfo {
        major_brand: details.major_brand,
        minor_version: details.minor_version,
        compatible_brands: details.compatible_brands,
        fast_start: details.fast_start,
        timescale: details.timescale,
        duration: details.duration,
        tracks: details
            .tracks
            .into_iter()
            .map(|track| track.summary)
            .collect(),
        segments: details.segments,
    }
}

fn root_probe_box_paths(options: ProbeOptions) -> Vec<BoxPath> {
    let mut paths = vec![
        BoxPath::from([FTYP]),
        BoxPath::from([MOOV]),
        BoxPath::from([MOOV, MVHD]),
        BoxPath::from([MOOV, TRAK]),
        BoxPath::from([MDAT]),
    ];
    if options.include_segments {
        paths.push(BoxPath::from([MOOF]));
    }
    paths
}

fn track_probe_box_paths(options: ProbeOptions) -> Vec<BoxPath> {
    let visual_sample_entries = [AVC1, HEV1, HVC1, AV01, VP08, VP09, ENCV];
    let audio_sample_entries = [MP4A, OPUS, AC_3, IPCM, FPCM, ENCA];
    let mut paths = vec![
        BoxPath::from([TKHD]),
        BoxPath::from([EDTS, ELST]),
        BoxPath::from([MDIA, MDHD]),
        BoxPath::from([MDIA, HDLR]),
        BoxPath::from([MDIA, MINF, STBL, STSD, AVC1]),
        BoxPath::from([MDIA, MINF, STBL, STSD, AVC1, AVCC]),
        BoxPath::from([MDIA, MINF, STBL, STSD, HEV1]),
        BoxPath::from([MDIA, MINF, STBL, STSD, HEV1, HVCC]),
        BoxPath::from([MDIA, MINF, STBL, STSD, HVC1]),
        BoxPath::from([MDIA, MINF, STBL, STSD, HVC1, HVCC]),
        BoxPath::from([MDIA, MINF, STBL, STSD, AV01]),
        BoxPath::from([MDIA, MINF, STBL, STSD, AV01, AV1C]),
        BoxPath::from([MDIA, MINF, STBL, STSD, VP08]),
        BoxPath::from([MDIA, MINF, STBL, STSD, VP08, VPCC]),
        BoxPath::from([MDIA, MINF, STBL, STSD, VP09]),
        BoxPath::from([MDIA, MINF, STBL, STSD, VP09, VPCC]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCV]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCV, AVCC]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCV, HVCC]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCV, AV1C]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCV, VPCC]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCV, SINF, FRMA]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCV, SINF, SCHM]),
        BoxPath::from([MDIA, MINF, STBL, STSD, MP4A]),
        BoxPath::from([MDIA, MINF, STBL, STSD, MP4A, ESDS]),
        BoxPath::from([MDIA, MINF, STBL, STSD, MP4A, WAVE, ESDS]),
        BoxPath::from([MDIA, MINF, STBL, STSD, OPUS]),
        BoxPath::from([MDIA, MINF, STBL, STSD, OPUS, DOPS]),
        BoxPath::from([MDIA, MINF, STBL, STSD, AC_3]),
        BoxPath::from([MDIA, MINF, STBL, STSD, AC_3, DAC3]),
        BoxPath::from([MDIA, MINF, STBL, STSD, IPCM]),
        BoxPath::from([MDIA, MINF, STBL, STSD, IPCM, PCMC]),
        BoxPath::from([MDIA, MINF, STBL, STSD, FPCM]),
        BoxPath::from([MDIA, MINF, STBL, STSD, FPCM, PCMC]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCA]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCA, ESDS]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCA, WAVE, ESDS]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCA, DOPS]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCA, DAC3]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCA, PCMC]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCA, SINF, FRMA]),
        BoxPath::from([MDIA, MINF, STBL, STSD, ENCA, SINF, SCHM]),
        BoxPath::from([MDIA, MINF, STBL, STSD, STPP]),
        BoxPath::from([MDIA, MINF, STBL, STSD, SBTT]),
        BoxPath::from([MDIA, MINF, STBL, STSD, WVTT]),
        BoxPath::from([MDIA, MINF, STBL, STSD, WVTT, VTTC_CONFIG]),
        BoxPath::from([MDIA, MINF, STBL, STSD, WVTT, VLAB]),
    ];

    for sample_entry in visual_sample_entries {
        paths.extend([
            BoxPath::from([MDIA, MINF, STBL, STSD, sample_entry, BTRT]),
            BoxPath::from([MDIA, MINF, STBL, STSD, sample_entry, COLR]),
            BoxPath::from([MDIA, MINF, STBL, STSD, sample_entry, PASP]),
            BoxPath::from([MDIA, MINF, STBL, STSD, sample_entry, FIEL]),
        ]);
    }

    for sample_entry in audio_sample_entries {
        paths.push(BoxPath::from([MDIA, MINF, STBL, STSD, sample_entry, BTRT]));
    }

    if options.expand_chunks {
        paths.extend([
            BoxPath::from([MDIA, MINF, STBL, STCO]),
            BoxPath::from([MDIA, MINF, STBL, CO64]),
            BoxPath::from([MDIA, MINF, STBL, STSC]),
        ]);
    }

    if options.expand_samples {
        paths.extend([
            BoxPath::from([MDIA, MINF, STBL, STTS]),
            BoxPath::from([MDIA, MINF, STBL, CTTS]),
            BoxPath::from([MDIA, MINF, STBL, STSZ]),
        ]);
    }

    paths
}

fn probe_trak_codec_detailed<R>(
    reader: &mut R,
    parent: &BoxInfo,
    options: ProbeOptions,
) -> Result<CodecDetailedTrackInfo, ProbeError>
where
    R: Read + Seek,
{
    let track = probe_trak_rich_details(reader, parent, options)?;
    Ok(CodecDetailedTrackInfo {
        summary: track.summary,
        codec_details: track.codec_details,
    })
}

fn probe_trak_media_characteristics<R>(
    reader: &mut R,
    parent: &BoxInfo,
    options: ProbeOptions,
) -> Result<MediaCharacteristicsTrackInfo, ProbeError>
where
    R: Read + Seek,
{
    let track = probe_trak_rich_details(reader, parent, options)?;
    Ok(MediaCharacteristicsTrackInfo {
        summary: track.summary,
        codec_details: track.codec_details,
        media_characteristics: track.media_characteristics,
    })
}

fn probe_trak_rich_details<R>(
    reader: &mut R,
    parent: &BoxInfo,
    options: ProbeOptions,
) -> Result<ParsedRichTrackInfo, ProbeError>
where
    R: Read + Seek,
{
    let paths = track_probe_box_paths(options);
    let boxes = extract_boxes_with_payload(reader, Some(parent), &paths)?;

    let mut track = DetailedTrackInfo::default();
    let mut tkhd = None;
    let mut mdhd = None;
    let mut visual_sample_entry = None;
    let mut avcc = None;
    let mut hvcc = None;
    let mut av1c = None;
    let mut vpcc = None;
    let mut audio_sample_entry = None;
    let mut esds = None;
    let mut dops = None;
    let mut dac3 = None;
    let mut pcmc = None;
    let mut xml_subtitle_sample_entry = None;
    let mut text_subtitle_sample_entry = None;
    let mut webvtt_configuration = None;
    let mut webvtt_source_label = None;
    let mut btrt = None;
    let mut colr = None;
    let mut pasp = None;
    let mut fiel = None;
    let mut original_format = None;
    let mut stco = None;
    let mut co64 = None;
    let mut stts = None;
    let mut ctts = None;
    let mut stsc = None;
    let mut stsz = None;

    for extracted in boxes {
        match extracted.info.box_type() {
            TKHD => {
                let payload = downcast_clone::<Tkhd>(&extracted)?;
                track.summary.track_id = payload.track_id;
                tkhd = Some(payload);
            }
            ELST => {
                let elst = downcast_clone::<crate::boxes::iso14496_12::Elst>(&extracted)?;
                track.summary.edit_list = elst
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(index, _)| EditListEntry {
                        media_time: elst.media_time(index),
                        segment_duration: elst.segment_duration(index),
                    })
                    .collect();
            }
            MDHD => {
                let payload = downcast_clone::<crate::boxes::iso14496_12::Mdhd>(&extracted)?;
                track.summary.timescale = payload.timescale;
                track.summary.duration = payload.duration();
                track.language = Some(decode_language(payload.language));
                mdhd = Some(payload);
            }
            HDLR => {
                let payload = downcast_clone::<Hdlr>(&extracted)?;
                track.handler_type = Some(payload.handler_type);
            }
            AVC1 => {
                track.summary.codec = TrackCodec::Avc1;
                track.codec_family = TrackCodecFamily::Avc;
                track.sample_entry_type = Some(AVC1);
                visual_sample_entry = Some(downcast_clone::<VisualSampleEntry>(&extracted)?);
            }
            AVCC => {
                avcc = Some(downcast_clone::<AVCDecoderConfiguration>(&extracted)?);
            }
            HVCC => {
                hvcc = Some(downcast_clone::<HEVCDecoderConfiguration>(&extracted)?);
            }
            HEV1 => {
                track.codec_family = TrackCodecFamily::Hevc;
                track.sample_entry_type = Some(HEV1);
                visual_sample_entry = Some(downcast_clone::<VisualSampleEntry>(&extracted)?);
            }
            HVC1 => {
                track.codec_family = TrackCodecFamily::Hevc;
                track.sample_entry_type = Some(HVC1);
                visual_sample_entry = Some(downcast_clone::<VisualSampleEntry>(&extracted)?);
            }
            AV01 => {
                track.codec_family = TrackCodecFamily::Av1;
                track.sample_entry_type = Some(AV01);
                visual_sample_entry = Some(downcast_clone::<VisualSampleEntry>(&extracted)?);
            }
            AV1C => {
                av1c = Some(downcast_clone::<AV1CodecConfiguration>(&extracted)?);
            }
            VP08 => {
                track.codec_family = TrackCodecFamily::Vp8;
                track.sample_entry_type = Some(VP08);
                visual_sample_entry = Some(downcast_clone::<VisualSampleEntry>(&extracted)?);
            }
            VP09 => {
                track.codec_family = TrackCodecFamily::Vp9;
                track.sample_entry_type = Some(VP09);
                visual_sample_entry = Some(downcast_clone::<VisualSampleEntry>(&extracted)?);
            }
            VPCC => {
                vpcc = Some(downcast_clone::<VpCodecConfiguration>(&extracted)?);
            }
            ENCV => {
                track.summary.codec = TrackCodec::Avc1;
                track.summary.encrypted = true;
                track.sample_entry_type = Some(ENCV);
                visual_sample_entry = Some(downcast_clone::<VisualSampleEntry>(&extracted)?);
            }
            MP4A => {
                track.summary.codec = TrackCodec::Mp4a;
                track.codec_family = TrackCodecFamily::Mp4Audio;
                track.sample_entry_type = Some(MP4A);
                audio_sample_entry = Some(downcast_clone::<AudioSampleEntry>(&extracted)?);
            }
            ENCA => {
                track.summary.codec = TrackCodec::Mp4a;
                track.summary.encrypted = true;
                track.sample_entry_type = Some(ENCA);
                audio_sample_entry = Some(downcast_clone::<AudioSampleEntry>(&extracted)?);
            }
            OPUS => {
                track.codec_family = TrackCodecFamily::Opus;
                track.sample_entry_type = Some(OPUS);
                audio_sample_entry = Some(downcast_clone::<AudioSampleEntry>(&extracted)?);
            }
            DOPS => {
                dops = Some(downcast_clone::<DOps>(&extracted)?);
            }
            AC_3 => {
                track.codec_family = TrackCodecFamily::Ac3;
                track.sample_entry_type = Some(AC_3);
                audio_sample_entry = Some(downcast_clone::<AudioSampleEntry>(&extracted)?);
            }
            DAC3 => {
                dac3 = Some(downcast_clone::<Dac3>(&extracted)?);
            }
            IPCM => {
                track.codec_family = TrackCodecFamily::Pcm;
                track.sample_entry_type = Some(IPCM);
                audio_sample_entry = Some(downcast_clone::<AudioSampleEntry>(&extracted)?);
            }
            FPCM => {
                track.codec_family = TrackCodecFamily::Pcm;
                track.sample_entry_type = Some(FPCM);
                audio_sample_entry = Some(downcast_clone::<AudioSampleEntry>(&extracted)?);
            }
            PCMC => {
                pcmc = Some(downcast_clone::<PcmC>(&extracted)?);
            }
            STPP => {
                track.codec_family = TrackCodecFamily::XmlSubtitle;
                track.sample_entry_type = Some(STPP);
                xml_subtitle_sample_entry =
                    Some(downcast_clone::<XMLSubtitleSampleEntry>(&extracted)?);
            }
            SBTT => {
                track.codec_family = TrackCodecFamily::TextSubtitle;
                track.sample_entry_type = Some(SBTT);
                text_subtitle_sample_entry =
                    Some(downcast_clone::<TextSubtitleSampleEntry>(&extracted)?);
            }
            WVTT => {
                track.codec_family = TrackCodecFamily::WebVtt;
                track.sample_entry_type = Some(WVTT);
            }
            VTTC_CONFIG => {
                webvtt_configuration = Some(downcast_clone::<WebVTTConfigurationBox>(&extracted)?);
            }
            VLAB => {
                webvtt_source_label = Some(downcast_clone::<WebVTTSourceLabelBox>(&extracted)?);
            }
            BTRT => {
                btrt = Some(downcast_clone::<Btrt>(&extracted)?);
            }
            COLR => {
                colr = Some(downcast_clone::<Colr>(&extracted)?);
            }
            PASP => {
                pasp = Some(downcast_clone::<Pasp>(&extracted)?);
            }
            FIEL => {
                fiel = Some(downcast_clone::<Fiel>(&extracted)?);
            }
            ESDS => {
                esds = Some(downcast_clone::<Esds>(&extracted)?);
            }
            FRMA => {
                let payload = downcast_clone::<Frma>(&extracted)?;
                original_format = Some(payload.data_format);
                track.original_format = Some(payload.data_format);
            }
            SCHM => {
                let payload = downcast_clone::<Schm>(&extracted)?;
                track.protection_scheme = Some(ProtectionSchemeInfo {
                    scheme_type: payload.scheme_type,
                    scheme_version: payload.scheme_version,
                });
            }
            STCO => {
                stco = Some(downcast_clone::<Stco>(&extracted)?);
            }
            CO64 => {
                co64 = Some(downcast_clone::<Co64>(&extracted)?);
            }
            STTS => {
                stts = Some(downcast_clone::<Stts>(&extracted)?);
            }
            CTTS => {
                ctts = Some(downcast_clone::<Ctts>(&extracted)?);
            }
            STSC => {
                stsc = Some(downcast_clone::<Stsc>(&extracted)?);
            }
            STSZ => {
                stsz = Some(downcast_clone::<Stsz>(&extracted)?);
            }
            _ => {}
        }
    }

    if tkhd.is_none() {
        return Err(ProbeError::MissingRequiredBox("tkhd"));
    }
    if mdhd.is_none() {
        return Err(ProbeError::MissingRequiredBox("mdhd"));
    }

    if let Some(entry) = visual_sample_entry.as_ref() {
        track.display_width = Some(entry.width);
        track.display_height = Some(entry.height);
    }

    if let Some(entry) = audio_sample_entry.as_ref() {
        track.channel_count = Some(entry.channel_count);
        track.sample_rate = Some(entry.sample_rate_int());
    }

    if let Some(original_format) = original_format {
        track.codec_family = codec_family_from_sample_entry(original_format);
    } else if let Some(sample_entry_type) = track.sample_entry_type {
        track.codec_family = codec_family_from_sample_entry(sample_entry_type);
    }

    if let (Some(entry), Some(avcc)) = (visual_sample_entry.as_ref(), avcc.as_ref()) {
        track.summary.avc = Some(AvcDecoderConfigInfo {
            configuration_version: avcc.configuration_version,
            profile: avcc.profile,
            profile_compatibility: avcc.profile_compatibility,
            level: avcc.level,
            length_size: u16::from(avcc.length_size_minus_one) + 1,
            width: entry.width,
            height: entry.height,
        });
    }

    if let (Some(entry), Some(esds)) = (audio_sample_entry.as_ref(), esds.as_ref())
        && let Some(profile) = detect_aac_profile(esds)?
    {
        track.summary.mp4a = Some(Mp4aInfo {
            object_type_indication: profile.object_type_indication,
            audio_object_type: profile.audio_object_type,
            channel_count: entry.channel_count,
        });
    }

    if options.expand_chunks {
        if let Some(stco) = stco.as_ref() {
            track
                .summary
                .chunks
                .extend(stco.chunk_offset.iter().map(|offset| ChunkInfo {
                    data_offset: *offset,
                    samples_per_chunk: 0,
                }));
        } else if let Some(co64) = co64.as_ref() {
            track
                .summary
                .chunks
                .extend(co64.chunk_offset.iter().map(|offset| ChunkInfo {
                    data_offset: *offset,
                    samples_per_chunk: 0,
                }));
        } else {
            return Err(ProbeError::MissingRequiredBox("stco/co64"));
        }

        let stsc = stsc.ok_or(ProbeError::MissingRequiredBox("stsc"))?;
        for (index, entry) in stsc.entries.iter().enumerate() {
            let mut end = track.summary.chunks.len() as u32;
            if index + 1 != stsc.entries.len() {
                end = end.min(stsc.entries[index + 1].first_chunk.saturating_sub(1));
            }
            for chunk_index in entry.first_chunk.saturating_sub(1)..end {
                if let Some(chunk) = track.summary.chunks.get_mut(chunk_index as usize) {
                    chunk.samples_per_chunk = entry.samples_per_chunk;
                }
            }
        }
    }

    if options.expand_samples {
        let stts = stts.ok_or(ProbeError::MissingRequiredBox("stts"))?;
        for entry in &stts.entries {
            for _ in 0..entry.sample_count {
                track.summary.samples.push(SampleInfo {
                    time_delta: entry.sample_delta,
                    ..SampleInfo::default()
                });
            }
        }

        if let Some(ctts) = ctts.as_ref() {
            let mut sample_index = 0usize;
            for (entry_index, entry) in ctts.entries.iter().enumerate() {
                for _ in 0..entry.sample_count {
                    if sample_index >= track.summary.samples.len() {
                        break;
                    }
                    track.summary.samples[sample_index].composition_time_offset =
                        ctts.sample_offset(entry_index);
                    sample_index += 1;
                }
            }
        }

        if let Some(stsz) = stsz.as_ref() {
            if stsz.sample_size != 0 {
                for sample in &mut track.summary.samples {
                    sample.size = stsz.sample_size;
                }
            } else {
                for (sample, entry_size) in
                    track.summary.samples.iter_mut().zip(stsz.entry_size.iter())
                {
                    sample.size =
                        (*entry_size)
                            .try_into()
                            .map_err(|_| ProbeError::NumericOverflow {
                                field_name: "stsz entry size",
                            })?;
                }
            }
        }
    }
    let codec_details = build_track_codec_details(
        &track,
        &TrackCodecConfigRefs {
            avcc: avcc.as_ref(),
            hvcc: hvcc.as_ref(),
            av1c: av1c.as_ref(),
            vpcc: vpcc.as_ref(),
            dops: dops.as_ref(),
            dac3: dac3.as_ref(),
            pcmc: pcmc.as_ref(),
            xml_subtitle_sample_entry: xml_subtitle_sample_entry.as_ref(),
            text_subtitle_sample_entry: text_subtitle_sample_entry.as_ref(),
            webvtt_configuration: webvtt_configuration.as_ref(),
            webvtt_source_label: webvtt_source_label.as_ref(),
        },
    );
    let media_characteristics = build_track_media_characteristics(&TrackMediaCharacteristicRefs {
        btrt: btrt.as_ref(),
        colr: colr.as_ref(),
        pasp: pasp.as_ref(),
        fiel: fiel.as_ref(),
    });

    Ok(ParsedRichTrackInfo {
        summary: track,
        codec_details,
        media_characteristics,
    })
}

fn codec_family_from_sample_entry(sample_entry_type: FourCc) -> TrackCodecFamily {
    match sample_entry_type {
        AVC1 => TrackCodecFamily::Avc,
        HEV1 | HVC1 => TrackCodecFamily::Hevc,
        AV01 => TrackCodecFamily::Av1,
        VP08 => TrackCodecFamily::Vp8,
        VP09 => TrackCodecFamily::Vp9,
        MP4A => TrackCodecFamily::Mp4Audio,
        OPUS => TrackCodecFamily::Opus,
        AC_3 => TrackCodecFamily::Ac3,
        IPCM | FPCM => TrackCodecFamily::Pcm,
        STPP => TrackCodecFamily::XmlSubtitle,
        SBTT => TrackCodecFamily::TextSubtitle,
        WVTT => TrackCodecFamily::WebVtt,
        _ => TrackCodecFamily::Unknown,
    }
}

fn build_track_codec_details(
    track: &DetailedTrackInfo,
    config_refs: &TrackCodecConfigRefs<'_>,
) -> TrackCodecDetails {
    if let Some(avc) = track.summary.avc.as_ref() {
        return TrackCodecDetails::Avc(AvcCodecDetails {
            configuration_version: avc.configuration_version,
            profile: avc.profile,
            profile_compatibility: avc.profile_compatibility,
            level: avc.level,
            length_size: avc.length_size,
            chroma_format: config_refs
                .avcc
                .filter(|config| config.high_profile_fields_enabled)
                .map(|config| config.chroma_format),
            bit_depth_luma: config_refs
                .avcc
                .filter(|config| config.high_profile_fields_enabled)
                .map(|config| config.bit_depth_luma_minus8.saturating_add(8)),
            bit_depth_chroma: config_refs
                .avcc
                .filter(|config| config.high_profile_fields_enabled)
                .map(|config| config.bit_depth_chroma_minus8.saturating_add(8)),
        });
    }

    if let Some(mp4a) = track.summary.mp4a.as_ref() {
        return TrackCodecDetails::Mp4Audio(Mp4AudioCodecDetails {
            object_type_indication: mp4a.object_type_indication,
            audio_object_type: mp4a.audio_object_type,
            channel_count: mp4a.channel_count,
            sample_rate: track.sample_rate,
        });
    }

    match track.codec_family {
        TrackCodecFamily::Hevc => {
            if let Some(hvcc) = config_refs.hvcc {
                return TrackCodecDetails::Hevc(HevcCodecDetails {
                    configuration_version: hvcc.configuration_version,
                    profile_space: hvcc.general_profile_space,
                    tier_flag: hvcc.general_tier_flag,
                    profile_idc: hvcc.general_profile_idc,
                    profile_compatibility_mask: hevc_profile_compatibility_mask(
                        &hvcc.general_profile_compatibility,
                    ),
                    constraint_indicator: hvcc.general_constraint_indicator,
                    level_idc: hvcc.general_level_idc,
                    min_spatial_segmentation_idc: hvcc.min_spatial_segmentation_idc,
                    parallelism_type: hvcc.parallelism_type,
                    chroma_format_idc: hvcc.chroma_format_idc,
                    bit_depth_luma: hvcc.bit_depth_luma_minus8.saturating_add(8),
                    bit_depth_chroma: hvcc.bit_depth_chroma_minus8.saturating_add(8),
                    avg_frame_rate: hvcc.avg_frame_rate,
                    constant_frame_rate: hvcc.constant_frame_rate,
                    num_temporal_layers: hvcc.num_temporal_layers,
                    temporal_id_nested: hvcc.temporal_id_nested,
                    length_size: u16::from(hvcc.length_size_minus_one) + 1,
                });
            }
        }
        TrackCodecFamily::Av1 => {
            if let Some(av1c) = config_refs.av1c {
                return TrackCodecDetails::Av1(Av1CodecDetails {
                    seq_profile: av1c.seq_profile,
                    seq_level_idx_0: av1c.seq_level_idx_0,
                    seq_tier_0: av1c.seq_tier_0,
                    bit_depth: av1_bit_depth(av1c),
                    monochrome: av1c.monochrome != 0,
                    chroma_subsampling_x: av1c.chroma_subsampling_x,
                    chroma_subsampling_y: av1c.chroma_subsampling_y,
                    chroma_sample_position: av1c.chroma_sample_position,
                    initial_presentation_delay_minus_one: if av1c.initial_presentation_delay_present
                        != 0
                    {
                        Some(av1c.initial_presentation_delay_minus_one)
                    } else {
                        None
                    },
                });
            }
        }
        TrackCodecFamily::Vp8 => {
            if let Some(vpcc) = config_refs.vpcc {
                return TrackCodecDetails::Vp8(vp_codec_details(vpcc));
            }
        }
        TrackCodecFamily::Vp9 => {
            if let Some(vpcc) = config_refs.vpcc {
                return TrackCodecDetails::Vp9(vp_codec_details(vpcc));
            }
        }
        TrackCodecFamily::Opus => {
            if let Some(dops) = config_refs.dops {
                return TrackCodecDetails::Opus(OpusCodecDetails {
                    output_channel_count: dops.output_channel_count,
                    pre_skip: dops.pre_skip,
                    input_sample_rate: dops.input_sample_rate,
                    output_gain: dops.output_gain,
                    channel_mapping_family: dops.channel_mapping_family,
                    stream_count: if dops.channel_mapping_family != 0 {
                        Some(dops.stream_count)
                    } else {
                        None
                    },
                    coupled_count: if dops.channel_mapping_family != 0 {
                        Some(dops.coupled_count)
                    } else {
                        None
                    },
                    channel_mapping: if dops.channel_mapping_family != 0 {
                        dops.channel_mapping.clone()
                    } else {
                        Vec::new()
                    },
                });
            }
        }
        TrackCodecFamily::Ac3 => {
            if let Some(dac3) = config_refs.dac3 {
                return TrackCodecDetails::Ac3(Ac3CodecDetails {
                    sample_rate_code: dac3.fscod,
                    bit_stream_identification: dac3.bsid,
                    bit_stream_mode: dac3.bsmod,
                    audio_coding_mode: dac3.acmod,
                    lfe_on: dac3.lfe_on != 0,
                    bit_rate_code: dac3.bit_rate_code,
                });
            }
        }
        TrackCodecFamily::Pcm => {
            if let Some(pcmc) = config_refs.pcmc {
                return TrackCodecDetails::Pcm(PcmCodecDetails {
                    format_flags: pcmc.format_flags,
                    sample_size: pcmc.pcm_sample_size,
                });
            }
        }
        TrackCodecFamily::XmlSubtitle => {
            if let Some(entry) = config_refs.xml_subtitle_sample_entry {
                return TrackCodecDetails::XmlSubtitle(XmlSubtitleCodecDetails {
                    namespace: entry.namespace.clone(),
                    schema_location: entry.schema_location.clone(),
                    auxiliary_mime_types: entry.auxiliary_mime_types.clone(),
                });
            }
        }
        TrackCodecFamily::TextSubtitle => {
            if let Some(entry) = config_refs.text_subtitle_sample_entry {
                return TrackCodecDetails::TextSubtitle(TextSubtitleCodecDetails {
                    content_encoding: entry.content_encoding.clone(),
                    mime_format: entry.mime_format.clone(),
                });
            }
        }
        TrackCodecFamily::WebVtt => {
            if config_refs.webvtt_configuration.is_some()
                || config_refs.webvtt_source_label.is_some()
            {
                return TrackCodecDetails::WebVtt(WebVttCodecDetails {
                    config: config_refs
                        .webvtt_configuration
                        .map(|value| value.config.clone()),
                    source_label: config_refs
                        .webvtt_source_label
                        .map(|value| value.source_label.clone()),
                });
            }
        }
        TrackCodecFamily::Unknown | TrackCodecFamily::Avc | TrackCodecFamily::Mp4Audio => {}
    }

    TrackCodecDetails::Unknown
}

fn build_track_media_characteristics(
    refs: &TrackMediaCharacteristicRefs<'_>,
) -> TrackMediaCharacteristics {
    TrackMediaCharacteristics {
        declared_bitrate: refs.btrt.map(|value| DeclaredBitrateInfo {
            buffer_size_db: value.buffer_size_db,
            max_bitrate: value.max_bitrate,
            avg_bitrate: value.avg_bitrate,
        }),
        color: refs.colr.map(track_color_info),
        pixel_aspect_ratio: refs.pasp.map(|value| PixelAspectRatioInfo {
            h_spacing: value.h_spacing,
            v_spacing: value.v_spacing,
        }),
        field_order: refs.fiel.map(track_field_order_info),
    }
}

fn track_color_info(value: &Colr) -> ColorInfo {
    let is_nclx = value.colour_type == COLR_NCLX;
    let stores_profile = matches!(value.colour_type, COLR_RICC | COLR_PROF);
    ColorInfo {
        colour_type: value.colour_type,
        colour_primaries: is_nclx.then_some(value.colour_primaries),
        transfer_characteristics: is_nclx.then_some(value.transfer_characteristics),
        matrix_coefficients: is_nclx.then_some(value.matrix_coefficients),
        full_range: is_nclx.then_some(value.full_range_flag),
        profile_size: stores_profile.then_some(value.profile.len()),
        unknown_size: (!is_nclx && !stores_profile).then_some(value.unknown.len()),
    }
}

fn track_field_order_info(value: &Fiel) -> FieldOrderInfo {
    FieldOrderInfo {
        field_count: value.field_count,
        field_ordering: value.field_ordering,
        // `fiel` uses `1` for progressive content and multiple fields for interlaced layouts.
        interlaced: value.field_count > 1,
    }
}

fn hevc_profile_compatibility_mask(flags: &[bool; 32]) -> u32 {
    let mut mask = 0_u32;
    for (index, value) in flags.iter().copied().enumerate() {
        if value {
            mask |= 1_u32 << (31 - index);
        }
    }
    mask
}

fn av1_bit_depth(config: &AV1CodecConfiguration) -> u8 {
    if config.high_bitdepth == 0 {
        8
    } else if config.twelve_bit != 0 {
        12
    } else {
        10
    }
}

fn vp_codec_details(config: &VpCodecConfiguration) -> VpCodecDetails {
    VpCodecDetails {
        profile: config.profile,
        level: config.level,
        bit_depth: config.bit_depth,
        chroma_subsampling: config.chroma_subsampling,
        full_range: config.video_full_range_flag != 0,
        colour_primaries: config.colour_primaries,
        transfer_characteristics: config.transfer_characteristics,
        matrix_coefficients: config.matrix_coefficients,
        codec_initialization_data_size: config.codec_initialization_data_size,
    }
}

fn decode_language(language: [u8; 3]) -> String {
    language
        .into_iter()
        .map(|value| char::from(value.saturating_add(0x60)))
        .collect()
}

fn probe_moof<R>(reader: &mut R, parent: &BoxInfo) -> Result<SegmentInfo, ProbeError>
where
    R: Read + Seek,
{
    let boxes = extract_boxes_with_payload(
        reader,
        Some(parent),
        &[
            BoxPath::from([TRAF, TFHD]),
            BoxPath::from([TRAF, TFDT]),
            BoxPath::from([TRAF, TRUN]),
        ],
    )?;

    let mut tfhd = None;
    let mut tfdt = None;
    let mut trun = None;
    for extracted in boxes {
        match extracted.info.box_type() {
            TFHD => tfhd = Some(downcast_clone::<Tfhd>(&extracted)?),
            TFDT => tfdt = Some(downcast_clone::<Tfdt>(&extracted)?),
            TRUN => trun = Some(downcast_clone::<Trun>(&extracted)?),
            _ => {}
        }
    }

    let tfhd = tfhd.ok_or(ProbeError::MissingRequiredBox("tfhd"))?;
    let mut segment = SegmentInfo {
        track_id: tfhd.track_id,
        moof_offset: parent.offset(),
        default_sample_duration: tfhd.default_sample_duration,
        ..SegmentInfo::default()
    };

    if let Some(tfdt) = tfdt.as_ref() {
        segment.base_media_decode_time = tfdt.base_media_decode_time();
    }

    if let Some(trun) = trun.as_ref() {
        segment.sample_count = trun.sample_count;

        if trun.flags() & crate::boxes::iso14496_12::TRUN_SAMPLE_DURATION_PRESENT != 0 {
            segment.duration = trun
                .entries
                .iter()
                .map(|entry| entry.sample_duration)
                .sum::<u32>();
        } else {
            segment.duration = tfhd
                .default_sample_duration
                .saturating_mul(segment.sample_count);
        }

        if trun.flags() & crate::boxes::iso14496_12::TRUN_SAMPLE_SIZE_PRESENT != 0 {
            segment.size = trun
                .entries
                .iter()
                .map(|entry| entry.sample_size)
                .sum::<u32>();
        } else {
            segment.size = tfhd
                .default_sample_size
                .saturating_mul(segment.sample_count);
        }

        let mut duration = 0_u32;
        let mut min_offset = None;
        for (index, entry) in trun.entries.iter().enumerate() {
            let offset = i64::from(duration) + trun.sample_composition_time_offset(index);
            min_offset = Some(min_offset.map_or(offset, |current: i64| current.min(offset)));
            duration = duration.saturating_add(
                if trun.flags() & crate::boxes::iso14496_12::TRUN_SAMPLE_DURATION_PRESENT != 0 {
                    entry.sample_duration
                } else {
                    tfhd.default_sample_duration
                },
            );
        }
        if let Some(offset) = min_offset {
            segment.composition_time_offset =
                offset.try_into().map_err(|_| ProbeError::NumericOverflow {
                    field_name: "segment composition time offset",
                })?;
        }
    }

    Ok(segment)
}

fn read_payload_as<R, B>(reader: &mut R, info: &BoxInfo) -> Result<B, ProbeError>
where
    R: Read + Seek,
    B: CodecBox + Default,
{
    info.seek_to_payload(reader)?;
    let mut decoded = B::default();
    unmarshal(reader, info.payload_size()?, &mut decoded, None)?;
    Ok(decoded)
}

fn downcast_clone<T>(extracted: &ExtractedBox) -> Result<T, ProbeError>
where
    T: Clone + 'static,
{
    extracted
        .payload
        .as_ref()
        .as_any()
        .downcast_ref::<T>()
        .cloned()
        .ok_or(ProbeError::UnexpectedPayloadType {
            box_type: extracted.info.box_type(),
        })
}

fn get_audio_object_type<R>(reader: &mut BitReader<R>) -> Result<(u8, usize), ProbeError>
where
    R: Read,
{
    let audio_object_type = read_bits_u8(reader, 5)?;
    if audio_object_type != 0x1f {
        return Ok((audio_object_type, 5));
    }

    let extended = read_bits_u8(reader, 6)?;
    Ok((extended.saturating_add(32), 11))
}

fn read_bits_u8<R>(reader: &mut BitReader<R>, width: usize) -> Result<u8, ProbeError>
where
    R: Read,
{
    let bits = reader.read_bits(width).map_err(ProbeError::Io)?;
    let mut value = 0_u16;
    for byte in bits {
        value = (value << 8) | u16::from(byte);
    }
    u8::try_from(value).map_err(|_| ProbeError::NumericOverflow {
        field_name: "bitfield read",
    })
}

fn read_bits_u16<R>(reader: &mut BitReader<R>, width: usize) -> Result<u16, ProbeError>
where
    R: Read,
{
    let bits = reader.read_bits(width).map_err(ProbeError::Io)?;
    let mut value = 0_u32;
    for byte in bits {
        value = (value << 8) | u32::from(byte);
    }
    u16::try_from(value).map_err(|_| ProbeError::NumericOverflow {
        field_name: "bitfield read",
    })
}

fn read_bits_u32<R>(reader: &mut BitReader<R>, width: usize) -> Result<u32, ProbeError>
where
    R: Read,
{
    let bits = reader.read_bits(width).map_err(ProbeError::Io)?;
    let mut value = 0_u64;
    for byte in bits {
        value = (value << 8) | u64::from(byte);
    }
    u32::try_from(value).map_err(|_| ProbeError::NumericOverflow {
        field_name: "bitfield read",
    })
}

/// Errors raised while probing files or derived codec summaries.
#[derive(Debug)]
pub enum ProbeError {
    Io(io::Error),
    Header(HeaderError),
    Codec(CodecError),
    Extract(ExtractError),
    MissingRequiredBox(&'static str),
    MissingDescriptor(&'static str),
    UnexpectedPayloadType { box_type: FourCc },
    NumericOverflow { field_name: &'static str },
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Header(error) => error.fmt(f),
            Self::Codec(error) => error.fmt(f),
            Self::Extract(error) => error.fmt(f),
            Self::MissingRequiredBox(name) => write!(f, "{name} box not found"),
            Self::MissingDescriptor(name) => write!(f, "{name} not found"),
            Self::UnexpectedPayloadType { box_type } => {
                write!(f, "unexpected payload type for {box_type}")
            }
            Self::NumericOverflow { field_name } => {
                write!(f, "numeric value does not fit while reading {field_name}")
            }
        }
    }
}

impl Error for ProbeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::Extract(error) => Some(error),
            Self::MissingRequiredBox(..)
            | Self::MissingDescriptor(..)
            | Self::UnexpectedPayloadType { .. }
            | Self::NumericOverflow { .. } => None,
        }
    }
}

impl From<io::Error> for ProbeError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<HeaderError> for ProbeError {
    fn from(value: HeaderError) -> Self {
        Self::Header(value)
    }
}

impl From<CodecError> for ProbeError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}

impl From<ExtractError> for ProbeError {
    fn from(value: ExtractError) -> Self {
        Self::Extract(value)
    }
}
