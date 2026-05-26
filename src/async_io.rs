//! Tokio-based async I/O traits for the library-side async surface.
//!
//! The existing sync APIs remain the default path in `mp4forge`. The first async rollout is
//! intentionally limited to seekable library readers and writers such as Tokio file handles or
//! in-memory buffers. Later queue-backed follow-ons can also use the forward-only async reader
//! and writer aliases in this module when a surface can operate progressively without seeks. The
//! CLI continues to use the sync surface.

/// Tokio async read trait used by the library-side async surface.
pub use tokio::io::AsyncRead;
/// Tokio async seek trait used by the library-side async surface.
pub use tokio::io::AsyncSeek;
/// Tokio async write trait used by the library-side async surface.
pub use tokio::io::AsyncWrite;

/// Async reader alias for forward-only library inputs.
///
/// Queue-backed progressive flows can use this bound when they only need incremental reads and do
/// not require random-access seeks. The alias still requires `Send` so callers can move
/// independent I/O jobs onto Tokio worker threads safely.
pub trait AsyncReadForward: AsyncRead + Unpin + Send {}

impl<T> AsyncReadForward for T where T: AsyncRead + Unpin + Send {}

/// Async writer alias for forward-only library outputs.
///
/// This alias covers additive async write surfaces that can emit bytes progressively without
/// later header backfill seeks, while still requiring `Send` for multithreaded Tokio tasks.
pub trait AsyncWriteForward: AsyncWrite + Unpin + Send {}

impl<T> AsyncWriteForward for T where T: AsyncWrite + Unpin + Send {}

/// Async reader alias for seekable library inputs.
///
/// The first async rollout targets inputs that support both asynchronous reads and random-access
/// seeks. Non-seekable streams are intentionally excluded from this initial surface, and the
/// additive async reader path requires `Send` so callers can move independent file work onto Tokio
/// worker threads.
pub trait AsyncReadSeek: AsyncRead + AsyncSeek + Unpin + Send {}

impl<T> AsyncReadSeek for T where T: AsyncRead + AsyncSeek + Unpin + Send {}

/// Async writer alias for seekable library outputs.
///
/// `mp4forge` write flows backfill box headers after payload bytes are written, so the async write
/// surface also requires seek support instead of treating outputs as one-way streams. The async
/// writer path also requires `Send` so independent write jobs can move across Tokio worker
/// threads.
pub trait AsyncWriteSeek: AsyncWrite + AsyncSeek + Unpin + Send {}

impl<T> AsyncWriteSeek for T where T: AsyncWrite + AsyncSeek + Unpin + Send {}
