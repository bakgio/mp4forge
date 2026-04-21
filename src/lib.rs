//! Low-level MP4 and ISOBMFF toolkit.

/// Bit-level reader and writer helpers used by the codec layer.
pub mod bitio;
/// Box definitions and registry helpers.
pub mod boxes;
/// Command-line routing and reusable command formatters.
pub mod cli;
/// Descriptor-driven binary codec primitives.
pub mod codec;
/// Path-based box extraction helpers built on the structure walker.
pub mod extract;
/// Four-character box identifier support.
pub mod fourcc;
/// MP4 box header parsing and encoding helpers.
pub mod header;
/// File-summary helpers built on the extraction and box layers.
pub mod probe;
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
