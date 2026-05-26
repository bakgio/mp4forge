#![cfg(feature = "mux")]

mod support;

use mp4forge::mux::inspect::{
    DirectIngestDetectedKind, DirectIngestPacketEntry, DirectIngestPacketReport,
    DirectIngestReport, DirectIngestReportFormat, DirectIngestSampleReport,
    DirectIngestSourceSegmentReport, DirectIngestStagedSourceReport, DirectIngestTrackReport,
    collect_packet_report_warnings, collect_track_report_warnings, inspect_direct_ingest_packets,
    inspect_direct_ingest_path, write_packet_report, write_report,
};

use support::{write_temp_file_with_extension, write_test_ogg_opus_file, write_test_vobsub_files};

fn sample_report(
    source_index: usize,
    data_offset: u64,
    decode_time: u64,
) -> DirectIngestSampleReport {
    DirectIngestSampleReport {
        source_index,
        data_offset,
        data_size: 3,
        decode_time,
        previous_decode_delta: if decode_time == 0 { None } else { Some(960) },
        composition_time_offset: 0,
        presentation_time: decode_time as i64,
        presentation_end_time: decode_time as i64 + 960,
        previous_presentation_delta: if decode_time == 0 { None } else { Some(960) },
        duration: 960,
        is_sync_sample: true,
    }
}

fn packet_entry(
    packet_index: usize,
    data_offset: u64,
    decode_time: u64,
    previous_decode_delta: Option<u64>,
    payload_crc32: u32,
) -> DirectIngestPacketEntry {
    DirectIngestPacketEntry {
        track_id: 1,
        packet_index,
        track_kind: "audio".to_string(),
        timescale: 48_000,
        sample_entry_type: "Opus".to_string(),
        source_index: 0,
        data_offset,
        data_size: 3,
        decode_time,
        composition_time_offset: 0,
        presentation_time: decode_time as i64,
        presentation_end_time: decode_time as i64 + 960,
        previous_presentation_delta: if packet_index == 0 { None } else { Some(960) },
        duration: 960,
        previous_decode_delta,
        payload_crc32,
        is_sync_sample: true,
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn example_track_report() -> DirectIngestTrackReport {
    DirectIngestTrackReport {
        track_id: 1,
        kind: "audio".to_string(),
        timescale: 48_000,
        language: "und".to_string(),
        handler_name: "SoundHandler".to_string(),
        sample_entry_type: "Opus".to_string(),
        sample_entry_box_hex: "000000104f7075730000000000000000".to_string(),
        width: None,
        height: None,
        source_edit_media_time: Some(312),
        sample_roll_distance: Some(3_840),
        sample_count: 2,
        sync_sample_count: 2,
        starts_with_sync_sample: true,
        total_duration: 1_920,
        total_payload_size: 6,
        average_sample_size: Some(3),
        minimum_sample_size: Some(3),
        maximum_sample_size: Some(3),
        minimum_sample_duration: Some(960),
        maximum_sample_duration: Some(960),
        average_bitrate_bits_per_second: Some(1_200),
        minimum_sync_sample_size: Some(3),
        maximum_sync_sample_size: Some(3),
        average_sync_sample_size: Some(3),
        average_non_sync_sample_size: None,
        minimum_composition_time_offset: Some(0),
        maximum_composition_time_offset: Some(0),
        minimum_presentation_time: Some(0),
        maximum_presentation_end_time: Some(1_920),
        minimum_previous_decode_delta: Some(960),
        maximum_previous_decode_delta: Some(960),
        minimum_previous_presentation_delta: Some(960),
        maximum_previous_presentation_delta: Some(960),
        presentation_gap_count: 0,
        presentation_overlap_count: 0,
        presentation_regression_count: 0,
        duration_change_count: 0,
        composition_time_offset_change_count: 0,
        minimum_sync_sample_distance: Some(1),
        maximum_sync_sample_distance: Some(1),
        average_sync_sample_distance: Some(1),
        minimum_sync_sample_decode_delta: Some(960),
        maximum_sync_sample_decode_delta: Some(960),
        average_sync_sample_decode_delta: Some(960),
        first_sync_sample_index: Some(0),
        last_sync_sample_index: Some(1),
        first_sync_decode_time: Some(0),
        last_sync_decode_time: Some(960),
        first_sync_presentation_time: Some(0),
        last_sync_presentation_time: Some(960),
        first_decode_time: 0,
        end_decode_time: 1_920,
        samples: vec![sample_report(0, 8, 0), sample_report(0, 11, 960)],
    }
}

fn example_report() -> DirectIngestReport {
    DirectIngestReport {
        input_path: "input.ogg".into(),
        detected_kind: DirectIngestDetectedKind::Raw {
            codec: "opus".to_string(),
        },
        supports_flat_mux: true,
        note: None,
        track_count: 1,
        total_sample_count: 2,
        total_sync_sample_count: 2,
        total_payload_size: 6,
        staged_sources: vec![DirectIngestStagedSourceReport {
            source_index: 0,
            path: "input.ogg".into(),
            segmented: true,
            total_size: 96,
            segment_count: Some(3),
            segments: Some(vec![
                DirectIngestSourceSegmentReport {
                    kind: "prefix".to_string(),
                    logical_offset: 0,
                    logical_size: 4,
                    source_offset: None,
                    source_path: None,
                    data_hex: Some("4f676753".to_string()),
                },
                DirectIngestSourceSegmentReport {
                    kind: "file_range".to_string(),
                    logical_offset: 4,
                    logical_size: 88,
                    source_offset: Some(4),
                    source_path: None,
                    data_hex: None,
                },
                DirectIngestSourceSegmentReport {
                    kind: "bytes".to_string(),
                    logical_offset: 92,
                    logical_size: 4,
                    source_offset: None,
                    source_path: None,
                    data_hex: Some("deadbeef".to_string()),
                },
            ]),
        }],
        tracks: vec![example_track_report()],
    }
}

fn example_packet_report() -> DirectIngestPacketReport {
    DirectIngestPacketReport {
        input_path: "input.ogg".into(),
        detected_kind: DirectIngestDetectedKind::Raw {
            codec: "opus".to_string(),
        },
        supports_flat_mux: true,
        note: None,
        track_count: 1,
        packet_count: 2,
        sync_packet_count: 2,
        starts_with_sync_packet: true,
        total_payload_size: 6,
        minimum_packet_size: Some(3),
        maximum_packet_size: Some(3),
        minimum_sync_packet_size: Some(3),
        maximum_sync_packet_size: Some(3),
        average_sync_packet_size: Some(3),
        average_non_sync_packet_size: None,
        minimum_packet_duration: Some(960),
        maximum_packet_duration: Some(960),
        minimum_previous_decode_delta: Some(960),
        maximum_previous_decode_delta: Some(960),
        minimum_composition_time_offset: Some(0),
        maximum_composition_time_offset: Some(0),
        minimum_presentation_time: Some(0),
        maximum_presentation_end_time: Some(1_920),
        minimum_previous_presentation_delta: Some(960),
        maximum_previous_presentation_delta: Some(960),
        presentation_gap_count: 0,
        presentation_overlap_count: 0,
        presentation_regression_count: 0,
        duration_change_count: 0,
        composition_time_offset_change_count: 0,
        minimum_sync_packet_distance: Some(1),
        maximum_sync_packet_distance: Some(1),
        average_sync_packet_distance: Some(1),
        minimum_sync_packet_decode_delta: Some(960),
        maximum_sync_packet_decode_delta: Some(960),
        average_sync_packet_decode_delta: Some(960),
        first_sync_packet_track_id: Some(1),
        first_sync_packet_index: Some(0),
        last_sync_packet_track_id: Some(1),
        last_sync_packet_index: Some(1),
        first_sync_decode_time: Some(0),
        last_sync_decode_time: Some(960),
        first_sync_presentation_time: Some(0),
        last_sync_presentation_time: Some(960),
        tracks: vec![example_track_report()],
        staged_sources: vec![DirectIngestStagedSourceReport {
            source_index: 0,
            path: "input.ogg".into(),
            segmented: true,
            total_size: 96,
            segment_count: Some(3),
            segments: Some(vec![
                DirectIngestSourceSegmentReport {
                    kind: "prefix".to_string(),
                    logical_offset: 0,
                    logical_size: 4,
                    source_offset: None,
                    source_path: None,
                    data_hex: Some("4f676753".to_string()),
                },
                DirectIngestSourceSegmentReport {
                    kind: "file_range".to_string(),
                    logical_offset: 4,
                    logical_size: 88,
                    source_offset: Some(4),
                    source_path: None,
                    data_hex: None,
                },
                DirectIngestSourceSegmentReport {
                    kind: "bytes".to_string(),
                    logical_offset: 92,
                    logical_size: 4,
                    source_offset: None,
                    source_path: None,
                    data_hex: Some("deadbeef".to_string()),
                },
            ]),
        }],
        packets: vec![
            packet_entry(0, 8, 0, None, crc32(b"abc")),
            packet_entry(1, 11, 960, Some(960), crc32(b"def")),
        ],
    }
}

#[test]
fn direct_ingest_warning_helpers_surface_track_level_timing_and_sync_issues() {
    let mut report = example_report();
    let track = &mut report.tracks[0];
    track.starts_with_sync_sample = false;
    track.sync_sample_count = 0;
    track.presentation_gap_count = 2;
    track.presentation_overlap_count = 1;
    track.presentation_regression_count = 3;
    track.duration_change_count = 4;
    track.maximum_sample_duration = Some(1_920);
    track.composition_time_offset_change_count = 5;
    track.maximum_composition_time_offset = Some(33);

    let warnings = collect_track_report_warnings(&report);

    assert!(
        warnings
            .iter()
            .any(|line| line.contains("does not start with a sync sample"))
    );
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("has no sync samples"))
    );
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("2 presentation gap(s)"))
    );
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("1 presentation overlap(s)"))
    );
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("3 presentation regression(s)"))
    );
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("changes decode duration 4 time(s)"))
    );
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("changes composition offset 5 time(s)"))
    );
}

#[test]
fn direct_ingest_warning_helpers_surface_packet_level_timing_and_sync_issues() {
    let mut report = example_packet_report();
    report.starts_with_sync_packet = false;
    report.sync_packet_count = 0;
    report.presentation_gap_count = 2;
    report.presentation_overlap_count = 1;
    report.presentation_regression_count = 3;
    report.duration_change_count = 4;
    report.maximum_packet_duration = Some(1_920);
    report.composition_time_offset_change_count = 5;
    report.maximum_composition_time_offset = Some(33);

    let warnings = collect_packet_report_warnings(&report);

    assert!(
        warnings
            .iter()
            .any(|line| line.contains("does not start with a sync packet"))
    );
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("has no sync packets"))
    );
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("2 presentation gap(s)"))
    );
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("1 presentation overlap(s)"))
    );
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("3 presentation regression(s)"))
    );
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("changes decode duration 4 time(s)"))
    );
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("changes composition offset 5 time(s)"))
    );
}

#[test]
fn direct_ingest_report_renders_json_yaml_and_nhml_with_stable_fields() {
    let report = example_report();

    let mut json = Vec::new();
    write_report(&mut json, &report, DirectIngestReportFormat::Json).unwrap();
    assert_eq!(
        String::from_utf8(json).unwrap(),
        concat!(
            "{\n",
            "  \"InputPath\": \"input.ogg\",\n",
            "  \"DetectedKind\": {\n",
            "    \"Kind\": \"raw\",\n",
            "    \"Codec\": \"opus\"\n",
            "  },\n",
            "  \"SupportsFlatMux\": true,\n",
            "  \"TrackCount\": 1,\n",
            "  \"TotalSampleCount\": 2,\n",
            "  \"TotalSyncSampleCount\": 2,\n",
            "  \"TotalPayloadSize\": 6,\n",
            "  \"StagedSources\": [\n",
            "    {\n",
            "      \"SourceIndex\": 0,\n",
            "      \"Path\": \"input.ogg\",\n",
            "      \"Segmented\": true,\n",
            "      \"TotalSize\": 96,\n",
            "      \"SegmentCount\": 3,\n",
            "      \"Segments\": [\n",
            "        {\n",
            "          \"Kind\": \"prefix\",\n",
            "          \"LogicalOffset\": 0,\n",
            "          \"LogicalSize\": 4,\n",
            "          \"DataHex\": \"4f676753\"\n",
            "        },\n",
            "        {\n",
            "          \"Kind\": \"file_range\",\n",
            "          \"LogicalOffset\": 4,\n",
            "          \"LogicalSize\": 88,\n",
            "          \"SourceOffset\": 4\n",
            "        },\n",
            "        {\n",
            "          \"Kind\": \"bytes\",\n",
            "          \"LogicalOffset\": 92,\n",
            "          \"LogicalSize\": 4,\n",
            "          \"DataHex\": \"deadbeef\"\n",
            "        }\n",
            "      ]\n",
            "    }\n",
            "  ],\n",
            "  \"Tracks\": [\n",
            "    {\n",
            "      \"TrackID\": 1,\n",
            "      \"Kind\": \"audio\",\n",
            "      \"Timescale\": 48000,\n",
            "      \"Language\": \"und\",\n",
            "      \"HandlerName\": \"SoundHandler\",\n",
            "      \"SampleEntryType\": \"Opus\",\n",
            "      \"SampleEntryBoxHex\": \"000000104f7075730000000000000000\",\n",
            "      \"SampleCount\": 2,\n",
            "      \"SyncSampleCount\": 2,\n",
            "      \"StartsWithSyncSample\": true,\n",
            "      \"TotalDuration\": 1920,\n",
            "      \"TotalPayloadSize\": 6,\n",
            "      \"AverageSampleSize\": 3,\n",
            "      \"MinimumSampleSize\": 3,\n",
            "      \"MaximumSampleSize\": 3,\n",
            "      \"MinimumSampleDuration\": 960,\n",
            "      \"MaximumSampleDuration\": 960,\n",
            "      \"AverageBitrateBitsPerSecond\": 1200,\n",
            "      \"MinimumSyncSampleSize\": 3,\n",
            "      \"MaximumSyncSampleSize\": 3,\n",
            "      \"AverageSyncSampleSize\": 3,\n",
            "      \"AverageNonSyncSampleSize\": null,\n",
            "      \"MinimumCompositionTimeOffset\": 0,\n",
            "      \"MaximumCompositionTimeOffset\": 0,\n",
            "      \"MinimumPresentationTime\": 0,\n",
            "      \"MaximumPresentationEndTime\": 1920,\n",
            "      \"MinimumPreviousDecodeDelta\": 960,\n",
            "      \"MaximumPreviousDecodeDelta\": 960,\n",
            "      \"MinimumPreviousPresentationDelta\": 960,\n",
            "      \"MaximumPreviousPresentationDelta\": 960,\n",
            "      \"PresentationGapCount\": 0,\n",
            "      \"PresentationOverlapCount\": 0,\n",
            "      \"PresentationRegressionCount\": 0,\n",
            "      \"DurationChangeCount\": 0,\n",
            "      \"CompositionTimeOffsetChangeCount\": 0,\n",
            "      \"MinimumSyncSampleDistance\": 1,\n",
            "      \"MaximumSyncSampleDistance\": 1,\n",
            "      \"AverageSyncSampleDistance\": 1,\n",
            "      \"MinimumSyncSampleDecodeDelta\": 960,\n",
            "      \"MaximumSyncSampleDecodeDelta\": 960,\n",
            "      \"AverageSyncSampleDecodeDelta\": 960,\n",
            "      \"FirstSyncSampleIndex\": 0,\n",
            "      \"LastSyncSampleIndex\": 1,\n",
            "      \"FirstSyncDecodeTime\": 0,\n",
            "      \"LastSyncDecodeTime\": 960,\n",
            "      \"FirstSyncPresentationTime\": 0,\n",
            "      \"LastSyncPresentationTime\": 960,\n",
            "      \"FirstDecodeTime\": 0,\n",
            "      \"EndDecodeTime\": 1920,\n",
            "      \"SourceEditMediaTime\": 312,\n",
            "      \"SampleRollDistance\": 3840,\n",
            "      \"Samples\": [\n",
            "        {\n",
            "          \"SourceIndex\": 0,\n",
            "          \"DataOffset\": 8,\n",
            "          \"DataSize\": 3,\n",
            "          \"DecodeTime\": 0,\n",
            "          \"PreviousDecodeDelta\": null,\n",
            "          \"CompositionTimeOffset\": 0,\n",
            "          \"PresentationTime\": 0,\n",
            "          \"PresentationEndTime\": 960,\n",
            "          \"PreviousPresentationDelta\": null,\n",
            "          \"Duration\": 960,\n",
            "          \"IsSyncSample\": true\n",
            "        },\n",
            "        {\n",
            "          \"SourceIndex\": 0,\n",
            "          \"DataOffset\": 11,\n",
            "          \"DataSize\": 3,\n",
            "          \"DecodeTime\": 960,\n",
            "          \"PreviousDecodeDelta\": 960,\n",
            "          \"CompositionTimeOffset\": 0,\n",
            "          \"PresentationTime\": 960,\n",
            "          \"PresentationEndTime\": 1920,\n",
            "          \"PreviousPresentationDelta\": 960,\n",
            "          \"Duration\": 960,\n",
            "          \"IsSyncSample\": true\n",
            "        }\n",
            "      ]\n",
            "    }\n",
            "  ]\n",
            "}\n"
        )
    );

    let mut yaml = Vec::new();
    write_report(&mut yaml, &report, DirectIngestReportFormat::Yaml).unwrap();
    assert_eq!(
        String::from_utf8(yaml).unwrap(),
        concat!(
            "input_path: input.ogg\n",
            "detected_kind:\n",
            "  kind: raw\n",
            "  codec: opus\n",
            "supports_flat_mux: true\n",
            "track_count: 1\n",
            "total_sample_count: 2\n",
            "total_sync_sample_count: 2\n",
            "total_payload_size: 6\n",
            "staged_sources:\n",
            "- source_index: 0\n",
            "  path: input.ogg\n",
            "  segmented: true\n",
            "  total_size: 96\n",
            "  segment_count: 3\n",
            "  segments:\n",
            "  - kind: prefix\n",
            "    logical_offset: 0\n",
            "    logical_size: 4\n",
            "    source_offset: null\n",
            "    data_hex: 4f676753\n",
            "  - kind: file_range\n",
            "    logical_offset: 4\n",
            "    logical_size: 88\n",
            "    source_offset: 4\n",
            "    data_hex: null\n",
            "  - kind: bytes\n",
            "    logical_offset: 92\n",
            "    logical_size: 4\n",
            "    source_offset: null\n",
            "    data_hex: deadbeef\n",
            "tracks:\n",
            "- track_id: 1\n",
            "  kind: audio\n",
            "  timescale: 48000\n",
            "  language: und\n",
            "  handler_name: SoundHandler\n",
            "  sample_entry_type: Opus\n",
            "  sample_entry_box_hex: 000000104f7075730000000000000000\n",
            "  source_edit_media_time: 312\n",
            "  sample_roll_distance: 3840\n",
            "  sample_count: 2\n",
            "  sync_sample_count: 2\n",
            "  starts_with_sync_sample: true\n",
            "  total_duration: 1920\n",
            "  total_payload_size: 6\n",
            "  average_sample_size: 3\n",
            "  minimum_sample_size: 3\n",
            "  maximum_sample_size: 3\n",
            "  minimum_sample_duration: 960\n",
            "  maximum_sample_duration: 960\n",
            "  average_bitrate_bits_per_second: 1200\n",
            "  minimum_sync_sample_size: 3\n",
            "  maximum_sync_sample_size: 3\n",
            "  average_sync_sample_size: 3\n",
            "  average_non_sync_sample_size: null\n",
            "  minimum_composition_time_offset: 0\n",
            "  maximum_composition_time_offset: 0\n",
            "  minimum_presentation_time: 0\n",
            "  maximum_presentation_end_time: 1920\n",
            "  minimum_previous_decode_delta: 960\n",
            "  maximum_previous_decode_delta: 960\n",
            "  minimum_previous_presentation_delta: 960\n",
            "  maximum_previous_presentation_delta: 960\n",
            "  presentation_gap_count: 0\n",
            "  presentation_overlap_count: 0\n",
            "  presentation_regression_count: 0\n",
            "  duration_change_count: 0\n",
            "  composition_time_offset_change_count: 0\n",
            "  minimum_sync_sample_distance: 1\n",
            "  maximum_sync_sample_distance: 1\n",
            "  average_sync_sample_distance: 1\n",
            "  minimum_sync_sample_decode_delta: 960\n",
            "  maximum_sync_sample_decode_delta: 960\n",
            "  average_sync_sample_decode_delta: 960\n",
            "  first_sync_sample_index: 0\n",
            "  last_sync_sample_index: 1\n",
            "  first_sync_decode_time: 0\n",
            "  last_sync_decode_time: 960\n",
            "  first_sync_presentation_time: 0\n",
            "  last_sync_presentation_time: 960\n",
            "  first_decode_time: 0\n",
            "  end_decode_time: 1920\n",
            "  samples:\n",
            "  - source_index: 0\n",
            "    data_offset: 8\n",
            "    data_size: 3\n",
            "    decode_time: 0\n",
            "    previous_decode_delta: null\n",
            "    composition_time_offset: 0\n",
            "    presentation_time: 0\n",
            "    presentation_end_time: 960\n",
            "    previous_presentation_delta: null\n",
            "    duration: 960\n",
            "    is_sync_sample: true\n",
            "  - source_index: 0\n",
            "    data_offset: 11\n",
            "    data_size: 3\n",
            "    decode_time: 960\n",
            "    previous_decode_delta: 960\n",
            "    composition_time_offset: 0\n",
            "    presentation_time: 960\n",
            "    presentation_end_time: 1920\n",
            "    previous_presentation_delta: 960\n",
            "    duration: 960\n",
            "    is_sync_sample: true\n"
        )
    );

    let mut nhml = Vec::new();
    write_report(&mut nhml, &report, DirectIngestReportFormat::Nhml).unwrap();
    assert_eq!(
        String::from_utf8(nhml).unwrap(),
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<nhml inputPath=\"input.ogg\" detectedKind=\"raw\" supportsFlatMux=\"true\" trackCount=\"1\" totalSampleCount=\"2\" totalSyncSampleCount=\"2\" totalPayloadSize=\"6\" codec=\"opus\">\n",
            "  <source index=\"0\" path=\"input.ogg\" segmented=\"true\" totalSize=\"96\" segmentCount=\"3\">\n",
            "    <segment kind=\"prefix\" logicalOffset=\"0\" logicalSize=\"4\" dataHex=\"4f676753\" />\n",
            "    <segment kind=\"file_range\" logicalOffset=\"4\" logicalSize=\"88\" sourceOffset=\"4\" />\n",
            "    <segment kind=\"bytes\" logicalOffset=\"92\" logicalSize=\"4\" dataHex=\"deadbeef\" />\n",
            "  </source>\n",
            "  <track trackID=\"1\" kind=\"audio\" timescale=\"48000\" language=\"und\" handlerName=\"SoundHandler\" sampleEntryType=\"Opus\" sampleEntryBoxHex=\"000000104f7075730000000000000000\" sampleCount=\"2\" syncSampleCount=\"2\" startsWithSyncSample=\"true\" totalDuration=\"1920\" totalPayloadSize=\"6\" firstDecodeTime=\"0\" endDecodeTime=\"1920\" sourceEditMediaTime=\"312\" sampleRollDistance=\"3840\" minimumSampleSize=\"3\" maximumSampleSize=\"3\" minimumSampleDuration=\"960\" maximumSampleDuration=\"960\" averageBitrateBitsPerSecond=\"1200\" averageSampleSize=\"3\" minimumSyncSampleSize=\"3\" maximumSyncSampleSize=\"3\" averageSyncSampleSize=\"3\" minimumCompositionTimeOffset=\"0\" maximumCompositionTimeOffset=\"0\" minimumPresentationTime=\"0\" maximumPresentationEndTime=\"1920\" minimumPreviousDecodeDelta=\"960\" maximumPreviousDecodeDelta=\"960\" minimumPreviousPresentationDelta=\"960\" maximumPreviousPresentationDelta=\"960\" presentationGapCount=\"0\" presentationOverlapCount=\"0\" presentationRegressionCount=\"0\" durationChangeCount=\"0\" compositionTimeOffsetChangeCount=\"0\" minimumSyncSampleDistance=\"1\" maximumSyncSampleDistance=\"1\" averageSyncSampleDistance=\"1\" minimumSyncSampleDecodeDelta=\"960\" maximumSyncSampleDecodeDelta=\"960\" averageSyncSampleDecodeDelta=\"960\" firstSyncSampleIndex=\"0\" lastSyncSampleIndex=\"1\" firstSyncDecodeTime=\"0\" lastSyncDecodeTime=\"960\" firstSyncPresentationTime=\"0\" lastSyncPresentationTime=\"960\">\n",
            "    <sample sourceIndex=\"0\" dataOffset=\"8\" dataSize=\"3\" decodeTime=\"0\" compositionTimeOffset=\"0\" presentationTime=\"0\" presentationEndTime=\"960\" duration=\"960\" sync=\"true\" />\n",
            "    <sample sourceIndex=\"0\" dataOffset=\"11\" dataSize=\"3\" decodeTime=\"960\" previousDecodeDelta=\"960\" compositionTimeOffset=\"0\" presentationTime=\"960\" presentationEndTime=\"1920\" previousPresentationDelta=\"960\" duration=\"960\" sync=\"true\" />\n",
            "  </track>\n",
            "</nhml>\n"
        )
    );
}

#[test]
fn direct_ingest_packet_report_renders_json_yaml_and_nhnt_with_stable_fields() {
    let report = example_packet_report();

    let mut json = Vec::new();
    write_packet_report(&mut json, &report, DirectIngestReportFormat::Json).unwrap();
    let expected_json = format!(
        concat!(
            "{{\n",
            "  \"InputPath\": \"input.ogg\",\n",
            "  \"DetectedKind\": {{\n",
            "    \"Kind\": \"raw\",\n",
            "    \"Codec\": \"opus\"\n",
            "  }},\n",
            "  \"SupportsFlatMux\": true,\n",
            "  \"TrackCount\": 1,\n",
            "  \"PacketCount\": 2,\n",
            "  \"SyncPacketCount\": 2,\n",
            "  \"StartsWithSyncPacket\": true,\n",
            "  \"TotalPayloadSize\": 6,\n",
            "  \"MinimumPacketSize\": 3,\n",
            "  \"MaximumPacketSize\": 3,\n",
            "  \"MinimumSyncPacketSize\": 3,\n",
            "  \"MaximumSyncPacketSize\": 3,\n",
            "  \"AverageSyncPacketSize\": 3,\n",
            "  \"MinimumPacketDuration\": 960,\n",
            "  \"MaximumPacketDuration\": 960,\n",
            "  \"MinimumPreviousDecodeDelta\": 960,\n",
            "  \"MaximumPreviousDecodeDelta\": 960,\n",
            "  \"MinimumCompositionTimeOffset\": 0,\n",
            "  \"MaximumCompositionTimeOffset\": 0,\n",
            "  \"MinimumPresentationTime\": 0,\n",
            "  \"MaximumPresentationEndTime\": 1920,\n",
            "  \"MinimumPreviousPresentationDelta\": 960,\n",
            "  \"MaximumPreviousPresentationDelta\": 960,\n",
            "  \"PresentationGapCount\": 0,\n",
            "  \"PresentationOverlapCount\": 0,\n",
            "  \"PresentationRegressionCount\": 0,\n",
            "  \"DurationChangeCount\": 0,\n",
            "  \"CompositionTimeOffsetChangeCount\": 0,\n",
            "  \"MinimumSyncPacketDistance\": 1,\n",
            "  \"MaximumSyncPacketDistance\": 1,\n",
            "  \"AverageSyncPacketDistance\": 1,\n",
            "  \"MinimumSyncPacketDecodeDelta\": 960,\n",
            "  \"MaximumSyncPacketDecodeDelta\": 960,\n",
            "  \"AverageSyncPacketDecodeDelta\": 960,\n",
            "  \"FirstSyncPacketTrackID\": 1,\n",
            "  \"FirstSyncPacketIndex\": 0,\n",
            "  \"LastSyncPacketTrackID\": 1,\n",
            "  \"LastSyncPacketIndex\": 1,\n",
            "  \"FirstSyncDecodeTime\": 0,\n",
            "  \"LastSyncDecodeTime\": 960,\n",
            "  \"FirstSyncPresentationTime\": 0,\n",
            "  \"LastSyncPresentationTime\": 960,\n",
            "  \"StagedSources\": [\n",
            "    {{\n",
            "      \"SourceIndex\": 0,\n",
            "      \"Path\": \"input.ogg\",\n",
            "      \"Segmented\": true,\n",
            "      \"TotalSize\": 96,\n",
            "      \"SegmentCount\": 3,\n",
            "      \"Segments\": [\n",
            "        {{\n",
            "          \"Kind\": \"prefix\",\n",
            "          \"LogicalOffset\": 0,\n",
            "          \"LogicalSize\": 4,\n",
            "          \"DataHex\": \"4f676753\"\n",
            "        }},\n",
            "        {{\n",
            "          \"Kind\": \"file_range\",\n",
            "          \"LogicalOffset\": 4,\n",
            "          \"LogicalSize\": 88,\n",
            "          \"SourceOffset\": 4\n",
            "        }},\n",
            "        {{\n",
            "          \"Kind\": \"bytes\",\n",
            "          \"LogicalOffset\": 92,\n",
            "          \"LogicalSize\": 4,\n",
            "          \"DataHex\": \"deadbeef\"\n",
            "        }}\n",
            "      ]\n",
            "    }}\n",
            "  ],\n",
            "  \"Packets\": [\n",
            "    {{\n",
            "      \"TrackID\": 1,\n",
            "      \"PacketIndex\": 0,\n",
            "      \"TrackKind\": \"audio\",\n",
            "      \"Timescale\": 48000,\n",
            "      \"SampleEntryType\": \"Opus\",\n",
            "      \"SourceIndex\": 0,\n",
            "      \"DataOffset\": 8,\n",
            "      \"DataSize\": 3,\n",
            "      \"DecodeTime\": 0,\n",
            "      \"CompositionTimeOffset\": 0,\n",
            "      \"PresentationTime\": 0,\n",
            "      \"PresentationEndTime\": 960,\n",
            "      \"PreviousPresentationDelta\": null,\n",
            "      \"Duration\": 960,\n",
            "      \"PreviousDecodeDelta\": null,\n",
            "      \"PayloadCrc32\": {},\n",
            "      \"IsSyncSample\": true\n",
            "    }},\n",
            "    {{\n",
            "      \"TrackID\": 1,\n",
            "      \"PacketIndex\": 1,\n",
            "      \"TrackKind\": \"audio\",\n",
            "      \"Timescale\": 48000,\n",
            "      \"SampleEntryType\": \"Opus\",\n",
            "      \"SourceIndex\": 0,\n",
            "      \"DataOffset\": 11,\n",
            "      \"DataSize\": 3,\n",
            "      \"DecodeTime\": 960,\n",
            "      \"CompositionTimeOffset\": 0,\n",
            "      \"PresentationTime\": 960,\n",
            "      \"PresentationEndTime\": 1920,\n",
            "      \"PreviousPresentationDelta\": 960,\n",
            "      \"Duration\": 960,\n",
            "      \"PreviousDecodeDelta\": 960,\n",
            "      \"PayloadCrc32\": {},\n",
            "      \"IsSyncSample\": true\n",
            "    }}\n",
            "  ]\n",
            "}}\n"
        ),
        crc32(b"abc"),
        crc32(b"def")
    );
    assert_eq!(String::from_utf8(json).unwrap(), expected_json);

    let mut yaml = Vec::new();
    write_packet_report(&mut yaml, &report, DirectIngestReportFormat::Yaml).unwrap();
    let expected_yaml = format!(
        concat!(
            "input_path: input.ogg\n",
            "detected_kind:\n",
            "  kind: raw\n",
            "  codec: opus\n",
            "supports_flat_mux: true\n",
            "track_count: 1\n",
            "packet_count: 2\n",
            "sync_packet_count: 2\n",
            "starts_with_sync_packet: true\n",
            "total_payload_size: 6\n",
            "minimum_packet_size: 3\n",
            "maximum_packet_size: 3\n",
            "minimum_sync_packet_size: 3\n",
            "maximum_sync_packet_size: 3\n",
            "average_sync_packet_size: 3\n",
            "minimum_packet_duration: 960\n",
            "maximum_packet_duration: 960\n",
            "minimum_previous_decode_delta: 960\n",
            "maximum_previous_decode_delta: 960\n",
            "minimum_composition_time_offset: 0\n",
            "maximum_composition_time_offset: 0\n",
            "minimum_presentation_time: 0\n",
            "maximum_presentation_end_time: 1920\n",
            "minimum_previous_presentation_delta: 960\n",
            "maximum_previous_presentation_delta: 960\n",
            "presentation_gap_count: 0\n",
            "presentation_overlap_count: 0\n",
            "presentation_regression_count: 0\n",
            "duration_change_count: 0\n",
            "composition_time_offset_change_count: 0\n",
            "minimum_sync_packet_distance: 1\n",
            "maximum_sync_packet_distance: 1\n",
            "average_sync_packet_distance: 1\n",
            "minimum_sync_packet_decode_delta: 960\n",
            "maximum_sync_packet_decode_delta: 960\n",
            "average_sync_packet_decode_delta: 960\n",
            "first_sync_packet_track_id: 1\n",
            "first_sync_packet_index: 0\n",
            "last_sync_packet_track_id: 1\n",
            "last_sync_packet_index: 1\n",
            "first_sync_decode_time: 0\n",
            "last_sync_decode_time: 960\n",
            "first_sync_presentation_time: 0\n",
            "last_sync_presentation_time: 960\n",
            "staged_sources:\n",
            "- source_index: 0\n",
            "  path: input.ogg\n",
            "  segmented: true\n",
            "  total_size: 96\n",
            "  segment_count: 3\n",
            "  segments:\n",
            "  - kind: prefix\n",
            "    logical_offset: 0\n",
            "    logical_size: 4\n",
            "    source_offset: null\n",
            "    data_hex: 4f676753\n",
            "  - kind: file_range\n",
            "    logical_offset: 4\n",
            "    logical_size: 88\n",
            "    source_offset: 4\n",
            "    data_hex: null\n",
            "  - kind: bytes\n",
            "    logical_offset: 92\n",
            "    logical_size: 4\n",
            "    source_offset: null\n",
            "    data_hex: deadbeef\n",
            "packets:\n",
            "- track_id: 1\n",
            "  packet_index: 0\n",
            "  track_kind: audio\n",
            "  timescale: 48000\n",
            "  sample_entry_type: Opus\n",
            "  source_index: 0\n",
            "  data_offset: 8\n",
            "  data_size: 3\n",
            "  decode_time: 0\n",
            "  composition_time_offset: 0\n",
            "  presentation_time: 0\n",
            "  presentation_end_time: 960\n",
            "  previous_presentation_delta: null\n",
            "  duration: 960\n",
            "  previous_decode_delta: null\n",
            "  payload_crc32: {}\n",
            "  is_sync_sample: true\n",
            "- track_id: 1\n",
            "  packet_index: 1\n",
            "  track_kind: audio\n",
            "  timescale: 48000\n",
            "  sample_entry_type: Opus\n",
            "  source_index: 0\n",
            "  data_offset: 11\n",
            "  data_size: 3\n",
            "  decode_time: 960\n",
            "  composition_time_offset: 0\n",
            "  presentation_time: 960\n",
            "  presentation_end_time: 1920\n",
            "  previous_presentation_delta: 960\n",
            "  duration: 960\n",
            "  previous_decode_delta: 960\n",
            "  payload_crc32: {}\n",
            "  is_sync_sample: true\n"
        ),
        crc32(b"abc"),
        crc32(b"def")
    );
    assert_eq!(String::from_utf8(yaml).unwrap(), expected_yaml);

    let mut nhnt = Vec::new();
    write_packet_report(&mut nhnt, &report, DirectIngestReportFormat::Nhnt).unwrap();
    let expected_nhnt = format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<nhnt inputPath=\"input.ogg\" detectedKind=\"raw\" supportsFlatMux=\"true\" trackCount=\"1\" packetCount=\"2\" syncPacketCount=\"2\" totalPayloadSize=\"6\" startsWithSyncPacket=\"true\" codec=\"opus\" minimumPacketSize=\"3\" maximumPacketSize=\"3\" minimumSyncPacketSize=\"3\" maximumSyncPacketSize=\"3\" averageSyncPacketSize=\"3\" minimumPacketDuration=\"960\" maximumPacketDuration=\"960\" minimumPreviousDecodeDelta=\"960\" maximumPreviousDecodeDelta=\"960\" minimumCompositionTimeOffset=\"0\" maximumCompositionTimeOffset=\"0\" minimumPresentationTime=\"0\" maximumPresentationEndTime=\"1920\" minimumPreviousPresentationDelta=\"960\" maximumPreviousPresentationDelta=\"960\" presentationGapCount=\"0\" presentationOverlapCount=\"0\" presentationRegressionCount=\"0\" durationChangeCount=\"0\" compositionTimeOffsetChangeCount=\"0\" minimumSyncPacketDistance=\"1\" maximumSyncPacketDistance=\"1\" averageSyncPacketDistance=\"1\" minimumSyncPacketDecodeDelta=\"960\" maximumSyncPacketDecodeDelta=\"960\" averageSyncPacketDecodeDelta=\"960\" firstSyncPacketTrackID=\"1\" firstSyncPacketIndex=\"0\" lastSyncPacketTrackID=\"1\" lastSyncPacketIndex=\"1\" firstSyncDecodeTime=\"0\" lastSyncDecodeTime=\"960\" firstSyncPresentationTime=\"0\" lastSyncPresentationTime=\"960\">\n",
            "  <source index=\"0\" path=\"input.ogg\" segmented=\"true\" totalSize=\"96\" segmentCount=\"3\">\n",
            "    <segment kind=\"prefix\" logicalOffset=\"0\" logicalSize=\"4\" dataHex=\"4f676753\" />\n",
            "    <segment kind=\"file_range\" logicalOffset=\"4\" logicalSize=\"88\" sourceOffset=\"4\" />\n",
            "    <segment kind=\"bytes\" logicalOffset=\"92\" logicalSize=\"4\" dataHex=\"deadbeef\" />\n",
            "  </source>\n",
            "  <track trackID=\"1\" kind=\"audio\" timescale=\"48000\" language=\"und\" handlerName=\"SoundHandler\" sampleEntryType=\"Opus\" sampleEntryBoxHex=\"000000104f7075730000000000000000\" sourceEditMediaTime=\"312\" sampleRollDistance=\"3840\" sampleCount=\"2\" syncSampleCount=\"2\" totalDuration=\"1920\" totalPayloadSize=\"6\" />\n",
            "  <packet trackID=\"1\" packetIndex=\"0\" trackKind=\"audio\" timescale=\"48000\" sampleEntryType=\"Opus\" sourceIndex=\"0\" dataOffset=\"8\" dataSize=\"3\" decodeTime=\"0\" compositionTimeOffset=\"0\" presentationTime=\"0\" presentationEndTime=\"960\" duration=\"960\" payloadCrc32=\"{}\" sync=\"true\" />\n",
            "  <packet trackID=\"1\" packetIndex=\"1\" trackKind=\"audio\" timescale=\"48000\" sampleEntryType=\"Opus\" sourceIndex=\"0\" dataOffset=\"11\" dataSize=\"3\" decodeTime=\"960\" compositionTimeOffset=\"0\" presentationTime=\"960\" presentationEndTime=\"1920\" previousPresentationDelta=\"960\" duration=\"960\" previousDecodeDelta=\"960\" payloadCrc32=\"{}\" sync=\"true\" />\n",
            "</nhnt>\n"
        ),
        crc32(b"abc"),
        crc32(b"def")
    );
    assert_eq!(String::from_utf8(nhnt).unwrap(), expected_nhnt);
}

#[test]
fn inspect_direct_ingest_path_reports_real_ogg_opus_tracks() {
    let input = write_test_ogg_opus_file("inspect-ogg-opus-input", &[b"abc", b"def"]);

    let report = inspect_direct_ingest_path(&input).unwrap();

    assert!(report.supports_flat_mux);
    assert_eq!(
        report.detected_kind,
        DirectIngestDetectedKind::Raw {
            codec: "opus".to_string()
        }
    );
    assert_eq!(report.track_count, 1);
    assert_eq!(report.total_sample_count, 2);
    assert_eq!(report.total_sync_sample_count, 2);
    assert_eq!(report.total_payload_size, 8);
    assert_eq!(report.staged_sources.len(), 1);
    assert!(report.staged_sources[0].segmented);
    assert_eq!(
        report.staged_sources[0]
            .segments
            .as_ref()
            .map(|segments| segments.len()),
        report.staged_sources[0].segment_count
    );
    assert!(
        report.staged_sources[0]
            .segments
            .as_ref()
            .is_some_and(|segments| !segments.is_empty())
    );
    assert_eq!(report.tracks.len(), 1);
    assert_eq!(report.tracks[0].kind, "audio");
    assert_eq!(report.tracks[0].sample_entry_type, "Opus");
    assert!(!report.tracks[0].sample_entry_box_hex.is_empty());
    assert_eq!(report.tracks[0].sample_count, 2);
    assert_eq!(report.tracks[0].sync_sample_count, 2);
    assert!(report.tracks[0].starts_with_sync_sample);
    assert_eq!(report.tracks[0].total_payload_size, 8);
    assert_eq!(report.tracks[0].average_sample_size, Some(4));
    assert_eq!(report.tracks[0].minimum_sample_size, Some(4));
    assert_eq!(report.tracks[0].maximum_sample_size, Some(4));
    assert_eq!(report.tracks[0].minimum_sample_duration, Some(480));
    assert_eq!(report.tracks[0].maximum_sample_duration, Some(480));
    assert_eq!(
        report.tracks[0].average_bitrate_bits_per_second,
        Some(3_200)
    );
    assert_eq!(report.tracks[0].minimum_sync_sample_size, Some(4));
    assert_eq!(report.tracks[0].maximum_sync_sample_size, Some(4));
    assert_eq!(report.tracks[0].average_sync_sample_size, Some(4));
    assert_eq!(report.tracks[0].average_non_sync_sample_size, None);
    assert_eq!(report.tracks[0].minimum_composition_time_offset, Some(0));
    assert_eq!(report.tracks[0].maximum_composition_time_offset, Some(0));
    assert_eq!(report.tracks[0].minimum_presentation_time, Some(0));
    assert_eq!(report.tracks[0].maximum_presentation_end_time, Some(960));
    assert_eq!(report.tracks[0].minimum_previous_decode_delta, Some(480));
    assert_eq!(report.tracks[0].maximum_previous_decode_delta, Some(480));
    assert_eq!(
        report.tracks[0].minimum_previous_presentation_delta,
        Some(480)
    );
    assert_eq!(
        report.tracks[0].maximum_previous_presentation_delta,
        Some(480)
    );
    assert_eq!(report.tracks[0].presentation_gap_count, 0);
    assert_eq!(report.tracks[0].presentation_overlap_count, 0);
    assert_eq!(report.tracks[0].presentation_regression_count, 0);
    assert_eq!(report.tracks[0].duration_change_count, 0);
    assert_eq!(report.tracks[0].composition_time_offset_change_count, 0);
    assert_eq!(report.tracks[0].minimum_sync_sample_distance, Some(1));
    assert_eq!(report.tracks[0].maximum_sync_sample_distance, Some(1));
    assert_eq!(report.tracks[0].average_sync_sample_distance, Some(1));
    assert_eq!(report.tracks[0].minimum_sync_sample_decode_delta, Some(480));
    assert_eq!(report.tracks[0].maximum_sync_sample_decode_delta, Some(480));
    assert_eq!(report.tracks[0].average_sync_sample_decode_delta, Some(480));
    assert_eq!(report.tracks[0].first_sync_sample_index, Some(0));
    assert_eq!(report.tracks[0].last_sync_sample_index, Some(1));
    assert_eq!(report.tracks[0].first_sync_decode_time, Some(0));
    assert_eq!(report.tracks[0].last_sync_decode_time, Some(480));
    assert_eq!(report.tracks[0].first_sync_presentation_time, Some(0));
    assert_eq!(report.tracks[0].last_sync_presentation_time, Some(480));
    assert_eq!(report.tracks[0].samples.len(), 2);
    assert_eq!(report.tracks[0].samples[0].decode_time, 0);
    assert_eq!(report.tracks[0].samples[1].decode_time, 480);
    assert_eq!(report.tracks[0].samples[0].previous_decode_delta, None);
    assert_eq!(report.tracks[0].samples[1].previous_decode_delta, Some(480));
    assert_eq!(report.tracks[0].samples[0].presentation_time, 0);
    assert_eq!(report.tracks[0].samples[1].presentation_end_time, 960);
    assert_eq!(
        report.tracks[0].samples[0].previous_presentation_delta,
        None
    );
    assert_eq!(
        report.tracks[0].samples[1].previous_presentation_delta,
        Some(480)
    );
}

#[test]
fn inspect_direct_ingest_path_round_trips_generated_nhml_sidecar() {
    let input = write_test_ogg_opus_file("inspect-nhml-roundtrip", &[b"abc", b"def"]);
    let report = inspect_direct_ingest_path(&input).unwrap();
    let mut rendered = Vec::new();
    write_report(&mut rendered, &report, DirectIngestReportFormat::Nhml).unwrap();
    let sidecar = write_temp_file_with_extension("inspect-nhml-roundtrip", "nhml", &rendered);

    let sidecar_report = inspect_direct_ingest_path(&sidecar).unwrap();
    assert_eq!(
        sidecar_report.detected_kind,
        DirectIngestDetectedKind::Container {
            container: "nhml".to_string(),
        }
    );
    assert!(sidecar_report.supports_flat_mux);
    assert_eq!(sidecar_report.staged_sources, report.staged_sources);
    assert_eq!(sidecar_report.tracks, report.tracks);
}

#[test]
fn inspect_direct_ingest_packets_flattens_real_ogg_opus_tracks() {
    let input = write_test_ogg_opus_file("inspect-packets-ogg-opus-input", &[b"abc", b"def"]);

    let report = inspect_direct_ingest_packets(&input).unwrap();

    assert!(report.supports_flat_mux);
    assert_eq!(report.track_count, 1);
    assert_eq!(report.packet_count, 2);
    assert_eq!(report.packets[0].previous_decode_delta, None);
    assert_eq!(report.packets[1].previous_decode_delta, Some(480));
    assert_ne!(report.packets[0].payload_crc32, 0);
    assert_ne!(report.packets[1].payload_crc32, 0);
    assert_eq!(report.sync_packet_count, 2);
    assert!(report.starts_with_sync_packet);
    assert_eq!(report.total_payload_size, 8);
    assert_eq!(report.minimum_packet_size, Some(4));
    assert_eq!(report.maximum_packet_size, Some(4));
    assert_eq!(report.minimum_sync_packet_size, Some(4));
    assert_eq!(report.maximum_sync_packet_size, Some(4));
    assert_eq!(report.average_sync_packet_size, Some(4));
    assert_eq!(report.average_non_sync_packet_size, None);
    assert_eq!(report.minimum_packet_duration, Some(480));
    assert_eq!(report.maximum_packet_duration, Some(480));
    assert_eq!(report.minimum_previous_decode_delta, Some(480));
    assert_eq!(report.maximum_previous_decode_delta, Some(480));
    assert_eq!(report.minimum_composition_time_offset, Some(0));
    assert_eq!(report.maximum_composition_time_offset, Some(0));
    assert_eq!(report.minimum_presentation_time, Some(0));
    assert_eq!(report.maximum_presentation_end_time, Some(960));
    assert_eq!(report.minimum_previous_presentation_delta, Some(480));
    assert_eq!(report.maximum_previous_presentation_delta, Some(480));
    assert_eq!(report.presentation_gap_count, 0);
    assert_eq!(report.presentation_overlap_count, 0);
    assert_eq!(report.presentation_regression_count, 0);
    assert_eq!(report.duration_change_count, 0);
    assert_eq!(report.composition_time_offset_change_count, 0);
    assert_eq!(report.minimum_sync_packet_distance, Some(1));
    assert_eq!(report.maximum_sync_packet_distance, Some(1));
    assert_eq!(report.average_sync_packet_distance, Some(1));
    assert_eq!(report.minimum_sync_packet_decode_delta, Some(480));
    assert_eq!(report.maximum_sync_packet_decode_delta, Some(480));
    assert_eq!(report.average_sync_packet_decode_delta, Some(480));
    assert_eq!(report.first_sync_packet_track_id, Some(1));
    assert_eq!(report.first_sync_packet_index, Some(0));
    assert_eq!(report.last_sync_packet_track_id, Some(1));
    assert_eq!(report.last_sync_packet_index, Some(1));
    assert_eq!(report.first_sync_decode_time, Some(0));
    assert_eq!(report.last_sync_decode_time, Some(480));
    assert_eq!(report.first_sync_presentation_time, Some(0));
    assert_eq!(report.last_sync_presentation_time, Some(480));
    assert_eq!(report.staged_sources.len(), 1);
    assert_eq!(
        report.staged_sources[0]
            .segments
            .as_ref()
            .map(|segments| segments.len()),
        report.staged_sources[0].segment_count
    );
    assert_eq!(report.packets.len(), 2);
    assert_eq!(report.packets[0].track_kind, "audio");
    assert_eq!(report.packets[0].sample_entry_type, "Opus");
    assert_eq!(report.packets[0].packet_index, 0);
    assert_eq!(report.packets[1].packet_index, 1);
    assert_eq!(report.packets[0].decode_time, 0);
    assert_eq!(report.packets[1].decode_time, 480);
    assert_eq!(report.packets[0].presentation_time, 0);
    assert_eq!(report.packets[1].presentation_end_time, 960);
    assert_eq!(report.packets[0].previous_presentation_delta, None);
    assert_eq!(report.packets[1].previous_presentation_delta, Some(480));
}

#[test]
fn inspect_direct_ingest_packets_round_trips_generated_nhnt_sidecar() {
    let input = write_test_ogg_opus_file("inspect-nhnt-roundtrip", &[b"abc", b"def"]);
    let report = inspect_direct_ingest_packets(&input).unwrap();
    let mut rendered = Vec::new();
    write_packet_report(&mut rendered, &report, DirectIngestReportFormat::Nhnt).unwrap();
    let sidecar = write_temp_file_with_extension("inspect-nhnt-roundtrip", "nhnt", &rendered);

    let sidecar_report = inspect_direct_ingest_packets(&sidecar).unwrap();
    assert_eq!(
        sidecar_report.detected_kind,
        DirectIngestDetectedKind::Container {
            container: "nhnt".to_string(),
        }
    );
    assert!(sidecar_report.supports_flat_mux);
    assert_eq!(sidecar_report.staged_sources, report.staged_sources);
    assert_eq!(sidecar_report.tracks, report.tracks);
    assert_eq!(sidecar_report.packets, report.packets);
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inspect_direct_ingest_path_async_matches_sync_for_real_ogg_opus_tracks() {
    let input = write_test_ogg_opus_file("inspect-ogg-opus-async-input", &[b"abc", b"def"]);

    let sync_report = inspect_direct_ingest_path(&input).unwrap();
    let async_report = mp4forge::mux::inspect::inspect_direct_ingest_path_async(&input)
        .await
        .unwrap();

    assert_eq!(async_report, sync_report);
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inspect_direct_ingest_packets_async_matches_sync_for_real_ogg_opus_tracks() {
    let input = write_test_ogg_opus_file("inspect-packets-ogg-opus-async-input", &[b"abc", b"def"]);

    let sync_report = inspect_direct_ingest_packets(&input).unwrap();
    let async_report = mp4forge::mux::inspect::inspect_direct_ingest_packets_async(&input)
        .await
        .unwrap();

    assert_eq!(async_report, sync_report);
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inspect_direct_ingest_path_async_matches_sync_for_vobsub_sidecars() {
    let (_idx_input, sub_input) =
        write_test_vobsub_files("inspect-vobsub-async-input", &[1_000], &[b"\xDE\xAD"]);

    let sync_report = inspect_direct_ingest_path(&sub_input).unwrap();
    let async_report = mp4forge::mux::inspect::inspect_direct_ingest_path_async(&sub_input)
        .await
        .unwrap();

    assert_eq!(async_report, sync_report);
}
