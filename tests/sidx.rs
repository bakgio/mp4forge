#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;
use std::io::Cursor;

use mp4forge::boxes::iso14496_12::{
    Ftyp, Hdlr, Mdhd, Mdia, Mfhd, Moof, Moov, Mvex, Sidx, SidxReference, Styp,
    TFHD_DEFAULT_SAMPLE_DURATION_PRESENT, TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT,
    TRUN_SAMPLE_DURATION_PRESENT, TRUN_SAMPLE_SIZE_PRESENT, Tfdt, Tfhd, Tkhd, Traf, Trak, Trex,
    Trun, TrunEntry,
};
use mp4forge::codec::{ImmutableBox, MutableBox};
use mp4forge::extract::{extract_box, extract_box_as};
use mp4forge::sidx::{
    SidxAnalysisError, SidxPlanError, SidxRewriteError, TopLevelSidxPlanAction,
    TopLevelSidxPlanOptions, analyze_top_level_sidx_update, analyze_top_level_sidx_update_bytes,
    apply_top_level_sidx_plan, apply_top_level_sidx_plan_bytes, build_top_level_sidx_plan,
    plan_top_level_sidx_update, plan_top_level_sidx_update_bytes,
};
#[cfg(feature = "async")]
use mp4forge::sidx::{apply_top_level_sidx_plan_async, plan_top_level_sidx_update_async};
use mp4forge::walk::BoxPath;

use support::{encode_raw_box, encode_supported_box, fixture_path, fourcc};

#[test]
fn analyze_top_level_sidx_update_prefers_video_over_first_track() {
    let input = build_audio_first_fragmented_file();

    let analysis = analyze_top_level_sidx_update_bytes(&input).unwrap();

    assert_eq!(analysis.timing_track.track_id, 2);
    assert_eq!(analysis.timing_track.handler_type, Some(fourcc("vide")));
    assert_eq!(analysis.timing_track.timescale, 1_000);
    assert!(analysis.placement.existing_top_level_sidxs.is_empty());
    assert_eq!(analysis.segments.len(), 1);

    let segment = &analysis.segments[0];
    assert_eq!(segment.moof_count, 1);
    assert_eq!(segment.timing_fragment_count, 1);
    assert_eq!(segment.base_decode_time, 900);
    assert_eq!(segment.presentation_time, 900);
    assert_eq!(segment.duration, 100);
}

#[test]
fn analyze_top_level_sidx_update_groups_styp_runs_separately() {
    let input = build_styp_fragmented_single_track_file();
    let analysis = analyze_top_level_sidx_update_bytes(&input).unwrap();

    let styps = extract_box(
        &mut Cursor::new(input.clone()),
        None,
        BoxPath::from([fourcc("styp")]),
    )
    .unwrap();

    assert!(analysis.placement.existing_top_level_sidxs.is_empty());
    assert_eq!(analysis.placement.insertion_box, styps[0]);
    assert_eq!(analysis.segments.len(), 2);

    let first = &analysis.segments[0];
    assert_eq!(first.first_box, styps[0]);
    assert_eq!(first.presentation_time, 105);
    assert_eq!(first.base_decode_time, 100);
    assert_eq!(first.duration, 60);
    assert_eq!(
        first.size,
        total_box_size(
            &input,
            &[fourcc("styp"), fourcc("moof"), fourcc("mdat")],
            0,
            1
        )
    );

    let second = &analysis.segments[1];
    assert_eq!(second.first_box, styps[1]);
    assert_eq!(second.presentation_time, 160);
    assert_eq!(second.base_decode_time, 160);
    assert_eq!(second.duration, 40);
    assert_eq!(
        second.size,
        total_box_size(
            &input,
            &[fourcc("styp"), fourcc("moof"), fourcc("mdat")],
            1,
            1
        )
    );
}

#[test]
fn analyze_top_level_sidx_update_uses_existing_top_level_sidx_boundaries() {
    let input = build_top_level_sidx_fragmented_single_track_file(false);
    let analysis = analyze_top_level_sidx_update_bytes(&input).unwrap();

    let sidx = extract_box(
        &mut Cursor::new(input.clone()),
        None,
        BoxPath::from([fourcc("sidx")]),
    )
    .unwrap();
    let moofs = extract_box(
        &mut Cursor::new(input.clone()),
        None,
        BoxPath::from([fourcc("moof")]),
    )
    .unwrap();
    let mdats = extract_box(
        &mut Cursor::new(input),
        None,
        BoxPath::from([fourcc("mdat")]),
    )
    .unwrap();

    assert_eq!(analysis.placement.insertion_box, moofs[0]);
    assert_eq!(analysis.placement.existing_top_level_sidxs.len(), 1);
    assert_eq!(analysis.placement.existing_top_level_sidxs[0].info, sidx[0]);
    assert_eq!(
        analysis.placement.existing_top_level_sidxs[0].segment_starts,
        moofs.iter().map(|info| info.offset()).collect::<Vec<_>>()
    );

    assert_eq!(analysis.segments.len(), 2);
    assert_eq!(analysis.segments[0].first_box, moofs[0]);
    assert_eq!(analysis.segments[0].duration, 60);
    assert_eq!(analysis.segments[0].size, moofs[0].size() + mdats[0].size());
    assert_eq!(analysis.segments[1].first_box, moofs[1]);
    assert_eq!(analysis.segments[1].duration, 40);
    assert_eq!(analysis.segments[1].size, moofs[1].size() + mdats[1].size());
}

#[test]
fn analyze_top_level_sidx_update_matches_interleaved_fixture_grouping() {
    let input = fs::read(fixture_path("sample_fragmented.mp4")).unwrap();
    let analysis = analyze_top_level_sidx_update(&mut Cursor::new(&input)).unwrap();

    let moofs = extract_box(
        &mut Cursor::new(&input),
        None,
        BoxPath::from([fourcc("moof")]),
    )
    .unwrap();
    let mdats = extract_box(
        &mut Cursor::new(&input),
        None,
        BoxPath::from([fourcc("mdat")]),
    )
    .unwrap();
    let expected_size = moofs.iter().map(|info| info.size()).sum::<u64>()
        + mdats.iter().map(|info| info.size()).sum::<u64>();

    assert_eq!(analysis.timing_track.track_id, 1);
    assert_eq!(analysis.timing_track.handler_type, Some(fourcc("vide")));
    assert!(analysis.placement.existing_top_level_sidxs.is_empty());
    assert_eq!(analysis.placement.insertion_box, moofs[0]);
    assert_eq!(analysis.segments.len(), 1);

    let segment = &analysis.segments[0];
    assert_eq!(segment.first_box, moofs[0]);
    assert_eq!(segment.first_moof_offset, moofs[0].offset());
    assert_eq!(segment.moof_count, moofs.len());
    assert_eq!(segment.timing_fragment_count, 4);
    assert_eq!(segment.size, expected_size);
}

#[test]
fn analyze_top_level_sidx_update_rejects_chained_top_level_entries() {
    let input = build_top_level_sidx_fragmented_single_track_file(true);
    let error = analyze_top_level_sidx_update_bytes(&input).unwrap_err();

    assert!(matches!(
        error,
        SidxAnalysisError::UnsupportedTopLevelSidxIndirectEntry { entry_index: 1, .. }
    ));
}

#[test]
fn plan_top_level_sidx_update_returns_none_when_add_if_not_exists_is_disabled() {
    let input = build_styp_fragmented_single_track_file();

    let plan = plan_top_level_sidx_update_bytes(
        &input,
        TopLevelSidxPlanOptions {
            add_if_not_exists: false,
            non_zero_ept: false,
        },
    )
    .unwrap();

    assert!(plan.is_none());
}

#[test]
fn plan_top_level_sidx_update_builds_insert_plan_with_default_values() {
    let input = build_styp_fragmented_single_track_file();

    let plan = plan_top_level_sidx_update_bytes(
        &input,
        TopLevelSidxPlanOptions {
            add_if_not_exists: true,
            non_zero_ept: false,
        },
    )
    .unwrap()
    .unwrap();

    let styps = extract_box(
        &mut Cursor::new(&input),
        None,
        BoxPath::from([fourcc("styp")]),
    )
    .unwrap();

    assert_eq!(plan.timing_track.track_id, 1);
    assert_eq!(plan.action, TopLevelSidxPlanAction::Insert);
    assert_eq!(plan.insertion_box, styps[0]);
    assert_eq!(plan.sidx.reference_id, 1);
    assert_eq!(plan.sidx.timescale, 1_000);
    assert_eq!(plan.sidx.earliest_presentation_time(), 0);
    assert_eq!(plan.sidx.first_offset(), 0);
    assert_eq!(plan.sidx.reference_count, 2);
    assert_eq!(plan.sidx.references.len(), 2);
    assert_eq!(
        plan.sidx.references,
        vec![
            SidxReference {
                reference_type: false,
                referenced_size: u32::try_from(plan.entries[0].segment.size).unwrap(),
                subsegment_duration: 60,
                starts_with_sap: true,
                sap_type: 1,
                sap_delta_time: 0,
            },
            SidxReference {
                reference_type: false,
                referenced_size: u32::try_from(plan.entries[1].segment.size).unwrap(),
                subsegment_duration: 40,
                starts_with_sap: true,
                sap_type: 1,
                sap_delta_time: 0,
            },
        ]
    );
    assert_eq!(plan.entries[0].start_offset, styps[0].offset());
    assert_eq!(
        plan.entries[0].end_offset,
        styps[0].offset() + plan.entries[0].segment.size
    );
    assert_eq!(plan.entries[1].start_offset, styps[1].offset());
    assert!(plan.encoded_box_size >= 44);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_plan_top_level_sidx_update_builds_insert_plan_with_default_values() {
    let input = build_styp_fragmented_single_track_file();

    let async_plan = plan_top_level_sidx_update_async(
        &mut Cursor::new(&input),
        TopLevelSidxPlanOptions {
            add_if_not_exists: true,
            non_zero_ept: false,
        },
    )
    .await
    .unwrap();
    let sync_plan = plan_top_level_sidx_update_bytes(
        &input,
        TopLevelSidxPlanOptions {
            add_if_not_exists: true,
            non_zero_ept: false,
        },
    )
    .unwrap();

    assert_eq!(async_plan, sync_plan);
}

#[test]
fn plan_top_level_sidx_update_builds_replace_plan_with_non_zero_ept() {
    let input = build_top_level_sidx_fragmented_single_track_file(false);

    let plan = plan_top_level_sidx_update(
        &mut Cursor::new(&input),
        TopLevelSidxPlanOptions {
            add_if_not_exists: false,
            non_zero_ept: true,
        },
    )
    .unwrap()
    .unwrap();

    let sidx = extract_box(
        &mut Cursor::new(&input),
        None,
        BoxPath::from([fourcc("sidx")]),
    )
    .unwrap();

    match &plan.action {
        TopLevelSidxPlanAction::Replace { existing } => {
            assert_eq!(existing.info, sidx[0]);
        }
        TopLevelSidxPlanAction::Insert => panic!("expected replace plan"),
    }
    assert_eq!(plan.insertion_box.offset(), plan.entries[0].start_offset);
    assert_eq!(plan.sidx.earliest_presentation_time(), 105);
    assert_eq!(plan.sidx.first_offset(), 0);
    assert_eq!(plan.sidx.reference_count, 2);
    assert_eq!(plan.sidx.references[0].subsegment_duration, 60);
    assert_eq!(plan.sidx.references[1].subsegment_duration, 40);
}

#[test]
fn plan_top_level_sidx_update_matches_interleaved_fixture_payload() {
    let input = fs::read(fixture_path("sample_fragmented.mp4")).unwrap();
    let probe = mp4forge::probe::probe(&mut Cursor::new(&input)).unwrap();

    let plan = plan_top_level_sidx_update_bytes(
        &input,
        TopLevelSidxPlanOptions {
            add_if_not_exists: true,
            non_zero_ept: false,
        },
    )
    .unwrap()
    .unwrap();

    let moofs = extract_box(
        &mut Cursor::new(&input),
        None,
        BoxPath::from([fourcc("moof")]),
    )
    .unwrap();
    let mdats = extract_box(
        &mut Cursor::new(&input),
        None,
        BoxPath::from([fourcc("mdat")]),
    )
    .unwrap();
    let expected_size = moofs.iter().map(|info| info.size()).sum::<u64>()
        + mdats.iter().map(|info| info.size()).sum::<u64>();
    let expected_duration = probe
        .segments
        .iter()
        .filter(|segment| segment.track_id == 1)
        .map(|segment| u64::from(segment.duration))
        .sum::<u64>();

    assert_eq!(plan.action, TopLevelSidxPlanAction::Insert);
    assert_eq!(plan.sidx.reference_count, 1);
    assert_eq!(plan.sidx.earliest_presentation_time(), 0);
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].segment.size, expected_size);
    assert_eq!(
        plan.entries[0].subsegment_duration,
        expected_duration as u32
    );
    assert_eq!(
        plan.sidx.references[0].referenced_size,
        expected_size as u32
    );
    assert_eq!(
        plan.sidx.references[0].subsegment_duration,
        expected_duration as u32
    );
}

#[test]
fn apply_top_level_sidx_plan_bytes_inserts_top_level_sidx_and_preserves_other_bytes() {
    let input = build_styp_fragmented_single_track_file();
    let plan = plan_top_level_sidx_update_bytes(
        &input,
        TopLevelSidxPlanOptions {
            add_if_not_exists: true,
            non_zero_ept: false,
        },
    )
    .unwrap()
    .unwrap();

    let output = apply_top_level_sidx_plan_bytes(&input, &plan).unwrap();
    let sidx = extract_box_as::<_, Sidx>(
        &mut Cursor::new(&output),
        None,
        BoxPath::from([fourcc("sidx")]),
    )
    .unwrap();
    let sidx_info = extract_box(
        &mut Cursor::new(&output),
        None,
        BoxPath::from([fourcc("sidx")]),
    )
    .unwrap();

    assert_eq!(sidx.len(), 1);
    assert_eq!(sidx[0], plan.sidx);
    let insertion_offset = plan.insertion_box.offset() as usize;
    let encoded_size = sidx_info[0].size() as usize;
    assert_eq!(&output[..insertion_offset], &input[..insertion_offset]);
    assert_eq!(
        &output[insertion_offset + encoded_size..],
        &input[insertion_offset..]
    );
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_apply_top_level_sidx_plan_inserts_top_level_sidx_and_preserves_other_bytes() {
    let input = build_styp_fragmented_single_track_file();
    let plan = plan_top_level_sidx_update_async(
        &mut Cursor::new(&input),
        TopLevelSidxPlanOptions {
            add_if_not_exists: true,
            non_zero_ept: false,
        },
    )
    .await
    .unwrap()
    .unwrap();

    let mut output = Cursor::new(Vec::new());
    let applied = apply_top_level_sidx_plan_async(&mut Cursor::new(&input), &mut output, &plan)
        .await
        .unwrap();
    let output = output.into_inner();
    let sidx = extract_box_as::<_, Sidx>(
        &mut Cursor::new(&output),
        None,
        BoxPath::from([fourcc("sidx")]),
    )
    .unwrap();

    assert_eq!(sidx.len(), 1);
    assert_eq!(sidx[0], applied.sidx);
    let insertion_offset = plan.insertion_box.offset() as usize;
    let encoded_size = applied.info.size() as usize;
    assert_eq!(&output[..insertion_offset], &input[..insertion_offset]);
    assert_eq!(
        &output[insertion_offset + encoded_size..],
        &input[insertion_offset..]
    );
}

#[test]
fn apply_top_level_sidx_plan_replaces_existing_box_and_preserves_following_bytes() {
    let input = build_gapped_top_level_sidx_fragmented_single_track_file_v0();
    let plan = plan_top_level_sidx_update_bytes(
        &input,
        TopLevelSidxPlanOptions {
            add_if_not_exists: false,
            non_zero_ept: true,
        },
    )
    .unwrap()
    .unwrap();

    let existing = match &plan.action {
        TopLevelSidxPlanAction::Replace { existing } => existing.clone(),
        TopLevelSidxPlanAction::Insert => panic!("expected replace plan"),
    };

    let mut output = Vec::new();
    let applied = apply_top_level_sidx_plan(&mut Cursor::new(&input), &mut output, &plan).unwrap();

    assert_eq!(applied.info.offset(), existing.info.offset());
    assert_eq!(applied.sidx.version(), 1);
    assert_eq!(applied.sidx.earliest_presentation_time(), 105);
    assert_eq!(applied.sidx.first_offset(), segment_gap_box().len() as u64);
    assert_eq!(
        &output[..existing.info.offset() as usize],
        &input[..existing.info.offset() as usize]
    );
    let new_end = (applied.info.offset() + applied.info.size()) as usize;
    let old_end = (existing.info.offset() + existing.info.size()) as usize;
    assert_eq!(&output[new_end..], &input[old_end..]);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_apply_top_level_sidx_plan_replaces_existing_box_and_preserves_following_bytes() {
    let input = build_gapped_top_level_sidx_fragmented_single_track_file_v0();
    let plan = plan_top_level_sidx_update_async(
        &mut Cursor::new(&input),
        TopLevelSidxPlanOptions {
            add_if_not_exists: false,
            non_zero_ept: true,
        },
    )
    .await
    .unwrap()
    .unwrap();

    let existing = match &plan.action {
        TopLevelSidxPlanAction::Replace { existing } => existing.clone(),
        TopLevelSidxPlanAction::Insert => panic!("expected replace plan"),
    };

    let mut output = Cursor::new(Vec::new());
    let applied = apply_top_level_sidx_plan_async(&mut Cursor::new(&input), &mut output, &plan)
        .await
        .unwrap();
    let output = output.into_inner();

    assert_eq!(applied.info.offset(), existing.info.offset());
    assert_eq!(applied.sidx.version(), 1);
    assert_eq!(applied.sidx.earliest_presentation_time(), 105);
    assert_eq!(applied.sidx.first_offset(), segment_gap_box().len() as u64);
    assert_eq!(
        &output[..existing.info.offset() as usize],
        &input[..existing.info.offset() as usize]
    );
    let new_end = (applied.info.offset() + applied.info.size()) as usize;
    let old_end = (existing.info.offset() + existing.info.size()) as usize;
    assert_eq!(&output[new_end..], &input[old_end..]);
}

#[test]
fn apply_top_level_sidx_plan_bytes_is_stable_after_replanning() {
    let input = build_styp_fragmented_single_track_file();
    let options = TopLevelSidxPlanOptions {
        add_if_not_exists: true,
        non_zero_ept: false,
    };

    let first_plan = plan_top_level_sidx_update_bytes(&input, options)
        .unwrap()
        .unwrap();
    let first_output = apply_top_level_sidx_plan_bytes(&input, &first_plan).unwrap();
    let second_plan = plan_top_level_sidx_update_bytes(&first_output, options)
        .unwrap()
        .unwrap();
    let second_output = apply_top_level_sidx_plan_bytes(&first_output, &second_plan).unwrap();

    assert!(matches!(
        second_plan.action,
        TopLevelSidxPlanAction::Replace { .. }
    ));
    assert_eq!(second_output, first_output);
}

#[test]
fn apply_top_level_sidx_plan_bytes_rejects_stale_input() {
    let input = build_styp_fragmented_single_track_file();
    let stale_input = build_audio_first_fragmented_file();
    let plan = plan_top_level_sidx_update_bytes(
        &input,
        TopLevelSidxPlanOptions {
            add_if_not_exists: true,
            non_zero_ept: false,
        },
    )
    .unwrap()
    .unwrap();

    let error = apply_top_level_sidx_plan_bytes(&stale_input, &plan).unwrap_err();

    assert!(matches!(
        error,
        SidxRewriteError::PlannedBoxMismatch {
            expected_type,
            ..
        } if expected_type == fourcc("styp")
    ));
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_apply_top_level_sidx_plan_rejects_stale_input() {
    let input = build_styp_fragmented_single_track_file();
    let stale_input = build_audio_first_fragmented_file();
    let plan = plan_top_level_sidx_update_async(
        &mut Cursor::new(&input),
        TopLevelSidxPlanOptions {
            add_if_not_exists: true,
            non_zero_ept: false,
        },
    )
    .await
    .unwrap()
    .unwrap();

    let error = apply_top_level_sidx_plan_async(
        &mut Cursor::new(&stale_input),
        &mut Cursor::new(Vec::new()),
        &plan,
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        SidxRewriteError::PlannedBoxMismatch {
            expected_type,
            ..
        } if expected_type == fourcc("styp")
    ));
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_sidx_helpers_can_run_on_tokio_worker_threads() {
    let input = build_styp_fragmented_single_track_file();
    let plan_handle = tokio::spawn(async move {
        let mut reader = Cursor::new(input);
        plan_top_level_sidx_update_async(
            &mut reader,
            TopLevelSidxPlanOptions {
                add_if_not_exists: true,
                non_zero_ept: false,
            },
        )
        .await
        .unwrap()
        .unwrap()
    });
    let plan = plan_handle.await.unwrap();
    assert_eq!(plan.sidx.reference_count, 2);

    let input = build_styp_fragmented_single_track_file();
    let apply_handle = tokio::spawn(async move {
        let mut reader = Cursor::new(input);
        let mut writer = Cursor::new(Vec::new());
        let applied = apply_top_level_sidx_plan_async(&mut reader, &mut writer, &plan)
            .await
            .unwrap();
        (applied, writer.into_inner())
    });
    let (applied, bytes) = apply_handle.await.unwrap();
    assert_eq!(applied.sidx.reference_count, 2);
    assert_eq!(
        extract_box_as::<_, Sidx>(
            &mut Cursor::new(bytes),
            None,
            BoxPath::from([fourcc("sidx")])
        )
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn build_top_level_sidx_plan_rejects_multiple_file_level_top_level_sidx_boxes() {
    let input = build_multiple_top_level_sidx_fragmented_single_track_file();
    let analysis = analyze_top_level_sidx_update_bytes(&input).unwrap();

    let error = build_top_level_sidx_plan(
        &analysis,
        TopLevelSidxPlanOptions {
            add_if_not_exists: true,
            non_zero_ept: false,
        },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        SidxPlanError::UnsupportedTopLevelSidxCount { count: 2 }
    ));
}

struct TrackSpec {
    track_id: u32,
    handler_type: &'static str,
    timescale: u32,
}

struct TrafSpec<'a> {
    track_id: u32,
    base_decode_time: u64,
    default_sample_duration: Option<u32>,
    sample_durations: &'a [u32],
    sample_sizes: &'a [u32],
    composition_offsets: &'a [u32],
}

fn build_audio_first_fragmented_file() -> Vec<u8> {
    let ftyp = fragmented_ftyp();
    let moov = build_fragmented_moov(&[
        TrackSpec {
            track_id: 1,
            handler_type: "soun",
            timescale: 48_000,
        },
        TrackSpec {
            track_id: 2,
            handler_type: "vide",
            timescale: 1_000,
        },
    ]);
    let moof = build_moof(&[
        TrafSpec {
            track_id: 1,
            base_decode_time: 0,
            default_sample_duration: Some(20),
            sample_durations: &[],
            sample_sizes: &[],
            composition_offsets: &[],
        },
        TrafSpec {
            track_id: 2,
            base_decode_time: 900,
            default_sample_duration: Some(50),
            sample_durations: &[],
            sample_sizes: &[],
            composition_offsets: &[],
        },
    ]);
    let mdat = encode_raw_box(fourcc("mdat"), &[0; 20]);
    [ftyp, moov, moof, mdat].concat()
}

fn build_styp_fragmented_single_track_file() -> Vec<u8> {
    let ftyp = fragmented_ftyp();
    let moov = build_fragmented_moov(&[TrackSpec {
        track_id: 1,
        handler_type: "vide",
        timescale: 1_000,
    }]);
    let styp1 = segment_styp();
    let moof1 = build_moof(&[TrafSpec {
        track_id: 1,
        base_decode_time: 100,
        default_sample_duration: None,
        sample_durations: &[30, 30],
        sample_sizes: &[4, 4],
        composition_offsets: &[5, 0],
    }]);
    let mdat1 = encode_raw_box(fourcc("mdat"), &[0; 8]);
    let styp2 = segment_styp();
    let moof2 = build_moof(&[TrafSpec {
        track_id: 1,
        base_decode_time: 160,
        default_sample_duration: None,
        sample_durations: &[40],
        sample_sizes: &[6],
        composition_offsets: &[0],
    }]);
    let mdat2 = encode_raw_box(fourcc("mdat"), &[0; 6]);

    [ftyp, moov, styp1, moof1, mdat1, styp2, moof2, mdat2].concat()
}

fn build_top_level_sidx_fragmented_single_track_file(indirect_first_entry: bool) -> Vec<u8> {
    let ftyp = fragmented_ftyp();
    let moov = build_fragmented_moov(&[TrackSpec {
        track_id: 1,
        handler_type: "vide",
        timescale: 1_000,
    }]);
    let moof1 = build_moof(&[TrafSpec {
        track_id: 1,
        base_decode_time: 100,
        default_sample_duration: None,
        sample_durations: &[30, 30],
        sample_sizes: &[4, 4],
        composition_offsets: &[5, 0],
    }]);
    let mdat1 = encode_raw_box(fourcc("mdat"), &[0; 8]);
    let moof2 = build_moof(&[TrafSpec {
        track_id: 1,
        base_decode_time: 160,
        default_sample_duration: None,
        sample_durations: &[40],
        sample_sizes: &[6],
        composition_offsets: &[0],
    }]);
    let mdat2 = encode_raw_box(fourcc("mdat"), &[0; 6]);

    let first_segment_size = u32::try_from(moof1.len() + mdat1.len()).unwrap();
    let second_segment_size = u32::try_from(moof2.len() + mdat2.len()).unwrap();

    let mut sidx = Sidx::default();
    sidx.set_version(1);
    sidx.reference_id = 1;
    sidx.timescale = 1_000;
    sidx.reference_count = 2;
    sidx.references = vec![
        SidxReference {
            reference_type: indirect_first_entry,
            referenced_size: first_segment_size,
            subsegment_duration: 60,
            starts_with_sap: true,
            sap_type: 1,
            sap_delta_time: 0,
        },
        SidxReference {
            reference_type: false,
            referenced_size: second_segment_size,
            subsegment_duration: 40,
            starts_with_sap: true,
            sap_type: 1,
            sap_delta_time: 0,
        },
    ];
    let sidx = encode_supported_box(&sidx, &[]);

    [ftyp, moov, sidx, moof1, mdat1, moof2, mdat2].concat()
}

fn build_gapped_top_level_sidx_fragmented_single_track_file_v0() -> Vec<u8> {
    let ftyp = fragmented_ftyp();
    let moov = build_fragmented_moov(&[TrackSpec {
        track_id: 1,
        handler_type: "vide",
        timescale: 1_000,
    }]);
    let gap = segment_gap_box();
    let moof1 = build_moof(&[TrafSpec {
        track_id: 1,
        base_decode_time: 100,
        default_sample_duration: None,
        sample_durations: &[30, 30],
        sample_sizes: &[4, 4],
        composition_offsets: &[5, 0],
    }]);
    let mdat1 = encode_raw_box(fourcc("mdat"), &[0; 8]);
    let moof2 = build_moof(&[TrafSpec {
        track_id: 1,
        base_decode_time: 160,
        default_sample_duration: None,
        sample_durations: &[40],
        sample_sizes: &[6],
        composition_offsets: &[0],
    }]);
    let mdat2 = encode_raw_box(fourcc("mdat"), &[0; 6]);

    let first_segment_size = u32::try_from(moof1.len() + mdat1.len()).unwrap();
    let second_segment_size = u32::try_from(moof2.len() + mdat2.len()).unwrap();

    let mut sidx = Sidx::default();
    sidx.set_version(0);
    sidx.reference_id = 1;
    sidx.timescale = 1_000;
    sidx.earliest_presentation_time_v0 = 105;
    sidx.first_offset_v0 = gap.len() as u32;
    sidx.reference_count = 2;
    sidx.references = vec![
        SidxReference {
            reference_type: false,
            referenced_size: first_segment_size,
            subsegment_duration: 60,
            starts_with_sap: true,
            sap_type: 1,
            sap_delta_time: 0,
        },
        SidxReference {
            reference_type: false,
            referenced_size: second_segment_size,
            subsegment_duration: 40,
            starts_with_sap: true,
            sap_type: 1,
            sap_delta_time: 0,
        },
    ];
    let sidx = encode_supported_box(&sidx, &[]);

    [ftyp, moov, sidx, gap, moof1, mdat1, moof2, mdat2].concat()
}

fn build_multiple_top_level_sidx_fragmented_single_track_file() -> Vec<u8> {
    let ftyp = fragmented_ftyp();
    let moov = build_fragmented_moov(&[TrackSpec {
        track_id: 1,
        handler_type: "vide",
        timescale: 1_000,
    }]);
    let moof1 = build_moof(&[TrafSpec {
        track_id: 1,
        base_decode_time: 100,
        default_sample_duration: None,
        sample_durations: &[30, 30],
        sample_sizes: &[4, 4],
        composition_offsets: &[5, 0],
    }]);
    let mdat1 = encode_raw_box(fourcc("mdat"), &[0; 8]);
    let moof2 = build_moof(&[TrafSpec {
        track_id: 1,
        base_decode_time: 160,
        default_sample_duration: None,
        sample_durations: &[40],
        sample_sizes: &[6],
        composition_offsets: &[0],
    }]);
    let mdat2 = encode_raw_box(fourcc("mdat"), &[0; 6]);

    let first_segment_size = u32::try_from(moof1.len() + mdat1.len()).unwrap();
    let second_segment_size = u32::try_from(moof2.len() + mdat2.len()).unwrap();

    let mut second_sidx = Sidx::default();
    second_sidx.set_version(1);
    second_sidx.reference_id = 1;
    second_sidx.timescale = 1_000;
    second_sidx.reference_count = 2;
    second_sidx.references = vec![
        SidxReference {
            reference_type: false,
            referenced_size: first_segment_size,
            subsegment_duration: 60,
            starts_with_sap: true,
            sap_type: 1,
            sap_delta_time: 0,
        },
        SidxReference {
            reference_type: false,
            referenced_size: second_segment_size,
            subsegment_duration: 40,
            starts_with_sap: true,
            sap_type: 1,
            sap_delta_time: 0,
        },
    ];
    let second_sidx_bytes = encode_supported_box(&second_sidx, &[]);

    let mut first_sidx = second_sidx.clone();
    first_sidx.first_offset_v1 = second_sidx_bytes.len() as u64;
    let first_sidx_bytes = encode_supported_box(&first_sidx, &[]);

    [
        ftyp,
        moov,
        first_sidx_bytes,
        second_sidx_bytes,
        moof1,
        mdat1,
        moof2,
        mdat2,
    ]
    .concat()
}

fn segment_gap_box() -> Vec<u8> {
    encode_raw_box(fourcc("free"), &[0x41, 0x42, 0x43, 0x44, 0x45])
}

fn fragmented_ftyp() -> Vec<u8> {
    encode_supported_box(
        &Ftyp {
            major_brand: fourcc("iso6"),
            minor_version: 1,
            compatible_brands: vec![fourcc("iso6"), fourcc("dash")],
        },
        &[],
    )
}

fn segment_styp() -> Vec<u8> {
    encode_supported_box(
        &Styp {
            major_brand: fourcc("msdh"),
            minor_version: 0,
            compatible_brands: vec![fourcc("msdh"), fourcc("msix")],
        },
        &[],
    )
}

fn build_fragmented_moov(track_specs: &[TrackSpec]) -> Vec<u8> {
    let tracks = track_specs
        .iter()
        .map(build_fragmented_trak)
        .collect::<Vec<_>>();

    let mut mvex_children = Vec::new();
    for track in track_specs {
        let mut trex = Trex::default();
        trex.track_id = track.track_id;
        mvex_children.extend_from_slice(&encode_supported_box(&trex, &[]));
    }

    let mut moov_children = Vec::new();
    for track in tracks {
        moov_children.extend_from_slice(&track);
    }
    moov_children.extend_from_slice(&encode_supported_box(&Mvex, &mvex_children));
    encode_supported_box(&Moov, &moov_children)
}

fn build_fragmented_trak(track: &TrackSpec) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = track.track_id;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = track.timescale;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);

    let mut hdlr = Hdlr::default();
    hdlr.handler_type = fourcc(track.handler_type);
    hdlr.name = track.handler_type.to_string();
    let hdlr = encode_supported_box(&hdlr, &[]);

    let mdia = encode_supported_box(&Mdia, &[mdhd, hdlr].concat());
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

fn build_moof(trafs: &[TrafSpec<'_>]) -> Vec<u8> {
    let mut mfhd = Mfhd::default();
    mfhd.sequence_number = 1;
    let mfhd = encode_supported_box(&mfhd, &[]);

    let mut moof_children = mfhd;
    for traf in trafs {
        moof_children.extend_from_slice(&build_traf(traf));
    }
    encode_supported_box(&Moof, &moof_children)
}

fn build_traf(spec: &TrafSpec<'_>) -> Vec<u8> {
    let mut tfhd = Tfhd::default();
    tfhd.track_id = spec.track_id;
    if let Some(default_sample_duration) = spec.default_sample_duration {
        tfhd.set_flags(TFHD_DEFAULT_SAMPLE_DURATION_PRESENT);
        tfhd.default_sample_duration = default_sample_duration;
    }
    let tfhd = encode_supported_box(&tfhd, &[]);

    let mut tfdt = Tfdt::default();
    tfdt.set_version(1);
    tfdt.base_media_decode_time_v1 = spec.base_decode_time;
    let tfdt = encode_supported_box(&tfdt, &[]);

    let mut trun = Trun::default();
    trun.sample_count = spec
        .sample_durations
        .len()
        .max(spec.sample_sizes.len())
        .max(if spec.default_sample_duration.is_some() {
            2
        } else {
            0
        }) as u32;
    if spec.sample_durations.is_empty() {
        trun.sample_count = if spec.default_sample_duration.is_some() {
            2
        } else {
            0
        };
    }

    let mut flags = 0_u32;
    if !spec.sample_durations.is_empty() {
        flags |= TRUN_SAMPLE_DURATION_PRESENT;
    }
    if !spec.sample_sizes.is_empty() {
        flags |= TRUN_SAMPLE_SIZE_PRESENT;
    }
    if !spec.composition_offsets.is_empty() {
        flags |= TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT;
    }
    trun.set_flags(flags);

    let entry_count = if flags == 0 {
        0
    } else {
        spec.sample_durations.len()
    };
    trun.sample_count = if flags == 0 { 2 } else { entry_count as u32 };
    trun.entries = (0..entry_count)
        .map(|index| TrunEntry {
            sample_duration: spec.sample_durations[index],
            sample_size: spec.sample_sizes[index],
            sample_composition_time_offset_v0: spec
                .composition_offsets
                .get(index)
                .copied()
                .unwrap_or(0),
            ..TrunEntry::default()
        })
        .collect();
    let trun = encode_supported_box(&trun, &[]);

    encode_supported_box(&Traf, &[tfhd, tfdt, trun].concat())
}

fn total_box_size(
    input: &[u8],
    types: &[mp4forge::FourCc],
    start_index: usize,
    count: usize,
) -> u64 {
    types
        .iter()
        .map(|box_type| {
            extract_box(&mut Cursor::new(input), None, BoxPath::from([*box_type])).unwrap()
                [start_index..start_index + count]
                .iter()
                .map(|info| info.size())
                .sum::<u64>()
        })
        .sum()
}
