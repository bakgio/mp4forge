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
//! sync and async real MP4 assembly, widened repeated track-spec parsing aligned with the
//! sync-only CLI, internal chunk and duration coordination on top of one mux event graph,
//! retained low-level staged payload-copy helpers, and the public `mp4forge::mux::sample_reader`
//! module built on staged mux plans. Those sample-reader helpers can also expose stable text or
//! subtitle track identity when you construct them with companion `MuxTrackConfig` values. Raw
//! codec-prefixed imports now cover the current widened codec set: self-describing families parse
//! their native framing directly, while broader raw families accept explicit layout parameters
//! when their source bytes are not self-describing enough to derive one safe MP4 sample-entry
//! shape automatically. MP4-track merges keep covering the broader registered sample-entry
//! families by preserving encoded sample-entry bytes from the source file.

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
