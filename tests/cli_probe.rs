#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;

use mp4forge::boxes::iso14496_12::{Ftyp, Moov, Mvhd};
use mp4forge::cli::probe::{self, ProbeFormat, ProbeReport, ProbeTrackReport};

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
