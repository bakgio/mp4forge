use std::collections::BTreeMap;

use super::{MuxError, MuxTrackPlan};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MuxDurationBoundaryKind {
    Segment,
    Fragment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrackCoordinationDirective {
    track_id: u32,
    chunk_sample_counts: Vec<u32>,
    duration_boundary_kind: Option<MuxDurationBoundaryKind>,
}

impl TrackCoordinationDirective {
    pub(crate) fn new(track_id: u32, chunk_sample_counts: Vec<u32>) -> Self {
        Self {
            track_id,
            chunk_sample_counts,
            duration_boundary_kind: None,
        }
    }

    pub(crate) fn with_duration_boundaries(
        mut self,
        duration_boundary_kind: MuxDurationBoundaryKind,
    ) -> Self {
        self.duration_boundary_kind = Some(duration_boundary_kind);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MuxCoordinationPlan {
    track_plans: BTreeMap<u32, TrackCoordinationPlan>,
}

impl MuxCoordinationPlan {
    pub(crate) fn from_track_plans(
        track_plans: &[MuxTrackPlan],
        directives: Vec<TrackCoordinationDirective>,
    ) -> Result<Self, MuxError> {
        let mut plans = BTreeMap::new();
        for track_plan in track_plans {
            plans.insert(
                track_plan.track_id(),
                TrackCoordinationPlan::default_for_item_count(track_plan.item_count())?,
            );
        }

        for directive in directives {
            let Some(track_plan) = track_plans
                .iter()
                .find(|track_plan| track_plan.track_id() == directive.track_id)
            else {
                return Err(MuxError::MissingTrackId {
                    track_id: directive.track_id,
                });
            };

            let plan = plans.get_mut(&directive.track_id).unwrap();
            *plan = TrackCoordinationPlan::from_directive(&directive, track_plan.item_count())?;
        }

        Ok(Self { track_plans: plans })
    }

    pub(crate) fn chunk_sample_counts(&self, track_id: u32) -> Result<&[u32], MuxError> {
        self.track_plans
            .get(&track_id)
            .map(TrackCoordinationPlan::chunk_sample_counts)
            .ok_or(MuxError::MissingTrackId { track_id })
    }

    pub(crate) fn duration_boundary_after_sample(
        &self,
        track_id: u32,
        sample_index_in_stream: u32,
    ) -> Option<MuxDurationBoundaryKind> {
        self.track_plans
            .get(&track_id)
            .and_then(|plan| plan.duration_boundary_after_sample(sample_index_in_stream))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TrackCoordinationPlan {
    chunk_sample_counts: Vec<u32>,
    duration_boundaries: Vec<TrackDurationBoundary>,
}

impl TrackCoordinationPlan {
    fn default_for_item_count(item_count: u32) -> Result<Self, MuxError> {
        let chunk_sample_counts = vec![
            1;
            usize::try_from(item_count).map_err(|_| {
                MuxError::LayoutOverflow("track chunk item-count conversion")
            })?
        ];
        Ok(Self {
            chunk_sample_counts,
            duration_boundaries: Vec::new(),
        })
    }

    fn from_directive(
        directive: &TrackCoordinationDirective,
        item_count: u32,
    ) -> Result<Self, MuxError> {
        validate_chunk_sample_counts(
            directive.track_id,
            &directive.chunk_sample_counts,
            item_count,
        )?;

        let duration_boundaries = if let Some(kind) = directive.duration_boundary_kind {
            let mut cumulative_sample_count = 0_u32;
            let mut boundaries = Vec::with_capacity(directive.chunk_sample_counts.len());
            for samples_per_chunk in &directive.chunk_sample_counts {
                cumulative_sample_count =
                    cumulative_sample_count
                        .checked_add(*samples_per_chunk)
                        .ok_or(MuxError::LayoutOverflow("duration-boundary sample count"))?;
                boundaries.push(TrackDurationBoundary {
                    sample_count: cumulative_sample_count,
                    kind,
                });
            }
            boundaries
        } else {
            Vec::new()
        };

        Ok(Self {
            chunk_sample_counts: directive.chunk_sample_counts.clone(),
            duration_boundaries,
        })
    }

    fn chunk_sample_counts(&self) -> &[u32] {
        &self.chunk_sample_counts
    }

    fn duration_boundary_after_sample(
        &self,
        sample_index_in_stream: u32,
    ) -> Option<MuxDurationBoundaryKind> {
        let sample_count = sample_index_in_stream.checked_add(1)?;
        self.duration_boundaries
            .iter()
            .find(|boundary| boundary.sample_count == sample_count)
            .map(|boundary| boundary.kind)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrackDurationBoundary {
    sample_count: u32,
    kind: MuxDurationBoundaryKind,
}

pub(crate) fn build_duration_chunk_sample_counts<I>(
    track_id: u32,
    sample_durations: I,
    target_ticks: u64,
) -> Result<Vec<u32>, MuxError>
where
    I: IntoIterator<Item = u32>,
{
    build_duration_chunk_sample_counts_with_start_time(track_id, sample_durations, target_ticks, 0)
}

pub(crate) fn build_capped_duration_chunk_sample_counts<I>(
    track_id: u32,
    sample_durations: I,
    target_ticks: u64,
) -> Result<Vec<u32>, MuxError>
where
    I: IntoIterator<Item = u32>,
{
    let mut counts = Vec::new();
    let mut current_count = 0_u32;
    let mut current_duration = 0_u64;
    for duration in sample_durations {
        let duration = u64::from(duration);
        if current_count != 0
            && current_duration
                .checked_add(duration)
                .ok_or(MuxError::LayoutOverflow("chunk duration"))?
                > target_ticks
        {
            counts.push(current_count);
            current_count = 0;
            current_duration = 0;
        }
        current_count = current_count
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("chunk sample count"))?;
        current_duration = current_duration
            .checked_add(duration)
            .ok_or(MuxError::LayoutOverflow("chunk duration"))?;
    }
    if current_count != 0 {
        counts.push(current_count);
    }
    if counts.is_empty() {
        return Err(MuxError::InvalidChunkPlan {
            track_id,
            message: "no chunk boundaries were produced".to_string(),
        });
    }
    Ok(counts)
}

pub(crate) fn rebalance_small_multi_audio_chunk_sample_counts(chunk_sample_counts: &mut [u32]) {
    if chunk_sample_counts.len() != 3 {
        return;
    }

    let last_index = chunk_sample_counts.len() - 1;
    let previous_index = last_index - 1;
    if chunk_sample_counts[0] != chunk_sample_counts[previous_index]
        || chunk_sample_counts[previous_index] > 4
    {
        return;
    }

    while chunk_sample_counts[last_index] + 1 < chunk_sample_counts[previous_index] {
        chunk_sample_counts[previous_index] -= 1;
        chunk_sample_counts[last_index] += 1;
    }
}

pub(crate) fn build_duration_chunk_sample_counts_with_start_time<I>(
    track_id: u32,
    sample_durations: I,
    target_ticks: u64,
    start_time_ticks: i64,
) -> Result<Vec<u32>, MuxError>
where
    I: IntoIterator<Item = u32>,
{
    let mut counts = Vec::new();
    let mut current_count = 0_u32;
    let mut cumulative_end_time = i128::from(start_time_ticks);
    let mut next_boundary = i128::from(target_ticks);
    for duration in sample_durations {
        current_count = current_count
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("chunk sample count"))?;
        cumulative_end_time = cumulative_end_time
            .checked_add(i128::from(duration))
            .ok_or(MuxError::LayoutOverflow("chunk duration"))?;
        if cumulative_end_time >= next_boundary {
            counts.push(current_count);
            current_count = 0;
            while cumulative_end_time >= next_boundary {
                next_boundary = next_boundary
                    .checked_add(i128::from(target_ticks))
                    .ok_or(MuxError::LayoutOverflow("chunk duration boundary"))?;
            }
        }
    }
    if current_count != 0 {
        counts.push(current_count);
    }
    if counts.is_empty() {
        return Err(MuxError::InvalidChunkPlan {
            track_id,
            message: "no chunk boundaries were produced".to_string(),
        });
    }
    Ok(counts)
}

pub(crate) fn build_sync_aligned_segment_chunk_sample_counts<I>(
    track_id: u32,
    samples: I,
    target_ticks: u64,
    start_time_ticks: i64,
) -> Result<Vec<u32>, MuxError>
where
    I: IntoIterator<Item = (u32, i64, bool)>,
{
    let mut counts = Vec::new();
    let mut current_count = 0_u32;
    let mut decode_start_time = 0_i128;
    let mut next_boundary = i128::from(target_ticks);
    let start_time_ticks = i128::from(start_time_ticks);

    for (duration_ticks, composition_offset_ticks, is_sync_sample) in samples {
        // Segment boundaries should start on sync samples after the adjusted presentation
        // timeline crosses the requested target.
        if current_count != 0 && is_sync_sample {
            let presentation_start_time = decode_start_time
                .checked_add(i128::from(composition_offset_ticks))
                .and_then(|value| value.checked_add(start_time_ticks))
                .ok_or(MuxError::LayoutOverflow("segment presentation start"))?;
            if presentation_start_time >= next_boundary {
                counts.push(current_count);
                current_count = 0;
                while presentation_start_time >= next_boundary {
                    next_boundary = next_boundary
                        .checked_add(i128::from(target_ticks))
                        .ok_or(MuxError::LayoutOverflow("segment duration boundary"))?;
                }
            }
        }

        current_count = current_count
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("chunk sample count"))?;
        decode_start_time = decode_start_time
            .checked_add(i128::from(duration_ticks))
            .ok_or(MuxError::LayoutOverflow("chunk duration"))?;
    }

    if current_count != 0 {
        counts.push(current_count);
    }
    if counts.is_empty() {
        return Err(MuxError::InvalidChunkPlan {
            track_id,
            message: "no chunk boundaries were produced".to_string(),
        });
    }
    Ok(counts)
}

fn validate_chunk_sample_counts(
    track_id: u32,
    chunk_sample_counts: &[u32],
    item_count: u32,
) -> Result<(), MuxError> {
    if chunk_sample_counts.is_empty() {
        return Err(MuxError::InvalidChunkPlan {
            track_id,
            message: "chunk plans may not be empty".to_string(),
        });
    }

    let mut total_samples = 0_u32;
    for samples_per_chunk in chunk_sample_counts {
        if *samples_per_chunk == 0 {
            return Err(MuxError::InvalidChunkPlan {
                track_id,
                message: "chunk plans may not contain zero-length chunks".to_string(),
            });
        }
        total_samples = total_samples
            .checked_add(*samples_per_chunk)
            .ok_or(MuxError::LayoutOverflow("chunk sample-count total"))?;
    }

    if total_samples != item_count {
        return Err(MuxError::InvalidChunkPlan {
            track_id,
            message: format!(
                "chunk plan resolves {total_samples} samples for {item_count} staged samples"
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_chunk_counts_roll_over_at_target_ticks() {
        let counts = build_duration_chunk_sample_counts(7, [10_u32, 10, 10], 15).unwrap();

        assert_eq!(counts, vec![2, 1]);
    }

    #[test]
    fn capped_duration_chunk_counts_split_before_overshoot() {
        let counts = build_capped_duration_chunk_sample_counts(
            7,
            std::iter::repeat_n(1_024_u32, 45),
            22_050,
        )
        .unwrap();

        assert_eq!(counts, vec![21, 21, 3]);
    }

    #[test]
    fn rebalance_small_multi_audio_chunk_counts_only_adjusts_retained_three_chunk_shape() {
        let mut counts = vec![4, 4, 2];

        rebalance_small_multi_audio_chunk_sample_counts(&mut counts);

        assert_eq!(counts, vec![4, 3, 3]);
    }

    #[test]
    fn duration_chunk_counts_honor_negative_start_time_offsets() {
        let counts = build_duration_chunk_sample_counts_with_start_time(
            7,
            std::iter::repeat_n(1_024_u32, 120),
            44_100,
            -1_024,
        )
        .unwrap();

        assert_eq!(counts, vec![45, 43, 32]);
    }

    #[test]
    fn sync_aligned_segment_chunk_counts_use_presentation_starts() {
        let counts = build_sync_aligned_segment_chunk_sample_counts(
            7,
            (0..82).map(|index| {
                let composition_offset = if matches!(index, 0 | 30 | 60) {
                    2_002_i64
                } else if index % 2 == 1 {
                    3_003_i64
                } else {
                    1_001_i64
                };
                (1_001_u32, composition_offset, matches!(index, 0 | 30 | 60))
            }),
            30_000,
            -2_002,
        )
        .unwrap();

        assert_eq!(counts, vec![30, 30, 22]);
    }

    #[test]
    fn coordination_plan_applies_duration_boundaries_to_chunk_ends() {
        let track_plans = [MuxTrackPlan {
            track_id: 7,
            item_count: 3,
            first_decode_time: 0,
            end_decode_time: 30,
        }];
        let plan = MuxCoordinationPlan::from_track_plans(
            &track_plans,
            vec![
                TrackCoordinationDirective::new(7, vec![2, 1])
                    .with_duration_boundaries(MuxDurationBoundaryKind::Fragment),
            ],
        )
        .unwrap();

        assert_eq!(plan.chunk_sample_counts(7).unwrap(), &[2, 1]);
        assert_eq!(plan.duration_boundary_after_sample(7, 0), None);
        assert_eq!(
            plan.duration_boundary_after_sample(7, 1),
            Some(MuxDurationBoundaryKind::Fragment)
        );
        assert_eq!(
            plan.duration_boundary_after_sample(7, 2),
            Some(MuxDurationBoundaryKind::Fragment)
        );
    }
}
