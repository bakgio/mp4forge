use std::collections::HashMap;

use super::coordination::{MuxCoordinationPlan, MuxDurationBoundaryKind};
use super::{MuxPlannedMediaItem, MuxTrackPlan};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MuxStreamDescription {
    stream_index: usize,
    track_id: u32,
    item_count: u32,
    first_decode_time: u64,
    end_decode_time: u64,
    first_output_offset: u64,
    end_output_offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MuxSampleEvent {
    stream_index: usize,
    sample_index_in_stream: u32,
    planned_item: MuxPlannedMediaItem,
}

impl MuxSampleEvent {
    pub(crate) const fn planned_item(&self) -> &MuxPlannedMediaItem {
        &self.planned_item
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MuxBoundaryEventKind {
    SegmentBoundary,
    FragmentBoundary,
    TrackDrain,
    PlanEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MuxBoundaryEvent {
    kind: MuxBoundaryEventKind,
    stream_index: Option<usize>,
    track_id: Option<u32>,
    output_offset: u64,
    decode_time: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MuxEvent {
    StreamDescription(MuxStreamDescription),
    Sample(MuxSampleEvent),
    Boundary(MuxBoundaryEvent),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MuxEventGraph {
    events: Vec<MuxEvent>,
}

impl MuxEventGraph {
    pub(crate) fn from_plan(
        planned_items: &[MuxPlannedMediaItem],
        track_plans: &[MuxTrackPlan],
        total_payload_size: u64,
        coordination: &MuxCoordinationPlan,
    ) -> Self {
        let mut first_output_offset_by_track = HashMap::<u32, u64>::new();
        let mut end_output_offset_by_track = HashMap::<u32, u64>::new();
        let mut last_sample_index_by_track = HashMap::<u32, usize>::new();
        let mut max_decode_end_time = 0_u64;

        for (planned_index, item) in planned_items.iter().enumerate() {
            let track_id = item.staged().track_id();
            first_output_offset_by_track
                .entry(track_id)
                .or_insert_with(|| item.output_offset());
            end_output_offset_by_track.insert(track_id, item.output_end_offset());
            last_sample_index_by_track.insert(track_id, planned_index);
            max_decode_end_time = max_decode_end_time.max(item.decode_end_time());
        }

        let mut stream_index_by_track = HashMap::<u32, usize>::new();
        let mut events = Vec::new();
        for (stream_index, track_plan) in track_plans.iter().enumerate() {
            stream_index_by_track.insert(track_plan.track_id(), stream_index);
            events.push(MuxEvent::StreamDescription(MuxStreamDescription {
                stream_index,
                track_id: track_plan.track_id(),
                item_count: track_plan.item_count(),
                first_decode_time: track_plan.first_decode_time(),
                end_decode_time: track_plan.end_decode_time(),
                first_output_offset: first_output_offset_by_track
                    .get(&track_plan.track_id())
                    .copied()
                    .unwrap_or(total_payload_size),
                end_output_offset: end_output_offset_by_track
                    .get(&track_plan.track_id())
                    .copied()
                    .unwrap_or(total_payload_size),
            }));
        }

        let mut sample_index_in_stream = HashMap::<u32, u32>::new();
        for (planned_index, item) in planned_items.iter().enumerate() {
            let track_id = item.staged().track_id();
            let stream_index = stream_index_by_track[&track_id];
            let sample_index = sample_index_in_stream.entry(track_id).or_insert(0);
            let event = MuxSampleEvent {
                stream_index,
                sample_index_in_stream: *sample_index,
                planned_item: *item,
            };
            let current_sample_index = *sample_index;
            *sample_index += 1;
            events.push(MuxEvent::Sample(event));

            if let Some(kind) =
                coordination.duration_boundary_after_sample(track_id, current_sample_index)
            {
                events.push(MuxEvent::Boundary(MuxBoundaryEvent {
                    kind: boundary_kind_from_duration(kind),
                    stream_index: Some(stream_index),
                    track_id: Some(track_id),
                    output_offset: item.output_end_offset(),
                    decode_time: item.decode_end_time(),
                }));
            }

            if last_sample_index_by_track.get(&track_id) == Some(&planned_index) {
                events.push(MuxEvent::Boundary(MuxBoundaryEvent {
                    kind: MuxBoundaryEventKind::TrackDrain,
                    stream_index: Some(stream_index),
                    track_id: Some(track_id),
                    output_offset: item.output_end_offset(),
                    decode_time: item.decode_end_time(),
                }));
            }
        }

        events.push(MuxEvent::Boundary(MuxBoundaryEvent {
            kind: MuxBoundaryEventKind::PlanEnd,
            stream_index: None,
            track_id: None,
            output_offset: total_payload_size,
            decode_time: max_decode_end_time,
        }));

        Self { events }
    }

    pub(crate) fn cursor(&self) -> MuxEventCursor<'_> {
        MuxEventCursor {
            events: &self.events,
            index: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn events(&self) -> &[MuxEvent] {
        &self.events
    }
}

const fn boundary_kind_from_duration(kind: MuxDurationBoundaryKind) -> MuxBoundaryEventKind {
    match kind {
        MuxDurationBoundaryKind::Segment => MuxBoundaryEventKind::SegmentBoundary,
        MuxDurationBoundaryKind::Fragment => MuxBoundaryEventKind::FragmentBoundary,
    }
}

pub(crate) struct MuxEventCursor<'a> {
    events: &'a [MuxEvent],
    index: usize,
}

impl<'a> MuxEventCursor<'a> {
    pub(crate) fn next_sample(&mut self) -> Option<&'a MuxSampleEvent> {
        while let Some(event) = self.events.get(self.index) {
            self.index += 1;
            if let MuxEvent::Sample(sample) = event {
                return Some(sample);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::{
        MuxDurationBoundaryKind, MuxInterleavePolicy, MuxStagedMediaItem,
        TrackCoordinationDirective, plan_staged_media_items,
        plan_staged_media_items_with_coordination,
    };

    #[test]
    fn event_graph_emits_streams_samples_track_drains_and_plan_end() {
        let plan = plan_staged_media_items(
            vec![
                MuxStagedMediaItem::new(0, 2, 10, 4, 13, 2),
                MuxStagedMediaItem::new(1, 1, 0, 5, 4, 4).with_sync_sample(true),
                MuxStagedMediaItem::new(0, 2, 0, 4, 4, 5).with_composition_time_offset(2),
            ],
            MuxInterleavePolicy::DecodeTime,
        )
        .unwrap();

        let events = plan.event_graph().events();
        assert!(matches!(
            &events[0],
            MuxEvent::StreamDescription(MuxStreamDescription {
                stream_index: 0,
                track_id: 1,
                item_count: 1,
                first_decode_time: 0,
                end_decode_time: 5,
                first_output_offset: 5,
                end_output_offset: 9,
            })
        ));
        assert!(matches!(
            &events[1],
            MuxEvent::StreamDescription(MuxStreamDescription {
                stream_index: 1,
                track_id: 2,
                item_count: 2,
                first_decode_time: 0,
                end_decode_time: 14,
                first_output_offset: 0,
                end_output_offset: 11,
            })
        ));
        assert!(matches!(
            &events[2],
            MuxEvent::Sample(MuxSampleEvent {
                stream_index: 1,
                sample_index_in_stream: 0,
                planned_item,
            }) if planned_item.output_offset() == 0
        ));
        assert!(matches!(
            &events[3],
            MuxEvent::Sample(MuxSampleEvent {
                stream_index: 0,
                sample_index_in_stream: 0,
                planned_item,
            }) if planned_item.output_offset() == 5
        ));
        assert!(matches!(
            &events[4],
            MuxEvent::Boundary(MuxBoundaryEvent {
                kind: MuxBoundaryEventKind::TrackDrain,
                stream_index: Some(0),
                track_id: Some(1),
                output_offset: 9,
                decode_time: 5,
            })
        ));
        assert!(matches!(
            &events[5],
            MuxEvent::Sample(MuxSampleEvent {
                stream_index: 1,
                sample_index_in_stream: 1,
                planned_item,
            }) if planned_item.output_offset() == 9
        ));
        assert!(matches!(
            &events[6],
            MuxEvent::Boundary(MuxBoundaryEvent {
                kind: MuxBoundaryEventKind::TrackDrain,
                stream_index: Some(1),
                track_id: Some(2),
                output_offset: 11,
                decode_time: 14,
            })
        ));
        assert!(matches!(
            &events[7],
            MuxEvent::Boundary(MuxBoundaryEvent {
                kind: MuxBoundaryEventKind::PlanEnd,
                stream_index: None,
                track_id: None,
                output_offset: 11,
                decode_time: 14,
            })
        ));
    }

    #[test]
    fn event_cursor_skips_non_sample_events_when_asked_for_samples() {
        let plan = plan_staged_media_items(
            vec![
                MuxStagedMediaItem::new(0, 1, 0, 4, 4, 5),
                MuxStagedMediaItem::new(1, 2, 5, 4, 4, 4),
            ],
            MuxInterleavePolicy::DecodeTime,
        )
        .unwrap();

        let mut cursor = plan.event_graph().cursor();

        let first = cursor.next_sample().unwrap();
        assert_eq!(first.stream_index, 0);
        assert_eq!(first.sample_index_in_stream, 0);
        assert_eq!(first.planned_item.output_offset(), 0);

        let second = cursor.next_sample().unwrap();
        assert_eq!(second.stream_index, 1);
        assert_eq!(second.sample_index_in_stream, 0);
        assert_eq!(second.planned_item.output_offset(), 5);

        assert!(cursor.next_sample().is_none());
    }

    #[test]
    fn event_graph_emits_duration_boundaries_from_coordination() {
        let plan = plan_staged_media_items_with_coordination(
            vec![
                MuxStagedMediaItem::new(0, 7, 0, 10, 0, 3),
                MuxStagedMediaItem::new(0, 7, 10, 10, 3, 3),
                MuxStagedMediaItem::new(0, 7, 20, 10, 6, 3),
            ],
            MuxInterleavePolicy::DecodeTime,
            vec![
                TrackCoordinationDirective::new(7, vec![2, 1])
                    .with_duration_boundaries(MuxDurationBoundaryKind::Fragment),
            ],
        )
        .unwrap();

        let events = plan.event_graph().events();
        assert!(matches!(
            &events[3],
            MuxEvent::Boundary(MuxBoundaryEvent {
                kind: MuxBoundaryEventKind::FragmentBoundary,
                stream_index: Some(0),
                track_id: Some(7),
                output_offset: 6,
                decode_time: 20,
            })
        ));
        assert!(matches!(
            &events[5],
            MuxEvent::Boundary(MuxBoundaryEvent {
                kind: MuxBoundaryEventKind::FragmentBoundary,
                stream_index: Some(0),
                track_id: Some(7),
                output_offset: 9,
                decode_time: 30,
            })
        ));
        assert!(matches!(
            &events[6],
            MuxEvent::Boundary(MuxBoundaryEvent {
                kind: MuxBoundaryEventKind::TrackDrain,
                stream_index: Some(0),
                track_id: Some(7),
                output_offset: 9,
                decode_time: 30,
            })
        ));
    }
}
