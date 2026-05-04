#![cfg(feature = "mux")]

mod support;

use std::fs;
use std::io::Cursor;
use std::str::FromStr;

use mp4forge::BoxInfo;
use mp4forge::boxes::iso14496_12::{
    AudioSampleEntry, Co64, Ctts, Dinf, Dref, Edts, Elst, Ftyp, Hdlr, Mdhd, Mdia, Mehd, Meta, Minf,
    Moov, Mvex, Mvhd, Nmhd, SampleEntry, Sidx, Smhd, Stbl, Sthd, Stsc, StscEntry, Stsd, Stss, Stsz,
    Stts, SttsEntry, Tfdt, Tfhd, Tkhd, Trak, Trex, Trun, Url, VisualSampleEntry, Vmhd,
    XMLSubtitleSampleEntry,
};
use mp4forge::boxes::iso14496_30::{WVTTSampleEntry, WebVTTConfigurationBox, WebVTTSourceLabelBox};
use mp4forge::boxes::metadata::Id32;
use mp4forge::codec::MutableBox;
use mp4forge::extract::{extract_box_as, extract_box_bytes};
#[cfg(feature = "async")]
use mp4forge::mux::mux_to_path_async;
use mp4forge::mux::{
    MuxDurationMode, MuxError, MuxFileConfig, MuxInterleavePolicy, MuxMp4TrackSelector,
    MuxOutputLayout, MuxRawCodec, MuxRequest, MuxStagedMediaItem, MuxTrackConfig, MuxTrackKind,
    MuxTrackParameter, MuxTrackSpec, copy_planned_payloads, copy_planned_payloads_async,
    copy_planned_payloads_async_progressive, copy_planned_payloads_progressive,
    copy_planned_payloads_to_path, copy_planned_payloads_to_path_async, mux_to_path,
    plan_staged_media_items, write_mp4_mux, write_mp4_mux_to_path, write_mp4_mux_to_path_async,
};
use mp4forge::walk::{BoxPath, WalkControl, walk_structure};
#[cfg(feature = "async")]
use tokio::io::AsyncWriteExt;

use support::{
    TestMuxSample, encode_raw_box, encode_supported_box, fourcc, write_single_track_mp4_input,
    write_temp_file, write_test_ac3_44100_file, write_test_ac3_file, write_test_ac4_file,
    write_test_adts_file, write_test_eac3_file, write_test_h265_annexb_file, write_test_mp3_file,
    write_test_mp3_file_with_leading_id3_tag,
};

#[test]
fn mux_plan_orders_items_by_decode_time_and_assigns_output_offsets() {
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 2, 10, 4, 20, 3),
            MuxStagedMediaItem::new(1, 1, 0, 5, 4, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(0, 2, 0, 4, 12, 2).with_composition_time_offset(2),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    assert_eq!(plan.total_payload_size(), 9);
    assert_eq!(plan.track_plans().len(), 2);
    assert_eq!(
        plan.planned_items()
            .iter()
            .map(|item| (
                item.staged().track_id(),
                item.staged().decode_time(),
                item.decode_end_time(),
                item.output_offset(),
                item.output_end_offset(),
                item.staged().composition_time_offset(),
                item.staged().is_sync_sample(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (1, 0, 5, 0, 4, 0, true),
            (2, 0, 4, 4, 6, 2, false),
            (2, 10, 14, 6, 9, 0, false)
        ]
    );
    assert_eq!(
        plan.track_plans()
            .iter()
            .map(|track| (
                track.track_id(),
                track.item_count(),
                track.first_decode_time(),
                track.end_decode_time(),
            ))
            .collect::<Vec<_>>(),
        vec![(1, 1, 0, 5), (2, 2, 0, 14)]
    );
}

#[test]
fn mux_track_spec_from_str_accepts_the_widened_public_grammar() {
    assert_eq!(
        MuxTrackSpec::from_str("h264:path/to/video.h264").unwrap(),
        MuxTrackSpec::raw(MuxRawCodec::H264, "path/to/video.h264")
    );
    assert_eq!(
        MuxTrackSpec::from_str("aac:path/to/audio.aac").unwrap(),
        MuxTrackSpec::raw(MuxRawCodec::Aac, "path/to/audio.aac")
    );
    assert_eq!(
        MuxTrackSpec::from_str("path/to/file.mp4#video").unwrap(),
        MuxTrackSpec::mp4("path/to/file.mp4", MuxMp4TrackSelector::Video)
    );
    assert_eq!(
        MuxTrackSpec::from_str("path/to/file.mp4#audio").unwrap(),
        MuxTrackSpec::mp4(
            "path/to/file.mp4",
            MuxMp4TrackSelector::Audio { occurrence: 1 }
        )
    );
    assert_eq!(
        MuxTrackSpec::from_str("path/to/file.mp4#audio:2").unwrap(),
        MuxTrackSpec::mp4(
            "path/to/file.mp4",
            MuxMp4TrackSelector::Audio { occurrence: 2 }
        )
    );
    assert_eq!(
        MuxTrackSpec::from_str("path/to/file.mp4#text").unwrap(),
        MuxTrackSpec::mp4(
            "path/to/file.mp4",
            MuxMp4TrackSelector::Text { occurrence: 1 }
        )
    );
    assert_eq!(
        MuxTrackSpec::from_str("path/to/file.mp4#track:7").unwrap(),
        MuxTrackSpec::mp4(
            "path/to/file.mp4",
            MuxMp4TrackSelector::TrackId { track_id: 7 }
        )
    );
    assert_eq!(
        MuxTrackSpec::from_str("h265:path/to/video.h265#sample_entry=hvc1,profile=main").unwrap(),
        MuxTrackSpec::Raw {
            codec: MuxRawCodec::H265,
            path: "path/to/video.h265".into(),
            parameters: vec![
                MuxTrackParameter::new("sample_entry", "hvc1"),
                MuxTrackParameter::new("profile", "main"),
            ],
        }
    );
    assert_eq!(
        MuxTrackSpec::from_str("video:path/to/video.h264").unwrap(),
        MuxTrackSpec::raw(MuxRawCodec::H264, "path/to/video.h264")
    );
}

#[test]
fn copy_planned_payloads_uses_the_planned_output_order() {
    let mut sources = [
        Cursor::new(b"AAAAhelloBBBBxy".to_vec()),
        Cursor::new(b"zzzzSYNCtail".to_vec()),
    ];
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 2, 10, 4, 13, 2),
            MuxStagedMediaItem::new(1, 1, 0, 5, 4, 4),
            MuxStagedMediaItem::new(0, 2, 0, 4, 4, 5),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    let mut output = Vec::new();
    copy_planned_payloads(&mut sources, &mut output, &plan).unwrap();

    assert_eq!(output, b"SYNChelloxy");
}

#[test]
fn copy_planned_payloads_progressive_supports_non_seekable_readers() {
    let mut first_source: &[u8] = b"AAAAhelloBBBBxy";
    let mut second_source: &[u8] = b"zzzzSYNCtail";
    let mut sources = [&mut first_source, &mut second_source];
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 1, 0, 4, 4, 5),
            MuxStagedMediaItem::new(1, 2, 5, 4, 4, 4),
            MuxStagedMediaItem::new(0, 1, 10, 4, 13, 2),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    let mut output = Vec::new();
    copy_planned_payloads_progressive(&mut sources, &mut output, &plan).unwrap();

    assert_eq!(output, b"helloSYNCxy");
}

#[test]
fn copy_planned_payloads_progressive_rejects_backward_offsets_per_source() {
    let mut source: &[u8] = b"AAAAhelloBBBBxy";
    let mut sources = [&mut source];
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 1, 0, 4, 13, 2),
            MuxStagedMediaItem::new(0, 1, 10, 4, 4, 5),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    let mut output = Vec::new();
    let error = copy_planned_payloads_progressive(&mut sources, &mut output, &plan).unwrap_err();

    assert_eq!(
        error.to_string(),
        "source index 0 would need to move backward from offset 15 to 4"
    );
    assert!(matches!(
        error,
        MuxError::NonMonotonicSourceOffset {
            source_index: 0,
            previous_offset: 15,
            next_offset: 4,
        }
    ));
}

#[test]
fn copy_planned_payloads_to_path_matches_in_memory_output() {
    let first_source = write_temp_file("mux-source-a", b"HEADvideoTAIL");
    let second_source = write_temp_file("mux-source-b", b"PREMaudPOST");
    let output_path = write_temp_file("mux-output-sync", &[]);
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 2, 10, 4, 4, 5),
            MuxStagedMediaItem::new(1, 1, 0, 4, 4, 3),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    copy_planned_payloads_to_path(&[&first_source, &second_source], &output_path, &plan).unwrap();

    assert_eq!(fs::read(output_path).unwrap(), b"audvideo");
}

#[test]
fn mux_to_path_merges_mp4_track_specs_and_uses_the_first_mp4_as_authority() {
    let audio_input = build_audio_input_file("mux-request-audio-input", fourcc("dash"), &[b"aud"]);
    let video_input =
        build_video_input_file("mux-request-video-input", fourcc("isom"), &[b"video"]);
    let output_path = write_temp_file("mux-request-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::mp4(
            audio_input.clone(),
            MuxMp4TrackSelector::Audio { occurrence: 1 },
        ),
        MuxTrackSpec::mp4(video_input.clone(), MuxMp4TrackSelector::Video),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        root_boxes.iter().map(BoxInfo::box_type).collect::<Vec<_>>(),
        vec![fourcc("ftyp"), fourcc("moov"), fourcc("mdat")]
    );
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"audvideo");

    let ftyp = extract_boxes::<Ftyp>(&output_bytes, BoxPath::from([fourcc("ftyp")]));
    assert_eq!(ftyp.len(), 1);
    assert_eq!(ftyp[0].major_brand, fourcc("dash"));
}

#[test]
fn mux_to_path_rejects_duration_modes_for_flat_layout() {
    let audio_input =
        build_audio_input_file("mux-flat-duration-audio-input", fourcc("dash"), &[b"aud"]);
    let output_path = write_temp_file("mux-flat-duration-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        audio_input,
        MuxMp4TrackSelector::Audio { occurrence: 1 },
    )])
    .with_duration_mode(MuxDurationMode::Fragment { seconds: 0.25 });

    let error = mux_to_path(&request, &output_path).unwrap_err();

    assert_eq!(
        error.to_string(),
        "invalid mux layout `flat`: flat output does not support `--fragment_duration`; use `--layout fragmented` instead"
    );
    assert!(matches!(
        error,
        MuxError::InvalidOutputLayout { layout: "flat", .. }
    ));
}

#[test]
fn mux_to_path_requires_one_duration_mode_for_fragmented_layout() {
    let audio_input = build_audio_input_file(
        "mux-fragmented-no-duration-input",
        fourcc("dash"),
        &[b"aud"],
    );
    let output_path = write_temp_file("mux-fragmented-no-duration-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        audio_input,
        MuxMp4TrackSelector::Audio { occurrence: 1 },
    )])
    .with_output_layout(MuxOutputLayout::Fragmented);

    let error = mux_to_path(&request, &output_path).unwrap_err();

    assert_eq!(
        error.to_string(),
        "invalid mux layout `fragmented`: fragmented output requires exactly one of `--segment_duration` or `--fragment_duration`"
    );
    assert!(matches!(
        error,
        MuxError::InvalidOutputLayout {
            layout: "fragmented",
            ..
        }
    ));
}

#[test]
fn mux_to_path_rejects_fragmented_multi_track_jobs() {
    let audio_input = build_audio_input_file(
        "mux-fragmented-multi-audio-input",
        fourcc("dash"),
        &[b"aud"],
    );
    let video_input = build_video_input_file(
        "mux-fragmented-multi-video-input",
        fourcc("isom"),
        &[b"video"],
    );
    let output_path = write_temp_file("mux-fragmented-multi-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::mp4(audio_input, MuxMp4TrackSelector::Audio { occurrence: 1 }),
        MuxTrackSpec::mp4(video_input, MuxMp4TrackSelector::Video),
    ])
    .with_output_layout(MuxOutputLayout::Fragmented)
    .with_duration_mode(MuxDurationMode::Fragment { seconds: 0.25 });

    let error = mux_to_path(&request, &output_path).unwrap_err();

    assert_eq!(
        error.to_string(),
        "invalid mux layout `fragmented`: the current fragmented mux follow-on only supports single-track jobs"
    );
    assert!(matches!(
        error,
        MuxError::InvalidOutputLayout {
            layout: "fragmented",
            ..
        }
    ));
}

#[test]
fn mux_to_path_writes_fragmented_single_track_output() {
    let audio_input = build_audio_input_file(
        "mux-fragment-source",
        fourcc("isom"),
        &[b"one", b"two", b"three"],
    );
    let output_path = write_temp_file("mux-fragment-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        audio_input,
        MuxMp4TrackSelector::Audio { occurrence: 1 },
    )])
    .with_output_layout(MuxOutputLayout::Fragmented)
    .with_duration_mode(MuxDurationMode::Fragment { seconds: 0.015 });

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        root_boxes.iter().map(BoxInfo::box_type).collect::<Vec<_>>(),
        vec![
            fourcc("ftyp"),
            fourcc("moov"),
            fourcc("sidx"),
            fourcc("moof"),
            fourcc("mdat"),
            fourcc("moof"),
            fourcc("mdat"),
        ]
    );

    let ftyp_boxes = extract_boxes::<Ftyp>(&output_bytes, BoxPath::from([fourcc("ftyp")]));
    assert_eq!(ftyp_boxes.len(), 1);
    assert_eq!(ftyp_boxes[0].major_brand, fourcc("mp41"));
    assert!(ftyp_boxes[0].compatible_brands.contains(&fourcc("dash")));
    assert!(ftyp_boxes[0].compatible_brands.contains(&fourcc("cmfc")));

    let mvhd_boxes = extract_boxes::<Mvhd>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("mvhd")]),
    );
    assert_eq!(mvhd_boxes.len(), 1);
    assert_eq!(mvhd_boxes[0].duration_v0, 0);

    let tkhd_boxes = extract_boxes::<Tkhd>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("tkhd")]),
    );
    assert_eq!(tkhd_boxes.len(), 1);
    assert_eq!(tkhd_boxes[0].duration_v0, 0);

    let mdhd_boxes = extract_boxes::<Mdhd>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].duration_v0, 0);

    let mvex_boxes = extract_boxes::<Mvex>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("mvex")]),
    );
    assert_eq!(mvex_boxes.len(), 1);
    let mehd_boxes = extract_boxes::<Mehd>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("mvex"), fourcc("mehd")]),
    );
    assert_eq!(mehd_boxes.len(), 1);
    assert_eq!(mehd_boxes[0].fragment_duration_v0, 30);
    let trex_boxes = extract_boxes::<Trex>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("mvex"), fourcc("trex")]),
    );
    assert_eq!(trex_boxes.len(), 1);
    assert_eq!(trex_boxes[0].default_sample_duration, 10);

    let edts_boxes = extract_boxes::<Edts>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("edts")]),
    );
    assert!(edts_boxes.is_empty());
    let elst_boxes = extract_boxes::<Elst>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("edts"),
            fourcc("elst"),
        ]),
    );
    assert!(elst_boxes.is_empty());

    let meta_boxes = extract_boxes::<Meta>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("meta")]),
    );
    assert_eq!(meta_boxes.len(), 1);
    let id32_boxes = extract_boxes::<Id32>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("meta"), fourcc("ID32")]),
    );
    assert_eq!(id32_boxes.len(), 1);
    assert!(!id32_boxes[0].id3v2_data.is_empty());

    let sidx_boxes = extract_boxes::<Sidx>(&output_bytes, BoxPath::from([fourcc("sidx")]));
    assert_eq!(sidx_boxes.len(), 1);
    assert_eq!(sidx_boxes[0].reference_count, 1);
    assert_eq!(sidx_boxes[0].references.len(), 1);

    let tfdt_boxes = extract_boxes::<Tfdt>(
        &output_bytes,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
    );
    assert_eq!(tfdt_boxes.len(), 2);
    assert_eq!(tfdt_boxes[0].base_media_decode_time_v0, 0);
    assert_eq!(tfdt_boxes[1].base_media_decode_time_v0, 20);

    let tfhd_boxes = extract_boxes::<Tfhd>(
        &output_bytes,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfhd")]),
    );
    assert_eq!(tfhd_boxes.len(), 2);

    let trun_boxes = extract_boxes::<Trun>(
        &output_bytes,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("trun")]),
    );
    assert_eq!(trun_boxes.len(), 2);
    assert_eq!(trun_boxes[0].sample_count, 2);
    assert_eq!(trun_boxes[1].sample_count, 1);
}

#[test]
fn mux_to_path_fragmented_segment_mode_honors_imported_edit_media_time() {
    let samples = std::iter::repeat_n(
        TestMuxSample {
            bytes: b"aaaa",
            duration: 1_024,
            composition_time_offset: 0,
            is_sync_sample: true,
        },
        120,
    )
    .collect::<Vec<_>>();
    let input = build_imported_track_input_file_with_edit_media_time(
        "mux-fragment-segment-edit-shift",
        &MuxFileConfig::new(44_100)
            .with_major_brand(fourcc("isom"))
            .with_compatible_brand(fourcc("mp42")),
        &MuxTrackConfig::new_audio(1, 44_100, audio_sample_entry_box()),
        121_856,
        1_024,
        &samples,
    );
    let output_path = write_temp_file("mux-fragment-segment-edit-shift-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        input,
        MuxMp4TrackSelector::Audio { occurrence: 1 },
    )])
    .with_output_layout(MuxOutputLayout::Fragmented)
    .with_duration_mode(MuxDurationMode::Segment { seconds: 1.0 });

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let trun_boxes = extract_boxes::<Trun>(
        &output_bytes,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("trun")]),
    );
    let tfdt_boxes = extract_boxes::<Tfdt>(
        &output_bytes,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
    );
    assert_eq!(
        trun_boxes
            .iter()
            .map(|trun| trun.sample_count)
            .collect::<Vec<_>>(),
        vec![45, 43, 32]
    );
    assert_eq!(
        tfdt_boxes
            .iter()
            .map(|tfdt| tfdt.base_media_decode_time())
            .collect::<Vec<_>>(),
        vec![0, 46_080, 90_112]
    );
}

#[test]
fn mux_to_path_fragmented_imported_alac_uses_dominant_trex_duration() {
    let input = build_imported_track_input_file(
        "mux-fragment-imported-alac",
        &MuxFileConfig::new(44_100)
            .with_major_brand(fourcc("isom"))
            .with_compatible_brand(fourcc("mp42")),
        &MuxTrackConfig::new_audio(
            1,
            44_100,
            audio_sample_entry_box_with_children(
                "alac",
                &[
                    encode_raw_box(fourcc("alac"), &[0; 20]),
                    encode_supported_box(&mp4forge::boxes::iso14496_12::Btrt::default(), &[]),
                ]
                .concat(),
            ),
        ),
        10_240,
        &[
            TestMuxSample {
                bytes: b"one",
                duration: 4_096,
                composition_time_offset: 0,
                is_sync_sample: true,
            },
            TestMuxSample {
                bytes: b"two",
                duration: 4_096,
                composition_time_offset: 0,
                is_sync_sample: true,
            },
            TestMuxSample {
                bytes: b"tri",
                duration: 2_048,
                composition_time_offset: 0,
                is_sync_sample: true,
            },
        ],
    );
    let output_path = write_temp_file("mux-fragment-imported-alac-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        input,
        MuxMp4TrackSelector::Audio { occurrence: 1 },
    )])
    .with_output_layout(MuxOutputLayout::Fragmented)
    .with_duration_mode(MuxDurationMode::Fragment { seconds: 10.0 });

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let trex_boxes = extract_boxes::<Trex>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("mvex"), fourcc("trex")]),
    );
    let sample_entry_boxes = extract_box_bytes(
        &mut Cursor::new(&output_bytes),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("alac"),
        ]),
    )
    .unwrap();
    assert_eq!(trex_boxes[0].default_sample_duration, 4_096);
    assert_eq!(sample_entry_boxes.len(), 1);
    assert_eq!(sample_entry_boxes[0].len(), 64);
}

#[test]
fn mux_to_path_fragmented_segment_mode_aligns_video_boundaries_to_sync_samples() {
    let samples = (0..82)
        .map(|index| TestMuxSample {
            bytes: b"vfrm",
            duration: 1_001,
            composition_time_offset: if matches!(index, 0 | 30 | 60) {
                2_002
            } else if index % 2 == 1 {
                3_003
            } else {
                1_001
            },
            is_sync_sample: matches!(index, 0 | 30 | 60),
        })
        .collect::<Vec<_>>();
    let input = build_imported_track_input_file_with_edit_media_time(
        "mux-fragment-segment-video-sync-boundaries",
        &MuxFileConfig::new(30_000)
            .with_major_brand(fourcc("isom"))
            .with_compatible_brand(fourcc("mp42")),
        &MuxTrackConfig::new_video(
            1,
            30_000,
            640,
            360,
            video_sample_entry_box_with_type("avc1"),
        ),
        82_082,
        2_002,
        &samples,
    );
    let output_path = write_temp_file("mux-fragment-segment-video-sync-boundaries-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(input, MuxMp4TrackSelector::Video)])
        .with_output_layout(MuxOutputLayout::Fragmented)
        .with_duration_mode(MuxDurationMode::Segment { seconds: 1.0 });

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let trun_boxes = extract_boxes::<Trun>(
        &output_bytes,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("trun")]),
    );
    let tfdt_boxes = extract_boxes::<Tfdt>(
        &output_bytes,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
    );
    assert_eq!(
        trun_boxes
            .iter()
            .map(|trun| trun.sample_count)
            .collect::<Vec<_>>(),
        vec![30, 30, 22]
    );
    assert_eq!(
        tfdt_boxes
            .iter()
            .map(|tfdt| tfdt.base_media_decode_time())
            .collect::<Vec<_>>(),
        vec![0, 30_030, 60_060]
    );
}

#[test]
fn mux_to_path_fragmented_imported_dtsx_preserves_udts_child_boxes() {
    let input = build_imported_track_input_file(
        "mux-fragment-imported-dtsx",
        &MuxFileConfig::new(48_000)
            .with_major_brand(fourcc("isom"))
            .with_compatible_brand(fourcc("mp42")),
        &MuxTrackConfig::new_audio(
            1,
            48_000,
            audio_sample_entry_box_with_children("dtsx", &encode_raw_box(fourcc("udts"), &[0; 8])),
        ),
        3_072,
        &[
            TestMuxSample {
                bytes: b"dtsx",
                duration: 1_024,
                composition_time_offset: 0,
                is_sync_sample: true,
            },
            TestMuxSample {
                bytes: b"more",
                duration: 1_024,
                composition_time_offset: 0,
                is_sync_sample: true,
            },
            TestMuxSample {
                bytes: b"data",
                duration: 1_024,
                composition_time_offset: 0,
                is_sync_sample: true,
            },
        ],
    );
    let output_path = write_temp_file("mux-fragment-imported-dtsx-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        input,
        MuxMp4TrackSelector::Audio { occurrence: 1 },
    )])
    .with_output_layout(MuxOutputLayout::Fragmented)
    .with_duration_mode(MuxDurationMode::Fragment { seconds: 10.0 });

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let sample_entry_boxes = extract_box_bytes(
        &mut Cursor::new(&output_bytes),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("dtsx"),
        ]),
    )
    .unwrap();
    assert_eq!(sample_entry_boxes.len(), 1);
    assert_eq!(sample_entry_boxes[0].len(), 52);
    assert!(
        sample_entry_boxes[0]
            .windows(4)
            .any(|bytes| bytes == b"udts")
    );
    assert!(
        !sample_entry_boxes[0]
            .windows(4)
            .any(|bytes| bytes == b"btrt")
    );
}

#[test]
fn mux_to_path_imports_mp4_text_track_selectors() {
    let text_input = build_wvtt_input_file("mux-text-selector-input", fourcc("dash"), &[b"wvtt"]);
    let output_path = write_temp_file("mux-text-selector-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        text_input,
        MuxMp4TrackSelector::Text { occurrence: 1 },
    )]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"wvtt");

    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].handler_type, fourcc("text"));

    let nmhd_boxes = extract_boxes::<Nmhd>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("nmhd"),
        ]),
    );
    assert_eq!(nmhd_boxes.len(), 1);
}

#[test]
fn mux_to_path_imports_mp4_text_occurrence_selectors() {
    let text_input = build_mixed_text_input_file("mux-text-occurrence-input", fourcc("isom"));
    let output_path = write_temp_file("mux-text-occurrence-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        text_input,
        MuxMp4TrackSelector::Text { occurrence: 2 },
    )]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"stpp");

    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].handler_type, fourcc("subt"));

    let sthd_boxes = extract_boxes::<Sthd>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("sthd"),
        ]),
    );
    assert_eq!(sthd_boxes.len(), 1);
}

#[test]
fn mux_to_path_imports_mp4_track_id_selectors_for_text_tracks() {
    let text_input = build_mixed_text_input_file("mux-text-trackid-input", fourcc("mp42"));
    let output_path = write_temp_file("mux-text-trackid-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        text_input,
        MuxMp4TrackSelector::TrackId { track_id: 2 },
    )]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"stpp");
}

#[test]
fn mux_to_path_preserves_language_and_handler_names_in_mixed_subtitle_jobs() {
    let video_input = build_video_input_file_with_metadata(
        "mux-mixed-video-input",
        fourcc("isom"),
        "avc1",
        *b"und",
        "PrimaryVideoHandler",
        &[b"video"],
    );
    let audio_input = build_audio_input_file_with_metadata(
        "mux-mixed-audio-input",
        fourcc("dash"),
        "mp4a",
        *b"eng",
        "EnglishAudioHandler",
        &[b"aud"],
    );
    let text_input = build_mixed_text_input_file("mux-mixed-text-input", fourcc("mp42"));
    let output_path = write_temp_file("mux-mixed-subtitle-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::mp4(video_input, MuxMp4TrackSelector::Video),
        MuxTrackSpec::mp4(audio_input, MuxMp4TrackSelector::Audio { occurrence: 1 }),
        MuxTrackSpec::mp4(
            text_input.clone(),
            MuxMp4TrackSelector::Text { occurrence: 1 },
        ),
        MuxTrackSpec::mp4(text_input, MuxMp4TrackSelector::Text { occurrence: 2 }),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        b"videoaudwvttstpp"
    );

    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );
    assert_eq!(hdlr_boxes.len(), 4);
    assert_eq!(
        hdlr_boxes
            .iter()
            .map(|box_value| box_value.handler_type)
            .collect::<Vec<_>>(),
        vec![
            fourcc("vide"),
            fourcc("soun"),
            fourcc("text"),
            fourcc("subt"),
        ]
    );
    assert_eq!(
        hdlr_boxes
            .iter()
            .map(|box_value| box_value.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "PrimaryVideoHandler",
            "EnglishAudioHandler",
            "EnglishCaptionHandler",
            "FrenchSubtitleHandler",
        ]
    );

    let mdhd_boxes = extract_boxes::<Mdhd>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    assert_eq!(mdhd_boxes.len(), 4);
    assert_eq!(
        mdhd_boxes
            .iter()
            .map(|box_value| decode_mdhd_language(box_value.language))
            .collect::<Vec<_>>(),
        vec![*b"und", *b"eng", *b"eng", *b"fra"]
    );
}

#[test]
fn mux_to_path_imports_mp4_broader_video_codec_track_families() {
    for sample_entry_type in ["avc1", "hvc1", "av01", "vp08", "vp09", "dvh1", "dvhe"] {
        let input = build_video_input_file_with_type(
            &format!("mux-video-family-{sample_entry_type}"),
            fourcc("isom"),
            sample_entry_type,
            &[sample_entry_type.as_bytes()],
        );
        let output_path =
            write_temp_file(&format!("mux-video-family-{sample_entry_type}-out"), &[]);
        let request = MuxRequest::new(vec![MuxTrackSpec::mp4(input, MuxMp4TrackSelector::Video)]);

        mux_to_path(&request, &output_path).unwrap();

        let output_bytes = fs::read(output_path).unwrap();
        let root_boxes = read_root_boxes(&output_bytes);
        assert_eq!(
            mdat_payload(&output_bytes, root_boxes[2]),
            sample_entry_type.as_bytes()
        );

        let entries = extract_boxes::<VisualSampleEntry>(
            &output_bytes,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stsd"),
                fourcc(sample_entry_type),
            ]),
        );
        assert_eq!(entries.len(), 1, "{sample_entry_type}");
        assert_eq!(entries[0].sample_entry.box_type, fourcc(sample_entry_type));
        assert_eq!(entries[0].width, 640, "{sample_entry_type}");
        assert_eq!(entries[0].height, 360, "{sample_entry_type}");
    }
}

#[test]
fn mux_to_path_imports_mp4_broader_audio_codec_track_families() {
    for sample_entry_type in [
        "mp4a", "ac-3", "ec-3", "ac-4", "alac", "dtsc", "dtse", "dtsh", "dtsl", "dtsm", "dtsx",
        "fLaC", "Opus", "iamf", "mha1", "mhm1",
    ] {
        let input = build_audio_input_file_with_type(
            &format!("mux-audio-family-{sample_entry_type}"),
            fourcc("isom"),
            sample_entry_type,
            &[sample_entry_type.as_bytes()],
        );
        let output_path =
            write_temp_file(&format!("mux-audio-family-{sample_entry_type}-out"), &[]);
        let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
            input,
            MuxMp4TrackSelector::Audio { occurrence: 1 },
        )]);

        mux_to_path(&request, &output_path).unwrap();

        let output_bytes = fs::read(output_path).unwrap();
        let root_boxes = read_root_boxes(&output_bytes);
        assert_eq!(
            mdat_payload(&output_bytes, root_boxes[2]),
            sample_entry_type.as_bytes()
        );

        let entries = extract_boxes::<AudioSampleEntry>(
            &output_bytes,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stsd"),
                fourcc(sample_entry_type),
            ]),
        );
        assert_eq!(entries.len(), 1, "{sample_entry_type}");
        assert_eq!(entries[0].sample_entry.box_type, fourcc(sample_entry_type));
        assert_eq!(entries[0].channel_count, 2, "{sample_entry_type}");
    }
}

#[test]
fn mux_to_path_imports_raw_aac_adts_inputs() {
    let aac_input = write_test_adts_file("mux-raw-aac-input", &[b"abc", b"defg"]);
    let output_path = write_temp_file("mux-raw-aac-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::raw(MuxRawCodec::Aac, aac_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        root_boxes.iter().map(BoxInfo::box_type).collect::<Vec<_>>(),
        vec![fourcc("ftyp"), fourcc("moov"), fourcc("mdat")]
    );
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"abcdefg");
}

#[test]
fn mux_to_path_imports_raw_h265_annexb_inputs_with_explicit_layout_parameters() {
    let h265_input = write_test_h265_annexb_file("mux-raw-h265-input", &[b"hevc"]);
    let output_path = write_temp_file("mux-raw-h265-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::from_str(&format!(
            "h265:{}#width=640,height=360,sample_entry=hvc1",
            h265_input.display()
        ))
        .unwrap(),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        root_boxes.iter().map(BoxInfo::box_type).collect::<Vec<_>>(),
        vec![fourcc("ftyp"), fourcc("moov"), fourcc("mdat")]
    );
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        &[0, 0, 0, 6, 0x26, 0x01, b'h', b'e', b'v', b'c']
    );

    let hvc1 = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("hvc1"),
        ]),
    );
    assert_eq!(hvc1.len(), 1);
    assert_eq!(hvc1[0].sample_entry.box_type, fourcc("hvc1"));
    assert_eq!(hvc1[0].width, 640);
    assert_eq!(hvc1[0].height, 360);
}

#[test]
fn mux_to_path_imports_raw_h265_annexb_inputs_with_dolby_vision_sample_entries() {
    let h265_input = write_test_h265_annexb_file("mux-raw-dvh1-input", &[b"dvh1"]);
    let output_path = write_temp_file("mux-raw-dvh1-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::from_str(&format!(
            "h265:{}#width=640,height=360,sample_entry=dvh1",
            h265_input.display()
        ))
        .unwrap(),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        &[0, 0, 0, 6, 0x26, 0x01, b'd', b'v', b'h', b'1']
    );

    let dvh1 = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("dvh1"),
        ]),
    );
    assert_eq!(dvh1.len(), 1);
    assert_eq!(dvh1[0].sample_entry.box_type, fourcc("dvh1"));
    assert_eq!(dvh1[0].width, 640);
    assert_eq!(dvh1[0].height, 360);
}

#[test]
fn mux_to_path_imports_parameterized_raw_video_codec_inputs() {
    for (codec, sample_entry_type, prefix) in [
        (MuxRawCodec::Av1, "av01", "mux-raw-av1"),
        (MuxRawCodec::Vp8, "vp08", "mux-raw-vp8"),
        (MuxRawCodec::Vp9, "vp09", "mux-raw-vp9"),
    ] {
        let input = write_temp_file(prefix, sample_entry_type.as_bytes());
        let output_path = write_temp_file(&format!("{prefix}-output"), &[]);
        let request = MuxRequest::new(vec![
            MuxTrackSpec::from_str(&format!(
                "{}:{}#width=640,height=360,timescale=1000,sample_duration=1000",
                codec.prefix(),
                input.display()
            ))
            .unwrap(),
        ]);

        mux_to_path(&request, &output_path).unwrap();

        let output_bytes = fs::read(output_path).unwrap();
        let root_boxes = read_root_boxes(&output_bytes);
        assert_eq!(
            mdat_payload(&output_bytes, root_boxes[2]),
            sample_entry_type.as_bytes(),
            "{sample_entry_type}"
        );

        let entries = extract_boxes::<VisualSampleEntry>(
            &output_bytes,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stsd"),
                fourcc(sample_entry_type),
            ]),
        );
        assert_eq!(entries.len(), 1, "{sample_entry_type}");
        assert_eq!(entries[0].sample_entry.box_type, fourcc(sample_entry_type));
        assert_eq!(entries[0].width, 640, "{sample_entry_type}");
        assert_eq!(entries[0].height, 360, "{sample_entry_type}");
    }
}

#[test]
fn mux_to_path_imports_raw_mp3_inputs() {
    let mp3_input = write_test_mp3_file("mux-raw-mp3-input", &[b"abc", b"defg"]);
    let expected = fs::read(&mp3_input).unwrap();
    let output_path = write_temp_file("mux-raw-mp3-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::raw(MuxRawCodec::Mp3, mp3_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        expected.as_slice()
    );
}

#[test]
fn mux_to_path_imports_id3_prefixed_raw_mp3_inputs() {
    let mp3_input = write_test_mp3_file_with_leading_id3_tag(
        "mux-raw-mp3-id3-input",
        b"test-id3",
        &[b"abc", b"defg"],
    );
    let expected = fs::read(&mp3_input).unwrap();
    let output_path = write_temp_file("mux-raw-mp3-id3-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::raw(MuxRawCodec::Mp3, mp3_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), &expected[18..]);
}

#[test]
fn mux_to_path_ignores_trailing_id3v1_metadata_after_raw_mp3_frames() {
    let frame_file = write_test_mp3_file("mux-raw-mp3-id3v1-frames", &[b"abc", b"defg"]);
    let expected = fs::read(&frame_file).unwrap();
    let mut bytes = expected.clone();
    let mut tag = [0_u8; 128];
    tag[..3].copy_from_slice(b"TAG");
    tag[3..22].copy_from_slice(b"sample for id3 test");
    bytes.extend_from_slice(&tag);
    let mp3_input = write_temp_file("mux-raw-mp3-id3v1-input", &bytes);
    let output_path = write_temp_file("mux-raw-mp3-id3v1-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::raw(MuxRawCodec::Mp3, mp3_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        expected.as_slice()
    );
}

#[test]
fn mux_to_path_imports_raw_ac3_inputs() {
    let ac3_input = write_test_ac3_file("mux-raw-ac3-input", &[b"ac3"]);
    let expected = fs::read(&ac3_input).unwrap();
    let output_path = write_temp_file("mux-raw-ac3-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::raw(MuxRawCodec::Ac3, ac3_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        expected.as_slice()
    );

    let ac3_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ac-3"),
        ]),
    );
    assert_eq!(ac3_entries.len(), 1);
    assert_eq!(ac3_entries[0].sample_entry.box_type, fourcc("ac-3"));
}

#[test]
fn mux_to_path_imports_raw_ac3_44100hz_inputs() {
    let ac3_input = write_test_ac3_44100_file("mux-raw-ac3-44100-input", &[b"ac3"]);
    let expected = fs::read(&ac3_input).unwrap();
    let output_path = write_temp_file("mux-raw-ac3-44100-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::raw(MuxRawCodec::Ac3, ac3_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        expected.as_slice()
    );

    let mdhd_boxes = extract_boxes::<Mdhd>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 44_100);
}

#[test]
fn mux_to_path_imports_raw_eac3_inputs() {
    let eac3_input = write_test_eac3_file("mux-raw-eac3-input", &[b"ec3"]);
    let expected = fs::read(&eac3_input).unwrap();
    let output_path = write_temp_file("mux-raw-eac3-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::raw(MuxRawCodec::Eac3, eac3_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        expected.as_slice()
    );

    let eac3_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ec-3"),
        ]),
    );
    assert_eq!(eac3_entries.len(), 1);
    assert_eq!(eac3_entries[0].sample_entry.box_type, fourcc("ec-3"));
}

#[test]
fn mux_to_path_imports_raw_ac4_inputs_with_explicit_audio_parameters() {
    let ac4_input = write_test_ac4_file("mux-raw-ac4-input", &[b"ac4"]);
    let expected = fs::read(&ac4_input).unwrap();
    let output_path = write_temp_file("mux-raw-ac4-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::from_str(&format!(
            "ac4:{}#sample_rate=48000,channel_count=2,sample_duration=1024,dac4=00112233",
            ac4_input.display()
        ))
        .unwrap(),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        expected.as_slice()
    );

    let ac4_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ac-4"),
        ]),
    );
    assert_eq!(ac4_entries.len(), 1);
    assert_eq!(ac4_entries[0].sample_entry.box_type, fourcc("ac-4"));
}

#[test]
fn mux_to_path_imports_raw_ac4_inputs_with_legacy_size_field_layout() {
    let ac4_input = write_temp_file(
        "mux-raw-ac4-legacy-input",
        &[0xAC, 0x40, 0x00, 0x05, b'a', b'c', b'4'],
    );
    let output_path = write_temp_file("mux-raw-ac4-legacy-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::from_str(&format!(
            "ac4:{}#sample_rate=48000,channel_count=2,sample_duration=1024,dac4=00112233",
            ac4_input.display()
        ))
        .unwrap(),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        &[0xAC, 0x40, 0x00, 0x05, b'a', b'c', b'4']
    );
}

#[test]
fn mux_to_path_imports_parameterized_raw_audio_codec_inputs() {
    for (codec, sample_entry_type, prefix) in [
        (MuxRawCodec::Alac, "alac", "mux-raw-alac"),
        (MuxRawCodec::Dtsc, "dtsc", "mux-raw-dtsc"),
        (MuxRawCodec::Dtse, "dtse", "mux-raw-dtse"),
        (MuxRawCodec::Dtsh, "dtsh", "mux-raw-dtsh"),
        (MuxRawCodec::Dtsl, "dtsl", "mux-raw-dtsl"),
        (MuxRawCodec::Dtsm, "dtsm", "mux-raw-dtsm"),
        (MuxRawCodec::Dtsx, "dtsx", "mux-raw-dtsx"),
        (MuxRawCodec::Flac, "fLaC", "mux-raw-flac"),
        (MuxRawCodec::Opus, "Opus", "mux-raw-opus"),
        (MuxRawCodec::Iamf, "iamf", "mux-raw-iamf"),
        (MuxRawCodec::Mha1, "mha1", "mux-raw-mha1"),
        (MuxRawCodec::Mhm1, "mhm1", "mux-raw-mhm1"),
    ] {
        let input = write_temp_file(prefix, sample_entry_type.as_bytes());
        let output_path = write_temp_file(&format!("{prefix}-output"), &[]);
        let request = MuxRequest::new(vec![
            MuxTrackSpec::from_str(&format!(
                "{}:{}#sample_rate=48000,channel_count=2,sample_duration=1024",
                codec.prefix(),
                input.display()
            ))
            .unwrap(),
        ]);

        mux_to_path(&request, &output_path).unwrap();

        let output_bytes = fs::read(output_path).unwrap();
        let root_boxes = read_root_boxes(&output_bytes);
        assert_eq!(
            mdat_payload(&output_bytes, root_boxes[2]),
            sample_entry_type.as_bytes(),
            "{sample_entry_type}"
        );

        let entries = extract_boxes::<AudioSampleEntry>(
            &output_bytes,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stsd"),
                fourcc(sample_entry_type),
            ]),
        );
        assert_eq!(entries.len(), 1, "{sample_entry_type}");
        assert_eq!(entries[0].sample_entry.box_type, fourcc(sample_entry_type));
        assert_eq!(entries[0].channel_count, 2, "{sample_entry_type}");
    }
}

#[test]
fn fragmented_parameterized_dts_outputs_keep_ddts_child_boxes_walkable() {
    let input = write_temp_file("mux-fragmented-dtsc-ddts-input", b"dtsc");
    let output_path = write_temp_file("mux-fragmented-dtsc-ddts-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::from_str(&format!(
            "dtsc:{}#sample_rate=48000,channel_count=2,sample_duration=1024",
            input.display()
        ))
        .unwrap(),
    ])
    .with_output_layout(MuxOutputLayout::Fragmented)
    .with_duration_mode(MuxDurationMode::Fragment { seconds: 10.0 });

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let mut ddts_paths = Vec::new();
    walk_structure(&mut Cursor::new(&output_bytes), |handle| {
        if handle.info().box_type() == fourcc("ddts") {
            ddts_paths.push(handle.path().to_string());
        }
        Ok(WalkControl::Descend)
    })
    .unwrap();

    assert_eq!(ddts_paths, vec!["moov/trak/mdia/minf/stbl/stsd/dtsc/ddts"]);
}

#[test]
fn mux_to_path_reimports_hevc_outputs_with_decoder_configuration() {
    let h265_input = write_test_h265_annexb_file("mux-hevc-reimport-source", &[b"hevc"]);
    let intermediate = write_temp_file("mux-hevc-reimport-intermediate", &[]);
    let final_output = write_temp_file("mux-hevc-reimport-output", &[]);
    let first_request = MuxRequest::new(vec![
        MuxTrackSpec::from_str(&format!(
            "h265:{}#width=640,height=360,sample_entry=hev1",
            h265_input.display()
        ))
        .unwrap(),
    ]);
    let second_request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        intermediate.clone(),
        MuxMp4TrackSelector::Video,
    )]);

    mux_to_path(&first_request, &intermediate).unwrap();
    mux_to_path(&second_request, &final_output).unwrap();

    let output_bytes = fs::read(final_output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        &[0, 0, 0, 6, 0x26, 0x01, b'h', b'e', b'v', b'c']
    );
}

#[test]
fn mux_to_path_accepts_imported_init_only_tracks_with_empty_sample_tables() {
    let input = build_imported_track_input_file(
        "mux-empty-av1-init-input",
        &MuxFileConfig::new(1_000).with_major_brand(fourcc("dash")),
        &MuxTrackConfig::new_video(1, 1_000, 640, 360, video_sample_entry_box_with_type("av01")),
        0,
        &[],
    );
    let output_path = write_temp_file("mux-empty-av1-init-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(input, MuxMp4TrackSelector::Video)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let stts_boxes = extract_boxes::<Stts>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stts"),
        ]),
    );
    let stsc_boxes = extract_boxes::<Stsc>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsc"),
        ]),
    );
    let stsz_boxes = extract_boxes::<Stsz>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsz"),
        ]),
    );
    let co64_boxes = extract_boxes::<Co64>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("co64"),
        ]),
    );
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entry_count, 0);
    assert_eq!(stsc_boxes.len(), 1);
    assert_eq!(stsc_boxes[0].entry_count, 0);
    assert_eq!(stsz_boxes.len(), 1);
    assert_eq!(stsz_boxes[0].sample_count, 0);
    assert_eq!(co64_boxes.len(), 1);
    assert_eq!(co64_boxes[0].entry_count, 0);
}

#[test]
fn mux_to_path_promotes_movie_timescale_for_imported_tracks_that_need_exact_scaling() {
    let video_input = build_imported_track_input_file(
        "mux-promoted-timescale-video-input",
        &MuxFileConfig::new(1_000).with_major_brand(fourcc("isom")),
        &MuxTrackConfig::new_video(
            1,
            30_000,
            640,
            360,
            video_sample_entry_box_with_type("avc1"),
        ),
        33,
        &[TestMuxSample {
            bytes: b"video",
            duration: 1_001,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    );
    let audio_input = build_imported_track_input_file(
        "mux-promoted-timescale-audio-input",
        &MuxFileConfig::new(1_000).with_major_brand(fourcc("isom")),
        &MuxTrackConfig::new_audio(1, 48_000, audio_sample_entry_box_with_type("dtsx")),
        21,
        &[TestMuxSample {
            bytes: b"dtsx",
            duration: 1_024,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    );

    for (input, selector, expected_timescale) in [
        (video_input, MuxMp4TrackSelector::Video, 30_000_u32),
        (
            audio_input,
            MuxMp4TrackSelector::Audio { occurrence: 1 },
            48_000_u32,
        ),
    ] {
        let output_path = write_temp_file(
            &format!("mux-promoted-timescale-output-{expected_timescale}"),
            &[],
        );
        let request = MuxRequest::new(vec![MuxTrackSpec::mp4(input, selector)]);

        mux_to_path(&request, &output_path).unwrap();

        let output_bytes = fs::read(output_path).unwrap();
        let mvhd_boxes = extract_boxes::<Mvhd>(
            &output_bytes,
            BoxPath::from([fourcc("moov"), fourcc("mvhd")]),
        );
        let mdhd_boxes = extract_boxes::<Mdhd>(
            &output_bytes,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("mdhd"),
            ]),
        );
        let stts_boxes = extract_boxes::<Stts>(
            &output_bytes,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stts"),
            ]),
        );
        assert_eq!(mvhd_boxes.len(), 1);
        assert_eq!(mvhd_boxes[0].timescale, expected_timescale);
        assert_eq!(mdhd_boxes.len(), 1);
        assert_eq!(mdhd_boxes[0].timescale, expected_timescale);
        assert_eq!(stts_boxes.len(), 1);
        assert_eq!(
            stts_boxes[0].entries[0].sample_delta,
            if expected_timescale == 30_000 {
                1_001
            } else {
                1_024
            }
        );
    }
}

#[test]
fn write_mp4_mux_builds_a_real_mp4_container() {
    let mut sources = [
        Cursor::new(b"AAAAhelloBBBBxy".to_vec()),
        Cursor::new(b"zzzzSYNCtail".to_vec()),
    ];
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 2, 10, 4, 13, 2),
            MuxStagedMediaItem::new(1, 1, 0, 5, 4, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(0, 2, 0, 4, 4, 5)
                .with_composition_time_offset(2)
                .with_sync_sample(true),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();
    let file_config = MuxFileConfig::new(1_000)
        .with_major_brand(fourcc("isom"))
        .with_compatible_brand(fourcc("mp42"));
    let track_configs = vec![
        MuxTrackConfig::new_audio(1, 1_000, audio_sample_entry_box()),
        MuxTrackConfig::new_video(2, 1_000, 640, 360, video_sample_entry_box()),
    ];

    let mut output = Cursor::new(Vec::new());
    write_mp4_mux(
        &mut sources,
        &mut output,
        &file_config,
        &track_configs,
        &plan,
    )
    .unwrap();

    let bytes = output.into_inner();
    let root_boxes = read_root_boxes(&bytes);
    assert_eq!(
        root_boxes.iter().map(BoxInfo::box_type).collect::<Vec<_>>(),
        vec![fourcc("ftyp"), fourcc("moov"), fourcc("mdat")]
    );
    assert_eq!(mdat_payload(&bytes, root_boxes[2]), b"SYNChelloxy");

    let tkhds = extract_boxes::<Tkhd>(
        &bytes,
        BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("tkhd")]),
    );
    assert_eq!(tkhds.len(), 2);
    assert_eq!(tkhds[0].track_id, 1);
    assert_eq!(tkhds[0].duration(), 5);
    assert_eq!(tkhds[0].volume, 0x0100);
    assert_eq!(tkhds[1].track_id, 2);
    assert_eq!(tkhds[1].duration(), 14);
    assert_eq!(tkhds[1].width, u32::from(640_u16) << 16);
    assert_eq!(tkhds[1].height, u32::from(360_u16) << 16);

    let mdhds = extract_boxes::<Mdhd>(
        &bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    assert_eq!(
        mdhds
            .iter()
            .map(|box_value| box_value.timescale)
            .collect::<Vec<_>>(),
        vec![1_000, 1_000]
    );
    assert_eq!(
        mdhds.iter().map(Mdhd::duration).collect::<Vec<_>>(),
        vec![5, 14]
    );

    let stts_boxes = extract_boxes::<Stts>(
        &bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stts"),
        ]),
    );
    assert_eq!(stts_boxes.len(), 2);
    assert_eq!(stts_boxes[0].entry_count, 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 1);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 5);
    assert_eq!(stts_boxes[1].entry_count, 1);
    assert_eq!(stts_boxes[1].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[1].entries[0].sample_delta, 4);

    let stsc_boxes = extract_boxes::<Stsc>(
        &bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsc"),
        ]),
    );
    assert_eq!(stsc_boxes.len(), 2);
    assert_eq!(stsc_boxes[0].entries[0].first_chunk, 1);
    assert_eq!(stsc_boxes[0].entries[0].samples_per_chunk, 1);
    assert_eq!(stsc_boxes[0].entries[0].sample_description_index, 1);
    assert_eq!(stsc_boxes[1].entries[0].samples_per_chunk, 1);

    let stsz_boxes = extract_boxes::<Stsz>(
        &bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsz"),
        ]),
    );
    assert_eq!(stsz_boxes.len(), 2);
    assert_eq!(stsz_boxes[0].sample_count, 1);
    assert_eq!(stsz_boxes[0].entry_size, vec![4]);
    assert_eq!(stsz_boxes[1].sample_count, 2);
    assert_eq!(stsz_boxes[1].entry_size, vec![5, 2]);

    let co64_boxes = extract_boxes::<Co64>(
        &bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("co64"),
        ]),
    );
    let mdat_data_start = root_boxes[2].offset() + root_boxes[2].header_size();
    assert_eq!(co64_boxes.len(), 2);
    assert_eq!(co64_boxes[0].chunk_offset, vec![mdat_data_start]);
    assert_eq!(
        co64_boxes[1].chunk_offset,
        vec![mdat_data_start + 4, mdat_data_start + 9]
    );

    let ctts_boxes = extract_boxes::<Ctts>(
        &bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("ctts"),
        ]),
    );
    assert_eq!(ctts_boxes.len(), 1);
    assert_eq!(ctts_boxes[0].entry_count, 2);
    assert_eq!(ctts_boxes[0].entries[0].sample_count, 1);
    assert_eq!(ctts_boxes[0].sample_offset(0), 2);
    assert_eq!(ctts_boxes[0].entries[1].sample_count, 1);
    assert_eq!(ctts_boxes[0].sample_offset(1), 0);

    let stss_boxes = extract_boxes::<Stss>(
        &bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stss"),
        ]),
    );
    assert_eq!(stss_boxes.len(), 1);
    assert_eq!(stss_boxes[0].sample_number, vec![1]);
}

#[test]
fn write_mp4_mux_to_path_matches_in_memory_container_output() {
    let first_source = write_temp_file("mux-container-source-a", b"AAAAhelloBBBBxy");
    let second_source = write_temp_file("mux-container-source-b", b"zzzzSYNCtail");
    let output_path = write_temp_file("mux-container-output-sync", &[]);
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 2, 10, 4, 13, 2),
            MuxStagedMediaItem::new(1, 1, 0, 5, 4, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(0, 2, 0, 4, 4, 5)
                .with_composition_time_offset(2)
                .with_sync_sample(true),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();
    let file_config = MuxFileConfig::new(1_000);
    let track_configs = vec![
        MuxTrackConfig::new_audio(1, 1_000, audio_sample_entry_box()),
        MuxTrackConfig::new_video(2, 1_000, 640, 360, video_sample_entry_box()),
    ];

    let mut in_memory_sources = [
        Cursor::new(b"AAAAhelloBBBBxy".to_vec()),
        Cursor::new(b"zzzzSYNCtail".to_vec()),
    ];
    let mut expected_output = Cursor::new(Vec::new());
    write_mp4_mux(
        &mut in_memory_sources,
        &mut expected_output,
        &file_config,
        &track_configs,
        &plan,
    )
    .unwrap();
    write_mp4_mux_to_path(
        &[&first_source, &second_source],
        &output_path,
        &file_config,
        &track_configs,
        &plan,
    )
    .unwrap();

    assert_eq!(fs::read(output_path).unwrap(), expected_output.into_inner());
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_planned_payloads_async_matches_sync_file_output() {
    let first_source = write_temp_file("mux-source-async-a", b"HEADvideoTAIL");
    let second_source = write_temp_file("mux-source-async-b", b"PREMaudPOST");
    let sync_output = write_temp_file("mux-output-sync-file", &[]);
    let async_output = write_temp_file("mux-output-async-file", &[]);
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 2, 10, 4, 4, 5),
            MuxStagedMediaItem::new(1, 1, 0, 4, 4, 3),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    copy_planned_payloads_to_path(&[&first_source, &second_source], &sync_output, &plan).unwrap();
    copy_planned_payloads_to_path_async(&[&first_source, &second_source], &async_output, &plan)
        .await
        .unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_mp4_mux_to_path_async_matches_sync_container_output() {
    let first_source = write_temp_file("mux-container-async-source-a", b"AAAAhelloBBBBxy");
    let second_source = write_temp_file("mux-container-async-source-b", b"zzzzSYNCtail");
    let sync_output = write_temp_file("mux-container-sync-output", &[]);
    let async_output = write_temp_file("mux-container-async-output", &[]);
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 2, 10, 4, 13, 2),
            MuxStagedMediaItem::new(1, 1, 0, 5, 4, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(0, 2, 0, 4, 4, 5)
                .with_composition_time_offset(2)
                .with_sync_sample(true),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();
    let file_config = MuxFileConfig::new(1_000);
    let track_configs = vec![
        MuxTrackConfig::new_audio(1, 1_000, audio_sample_entry_box()),
        MuxTrackConfig::new_video(2, 1_000, 640, 360, video_sample_entry_box()),
    ];

    write_mp4_mux_to_path(
        &[&first_source, &second_source],
        &sync_output,
        &file_config,
        &track_configs,
        &plan,
    )
    .unwrap();
    write_mp4_mux_to_path_async(
        &[&first_source, &second_source],
        &async_output,
        &file_config,
        &track_configs,
        &plan,
    )
    .await
    .unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_track_spec_output() {
    let audio_input = write_temp_file("mux-async-audio-input", b"alac");
    let video_input = write_temp_file("mux-async-video-input", b"av01");
    let sync_output = write_temp_file("mux-async-sync-output", &[]);
    let async_output = write_temp_file("mux-async-async-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::from_str(&format!(
            "alac:{}#sample_rate=48000,channel_count=2,sample_duration=1024",
            audio_input.display()
        ))
        .unwrap(),
        MuxTrackSpec::from_str(&format!(
            "av1:{}#width=640,height=360,timescale=1000,sample_duration=1000",
            video_input.display()
        ))
        .unwrap(),
    ]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_transformed_raw_track_output() {
    let audio_input = write_test_adts_file("mux-async-adts-input", &[b"abc", b"defg"]);
    let video_input = write_test_h265_annexb_file("mux-async-h265-input", &[b"hevc"]);
    let sync_output = write_temp_file("mux-async-transformed-sync-output", &[]);
    let async_output = write_temp_file("mux-async-transformed-async-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::raw(MuxRawCodec::Aac, audio_input),
        MuxTrackSpec::from_str(&format!(
            "h265:{}#width=640,height=360,timescale=1000,sample_duration=1000",
            video_input.display()
        ))
        .unwrap(),
    ]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_raw_eac3_output() {
    let eac3_input = write_test_eac3_file("mux-async-eac3-input", &[b"ec3"]);
    let sync_output = write_temp_file("mux-async-eac3-sync-output", &[]);
    let async_output = write_temp_file("mux-async-eac3-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::raw(MuxRawCodec::Eac3, eac3_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_fragmented_output() {
    let audio_input = build_audio_input_file(
        "mux-async-fragmented-source",
        fourcc("isom"),
        &[b"one", b"two", b"three"],
    );
    let sync_output = write_temp_file("mux-async-fragmented-sync-output", &[]);
    let async_output = write_temp_file("mux-async-fragmented-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        audio_input,
        MuxMp4TrackSelector::Audio { occurrence: 1 },
    )])
    .with_output_layout(MuxOutputLayout::Fragmented)
    .with_duration_mode(MuxDurationMode::Fragment { seconds: 0.015 });

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_mixed_subtitle_output() {
    let video_input = build_video_input_file_with_metadata(
        "mux-async-mixed-video-input",
        fourcc("isom"),
        "avc1",
        *b"und",
        "PrimaryVideoHandler",
        &[b"video"],
    );
    let audio_input = build_audio_input_file_with_metadata(
        "mux-async-mixed-audio-input",
        fourcc("dash"),
        "mp4a",
        *b"eng",
        "EnglishAudioHandler",
        &[b"aud"],
    );
    let text_input = build_mixed_text_input_file("mux-async-mixed-text-input", fourcc("mp42"));
    let sync_output = write_temp_file("mux-async-mixed-sync-output", &[]);
    let async_output = write_temp_file("mux-async-mixed-async-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::mp4(video_input, MuxMp4TrackSelector::Video),
        MuxTrackSpec::mp4(audio_input, MuxMp4TrackSelector::Audio { occurrence: 1 }),
        MuxTrackSpec::mp4(
            text_input.clone(),
            MuxMp4TrackSelector::Text { occurrence: 1 },
        ),
        MuxTrackSpec::mp4(text_input, MuxMp4TrackSelector::Text { occurrence: 2 }),
    ]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_planned_payloads_async_supports_seekable_async_readers_and_writers() {
    let mut sources = [
        Cursor::new(b"AAAAhelloBBBBxy".to_vec()),
        Cursor::new(b"zzzzSYNCtail".to_vec()),
    ];
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 2, 10, 4, 13, 2),
            MuxStagedMediaItem::new(1, 1, 0, 5, 4, 4),
            MuxStagedMediaItem::new(0, 2, 0, 4, 4, 5),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    let mut output = Cursor::new(Vec::new());
    copy_planned_payloads_async(&mut sources, &mut output, &plan)
        .await
        .unwrap();

    assert_eq!(output.into_inner(), b"SYNChelloxy");
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_planned_payloads_async_progressive_supports_non_seekable_readers() {
    let (mut first_writer, first_source) = tokio::io::duplex(64);
    let (mut second_writer, second_source) = tokio::io::duplex(64);
    first_writer.write_all(b"AAAAhelloBBBBxy").await.unwrap();
    first_writer.shutdown().await.unwrap();
    second_writer.write_all(b"zzzzSYNCtail").await.unwrap();
    second_writer.shutdown().await.unwrap();

    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 1, 0, 4, 4, 5),
            MuxStagedMediaItem::new(1, 2, 5, 4, 4, 4),
            MuxStagedMediaItem::new(0, 1, 10, 4, 13, 2),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    let mut output = Cursor::new(Vec::new());
    let mut sources = [first_source, second_source];
    copy_planned_payloads_async_progressive(&mut sources, &mut output, &plan)
        .await
        .unwrap();

    assert_eq!(output.into_inner(), b"helloSYNCxy");
}

fn build_audio_input_file(
    prefix: &str,
    major_brand: mp4forge::FourCc,
    payloads: &[&[u8]],
) -> std::path::PathBuf {
    build_audio_input_file_with_type(prefix, major_brand, "mp4a", payloads)
}

fn build_audio_input_file_with_metadata(
    prefix: &str,
    major_brand: mp4forge::FourCc,
    sample_entry_type: &str,
    language: [u8; 3],
    handler_name: &str,
    payloads: &[&[u8]],
) -> std::path::PathBuf {
    let samples = payloads
        .iter()
        .copied()
        .map(|bytes| TestMuxSample {
            bytes,
            duration: 10,
            composition_time_offset: 0,
            is_sync_sample: true,
        })
        .collect::<Vec<_>>();
    write_single_track_mp4_input(
        prefix,
        &MuxFileConfig::new(1_000)
            .with_major_brand(major_brand)
            .with_compatible_brand(fourcc("mp42")),
        MuxTrackConfig::new_audio(
            1,
            1_000,
            audio_sample_entry_box_with_type(sample_entry_type),
        )
        .with_language(language)
        .with_handler_name(handler_name),
        &samples,
    )
}

fn build_imported_track_input_file(
    prefix: &str,
    file_config: &MuxFileConfig,
    track_config: &MuxTrackConfig,
    movie_duration: u32,
    samples: &[TestMuxSample<'_>],
) -> std::path::PathBuf {
    build_imported_track_input_file_with_edit_media_time(
        prefix,
        file_config,
        track_config,
        movie_duration,
        0,
        samples,
    )
}

fn build_imported_track_input_file_with_edit_media_time(
    prefix: &str,
    file_config: &MuxFileConfig,
    track_config: &MuxTrackConfig,
    movie_duration: u32,
    edit_media_time: u32,
    samples: &[TestMuxSample<'_>],
) -> std::path::PathBuf {
    let ftyp = Ftyp {
        major_brand: file_config.major_brand(),
        minor_version: file_config.minor_version(),
        compatible_brands: file_config.compatible_brands().to_vec(),
    };
    let ftyp_bytes = encode_supported_box(&ftyp, &[]);

    let payload = samples
        .iter()
        .flat_map(|sample| sample.bytes)
        .copied()
        .collect::<Vec<_>>();
    let provisional_moov = build_imported_track_moov_bytes(
        file_config,
        track_config,
        movie_duration,
        edit_media_time,
        samples,
        &[],
    );
    let mdat_header = BoxInfo::new(fourcc("mdat"), 8 + payload.len() as u64).encode();
    let chunk_offsets = if samples.is_empty() {
        Vec::new()
    } else {
        vec![u64::try_from(ftyp_bytes.len() + provisional_moov.len() + mdat_header.len()).unwrap()]
    };
    let moov_bytes = build_imported_track_moov_bytes(
        file_config,
        track_config,
        movie_duration,
        edit_media_time,
        samples,
        &chunk_offsets,
    );

    let bytes = [ftyp_bytes, moov_bytes, mdat_header, payload].concat();
    write_temp_file(prefix, &bytes)
}

fn build_audio_input_file_with_type(
    prefix: &str,
    major_brand: mp4forge::FourCc,
    sample_entry_type: &str,
    payloads: &[&[u8]],
) -> std::path::PathBuf {
    let samples = payloads
        .iter()
        .copied()
        .map(|bytes| TestMuxSample {
            bytes,
            duration: 10,
            composition_time_offset: 0,
            is_sync_sample: true,
        })
        .collect::<Vec<_>>();
    write_single_track_mp4_input(
        prefix,
        &MuxFileConfig::new(1_000)
            .with_major_brand(major_brand)
            .with_compatible_brand(fourcc("mp42")),
        MuxTrackConfig::new_audio(
            1,
            1_000,
            audio_sample_entry_box_with_type(sample_entry_type),
        ),
        &samples,
    )
}

fn build_video_input_file(
    prefix: &str,
    major_brand: mp4forge::FourCc,
    payloads: &[&[u8]],
) -> std::path::PathBuf {
    build_video_input_file_with_type(prefix, major_brand, "avc1", payloads)
}

fn build_video_input_file_with_metadata(
    prefix: &str,
    major_brand: mp4forge::FourCc,
    sample_entry_type: &str,
    language: [u8; 3],
    handler_name: &str,
    payloads: &[&[u8]],
) -> std::path::PathBuf {
    let samples = payloads
        .iter()
        .copied()
        .map(|bytes| TestMuxSample {
            bytes,
            duration: 10,
            composition_time_offset: 0,
            is_sync_sample: true,
        })
        .collect::<Vec<_>>();
    write_single_track_mp4_input(
        prefix,
        &MuxFileConfig::new(1_000)
            .with_major_brand(major_brand)
            .with_compatible_brand(fourcc("mp42")),
        MuxTrackConfig::new_video(
            1,
            1_000,
            640,
            360,
            video_sample_entry_box_with_type(sample_entry_type),
        )
        .with_language(language)
        .with_handler_name(handler_name),
        &samples,
    )
}

fn build_video_input_file_with_type(
    prefix: &str,
    major_brand: mp4forge::FourCc,
    sample_entry_type: &str,
    payloads: &[&[u8]],
) -> std::path::PathBuf {
    let samples = payloads
        .iter()
        .copied()
        .map(|bytes| TestMuxSample {
            bytes,
            duration: 10,
            composition_time_offset: 0,
            is_sync_sample: true,
        })
        .collect::<Vec<_>>();
    write_single_track_mp4_input(
        prefix,
        &MuxFileConfig::new(1_000)
            .with_major_brand(major_brand)
            .with_compatible_brand(fourcc("mp42")),
        MuxTrackConfig::new_video(
            1,
            1_000,
            640,
            360,
            video_sample_entry_box_with_type(sample_entry_type),
        ),
        &samples,
    )
}

fn build_imported_track_moov_bytes(
    file_config: &MuxFileConfig,
    track_config: &MuxTrackConfig,
    movie_duration: u32,
    edit_media_time: u32,
    samples: &[TestMuxSample<'_>],
    chunk_offsets: &[u64],
) -> Vec<u8> {
    let mut mvhd = Mvhd::default();
    mvhd.timescale = file_config.movie_timescale();
    mvhd.duration_v0 = movie_duration;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = track_config.track_id() + 1;
    let mvhd_bytes = encode_supported_box(&mvhd, &[]);

    let media_duration = samples
        .iter()
        .map(|sample| sample.duration)
        .fold(0_u32, u32::saturating_add);

    let mut tkhd = Tkhd::default();
    tkhd.track_id = track_config.track_id();
    tkhd.duration_v0 = movie_duration;
    tkhd.volume = track_config.volume();
    tkhd.width = u32::from(track_config.track_width()) << 16;
    tkhd.height = u32::from(track_config.track_height()) << 16;
    let tkhd_bytes = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = track_config.timescale();
    mdhd.duration_v0 = media_duration;
    mdhd.language = encode_mdhd_language(track_config.language());
    let mdhd_bytes = encode_supported_box(&mdhd, &[]);

    let mut hdlr = Hdlr::default();
    hdlr.handler_type = match track_config.kind() {
        MuxTrackKind::Audio => fourcc("soun"),
        MuxTrackKind::Video => fourcc("vide"),
        MuxTrackKind::Text => fourcc("text"),
        MuxTrackKind::Subtitle => fourcc("subt"),
    };
    hdlr.name = track_config.handler_name().to_string();
    let hdlr_bytes = encode_supported_box(&hdlr, &[]);

    let media_header = match track_config.kind() {
        MuxTrackKind::Audio => encode_supported_box(&Smhd::default(), &[]),
        MuxTrackKind::Video => {
            let mut vmhd = Vmhd::default();
            vmhd.set_flags(1);
            encode_supported_box(&vmhd, &[])
        }
        MuxTrackKind::Text => encode_supported_box(&Nmhd::default(), &[]),
        MuxTrackKind::Subtitle => encode_supported_box(&Sthd::default(), &[]),
    };

    let mut url = Url::default();
    url.set_flags(1);
    let mut dref = Dref::default();
    dref.entry_count = 1;
    let dref_bytes = encode_supported_box(&dref, &encode_supported_box(&url, &[]));
    let dinf_bytes = encode_supported_box(&Dinf, &dref_bytes);

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd_bytes = encode_supported_box(&stsd, track_config.sample_entry_box());

    let mut stts = Stts::default();
    let mut stts_entries = Vec::<SttsEntry>::new();
    for sample in samples {
        if let Some(last) = stts_entries.last_mut()
            && last.sample_delta == sample.duration
        {
            last.sample_count += 1;
        } else {
            stts_entries.push(SttsEntry {
                sample_count: 1,
                sample_delta: sample.duration,
            });
        }
    }
    stts.entry_count = u32::try_from(stts_entries.len()).unwrap();
    stts.entries = stts_entries;
    let stts_bytes = encode_supported_box(&stts, &[]);

    let ctts_bytes = if samples
        .iter()
        .any(|sample| sample.composition_time_offset != 0)
    {
        let mut ctts = Ctts::default();
        let mut ctts_entries = Vec::<mp4forge::boxes::iso14496_12::CttsEntry>::new();
        for sample in samples {
            let sample_offset = u32::try_from(sample.composition_time_offset).unwrap();
            if let Some(last) = ctts_entries.last_mut()
                && last.sample_offset_v0 == sample_offset
            {
                last.sample_count += 1;
            } else {
                ctts_entries.push(mp4forge::boxes::iso14496_12::CttsEntry {
                    sample_count: 1,
                    sample_offset_v0: sample_offset,
                    ..mp4forge::boxes::iso14496_12::CttsEntry::default()
                });
            }
        }
        ctts.entry_count = u32::try_from(ctts_entries.len()).unwrap();
        ctts.entries = ctts_entries;
        Some(encode_supported_box(&ctts, &[]))
    } else {
        None
    };

    let mut stsc = Stsc::default();
    if !samples.is_empty() {
        stsc.entry_count = 1;
        stsc.entries = vec![StscEntry {
            first_chunk: 1,
            samples_per_chunk: u32::try_from(samples.len()).unwrap(),
            sample_description_index: 1,
        }];
    }
    let stsc_bytes = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = u32::try_from(samples.len()).unwrap();
    stsz.entry_size = samples
        .iter()
        .map(|sample| u64::try_from(sample.bytes.len()).unwrap())
        .collect();
    let stsz_bytes = encode_supported_box(&stsz, &[]);

    let mut co64 = Co64::default();
    co64.entry_count = u32::try_from(chunk_offsets.len()).unwrap();
    co64.chunk_offset = chunk_offsets.to_vec();
    let co64_bytes = encode_supported_box(&co64, &[]);

    let mut stbl_children = vec![stsd_bytes, stts_bytes];
    if let Some(ctts_bytes) = ctts_bytes {
        stbl_children.push(ctts_bytes);
    }
    stbl_children.extend([stsc_bytes, stsz_bytes, co64_bytes]);
    if samples.iter().any(|sample| !sample.is_sync_sample) {
        let mut stss = Stss::default();
        stss.sample_number = samples
            .iter()
            .enumerate()
            .filter(|(_, sample)| sample.is_sync_sample)
            .map(|(index, _)| u64::try_from(index + 1).unwrap())
            .collect();
        stss.entry_count = u32::try_from(stss.sample_number.len()).unwrap();
        stbl_children.push(encode_supported_box(&stss, &[]));
    }

    let stbl_bytes = encode_supported_box(&Stbl, &stbl_children.concat());
    let minf_bytes = encode_supported_box(&Minf, &[media_header, dinf_bytes, stbl_bytes].concat());
    let mdia_bytes = encode_supported_box(&Mdia, &[mdhd_bytes, hdlr_bytes, minf_bytes].concat());
    let edts_bytes = if edit_media_time == 0 {
        None
    } else {
        let mut elst = Elst::default();
        elst.entry_count = 1;
        elst.entries.push(mp4forge::boxes::iso14496_12::ElstEntry {
            segment_duration_v0: 0,
            media_time_v0: i32::try_from(edit_media_time).unwrap(),
            media_rate_integer: 1,
            ..mp4forge::boxes::iso14496_12::ElstEntry::default()
        });
        Some(encode_supported_box(
            &Edts,
            &encode_supported_box(&elst, &[]),
        ))
    };
    let mut trak_children = vec![tkhd_bytes];
    if let Some(edts_bytes) = edts_bytes {
        trak_children.push(edts_bytes);
    }
    trak_children.push(mdia_bytes);
    let trak_bytes = encode_supported_box(&Trak, &trak_children.concat());
    encode_supported_box(&Moov, &[mvhd_bytes, trak_bytes].concat())
}

fn audio_sample_entry_box() -> Vec<u8> {
    audio_sample_entry_box_with_type("mp4a")
}

fn audio_sample_entry_box_with_type(box_type: &str) -> Vec<u8> {
    audio_sample_entry_box_with_children(box_type, &[])
}

fn audio_sample_entry_box_with_children(box_type: &str, children: &[u8]) -> Vec<u8> {
    encode_supported_box(
        &AudioSampleEntry {
            sample_entry: SampleEntry {
                box_type: fourcc(box_type),
                data_reference_index: 1,
            },
            channel_count: 2,
            sample_size: 16,
            sample_rate: 48_000_u32 << 16,
            ..AudioSampleEntry::default()
        },
        children,
    )
}

fn video_sample_entry_box() -> Vec<u8> {
    video_sample_entry_box_with_type("avc1")
}

fn video_sample_entry_box_with_type(box_type: &str) -> Vec<u8> {
    encode_supported_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: fourcc(box_type),
                data_reference_index: 1,
            },
            width: 640,
            height: 360,
            horizresolution: 72_u32 << 16,
            vertresolution: 72_u32 << 16,
            frame_count: 1,
            depth: 0x0018,
            pre_defined3: -1,
            ..VisualSampleEntry::default()
        },
        &[],
    )
}

fn build_wvtt_input_file(
    prefix: &str,
    major_brand: mp4forge::FourCc,
    payloads: &[&[u8]],
) -> std::path::PathBuf {
    let samples = payloads
        .iter()
        .copied()
        .map(|bytes| TestMuxSample {
            bytes,
            duration: 10,
            composition_time_offset: 0,
            is_sync_sample: true,
        })
        .collect::<Vec<_>>();
    write_single_track_mp4_input(
        prefix,
        &MuxFileConfig::new(1_000)
            .with_major_brand(major_brand)
            .with_compatible_brand(fourcc("mp42")),
        MuxTrackConfig::new_text(1, 1_000, 0, 0, wvtt_sample_entry_box()),
        &samples,
    )
}

fn build_mixed_text_input_file(prefix: &str, major_brand: mp4forge::FourCc) -> std::path::PathBuf {
    let first_source = write_temp_file(&format!("{prefix}-source-text"), b"wvtt");
    let second_source = write_temp_file(&format!("{prefix}-source-subtitle"), b"stpp");
    let output_path = write_temp_file(prefix, &[]);
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 1, 0, 10, 0, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(1, 2, 0, 10, 0, 4).with_sync_sample(true),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();
    let file_config = MuxFileConfig::new(1_000)
        .with_major_brand(major_brand)
        .with_compatible_brand(fourcc("mp42"));
    let track_configs = vec![
        MuxTrackConfig::new_text(1, 1_000, 0, 0, wvtt_sample_entry_box())
            .with_language(*b"eng")
            .with_handler_name("EnglishCaptionHandler"),
        MuxTrackConfig::new_subtitle(2, 1_000, 0, 0, stpp_sample_entry_box())
            .with_language(*b"fra")
            .with_handler_name("FrenchSubtitleHandler"),
    ];

    write_mp4_mux_to_path(
        &[&first_source, &second_source],
        &output_path,
        &file_config,
        &track_configs,
        &plan,
    )
    .unwrap();
    output_path
}

fn decode_mdhd_language(encoded: [u8; 3]) -> [u8; 3] {
    [encoded[0] + b'`', encoded[1] + b'`', encoded[2] + b'`']
}

fn encode_mdhd_language(language: [u8; 3]) -> [u8; 3] {
    [language[0] - b'`', language[1] - b'`', language[2] - b'`']
}

fn wvtt_sample_entry_box() -> Vec<u8> {
    let children = [
        encode_supported_box(
            &WebVTTConfigurationBox {
                config: "WEBVTT".to_string(),
            },
            &[],
        ),
        encode_supported_box(
            &WebVTTSourceLabelBox {
                source_label: "source_label".to_string(),
            },
            &[],
        ),
    ]
    .concat();
    encode_supported_box(
        &WVTTSampleEntry {
            sample_entry: SampleEntry {
                box_type: fourcc("wvtt"),
                data_reference_index: 1,
            },
        },
        &children,
    )
}

fn stpp_sample_entry_box() -> Vec<u8> {
    encode_supported_box(
        &XMLSubtitleSampleEntry {
            sample_entry: SampleEntry {
                box_type: fourcc("stpp"),
                data_reference_index: 1,
            },
            namespace: "http://www.w3.org/ns/ttml".to_string(),
            schema_location: String::new(),
            auxiliary_mime_types: String::new(),
        },
        &[],
    )
}

fn read_root_boxes(bytes: &[u8]) -> Vec<BoxInfo> {
    let mut reader = Cursor::new(bytes);
    let mut root_boxes = Vec::new();
    while usize::try_from(reader.position())
        .ok()
        .is_some_and(|offset| offset < bytes.len())
    {
        let info = BoxInfo::read(&mut reader).unwrap();
        info.seek_to_end(&mut reader).unwrap();
        root_boxes.push(info);
    }
    root_boxes
}

fn mdat_payload(bytes: &[u8], mdat: BoxInfo) -> &[u8] {
    let start = usize::try_from(mdat.offset() + mdat.header_size()).unwrap();
    let end = usize::try_from(mdat.offset() + mdat.size()).unwrap();
    &bytes[start..end]
}

fn extract_boxes<T>(bytes: &[u8], path: BoxPath) -> Vec<T>
where
    T: mp4forge::codec::CodecBox + Clone + 'static,
{
    let mut reader = Cursor::new(bytes);
    extract_box_as::<_, T>(&mut reader, None, path).unwrap()
}
