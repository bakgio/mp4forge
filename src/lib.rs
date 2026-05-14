//! MP4 and ISOBMFF toolkit with low-level building blocks and thin ergonomic helpers.
//!
//! The default surface is synchronous. Enable the optional `async` feature when you want the
//! additive Tokio-based library companions for seekable readers and writers. That async surface is
//! intended for supported seekable Tokio I/O such as `tokio::fs::File` and seekable in-memory
//! cursors, and it supports normal multithreaded `tokio::spawn` use for independent-file library
//! work. The CLI remains on the synchronous path.
//!
//! Enable the optional `decrypt` feature when you want the additive decryption input and
//! progress types plus the feature-gated decryption surface. That landed surface covers the
//! Common Encryption family, PIFF compatibility, OMA DCF, Marlin IPMP, and the retained IAEC
//! protected-movie path while keeping the CLI on the synchronous path. Enable both `decrypt` and
//! `async` when you want the additive file-backed async decrypt companions on top of the existing
//! synchronous in-memory decrypt helpers.
//!
//! Enable the optional `mux` feature when you want the additive mux task surface plus the retained
//! low-level helpers underneath it. The mux surface exposes track-based `MuxRequest` helpers for
//! sync and async real MP4 assembly, path-first repeated track-spec parsing aligned with the
//! sync-only CLI, internal chunk and duration coordination on top of one mux event graph,
//! retained low-level staged payload-copy helpers, the public `mp4forge::mux::sample_reader`
//! module built on staged mux plans, the public `mp4forge::mux::inspect` module for path-first
//! direct-ingest inspection and export plus additive packet-focused reports, and the public
//! `mp4forge::mux::rewrite` module for rewriting extracted AVC/HEVC/VVC sample payloads back into
//! Annex B plus additive AV1, AAC ADTS, and MHAS elementary export helpers.
//! Those sample-reader helpers can also expose stable text or subtitle track identity when you
//! construct them with companion `MuxTrackConfig` values. The current path-first mux surface
//! accepts one repeated input path with optional selector suffixes such as `#video`, `#audio`,
//! `#text`, or `#track:ID`. Path-only MP4 inputs import every supported track from that source,
//! while the landed path-only raw auto-detection currently
//! covers MP4, supported AVI audio streams plus H.263/JPEG/PNG/MPEG-4 Part 2/H.264/AVC1 video streams, supported
//! MPEG-PS MPEG audio streams plus MPEG-4 Part 2/H.264/H.265/VVC video streams, supported
//! MPEG-TS MPEG audio streams plus AAC LATM/MHAS plus AC-3/E-AC-3/AC-4/DTS/TrueHD audio plus MPEG-2/AV1/AVS3/MPEG-4 Part 2/H.264/H.265/VVC
//! video streams, AAC ADTS, AAC LATM, MP3, AC-3, E-AC-3, AC-4, AMR, AMR-WB, QCP voice audio, DTS
//! core audio, Dolby TrueHD, leading-sync MHAS MPEG-H, IAMF, H.263 elementary video, MPEG-2
//! elementary video, MPEG-4 Part 2 elementary video, H.264 Annex B, H.265 Annex B, VVC Annex B,
//! raw AV1 OBU, raw AV1 Annex B, IVF-backed AV1, IVF-backed VP8, IVF-backed VP9, IVF-backed VP10, JPEG still
//! images, WAVE/AIFF/AIFC PCM, native FLAC, Ogg-backed FLAC, Ogg-backed Opus, Ogg-backed Vorbis,
//! Ogg-backed Speex, Ogg-backed Theora, and CAF-backed ALAC. Broader DTS-family sample-entry variants remain
//! supported through MP4 track import, and broader truthful demux-backed input paths continue to
//! land behind that same public shape.

#[cfg(test)]
extern crate self as mp4forge;

/// Tokio-based async I/O traits for the additive library-side async surface.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub mod async_io;
/// Bit-level reader and writer helpers used by the codec layer.
pub mod bitio;
/// Box definitions and registry helpers.
pub mod boxes;
/// Command-line routing and reusable command formatters.
pub mod cli;
/// Descriptor-driven binary codec primitives.
pub mod codec;
/// Feature-gated synchronous decryption types and helpers.
#[cfg(feature = "decrypt")]
#[cfg_attr(docsrs, doc(cfg(feature = "decrypt")))]
pub mod decrypt;
/// Resolved common-encryption metadata helpers built on typed box models.
pub mod encryption;
/// Path-based box extraction helpers, including typed convenience reads.
pub mod extract;
/// Four-character box identifier support.
pub mod fourcc;
/// MP4 box header parsing and encoding helpers.
pub mod header;
/// Feature-gated mux planning, real container assembly, and staged payload-copy helpers.
#[cfg(feature = "mux")]
#[cfg_attr(docsrs, doc(cfg(feature = "mux")))]
pub mod mux;
/// File-summary helpers built on the extraction and box layers.
pub mod probe;
#[cfg(any(feature = "decrypt", feature = "mux"))]
pub(crate) mod queue;
/// Path-based typed payload rewrite helpers built on the writer layer.
pub mod rewrite;
/// Fragmented top-level `sidx` analysis, planning, and rewrite helpers.
pub mod sidx;
/// Stable field-order string rendering for descriptor-backed boxes.
pub mod stringify;
/// Depth-first structure walking with path tracking and lazy payload access.
pub mod walk;
/// Box-writing helpers with header backfill support.
pub mod writer;

/// Four-character box identifier type.
pub use fourcc::FourCc;
/// Common header-related exports used by downstream callers.
pub use header::{BoxInfo, HeaderError, HeaderForm, LARGE_HEADER_SIZE, SMALL_HEADER_SIZE};
