//! Tokio-based async I/O traits for the library-side async surface.
//!
//! The existing sync APIs remain the default path in `mp4forge`. The first async rollout is
//! intentionally limited to seekable library readers and writers such as Tokio file handles or
//! in-memory buffers. The CLI continues to use the sync surface.

/// Tokio async read trait used by the library-side async surface.
pub use tokio::io::AsyncRead;
/// Tokio async seek trait used by the library-side async surface.
pub use tokio::io::AsyncSeek;
/// Tokio async write trait used by the library-side async surface.
pub use tokio::io::AsyncWrite;

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
