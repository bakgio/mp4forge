//! Feature-gated mux sample-reader helpers built on mux plans.
//!
//! This additive surface exposes one-sample-at-a-time readers for callers that want to consume
//! staged sample payloads directly without depending on the crate-private queue layer. The public
//! API stays aligned with the mux plan semantics: callers enable the crate's `mux` feature, bring
//! one [`crate::mux::MuxPlan`], then choose either seekable or progressive readers from
//! [`crate::mux::sample_reader`] depending on the source handles they have. Internally, these
//! readers now walk the mux event graph instead of depending on the older queue-parser stage loop
//! directly.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};

#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::{MuxEventCursor, MuxPlan, MuxSampleEvent, MuxTrackConfig, MuxTrackKind};
#[cfg(feature = "async")]
use crate::async_io::{AsyncReadForward, AsyncReadSeek};

/// Stable metadata for one sample emitted by the planned sample readers.
///
/// This mirrors the current mux boundary surface intentionally: callers get one sample at a time
/// with both its decode interval and its output payload span, without needing a separate event
/// graph above the staged mux plan.
///
/// When readers are constructed with companion [`MuxTrackConfig`] values, the metadata also
/// carries stable track identity for the landed text and subtitle paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SampleTrackMetadata {
    kind: MuxTrackKind,
    language: [u8; 3],
}

impl SampleTrackMetadata {
    /// Returns the mux track kind that produced this sample.
    pub const fn kind(&self) -> MuxTrackKind {
        self.kind
    }

    /// Returns the three-letter ISO-639-2 language code carried by this sample's track.
    pub const fn language(&self) -> [u8; 3] {
        self.language
    }
}

/// Stable metadata for one sample emitted by the planned sample readers.
///
/// Every reader exposes the staged source and timing fields that come from the mux plan itself.
/// When the reader is constructed with companion [`MuxTrackConfig`] values, the metadata also
/// carries stable per-track identity for mixed audio, text, and subtitle jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SampleMetadata {
    source_index: usize,
    track_id: u32,
    track: Option<SampleTrackMetadata>,
    decode_time: u64,
    composition_time_offset: i32,
    duration: u32,
    data_offset: u64,
    data_size: u32,
    output_offset: u64,
    is_sync_sample: bool,
}

impl SampleMetadata {
    /// Returns the staged source index that supplies this sample's bytes.
    pub const fn source_index(&self) -> usize {
        self.source_index
    }

    /// Returns the destination track identifier carried by this sample.
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// Returns stable per-track metadata when the reader was constructed with track configs.
    pub const fn track(&self) -> Option<SampleTrackMetadata> {
        self.track
    }

    /// Returns the normalized decode time used by the plan.
    pub const fn decode_time(&self) -> u64 {
        self.decode_time
    }

    /// Returns the composition-time offset carried by this sample.
    pub const fn composition_time_offset(&self) -> i32 {
        self.composition_time_offset
    }

    /// Returns the decode duration carried by this sample.
    pub const fn duration(&self) -> u32 {
        self.duration
    }

    /// Returns the staged source byte offset for this sample payload.
    pub const fn data_offset(&self) -> u64 {
        self.data_offset
    }

    /// Returns the number of payload bytes described by the plan for this sample.
    pub const fn data_size(&self) -> u32 {
        self.data_size
    }

    /// Returns the output payload offset assigned by the plan.
    pub const fn output_offset(&self) -> u64 {
        self.output_offset
    }

    /// Returns the first byte offset after this sample's payload in the planned output order.
    pub const fn output_end_offset(&self) -> u64 {
        self.output_offset + self.data_size as u64
    }

    /// Returns the decode end time of this sample on the planned mux timeline.
    pub const fn decode_end_time(&self) -> u64 {
        self.decode_time + self.duration as u64
    }

    /// Returns whether this sample is marked as a sync sample.
    pub const fn is_sync_sample(&self) -> bool {
        self.is_sync_sample
    }
}

/// One owned sample payload emitted by a planned sample reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SamplePacket {
    metadata: SampleMetadata,
    bytes: Vec<u8>,
}

impl SamplePacket {
    /// Returns the stable metadata associated with this sample payload.
    pub const fn metadata(&self) -> &SampleMetadata {
        &self.metadata
    }

    /// Returns the owned sample bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Splits this owned sample into metadata and bytes.
    pub fn into_parts(self) -> (SampleMetadata, Vec<u8>) {
        (self.metadata, self.bytes)
    }
}

/// Errors returned by the planned sample-reader helpers.
#[derive(Debug)]
pub enum SampleReaderError {
    /// The planned sample size does not fit in memory on the current platform.
    SampleSizeOverflow { size: u64 },
    /// One planned sample referenced a staged source index the caller did not provide.
    MissingSourceIndex {
        source_index: usize,
        source_count: usize,
    },
    /// A progressive source would need to seek backward to satisfy the plan.
    NonMonotonicSourceOffset {
        source_index: usize,
        previous_offset: u64,
        next_offset: u64,
    },
    /// A progressive source ended before it reached the staged offset needed by the next sample.
    IncompleteAdvance {
        source_index: usize,
        expected_offset: u64,
        actual_offset: u64,
    },
    /// A source ended before it produced the full sample payload.
    IncompleteSample {
        source_index: usize,
        expected_size: u64,
        actual_size: u64,
    },
    /// An I/O error occurred while reading sample data.
    Io(io::Error),
}

impl fmt::Display for SampleReaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SampleSizeOverflow { size } => write!(
                f,
                "planned sample size {size} does not fit in memory on this platform"
            ),
            Self::MissingSourceIndex {
                source_index,
                source_count,
            } => write!(
                f,
                "sample plan referenced source index {source_index}, but only {source_count} sources were provided"
            ),
            Self::NonMonotonicSourceOffset {
                source_index,
                previous_offset,
                next_offset,
            } => write!(
                f,
                "source index {source_index} would need to move backward from offset {previous_offset} to {next_offset}"
            ),
            Self::IncompleteAdvance {
                source_index,
                expected_offset,
                actual_offset,
            } => write!(
                f,
                "source index {source_index} ended while advancing to offset {expected_offset}; only reached {actual_offset}"
            ),
            Self::IncompleteSample {
                source_index,
                expected_size,
                actual_size,
            } => write!(
                f,
                "source index {source_index} produced {actual_size} bytes for one sample, expected {expected_size}"
            ),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl Error for SampleReaderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for SampleReaderError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// One seekable planned-sample reader.
///
/// This reader follows the sample order assigned by [`crate::mux::plan_staged_media_items`] and
/// can freely seek inside each staged source as needed.
pub struct PlannedSampleReader<'a, R> {
    sources: &'a mut [R],
    cursor: MuxEventCursor<'a>,
    track_metadata: BTreeMap<u32, SampleTrackMetadata>,
}

impl<'a, R> PlannedSampleReader<'a, R>
where
    R: Read + Seek,
{
    /// Creates one seekable planned-sample reader over the staged `sources` and `plan`.
    pub fn new(sources: &'a mut [R], plan: &'a MuxPlan) -> Self {
        Self {
            sources,
            cursor: plan.event_graph().cursor(),
            track_metadata: BTreeMap::new(),
        }
    }

    /// Creates one seekable planned-sample reader with companion track identity metadata.
    pub fn new_with_track_configs(
        sources: &'a mut [R],
        plan: &'a MuxPlan,
        track_configs: &[MuxTrackConfig],
    ) -> Self {
        Self {
            sources,
            cursor: plan.event_graph().cursor(),
            track_metadata: build_track_metadata(track_configs),
        }
    }

    /// Reads the next sample in planned order.
    pub fn next_sample(&mut self) -> Result<Option<SamplePacket>, SampleReaderError> {
        let Some(event) = next_sample_event(&mut self.cursor) else {
            return Ok(None);
        };
        let staged = event.planned_item().staged();
        let Some(source) = self.sources.get_mut(staged.source_index()) else {
            return Err(SampleReaderError::MissingSourceIndex {
                source_index: staged.source_index(),
                source_count: self.sources.len(),
            });
        };

        source.seek(SeekFrom::Start(staged.data_offset()))?;
        let bytes =
            read_sample_bytes(source, staged.source_index(), u64::from(staged.data_size()))?;
        Ok(Some(SamplePacket {
            metadata: metadata_from_sample_event(event, &self.track_metadata),
            bytes,
        }))
    }
}

/// One progressive planned-sample reader for forward-only sync sources.
///
/// This reader supports only plans whose staged items consume each source in monotonic byte-offset
/// order.
pub struct ProgressiveSampleReader<'a, R> {
    sources: &'a mut [R],
    cursor: MuxEventCursor<'a>,
    track_metadata: BTreeMap<u32, SampleTrackMetadata>,
    source_offsets: Vec<u64>,
    advance_buffer: Vec<u8>,
}

impl<'a, R> ProgressiveSampleReader<'a, R>
where
    R: Read,
{
    /// Creates one progressive planned-sample reader over forward-only sync `sources`.
    pub fn new(sources: &'a mut [R], plan: &'a MuxPlan) -> Self {
        Self {
            source_offsets: vec![0_u64; sources.len()],
            sources,
            cursor: plan.event_graph().cursor(),
            track_metadata: BTreeMap::new(),
            advance_buffer: vec![0_u8; 16 * 1024],
        }
    }

    /// Creates one progressive planned-sample reader with companion track identity metadata.
    pub fn new_with_track_configs(
        sources: &'a mut [R],
        plan: &'a MuxPlan,
        track_configs: &[MuxTrackConfig],
    ) -> Self {
        Self {
            source_offsets: vec![0_u64; sources.len()],
            sources,
            cursor: plan.event_graph().cursor(),
            track_metadata: build_track_metadata(track_configs),
            advance_buffer: vec![0_u8; 16 * 1024],
        }
    }

    /// Reads the next sample in planned order.
    pub fn next_sample(&mut self) -> Result<Option<SamplePacket>, SampleReaderError> {
        let Some(event) = next_sample_event(&mut self.cursor) else {
            return Ok(None);
        };
        let staged = event.planned_item().staged();
        let Some(source) = self.sources.get_mut(staged.source_index()) else {
            return Err(SampleReaderError::MissingSourceIndex {
                source_index: staged.source_index(),
                source_count: self.sources.len(),
            });
        };

        let source_offset = self.source_offsets.get_mut(staged.source_index()).unwrap();
        advance_progressive_source(
            source,
            staged.source_index(),
            source_offset,
            staged.data_offset(),
            &mut self.advance_buffer,
        )?;
        let bytes = read_progressive_sample(
            source,
            staged.source_index(),
            source_offset,
            u64::from(staged.data_size()),
        )?;
        Ok(Some(SamplePacket {
            metadata: metadata_from_sample_event(event, &self.track_metadata),
            bytes,
        }))
    }
}

/// One seekable async planned-sample reader.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(all(feature = "mux", feature = "async"))))]
pub struct AsyncPlannedSampleReader<'a, R> {
    sources: &'a mut [R],
    cursor: MuxEventCursor<'a>,
    track_metadata: BTreeMap<u32, SampleTrackMetadata>,
}

#[cfg(feature = "async")]
impl<'a, R> AsyncPlannedSampleReader<'a, R>
where
    R: AsyncReadSeek,
{
    /// Creates one seekable async planned-sample reader over `sources` and `plan`.
    pub fn new(sources: &'a mut [R], plan: &'a MuxPlan) -> Self {
        Self {
            sources,
            cursor: plan.event_graph().cursor(),
            track_metadata: BTreeMap::new(),
        }
    }

    /// Creates one seekable async planned-sample reader with companion track identity metadata.
    pub fn new_with_track_configs(
        sources: &'a mut [R],
        plan: &'a MuxPlan,
        track_configs: &[MuxTrackConfig],
    ) -> Self {
        Self {
            sources,
            cursor: plan.event_graph().cursor(),
            track_metadata: build_track_metadata(track_configs),
        }
    }

    /// Reads the next sample in planned order.
    pub async fn next_sample(&mut self) -> Result<Option<SamplePacket>, SampleReaderError> {
        let Some(event) = next_sample_event(&mut self.cursor) else {
            return Ok(None);
        };
        let staged = event.planned_item().staged();
        let Some(source) = self.sources.get_mut(staged.source_index()) else {
            return Err(SampleReaderError::MissingSourceIndex {
                source_index: staged.source_index(),
                source_count: self.sources.len(),
            });
        };

        source.seek(SeekFrom::Start(staged.data_offset())).await?;
        let bytes =
            read_sample_bytes_async(source, staged.source_index(), u64::from(staged.data_size()))
                .await?;
        Ok(Some(SamplePacket {
            metadata: metadata_from_sample_event(event, &self.track_metadata),
            bytes,
        }))
    }
}

/// One progressive async planned-sample reader for forward-only sources.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(all(feature = "mux", feature = "async"))))]
pub struct AsyncProgressiveSampleReader<'a, R> {
    sources: &'a mut [R],
    cursor: MuxEventCursor<'a>,
    track_metadata: BTreeMap<u32, SampleTrackMetadata>,
    source_offsets: Vec<u64>,
    advance_buffer: Vec<u8>,
}

#[cfg(feature = "async")]
impl<'a, R> AsyncProgressiveSampleReader<'a, R>
where
    R: AsyncReadForward,
{
    /// Creates one progressive async planned-sample reader over forward-only sources.
    pub fn new(sources: &'a mut [R], plan: &'a MuxPlan) -> Self {
        Self {
            source_offsets: vec![0_u64; sources.len()],
            sources,
            cursor: plan.event_graph().cursor(),
            track_metadata: BTreeMap::new(),
            advance_buffer: vec![0_u8; 16 * 1024],
        }
    }

    /// Creates one progressive async planned-sample reader with companion track identity metadata.
    pub fn new_with_track_configs(
        sources: &'a mut [R],
        plan: &'a MuxPlan,
        track_configs: &[MuxTrackConfig],
    ) -> Self {
        Self {
            source_offsets: vec![0_u64; sources.len()],
            sources,
            cursor: plan.event_graph().cursor(),
            track_metadata: build_track_metadata(track_configs),
            advance_buffer: vec![0_u8; 16 * 1024],
        }
    }

    /// Reads the next sample in planned order.
    pub async fn next_sample(&mut self) -> Result<Option<SamplePacket>, SampleReaderError> {
        let Some(event) = next_sample_event(&mut self.cursor) else {
            return Ok(None);
        };
        let staged = event.planned_item().staged();
        let Some(source) = self.sources.get_mut(staged.source_index()) else {
            return Err(SampleReaderError::MissingSourceIndex {
                source_index: staged.source_index(),
                source_count: self.sources.len(),
            });
        };

        let source_offset = self.source_offsets.get_mut(staged.source_index()).unwrap();
        advance_progressive_source_async(
            source,
            staged.source_index(),
            source_offset,
            staged.data_offset(),
            &mut self.advance_buffer,
        )
        .await?;
        let bytes = read_progressive_sample_async(
            source,
            staged.source_index(),
            source_offset,
            u64::from(staged.data_size()),
        )
        .await?;
        Ok(Some(SamplePacket {
            metadata: metadata_from_sample_event(event, &self.track_metadata),
            bytes,
        }))
    }
}

fn next_sample_event<'a>(cursor: &mut MuxEventCursor<'a>) -> Option<&'a MuxSampleEvent> {
    cursor.next_sample()
}

fn build_track_metadata(track_configs: &[MuxTrackConfig]) -> BTreeMap<u32, SampleTrackMetadata> {
    track_configs
        .iter()
        .map(|track| {
            (
                track.track_id(),
                SampleTrackMetadata {
                    kind: track.kind(),
                    language: track.language(),
                },
            )
        })
        .collect()
}

fn metadata_from_sample_event(
    event: &MuxSampleEvent,
    track_metadata: &BTreeMap<u32, SampleTrackMetadata>,
) -> SampleMetadata {
    let staged = event.planned_item().staged();
    SampleMetadata {
        source_index: staged.source_index(),
        track_id: staged.track_id(),
        track: track_metadata.get(&staged.track_id()).copied(),
        decode_time: staged.decode_time(),
        composition_time_offset: staged.composition_time_offset(),
        duration: staged.duration(),
        data_offset: staged.data_offset(),
        data_size: staged.data_size(),
        output_offset: event.planned_item().output_offset(),
        is_sync_sample: staged.is_sync_sample(),
    }
}

fn read_sample_bytes<R>(
    source: &mut R,
    source_index: usize,
    size: u64,
) -> Result<Vec<u8>, SampleReaderError>
where
    R: Read,
{
    let len = usize::try_from(size).map_err(|_| SampleReaderError::SampleSizeOverflow { size })?;
    let mut bytes = vec![0_u8; len];
    let mut copied = 0_usize;
    while copied < len {
        let read = source.read(&mut bytes[copied..])?;
        if read == 0 {
            return Err(SampleReaderError::IncompleteSample {
                source_index,
                expected_size: size,
                actual_size: copied as u64,
            });
        }
        copied += read;
    }
    Ok(bytes)
}

fn advance_progressive_source<R>(
    source: &mut R,
    source_index: usize,
    current_offset: &mut u64,
    target_offset: u64,
    buffer: &mut [u8],
) -> Result<(), SampleReaderError>
where
    R: Read,
{
    if target_offset < *current_offset {
        return Err(SampleReaderError::NonMonotonicSourceOffset {
            source_index,
            previous_offset: *current_offset,
            next_offset: target_offset,
        });
    }

    let mut remaining = target_offset - *current_offset;
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len() as u64) as usize;
        let read = source.read(&mut buffer[..chunk_len])?;
        if read == 0 {
            return Err(SampleReaderError::IncompleteAdvance {
                source_index,
                expected_offset: target_offset,
                actual_offset: *current_offset,
            });
        }
        *current_offset += read as u64;
        remaining -= read as u64;
    }
    Ok(())
}

fn read_progressive_sample<R>(
    source: &mut R,
    source_index: usize,
    current_offset: &mut u64,
    size: u64,
) -> Result<Vec<u8>, SampleReaderError>
where
    R: Read,
{
    let len = usize::try_from(size).map_err(|_| SampleReaderError::SampleSizeOverflow { size })?;
    let mut bytes = vec![0_u8; len];
    let mut copied = 0_usize;
    while copied < len {
        let read = source.read(&mut bytes[copied..])?;
        if read == 0 {
            return Err(SampleReaderError::IncompleteSample {
                source_index,
                expected_size: size,
                actual_size: copied as u64,
            });
        }
        copied += read;
    }
    *current_offset = current_offset
        .checked_add(size)
        .ok_or(SampleReaderError::SampleSizeOverflow { size })?;
    Ok(bytes)
}

#[cfg(feature = "async")]
async fn read_sample_bytes_async<R>(
    source: &mut R,
    source_index: usize,
    size: u64,
) -> Result<Vec<u8>, SampleReaderError>
where
    R: AsyncReadForward,
{
    let len = usize::try_from(size).map_err(|_| SampleReaderError::SampleSizeOverflow { size })?;
    let mut bytes = vec![0_u8; len];
    let mut copied = 0_usize;
    while copied < len {
        let read = source.read(&mut bytes[copied..]).await?;
        if read == 0 {
            return Err(SampleReaderError::IncompleteSample {
                source_index,
                expected_size: size,
                actual_size: copied as u64,
            });
        }
        copied += read;
    }
    Ok(bytes)
}

#[cfg(feature = "async")]
async fn advance_progressive_source_async<R>(
    source: &mut R,
    source_index: usize,
    current_offset: &mut u64,
    target_offset: u64,
    buffer: &mut [u8],
) -> Result<(), SampleReaderError>
where
    R: AsyncReadForward,
{
    if target_offset < *current_offset {
        return Err(SampleReaderError::NonMonotonicSourceOffset {
            source_index,
            previous_offset: *current_offset,
            next_offset: target_offset,
        });
    }

    let mut remaining = target_offset - *current_offset;
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len() as u64) as usize;
        let read = source.read(&mut buffer[..chunk_len]).await?;
        if read == 0 {
            return Err(SampleReaderError::IncompleteAdvance {
                source_index,
                expected_offset: target_offset,
                actual_offset: *current_offset,
            });
        }
        *current_offset += read as u64;
        remaining -= read as u64;
    }
    Ok(())
}

#[cfg(feature = "async")]
async fn read_progressive_sample_async<R>(
    source: &mut R,
    source_index: usize,
    current_offset: &mut u64,
    size: u64,
) -> Result<Vec<u8>, SampleReaderError>
where
    R: AsyncReadForward,
{
    let len = usize::try_from(size).map_err(|_| SampleReaderError::SampleSizeOverflow { size })?;
    let mut bytes = vec![0_u8; len];
    let mut copied = 0_usize;
    while copied < len {
        let read = source.read(&mut bytes[copied..]).await?;
        if read == 0 {
            return Err(SampleReaderError::IncompleteSample {
                source_index,
                expected_size: size,
                actual_size: copied as u64,
            });
        }
        copied += read;
    }
    *current_offset = current_offset
        .checked_add(size)
        .ok_or(SampleReaderError::SampleSizeOverflow { size })?;
    Ok(bytes)
}
