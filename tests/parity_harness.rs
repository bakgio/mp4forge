mod support;

use std::fs;
use std::io::Cursor;
use std::path::Path;

use mp4forge::BoxInfo;
use mp4forge::FourCc;
use mp4forge::cli::{divide, edit, extract, probe as cli_probe, pssh};
use mp4forge::extract::extract_box;
use mp4forge::probe::{
    ProbeError, ProbeOptions, TrackCodec, average_sample_bitrate, average_segment_bitrate,
    find_idr_frames, max_sample_bitrate, max_segment_bitrate, probe, probe_with_options,
};
#[cfg(feature = "async")]
use mp4forge::probe::{find_idr_frames_async, probe_async, probe_with_options_async};
use mp4forge::sidx::{
    TopLevelSidxPlanOptions, apply_top_level_sidx_plan_bytes, plan_top_level_sidx_update_bytes,
};
use mp4forge::walk::BoxPath;
#[cfg(feature = "async")]
use tokio::fs as tokio_fs;

use support::{fixture_path, read_golden, read_text, temp_output_dir, write_temp_file};

struct FixtureExpectation {
    file_name: &'static str,
    major_brand: &'static str,
    compatible_brands: &'static [&'static str],
    fast_start: bool,
    timescale: u32,
    duration: u64,
    segment_count: usize,
    tracks: &'static [TrackExpectation],
}

struct TrackExpectation {
    track_id: u32,
    codec: TrackCodec,
    codec_string: &'static str,
    encrypted: bool,
    width: Option<u16>,
    height: Option<u16>,
    sample_num: Option<usize>,
    chunk_num: Option<usize>,
    idr_frame_num: Option<usize>,
}

#[test]
fn probe_report_matches_library_summary_across_shared_fixtures() {
    let fixtures = [
        FixtureExpectation {
            file_name: "sample.mp4",
            major_brand: "isom",
            compatible_brands: &["isom", "iso2", "avc1", "mp41"],
            fast_start: false,
            timescale: 1_000,
            duration: 1_024,
            segment_count: 0,
            tracks: &[
                TrackExpectation {
                    track_id: 1,
                    codec: TrackCodec::Avc1,
                    codec_string: "avc1.64000C",
                    encrypted: false,
                    width: Some(320),
                    height: Some(180),
                    sample_num: Some(10),
                    chunk_num: Some(9),
                    idr_frame_num: Some(1),
                },
                TrackExpectation {
                    track_id: 2,
                    codec: TrackCodec::Mp4a,
                    codec_string: "mp4a.40.2",
                    encrypted: false,
                    width: None,
                    height: None,
                    sample_num: Some(44),
                    chunk_num: Some(9),
                    idr_frame_num: None,
                },
            ],
        },
        FixtureExpectation {
            file_name: "sample_fragmented.mp4",
            major_brand: "iso5",
            compatible_brands: &["iso6", "mp41"],
            fast_start: true,
            timescale: 1_000,
            duration: 0,
            segment_count: 8,
            tracks: &[
                TrackExpectation {
                    track_id: 1,
                    codec: TrackCodec::Avc1,
                    codec_string: "avc1.4D401F",
                    encrypted: false,
                    width: Some(1280),
                    height: Some(720),
                    sample_num: None,
                    chunk_num: None,
                    idr_frame_num: None,
                },
                TrackExpectation {
                    track_id: 2,
                    codec: TrackCodec::Mp4a,
                    codec_string: "mp4a.40.2",
                    encrypted: false,
                    width: None,
                    height: None,
                    sample_num: None,
                    chunk_num: None,
                    idr_frame_num: None,
                },
            ],
        },
        FixtureExpectation {
            file_name: "sample_init.encv.mp4",
            major_brand: "iso5",
            compatible_brands: &["iso6", "mp41"],
            fast_start: true,
            timescale: 1_000,
            duration: 0,
            segment_count: 0,
            tracks: &[
                TrackExpectation {
                    track_id: 1,
                    codec: TrackCodec::Avc1,
                    codec_string: "avc1.4D401F",
                    encrypted: true,
                    width: Some(1280),
                    height: Some(720),
                    sample_num: None,
                    chunk_num: None,
                    idr_frame_num: None,
                },
                TrackExpectation {
                    track_id: 2,
                    codec: TrackCodec::Mp4a,
                    codec_string: "mp4a.40.2",
                    encrypted: false,
                    width: None,
                    height: None,
                    sample_num: None,
                    chunk_num: None,
                    idr_frame_num: None,
                },
            ],
        },
        FixtureExpectation {
            file_name: "sample_init.enca.mp4",
            major_brand: "iso5",
            compatible_brands: &["iso6", "mp41"],
            fast_start: true,
            timescale: 1_000,
            duration: 0,
            segment_count: 0,
            tracks: &[
                TrackExpectation {
                    track_id: 1,
                    codec: TrackCodec::Avc1,
                    codec_string: "avc1.4D401F",
                    encrypted: false,
                    width: Some(1280),
                    height: Some(720),
                    sample_num: None,
                    chunk_num: None,
                    idr_frame_num: None,
                },
                TrackExpectation {
                    track_id: 2,
                    codec: TrackCodec::Mp4a,
                    codec_string: "mp4a.40.2",
                    encrypted: true,
                    width: None,
                    height: None,
                    sample_num: None,
                    chunk_num: None,
                    idr_frame_num: None,
                },
            ],
        },
        FixtureExpectation {
            file_name: "sample_qt.mp4",
            major_brand: "qt  ",
            compatible_brands: &["qt  "],
            fast_start: true,
            timescale: 1_000,
            duration: 596_458,
            segment_count: 0,
            tracks: &[
                TrackExpectation {
                    track_id: 1,
                    codec: TrackCodec::Avc1,
                    codec_string: "avc1.42C01E",
                    encrypted: false,
                    width: Some(424),
                    height: Some(240),
                    sample_num: Some(14_315),
                    chunk_num: Some(14_315),
                    idr_frame_num: None,
                },
                TrackExpectation {
                    track_id: 2,
                    codec: TrackCodec::Mp4a,
                    codec_string: "mp4a.40.2",
                    encrypted: false,
                    width: None,
                    height: None,
                    sample_num: Some(27_958),
                    chunk_num: Some(27_958),
                    idr_frame_num: None,
                },
            ],
        },
    ];

    for fixture in fixtures {
        let path = fixture_path(fixture.file_name);

        let mut summary_file = fs::File::open(&path).unwrap();
        let summary = probe(&mut summary_file).unwrap();

        assert_eq!(summary.major_brand.to_string(), fixture.major_brand);
        assert_eq!(
            summary
                .compatible_brands
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            fixture.compatible_brands
        );
        assert_eq!(summary.fast_start, fixture.fast_start);
        assert_eq!(summary.timescale, fixture.timescale);
        assert_eq!(summary.duration, fixture.duration);
        assert_eq!(summary.segments.len(), fixture.segment_count);
        assert_eq!(summary.tracks.len(), fixture.tracks.len());

        let mut report_file = fs::File::open(&path).unwrap();
        let report = cli_probe::build_report(&mut report_file).unwrap();

        assert_eq!(report.major_brand, fixture.major_brand);
        assert_eq!(report.compatible_brands, fixture.compatible_brands);
        assert_eq!(report.fast_start, fixture.fast_start);
        assert_eq!(report.timescale, fixture.timescale);
        assert_eq!(report.duration, fixture.duration);
        assert_eq!(report.tracks.len(), fixture.tracks.len());

        for ((summary_track, report_track), expected_track) in summary
            .tracks
            .iter()
            .zip(report.tracks.iter())
            .zip(fixture.tracks.iter())
        {
            assert_eq!(summary_track.track_id, expected_track.track_id);
            assert_eq!(summary_track.codec, expected_track.codec);
            assert_eq!(summary_track.encrypted, expected_track.encrypted);
            assert_eq!(
                summary_track.avc.as_ref().map(|avc| avc.width),
                expected_track.width
            );
            assert_eq!(
                summary_track.avc.as_ref().map(|avc| avc.height),
                expected_track.height
            );
            assert_eq!(
                some_if_nonzero(summary_track.samples.len()),
                expected_track.sample_num
            );
            assert_eq!(
                some_if_nonzero(summary_track.chunks.len()),
                expected_track.chunk_num
            );

            assert_eq!(report_track.track_id, summary_track.track_id);
            assert_eq!(report_track.timescale, summary_track.timescale);
            assert_eq!(report_track.duration, summary_track.duration);
            assert_eq!(report_track.codec, expected_track.codec_string);
            assert_eq!(report_track.encrypted, summary_track.encrypted);
            assert_eq!(report_track.width, expected_track.width);
            assert_eq!(report_track.height, expected_track.height);
            assert_eq!(report_track.sample_num, expected_track.sample_num);
            assert_eq!(report_track.chunk_num, expected_track.chunk_num);
            assert_eq!(
                report_track.bitrate,
                expected_bitrate(&summary, summary_track).map(|(bitrate, _)| bitrate)
            );
            assert_eq!(
                report_track.max_bitrate,
                expected_bitrate(&summary, summary_track).map(|(_, max_bitrate)| max_bitrate)
            );

            let mut idr_file = fs::File::open(&path).unwrap();
            let expected_idr = match find_idr_frames(&mut idr_file, summary_track) {
                Ok(indices) => some_if_nonzero(indices.len()),
                Err(ProbeError::Io(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    None
                }
                Err(error) => panic!(
                    "unexpected IDR scan failure for {} track {}: {error}",
                    fixture.file_name, summary_track.track_id
                ),
            };
            assert_eq!(expected_idr, expected_track.idr_frame_num);
            assert_eq!(report_track.idr_frame_num, expected_idr);
        }
    }
}

#[test]
fn lightweight_probe_report_matches_library_summary_across_representative_fixtures() {
    for file_name in ["sample.mp4", "sample_fragmented.mp4", "sample_qt.mp4"] {
        let path = fixture_path(file_name);

        let mut summary_file = fs::File::open(&path).unwrap();
        let summary = probe_with_options(&mut summary_file, ProbeOptions::lightweight()).unwrap();

        let mut report_file = fs::File::open(&path).unwrap();
        let report = cli_probe::build_report_with_options(
            &mut report_file,
            cli_probe::ProbeReportOptions::lightweight(),
        )
        .unwrap();

        assert_eq!(report.major_brand, summary.major_brand.to_string());
        assert_eq!(
            report.compatible_brands,
            summary
                .compatible_brands
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
        assert_eq!(report.fast_start, summary.fast_start);
        assert_eq!(report.timescale, summary.timescale);
        assert_eq!(report.duration, summary.duration);
        assert!(summary.segments.is_empty());
        assert_eq!(report.tracks.len(), summary.tracks.len());

        for (summary_track, report_track) in summary.tracks.iter().zip(report.tracks.iter()) {
            assert!(summary_track.samples.is_empty());
            assert!(summary_track.chunks.is_empty());

            assert_eq!(report_track.track_id, summary_track.track_id);
            assert_eq!(report_track.timescale, summary_track.timescale);
            assert_eq!(report_track.duration, summary_track.duration);
            assert_eq!(report_track.encrypted, summary_track.encrypted);
            assert_eq!(
                report_track.width,
                summary_track.avc.as_ref().map(|avc| avc.width)
            );
            assert_eq!(
                report_track.height,
                summary_track.avc.as_ref().map(|avc| avc.height)
            );
            assert_eq!(report_track.sample_num, None);
            assert_eq!(report_track.chunk_num, None);
            assert_eq!(report_track.idr_frame_num, None);
            assert_eq!(report_track.bitrate, None);
            assert_eq!(report_track.max_bitrate, None);
        }
    }
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_probe_surfaces_match_sync_summaries_across_shared_fixtures() {
    for file_name in ["sample.mp4", "sample_fragmented.mp4", "sample_qt.mp4"] {
        let path = fixture_path(file_name);

        let expected = probe(&mut std::fs::File::open(&path).unwrap()).unwrap();
        let actual = probe_async(&mut tokio_fs::File::open(&path).await.unwrap())
            .await
            .unwrap();
        assert_eq!(actual, expected, "fixture={file_name}");

        let expected_lightweight = probe_with_options(
            &mut std::fs::File::open(&path).unwrap(),
            ProbeOptions::lightweight(),
        )
        .unwrap();
        let actual_lightweight = probe_with_options_async(
            &mut tokio_fs::File::open(&path).await.unwrap(),
            ProbeOptions::lightweight(),
        )
        .await
        .unwrap();
        assert_eq!(
            actual_lightweight, expected_lightweight,
            "fixture={file_name}"
        );
    }

    let sample_path = fixture_path("sample.mp4");
    let summary = probe(&mut std::fs::File::open(&sample_path).unwrap()).unwrap();
    let video_track = &summary.tracks[0];

    let expected_idr =
        find_idr_frames(&mut std::fs::File::open(&sample_path).unwrap(), video_track).unwrap();
    let actual_idr = find_idr_frames_async(
        &mut tokio_fs::File::open(&sample_path).await.unwrap(),
        video_track,
    )
    .await
    .unwrap();
    assert_eq!(actual_idr, expected_idr);
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_probe_file_helpers_can_run_on_tokio_worker_threads() {
    let sample_path = fixture_path("sample.mp4");
    let fragmented_path = fixture_path("sample_fragmented.mp4");
    let expected_summary = probe(&mut std::fs::File::open(&sample_path).unwrap()).unwrap();
    let expected_fragmented = probe_with_options(
        &mut std::fs::File::open(&fragmented_path).unwrap(),
        ProbeOptions::lightweight(),
    )
    .unwrap();

    let summary_handle = tokio::spawn(async move {
        let mut file = tokio_fs::File::open(&sample_path).await.unwrap();
        probe_async(&mut file).await.unwrap()
    });
    let fragmented_handle = tokio::spawn(async move {
        let mut file = tokio_fs::File::open(&fragmented_path).await.unwrap();
        probe_with_options_async(&mut file, ProbeOptions::lightweight())
            .await
            .unwrap()
    });

    assert_eq!(summary_handle.await.unwrap(), expected_summary);
    assert_eq!(fragmented_handle.await.unwrap(), expected_fragmented);
}

#[test]
fn extract_command_matches_library_box_boundaries_on_shared_fixtures() {
    let cases = [
        (
            "sample.mp4",
            "ftyp",
            BoxPath::from([fourcc("ftyp")]),
            fourcc("ftyp"),
            1_usize,
            32_u64,
        ),
        (
            "sample.mp4",
            "mdhd",
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("mdhd"),
            ]),
            fourcc("mdhd"),
            2,
            64,
        ),
        (
            "sample_fragmented.mp4",
            "trun",
            BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("trun")]),
            fourcc("trun"),
            8,
            452,
        ),
    ];

    for (file_name, box_type, path, expected_type, expected_count, expected_total_size) in cases {
        let fixture = fixture_path(file_name);
        let bytes = fs::read(&fixture).unwrap();
        let extracted = extract_box(&mut Cursor::new(bytes), None, path).unwrap();

        assert_eq!(extracted.len(), expected_count, "fixture={file_name}");
        assert!(
            extracted
                .iter()
                .all(|info| info.box_type() == expected_type)
        );
        assert_eq!(
            extracted.iter().map(BoxInfo::size).sum::<u64>(),
            expected_total_size,
            "fixture={file_name}"
        );

        let args = vec![box_type.to_string(), fixture.to_string_lossy().into_owned()];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = extract::run(&args, &mut stdout, &mut stderr);

        assert_eq!(exit_code, 0, "fixture={file_name} type={box_type}");
        assert_eq!(String::from_utf8(stderr).unwrap(), "");

        let cli_infos = parse_box_stream(&stdout);
        assert_eq!(cli_infos.len(), extracted.len(), "fixture={file_name}");
        assert_eq!(
            cli_infos.iter().map(BoxInfo::size).sum::<u64>(),
            expected_total_size,
            "fixture={file_name}"
        );
        assert!(
            cli_infos
                .iter()
                .all(|info| info.box_type() == expected_type)
        );
    }
}

#[test]
fn fragmented_and_encrypted_cli_surfaces_match_shared_fixture_expectations() {
    for fixture_name in ["sample_init.encv.mp4", "sample_init.enca.mp4"] {
        let args = vec![fixture_path(fixture_name).to_string_lossy().into_owned()];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = pssh::run(&args, &mut stdout, &mut stderr);

        assert_eq!(exit_code, 0, "fixture={fixture_name}");
        assert_eq!(String::from_utf8(stderr).unwrap(), "");
        assert_eq!(
            read_golden("cli_psshdump/sample_init.txt"),
            String::from_utf8(stdout).unwrap().replace("\r\n", "\n")
        );
    }

    let fragmented_path = fixture_path("sample_fragmented.mp4");
    let mut fragmented_file = fs::File::open(&fragmented_path).unwrap();
    let original_summary = probe(&mut fragmented_file).unwrap();

    let divide_output_dir = temp_output_dir("parity-harness-divide");
    let divide_args = vec![
        fragmented_path.to_string_lossy().into_owned(),
        divide_output_dir.to_string_lossy().into_owned(),
    ];
    let mut divide_stderr = Vec::new();
    let divide_exit_code = divide::run(&divide_args, &mut divide_stderr);

    assert_eq!(divide_exit_code, 0);
    assert_eq!(String::from_utf8(divide_stderr).unwrap(), "");
    assert_eq!(
        read_text(&divide_output_dir.join("playlist.m3u8")),
        read_golden("cli_divide/sample_fragmented/master.m3u8")
    );
    assert_eq!(
        read_text(&divide_output_dir.join("video").join("playlist.m3u8")),
        read_golden("cli_divide/sample_fragmented/video.m3u8")
    );
    assert_eq!(
        read_text(&divide_output_dir.join("audio").join("playlist.m3u8")),
        read_golden("cli_divide/sample_fragmented/audio.m3u8")
    );
    assert_eq!(
        media_segment_count(&divide_output_dir.join("video")),
        original_summary
            .segments
            .iter()
            .filter(|segment| segment.track_id == 1)
            .count()
    );
    assert_eq!(
        media_segment_count(&divide_output_dir.join("audio")),
        original_summary
            .segments
            .iter()
            .filter(|segment| segment.track_id == 2)
            .count()
    );

    let video_init = probe_file(&divide_output_dir.join("video").join("init.mp4"));
    assert_eq!(video_init.tracks.len(), 1);
    assert_eq!(video_init.tracks[0].track_id, 1);
    assert_eq!(video_init.tracks[0].codec, TrackCodec::Avc1);

    let audio_init = probe_file(&divide_output_dir.join("audio").join("init.mp4"));
    assert_eq!(audio_init.tracks.len(), 1);
    assert_eq!(audio_init.tracks[0].track_id, 2);
    assert_eq!(audio_init.tracks[0].codec, TrackCodec::Mp4a);

    let edit_output = write_temp_file("parity-harness-edit", &[]);
    let edit_args = vec![
        "-base_media_decode_time".to_string(),
        "123456".to_string(),
        "-drop".to_string(),
        "mfra".to_string(),
        fragmented_path.to_string_lossy().into_owned(),
        edit_output.to_string_lossy().into_owned(),
    ];
    let mut edit_stderr = Vec::new();
    let edit_exit_code = edit::run(&edit_args, &mut edit_stderr);

    assert_eq!(edit_exit_code, 0);
    assert_eq!(String::from_utf8(edit_stderr).unwrap(), "");

    let edited = fs::read(&edit_output).unwrap();
    let mut edited_reader = Cursor::new(edited.clone());
    let edited_summary = probe(&mut edited_reader).unwrap();
    let mfra = extract_box(
        &mut Cursor::new(edited),
        None,
        BoxPath::from([fourcc("mfra")]),
    )
    .unwrap();

    assert_eq!(edited_summary.tracks, original_summary.tracks);
    assert_eq!(
        edited_summary.segments.len(),
        original_summary.segments.len()
    );
    assert!(
        edited_summary
            .segments
            .iter()
            .all(|segment| segment.base_media_decode_time == 123_456)
    );
    assert!(mfra.is_empty());

    let _ = fs::remove_file(&edit_output);
    let _ = fs::remove_dir_all(&divide_output_dir);
}

#[test]
fn probe_surfaces_stay_stable_after_top_level_sidx_refresh_on_shared_fixture() {
    let input = fs::read(fixture_path("sample_fragmented.mp4")).unwrap();
    let original_summary = probe(&mut Cursor::new(&input)).unwrap();
    let original_report = cli_probe::build_report(&mut Cursor::new(&input)).unwrap();

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

    let refreshed_summary = probe(&mut Cursor::new(&output)).unwrap();
    let refreshed_report = cli_probe::build_report(&mut Cursor::new(&output)).unwrap();
    let sidx = extract_box(
        &mut Cursor::new(&output),
        None,
        BoxPath::from([fourcc("sidx")]),
    )
    .unwrap();

    assert_eq!(sidx.len(), 1);
    assert_eq!(original_report, refreshed_report);
    assert_eq!(original_summary.major_brand, refreshed_summary.major_brand);
    assert_eq!(
        original_summary.minor_version,
        refreshed_summary.minor_version
    );
    assert_eq!(
        original_summary.compatible_brands,
        refreshed_summary.compatible_brands
    );
    assert_eq!(original_summary.fast_start, refreshed_summary.fast_start);
    assert_eq!(original_summary.timescale, refreshed_summary.timescale);
    assert_eq!(original_summary.duration, refreshed_summary.duration);
    assert_eq!(original_summary.tracks, refreshed_summary.tracks);
    assert_eq!(
        original_summary.segments.len(),
        refreshed_summary.segments.len()
    );

    let offset_delta = sidx[0].size();
    for (original_segment, refreshed_segment) in original_summary
        .segments
        .iter()
        .zip(refreshed_summary.segments.iter())
    {
        assert_eq!(original_segment.track_id, refreshed_segment.track_id);
        assert_eq!(
            original_segment.moof_offset + offset_delta,
            refreshed_segment.moof_offset
        );
        assert_eq!(
            original_segment.base_media_decode_time,
            refreshed_segment.base_media_decode_time
        );
        assert_eq!(
            original_segment.default_sample_duration,
            refreshed_segment.default_sample_duration
        );
        assert_eq!(
            original_segment.sample_count,
            refreshed_segment.sample_count
        );
        assert_eq!(original_segment.duration, refreshed_segment.duration);
        assert_eq!(
            original_segment.composition_time_offset,
            refreshed_segment.composition_time_offset
        );
        assert_eq!(original_segment.size, refreshed_segment.size);
    }
}

fn expected_bitrate(
    summary: &mp4forge::probe::ProbeInfo,
    track: &mp4forge::probe::TrackInfo,
) -> Option<(u64, u64)> {
    let mut bitrate = average_sample_bitrate(&track.samples, track.timescale);
    let mut max_bitrate =
        max_sample_bitrate(&track.samples, track.timescale, track.timescale.into());
    if bitrate == 0 || max_bitrate == 0 {
        bitrate = average_segment_bitrate(&summary.segments, track.track_id, track.timescale);
        max_bitrate = max_segment_bitrate(&summary.segments, track.track_id, track.timescale);
    }

    let bitrate = some_if_nonzero(bitrate)?;
    let max_bitrate = some_if_nonzero(max_bitrate)?;
    Some((bitrate, max_bitrate))
}

fn media_segment_count(path: &Path) -> usize {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".mp4") && name != "init.mp4")
        .count()
}

fn probe_file(path: &Path) -> mp4forge::probe::ProbeInfo {
    let mut file = fs::File::open(path).unwrap();
    probe(&mut file).unwrap()
}

fn parse_box_stream(bytes: &[u8]) -> Vec<BoxInfo> {
    let mut reader = Cursor::new(bytes);
    let mut infos = Vec::new();
    while reader.position() < bytes.len() as u64 {
        let info = BoxInfo::read(&mut reader).unwrap();
        info.seek_to_end(&mut reader).unwrap();
        infos.push(info);
    }
    infos
}

fn some_if_nonzero<T>(value: T) -> Option<T>
where
    T: Default + PartialEq,
{
    if value == T::default() {
        None
    } else {
        Some(value)
    }
}

fn fourcc(value: &str) -> FourCc {
    FourCc::try_from(value).unwrap()
}
