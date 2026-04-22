#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;

use mp4forge::boxes::iso14496_12::{Ftyp, Moov, Mvhd};
use mp4forge::cli::probe::{
    self, CodecDetailedProbeReport, CodecDetailedProbeTrackReport, DetailedProbeReport,
    DetailedProbeTrackReport, MediaCharacteristicsProbeReport,
    MediaCharacteristicsProbeTrackReport, ProbeFormat, ProbeReport, ProbeReportOptions,
    ProbeTrackReport,
};
use mp4forge::probe::{
    Av1CodecDetails, ColorInfo, DeclaredBitrateInfo, FieldOrderInfo, PixelAspectRatioInfo,
    TrackCodecDetails, TrackMediaCharacteristics,
};

use support::{
    encode_supported_box, fixture_path, fourcc, normalize_text, read_golden, write_temp_file,
};

#[test]
fn probe_report_renders_json_and_yaml_with_stable_field_order() {
    let report = ProbeReport {
        major_brand: "isom".to_string(),
        minor_version: 512,
        compatible_brands: vec!["isom".to_string(), "iso2".to_string(), "avc1".to_string()],
        fast_start: true,
        timescale: 1_000,
        duration: 2_000,
        duration_seconds: 2.0,
        tracks: vec![
            ProbeTrackReport {
                track_id: 1,
                timescale: 90_000,
                duration: 3_072,
                duration_seconds: 0.034133334,
                codec: "avc1.64001F".to_string(),
                encrypted: false,
                width: Some(320),
                height: Some(180),
                sample_num: Some(3),
                chunk_num: Some(2),
                idr_frame_num: Some(1),
                bitrate: Some(20_000),
                max_bitrate: Some(32_000),
            },
            ProbeTrackReport {
                track_id: 2,
                timescale: 48_000,
                duration: 2_048,
                duration_seconds: 0.042666666,
                codec: "mp4a.40.2".to_string(),
                encrypted: false,
                width: None,
                height: None,
                sample_num: Some(2),
                chunk_num: Some(2),
                idr_frame_num: None,
                bitrate: Some(15_000),
                max_bitrate: Some(18_000),
            },
        ],
    };

    let mut json = Vec::new();
    probe::write_report(&mut json, &report, ProbeFormat::Json).unwrap();
    assert_eq!(
        String::from_utf8(json).unwrap(),
        concat!(
            "{\n",
            "  \"MajorBrand\": \"isom\",\n",
            "  \"MinorVersion\": 512,\n",
            "  \"CompatibleBrands\": [\n",
            "    \"isom\",\n",
            "    \"iso2\",\n",
            "    \"avc1\"\n",
            "  ],\n",
            "  \"FastStart\": true,\n",
            "  \"Timescale\": 1000,\n",
            "  \"Duration\": 2000,\n",
            "  \"DurationSeconds\": 2,\n",
            "  \"Tracks\": [\n",
            "    {\n",
            "      \"TrackID\": 1,\n",
            "      \"Timescale\": 90000,\n",
            "      \"Duration\": 3072,\n",
            "      \"DurationSeconds\": 0.034133,\n",
            "      \"Codec\": \"avc1.64001F\",\n",
            "      \"Encrypted\": false,\n",
            "      \"Width\": 320,\n",
            "      \"Height\": 180,\n",
            "      \"SampleNum\": 3,\n",
            "      \"ChunkNum\": 2,\n",
            "      \"IDRFrameNum\": 1,\n",
            "      \"Bitrate\": 20000,\n",
            "      \"MaxBitrate\": 32000\n",
            "  },\n",
            "    {\n",
            "      \"TrackID\": 2,\n",
            "      \"Timescale\": 48000,\n",
            "      \"Duration\": 2048,\n",
            "      \"DurationSeconds\": 0.042667,\n",
            "      \"Codec\": \"mp4a.40.2\",\n",
            "      \"Encrypted\": false,\n",
            "      \"SampleNum\": 2,\n",
            "      \"ChunkNum\": 2,\n",
            "      \"Bitrate\": 15000,\n",
            "      \"MaxBitrate\": 18000\n",
            "  }\n",
            "  ]\n",
            "}\n"
        )
    );

    let mut yaml = Vec::new();
    probe::write_report(&mut yaml, &report, ProbeFormat::Yaml).unwrap();
    assert_eq!(
        String::from_utf8(yaml).unwrap(),
        concat!(
            "major_brand: isom\n",
            "minor_version: 512\n",
            "compatible_brands:\n",
            "- isom\n",
            "- iso2\n",
            "- avc1\n",
            "fast_start: true\n",
            "timescale: 1000\n",
            "duration: 2000\n",
            "duration_seconds: 2\n",
            "tracks:\n",
            "- track_id: 1\n",
            "  timescale: 90000\n",
            "  duration: 3072\n",
            "  duration_seconds: 0.034133\n",
            "  codec: avc1.64001F\n",
            "  encrypted: false\n",
            "  width: 320\n",
            "  height: 180\n",
            "  sample_num: 3\n",
            "  chunk_num: 2\n",
            "  idr_frame_num: 1\n",
            "  bitrate: 20000\n",
            "  max_bitrate: 32000\n",
            "- track_id: 2\n",
            "  timescale: 48000\n",
            "  duration: 2048\n",
            "  duration_seconds: 0.042667\n",
            "  codec: mp4a.40.2\n",
            "  encrypted: false\n",
            "  sample_num: 2\n",
            "  chunk_num: 2\n",
            "  bitrate: 15000\n",
            "  max_bitrate: 18000\n"
        )
    );
}

#[test]
fn detailed_probe_report_renders_json_and_yaml_with_stable_field_order() {
    let report = DetailedProbeReport {
        major_brand: "isom".to_string(),
        minor_version: 512,
        compatible_brands: vec!["isom".to_string(), "iso8".to_string(), "av01".to_string()],
        fast_start: true,
        timescale: 1_000,
        duration: 2_000,
        duration_seconds: 2.0,
        tracks: vec![DetailedProbeTrackReport {
            track_id: 1,
            timescale: 1_000,
            duration: 1_000,
            duration_seconds: 1.0,
            codec: "av01".to_string(),
            codec_family: "av1".to_string(),
            encrypted: false,
            handler_type: Some("vide".to_string()),
            language: Some("eng".to_string()),
            sample_entry_type: Some("av01".to_string()),
            original_format: None,
            protection_scheme_type: None,
            protection_scheme_version: None,
            width: Some(640),
            height: Some(360),
            channel_count: None,
            sample_rate: None,
            sample_num: Some(1),
            chunk_num: Some(1),
            idr_frame_num: None,
            bitrate: Some(32_000),
            max_bitrate: Some(32_000),
        }],
    };

    let mut json = Vec::new();
    probe::write_detailed_report(&mut json, &report, ProbeFormat::Json).unwrap();
    assert_eq!(
        String::from_utf8(json).unwrap(),
        concat!(
            "{\n",
            "  \"MajorBrand\": \"isom\",\n",
            "  \"MinorVersion\": 512,\n",
            "  \"CompatibleBrands\": [\n",
            "    \"isom\",\n",
            "    \"iso8\",\n",
            "    \"av01\"\n",
            "  ],\n",
            "  \"FastStart\": true,\n",
            "  \"Timescale\": 1000,\n",
            "  \"Duration\": 2000,\n",
            "  \"DurationSeconds\": 2,\n",
            "  \"Tracks\": [\n",
            "    {\n",
            "      \"TrackID\": 1,\n",
            "      \"Timescale\": 1000,\n",
            "      \"Duration\": 1000,\n",
            "      \"DurationSeconds\": 1,\n",
            "      \"Codec\": \"av01\",\n",
            "      \"CodecFamily\": \"av1\",\n",
            "      \"Encrypted\": false,\n",
            "      \"HandlerType\": \"vide\",\n",
            "      \"Language\": \"eng\",\n",
            "      \"SampleEntryType\": \"av01\",\n",
            "      \"Width\": 640,\n",
            "      \"Height\": 360,\n",
            "      \"SampleNum\": 1,\n",
            "      \"ChunkNum\": 1,\n",
            "      \"Bitrate\": 32000,\n",
            "      \"MaxBitrate\": 32000\n",
            "  }\n",
            "  ]\n",
            "}\n"
        )
    );

    let mut yaml = Vec::new();
    probe::write_detailed_report(&mut yaml, &report, ProbeFormat::Yaml).unwrap();
    assert_eq!(
        String::from_utf8(yaml).unwrap(),
        concat!(
            "major_brand: isom\n",
            "minor_version: 512\n",
            "compatible_brands:\n",
            "- isom\n",
            "- iso8\n",
            "- av01\n",
            "fast_start: true\n",
            "timescale: 1000\n",
            "duration: 2000\n",
            "duration_seconds: 2\n",
            "tracks:\n",
            "- track_id: 1\n",
            "  timescale: 1000\n",
            "  duration: 1000\n",
            "  duration_seconds: 1\n",
            "  codec: av01\n",
            "  codec_family: av1\n",
            "  encrypted: false\n",
            "  handler_type: vide\n",
            "  language: eng\n",
            "  sample_entry_type: av01\n",
            "  width: 640\n",
            "  height: 360\n",
            "  sample_num: 1\n",
            "  chunk_num: 1\n",
            "  bitrate: 32000\n",
            "  max_bitrate: 32000\n"
        )
    );
}

#[test]
fn codec_detailed_probe_report_renders_json_and_yaml_with_stable_field_order() {
    let report = CodecDetailedProbeReport {
        major_brand: "isom".to_string(),
        minor_version: 512,
        compatible_brands: vec!["isom".to_string(), "iso8".to_string(), "av01".to_string()],
        fast_start: true,
        timescale: 1_000,
        duration: 2_000,
        duration_seconds: 2.0,
        tracks: vec![CodecDetailedProbeTrackReport {
            track_id: 1,
            timescale: 1_000,
            duration: 1_000,
            duration_seconds: 1.0,
            codec: "av01".to_string(),
            codec_family: "av1".to_string(),
            codec_details: TrackCodecDetails::Av1(Av1CodecDetails {
                seq_profile: 0,
                seq_level_idx_0: 13,
                seq_tier_0: 1,
                bit_depth: 10,
                monochrome: false,
                chroma_subsampling_x: 1,
                chroma_subsampling_y: 0,
                chroma_sample_position: 2,
                initial_presentation_delay_minus_one: Some(3),
            }),
            encrypted: false,
            handler_type: Some("vide".to_string()),
            language: Some("eng".to_string()),
            sample_entry_type: Some("av01".to_string()),
            original_format: None,
            protection_scheme_type: None,
            protection_scheme_version: None,
            width: Some(640),
            height: Some(360),
            channel_count: None,
            sample_rate: None,
            sample_num: Some(1),
            chunk_num: Some(1),
            idr_frame_num: None,
            bitrate: Some(32_000),
            max_bitrate: Some(32_000),
        }],
    };

    let mut json = Vec::new();
    probe::write_codec_detailed_report(&mut json, &report, ProbeFormat::Json).unwrap();
    assert_eq!(
        String::from_utf8(json).unwrap(),
        concat!(
            "{\n",
            "  \"MajorBrand\": \"isom\",\n",
            "  \"MinorVersion\": 512,\n",
            "  \"CompatibleBrands\": [\n",
            "    \"isom\",\n",
            "    \"iso8\",\n",
            "    \"av01\"\n",
            "  ],\n",
            "  \"FastStart\": true,\n",
            "  \"Timescale\": 1000,\n",
            "  \"Duration\": 2000,\n",
            "  \"DurationSeconds\": 2,\n",
            "  \"Tracks\": [\n",
            "    {\n",
            "      \"TrackID\": 1,\n",
            "      \"Timescale\": 1000,\n",
            "      \"Duration\": 1000,\n",
            "      \"DurationSeconds\": 1,\n",
            "      \"Codec\": \"av01\",\n",
            "      \"CodecFamily\": \"av1\",\n",
            "      \"Encrypted\": false,\n",
            "      \"HandlerType\": \"vide\",\n",
            "      \"Language\": \"eng\",\n",
            "      \"SampleEntryType\": \"av01\",\n",
            "      \"Width\": 640,\n",
            "      \"Height\": 360,\n",
            "      \"SampleNum\": 1,\n",
            "      \"ChunkNum\": 1,\n",
            "      \"Bitrate\": 32000,\n",
            "      \"MaxBitrate\": 32000,\n",
            "      \"CodecDetails\": {\n",
            "        \"Kind\": \"av1\",\n",
            "        \"SeqProfile\": 0,\n",
            "        \"SeqLevelIdx0\": 13,\n",
            "        \"SeqTier0\": 1,\n",
            "        \"BitDepth\": 10,\n",
            "        \"Monochrome\": false,\n",
            "        \"ChromaSubsamplingX\": 1,\n",
            "        \"ChromaSubsamplingY\": 0,\n",
            "        \"ChromaSamplePosition\": 2,\n",
            "        \"InitialPresentationDelayMinusOne\": 3\n",
            "      }\n",
            "  }\n",
            "  ]\n",
            "}\n"
        )
    );

    let mut yaml = Vec::new();
    probe::write_codec_detailed_report(&mut yaml, &report, ProbeFormat::Yaml).unwrap();
    assert_eq!(
        String::from_utf8(yaml).unwrap(),
        concat!(
            "major_brand: isom\n",
            "minor_version: 512\n",
            "compatible_brands:\n",
            "- isom\n",
            "- iso8\n",
            "- av01\n",
            "fast_start: true\n",
            "timescale: 1000\n",
            "duration: 2000\n",
            "duration_seconds: 2\n",
            "tracks:\n",
            "- track_id: 1\n",
            "  timescale: 1000\n",
            "  duration: 1000\n",
            "  duration_seconds: 1\n",
            "  codec: av01\n",
            "  codec_family: av1\n",
            "  encrypted: false\n",
            "  handler_type: vide\n",
            "  language: eng\n",
            "  sample_entry_type: av01\n",
            "  width: 640\n",
            "  height: 360\n",
            "  sample_num: 1\n",
            "  chunk_num: 1\n",
            "  bitrate: 32000\n",
            "  max_bitrate: 32000\n",
            "  codec_details:\n",
            "    kind: av1\n",
            "    seq_profile: 0\n",
            "    seq_level_idx_0: 13\n",
            "    seq_tier_0: 1\n",
            "    bit_depth: 10\n",
            "    monochrome: false\n",
            "    chroma_subsampling_x: 1\n",
            "    chroma_subsampling_y: 0\n",
            "    chroma_sample_position: 2\n",
            "    initial_presentation_delay_minus_one: 3\n"
        )
    );
}

#[test]
fn media_characteristics_probe_report_renders_json_and_yaml_with_stable_field_order() {
    let report = MediaCharacteristicsProbeReport {
        major_brand: "isom".to_string(),
        minor_version: 512,
        compatible_brands: vec!["isom".to_string(), "iso8".to_string(), "avc1".to_string()],
        fast_start: true,
        timescale: 1_000,
        duration: 2_000,
        duration_seconds: 2.0,
        tracks: vec![MediaCharacteristicsProbeTrackReport {
            track_id: 1,
            timescale: 1_000,
            duration: 1_000,
            duration_seconds: 1.0,
            codec: "avc1.64001F".to_string(),
            codec_family: "avc".to_string(),
            codec_details: TrackCodecDetails::Unknown,
            media_characteristics: TrackMediaCharacteristics {
                declared_bitrate: Some(DeclaredBitrateInfo {
                    buffer_size_db: 32_768,
                    max_bitrate: 4_000_000,
                    avg_bitrate: 2_500_000,
                }),
                color: Some(ColorInfo {
                    colour_type: fourcc("nclx"),
                    colour_primaries: Some(9),
                    transfer_characteristics: Some(16),
                    matrix_coefficients: Some(9),
                    full_range: Some(true),
                    profile_size: None,
                    unknown_size: None,
                }),
                pixel_aspect_ratio: Some(PixelAspectRatioInfo {
                    h_spacing: 4,
                    v_spacing: 3,
                }),
                field_order: Some(FieldOrderInfo {
                    field_count: 2,
                    field_ordering: 6,
                    interlaced: true,
                }),
            },
            encrypted: false,
            handler_type: Some("vide".to_string()),
            language: Some("eng".to_string()),
            sample_entry_type: Some("avc1".to_string()),
            original_format: None,
            protection_scheme_type: None,
            protection_scheme_version: None,
            width: Some(640),
            height: Some(360),
            channel_count: None,
            sample_rate: None,
            sample_num: Some(1),
            chunk_num: Some(1),
            idr_frame_num: None,
            bitrate: Some(32_000),
            max_bitrate: Some(32_000),
        }],
    };

    let mut json = Vec::new();
    probe::write_media_characteristics_report(&mut json, &report, ProbeFormat::Json).unwrap();
    assert_eq!(
        String::from_utf8(json).unwrap(),
        concat!(
            "{\n",
            "  \"MajorBrand\": \"isom\",\n",
            "  \"MinorVersion\": 512,\n",
            "  \"CompatibleBrands\": [\n",
            "    \"isom\",\n",
            "    \"iso8\",\n",
            "    \"avc1\"\n",
            "  ],\n",
            "  \"FastStart\": true,\n",
            "  \"Timescale\": 1000,\n",
            "  \"Duration\": 2000,\n",
            "  \"DurationSeconds\": 2,\n",
            "  \"Tracks\": [\n",
            "    {\n",
            "      \"TrackID\": 1,\n",
            "      \"Timescale\": 1000,\n",
            "      \"Duration\": 1000,\n",
            "      \"DurationSeconds\": 1,\n",
            "      \"Codec\": \"avc1.64001F\",\n",
            "      \"CodecFamily\": \"avc\",\n",
            "      \"Encrypted\": false,\n",
            "      \"HandlerType\": \"vide\",\n",
            "      \"Language\": \"eng\",\n",
            "      \"SampleEntryType\": \"avc1\",\n",
            "      \"Width\": 640,\n",
            "      \"Height\": 360,\n",
            "      \"SampleNum\": 1,\n",
            "      \"ChunkNum\": 1,\n",
            "      \"Bitrate\": 32000,\n",
            "      \"MaxBitrate\": 32000,\n",
            "      \"CodecDetails\": {\n",
            "        \"Kind\": \"avc\"\n",
            "      },\n",
            "      \"MediaCharacteristics\": {\n",
            "        \"DeclaredBitrate\": {\n",
            "          \"BufferSizeDB\": 32768,\n",
            "          \"MaxBitrate\": 4000000,\n",
            "          \"AvgBitrate\": 2500000\n",
            "        },\n",
            "        \"Color\": {\n",
            "          \"ColourType\": \"nclx\",\n",
            "          \"ColourPrimaries\": 9,\n",
            "          \"TransferCharacteristics\": 16,\n",
            "          \"MatrixCoefficients\": 9,\n",
            "          \"FullRange\": true\n",
            "        },\n",
            "        \"PixelAspectRatio\": {\n",
            "          \"HSpacing\": 4,\n",
            "          \"VSpacing\": 3\n",
            "        },\n",
            "        \"FieldOrder\": {\n",
            "          \"FieldCount\": 2,\n",
            "          \"FieldOrdering\": 6,\n",
            "          \"Interlaced\": true\n",
            "        }\n",
            "      }\n",
            "  }\n",
            "  ]\n",
            "}\n"
        )
    );

    let mut yaml = Vec::new();
    probe::write_media_characteristics_report(&mut yaml, &report, ProbeFormat::Yaml).unwrap();
    assert_eq!(
        String::from_utf8(yaml).unwrap(),
        concat!(
            "major_brand: isom\n",
            "minor_version: 512\n",
            "compatible_brands:\n",
            "- isom\n",
            "- iso8\n",
            "- avc1\n",
            "fast_start: true\n",
            "timescale: 1000\n",
            "duration: 2000\n",
            "duration_seconds: 2\n",
            "tracks:\n",
            "- track_id: 1\n",
            "  timescale: 1000\n",
            "  duration: 1000\n",
            "  duration_seconds: 1\n",
            "  codec: avc1.64001F\n",
            "  codec_family: avc\n",
            "  encrypted: false\n",
            "  handler_type: vide\n",
            "  language: eng\n",
            "  sample_entry_type: avc1\n",
            "  width: 640\n",
            "  height: 360\n",
            "  sample_num: 1\n",
            "  chunk_num: 1\n",
            "  bitrate: 32000\n",
            "  max_bitrate: 32000\n",
            "  codec_details:\n",
            "    kind: avc\n",
            "  media_characteristics:\n",
            "    declared_bitrate:\n",
            "      buffer_size_db: 32768\n",
            "      max_bitrate: 4000000\n",
            "      avg_bitrate: 2500000\n",
            "    color:\n",
            "      colour_type: nclx\n",
            "      colour_primaries: 9\n",
            "      transfer_characteristics: 16\n",
            "      matrix_coefficients: 9\n",
            "      full_range: true\n",
            "    pixel_aspect_ratio:\n",
            "      h_spacing: 4\n",
            "      v_spacing: 3\n",
            "    field_order:\n",
            "      field_count: 2\n",
            "      field_ordering: 6\n",
            "      interlaced: true\n"
        )
    );
}

#[test]
fn probe_command_reads_a_file_and_honors_the_yaml_flag() {
    let path = write_temp_file("probe-cli", &build_probe_input_file());
    let args = vec![
        "-format".to_string(),
        "yaml".to_string(),
        path.to_string_lossy().into_owned(),
    ];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = probe::run(&args, &mut stdout, &mut stderr);

    let _ = fs::remove_file(&path);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        concat!(
            "major_brand: isom\n",
            "minor_version: 512\n",
            "compatible_brands:\n",
            "- isom\n",
            "- iso2\n",
            "fast_start: true\n",
            "timescale: 1000\n",
            "duration: 4096\n",
            "duration_seconds: 4.096\n",
            "tracks:\n"
        )
    );
}

#[test]
fn probe_command_matches_shared_fixture_goldens() {
    let fixture = fixture_path("sample.mp4");
    let cases: &[(&[&str], &str)] = &[
        (&[], "cli_probe/sample.json"),
        (&["-format", "json"], "cli_probe/sample.json"),
        (&["-format", "yaml"], "cli_probe/sample.yaml"),
        (&["-detail", "light"], "cli_probe/sample_light.json"),
        (
            &["-detail", "light", "-format", "yaml"],
            "cli_probe/sample_light.yaml",
        ),
    ];

    for (options, golden) in cases {
        let mut args = options
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        args.push(fixture.to_string_lossy().into_owned());

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = probe::run(&args, &mut stdout, &mut stderr);

        assert_eq!(exit_code, 0, "fixture probe failed for {golden}");
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            "",
            "stderr for {golden}"
        );
        assert_eq!(
            normalize_text(&String::from_utf8(stdout).unwrap()),
            read_golden(golden),
            "golden mismatch for {golden}"
        );
    }
}

#[test]
fn probe_report_with_lightweight_options_omits_expensive_fields() {
    let mut file = fs::File::open(fixture_path("sample.mp4")).unwrap();
    let report = probe::build_codec_detailed_report_with_options(
        &mut file,
        ProbeReportOptions::lightweight(),
    )
    .unwrap();

    assert_eq!(report.major_brand, "isom");
    assert!(!report.fast_start);
    assert_eq!(report.tracks.len(), 2);

    let video = &report.tracks[0];
    assert_eq!(video.track_id, 1);
    assert_eq!(video.codec, "avc1.64000C");
    assert_eq!(video.codec_family, "avc");
    assert_eq!(video.width, Some(320));
    assert_eq!(video.height, Some(180));
    assert_eq!(video.sample_num, None);
    assert_eq!(video.chunk_num, None);
    assert_eq!(video.idr_frame_num, None);
    assert_eq!(video.bitrate, None);
    assert_eq!(video.max_bitrate, None);

    let audio = &report.tracks[1];
    assert_eq!(audio.track_id, 2);
    assert_eq!(audio.codec, "mp4a.40.2");
    assert_eq!(audio.codec_family, "mp4_audio");
    assert_eq!(audio.channel_count, Some(2));
    assert_eq!(audio.sample_rate, Some(44100));
    assert_eq!(audio.sample_num, None);
    assert_eq!(audio.chunk_num, None);
    assert_eq!(audio.idr_frame_num, None);
    assert_eq!(audio.bitrate, None);
    assert_eq!(audio.max_bitrate, None);
}

#[test]
fn probe_library_handles_quicktime_fixture() {
    let mut file = fs::File::open(fixture_path("sample_qt.mp4")).unwrap();
    let summary = mp4forge::probe::probe(&mut file).unwrap();

    assert_eq!(summary.major_brand.to_string(), "qt  ");
    assert_eq!(summary.tracks.len(), 2);
    assert_eq!(summary.tracks[0].codec, mp4forge::probe::TrackCodec::Avc1);
    assert_eq!(summary.tracks[1].codec, mp4forge::probe::TrackCodec::Mp4a);
}

#[test]
fn probe_report_handles_quicktime_fixture() {
    let mut file = fs::File::open(fixture_path("sample_qt.mp4")).unwrap();
    let report = probe::build_report(&mut file).unwrap();

    assert_eq!(report.major_brand, "qt  ");
    assert_eq!(report.tracks.len(), 2);
    assert_eq!(report.tracks[0].codec, "avc1.42C01E");
    assert_eq!(report.tracks[1].codec, "mp4a.40.2");
    assert_eq!(report.tracks[0].idr_frame_num, None);
}

fn build_probe_input_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 512,
            compatible_brands: vec![fourcc("isom"), fourcc("iso2")],
        },
        &[],
    );

    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 4_096;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    let moov = encode_supported_box(&Moov, &encode_supported_box(&mvhd, &[]));

    [ftyp, moov].concat()
}
