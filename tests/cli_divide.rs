#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;
use std::path::Path;

use mp4forge::boxes::AnyTypeBox;
use mp4forge::boxes::iso14496_12::{
    AVCDecoderConfiguration, Mdhd, Stco, Stsc, StscEntry, Stsd, Stsz, Stts, SttsEntry,
    TFHD_DEFAULT_SAMPLE_DURATION_PRESENT, TFHD_DEFAULT_SAMPLE_SIZE_PRESENT, Tfdt, Tfhd, Tkhd, Trun,
    VisualSampleEntry,
};
use mp4forge::cli::divide;
use mp4forge::codec::MutableBox;
use mp4forge::probe::{TrackCodec, probe};

use support::{
    encode_raw_box, encode_supported_box, fixture_path, fourcc, read_golden, read_text,
    temp_output_dir, write_temp_file,
};

#[test]
fn divide_command_writes_playlists_and_segments() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-input", &input);
    let output_dir = temp_output_dir("divide-output");
    let args = vec![
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr.clone()).unwrap(), "");

    let master_playlist = fs::read_to_string(output_dir.join("playlist.m3u8")).unwrap();
    let video_playlist =
        fs::read_to_string(output_dir.join("video").join("playlist.m3u8")).unwrap();
    let init = fs::read(output_dir.join("video").join("init.mp4")).unwrap();
    let segment0 = fs::read(output_dir.join("video").join("0.mp4")).unwrap();
    let segment1 = fs::read(output_dir.join("video").join("1.mp4")).unwrap();

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);

    assert_eq!(
        master_playlist,
        concat!(
            "#EXTM3U\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=128,CODECS=\"avc1.64001f,mp4a.40.2\",RESOLUTION=1920x1080\n",
            "video/playlist.m3u8\n"
        )
    );
    assert_eq!(
        video_playlist,
        concat!(
            "#EXTM3U\n",
            "#EXT-X-VERSION:7\n",
            "#EXT-X-TARGETDURATION:1\n",
            "#EXT-X-PLAYLIST-TYPE:VOD\n",
            "#EXT-X-MAP:URI=\"init.mp4\"\n",
            "#EXTINF:1.000000,\n",
            "0.mp4\n",
            "#EXTINF:1.000000,\n",
            "1.mp4\n",
            "#EXT-X-ENDLIST\n"
        )
    );
    assert!(init.windows(4).any(|window| window == b"ftyp"));
    assert!(init.windows(4).any(|window| window == b"moov"));
    assert!(segment0.windows(4).any(|window| window == b"moof"));
    assert!(segment0.windows(4).any(|window| window == b"mdat"));
    assert!(segment1.windows(4).any(|window| window == b"moof"));
    assert!(segment1.windows(4).any(|window| window == b"mdat"));
}

#[test]
fn divide_command_validates_argument_shape() {
    let mut stderr = Vec::new();
    assert_eq!(divide::run(&[], &mut stderr), 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "USAGE: mp4forge divide INPUT.mp4 OUTPUT_DIR\n"
    );
}

#[test]
fn divide_command_matches_shared_fragmented_fixture_outputs() {
    let input_path = fixture_path("sample_fragmented.mp4");
    let mut input = fs::File::open(&input_path).unwrap();
    let input_summary = probe(&mut input).unwrap();

    let output_dir = temp_output_dir("divide-fixture-output");
    let args = vec![
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr.clone()).unwrap(), "");

    assert_eq!(
        read_text(&output_dir.join("playlist.m3u8")),
        read_golden("cli_divide/sample_fragmented/master.m3u8")
    );
    assert_eq!(
        read_text(&output_dir.join("video").join("playlist.m3u8")),
        read_golden("cli_divide/sample_fragmented/video.m3u8")
    );
    assert_eq!(
        read_text(&output_dir.join("audio").join("playlist.m3u8")),
        read_golden("cli_divide/sample_fragmented/audio.m3u8")
    );

    assert_eq!(
        sorted_file_names(&output_dir.join("video")),
        [
            "0.mp4",
            "1.mp4",
            "2.mp4",
            "3.mp4",
            "init.mp4",
            "playlist.m3u8"
        ]
    );
    assert_eq!(
        sorted_file_names(&output_dir.join("audio")),
        [
            "0.mp4",
            "1.mp4",
            "2.mp4",
            "3.mp4",
            "init.mp4",
            "playlist.m3u8"
        ]
    );

    let video_init = probe_file(&output_dir.join("video").join("init.mp4"));
    assert_eq!(video_init.tracks.len(), 1);
    assert_eq!(video_init.tracks[0].track_id, 1);
    assert_eq!(video_init.tracks[0].codec, TrackCodec::Avc1);
    assert_eq!(video_init.tracks[0].avc.as_ref().unwrap().width, 1280);
    assert_eq!(video_init.tracks[0].avc.as_ref().unwrap().height, 720);
    assert!(video_init.segments.is_empty());

    let audio_init = probe_file(&output_dir.join("audio").join("init.mp4"));
    assert_eq!(audio_init.tracks.len(), 1);
    assert_eq!(audio_init.tracks[0].track_id, 2);
    assert_eq!(audio_init.tracks[0].codec, TrackCodec::Mp4a);
    assert!(audio_init.segments.is_empty());

    let expected_video = input_summary
        .segments
        .iter()
        .filter(|segment| segment.track_id == 1)
        .collect::<Vec<_>>();
    let expected_audio = input_summary
        .segments
        .iter()
        .filter(|segment| segment.track_id == 2)
        .collect::<Vec<_>>();

    for (index, expected) in expected_video.iter().enumerate() {
        assert_segment_matches(
            expected,
            &output_dir.join("video").join(format!("{index}.mp4")),
        );
    }
    for (index, expected) in expected_audio.iter().enumerate() {
        assert_segment_matches(
            expected,
            &output_dir.join("audio").join(format!("{index}.mp4")),
        );
    }

    let _ = fs::remove_dir_all(&output_dir);
}

fn build_divide_input_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &mp4forge::boxes::iso14496_12::Ftyp {
            major_brand: fourcc("iso6"),
            minor_version: 1,
            compatible_brands: vec![fourcc("iso6"), fourcc("dash")],
        },
        &[],
    );
    let moov = encode_raw_box(fourcc("moov"), &build_video_trak());
    let segment0 = build_video_segment(0);
    let segment1 = build_video_segment(1_000);
    [ftyp, moov, segment0, segment1].concat()
}

fn build_video_trak() -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = 1;
    tkhd.width = 1_920 << 16;
    tkhd.height = 1_080 << 16;

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.duration_v0 = 2_000;

    let avcc = encode_supported_box(
        &AVCDecoderConfiguration {
            configuration_version: 1,
            profile: 0x64,
            profile_compatibility: 0,
            level: 0x1f,
            length_size_minus_one: 3,
            ..AVCDecoderConfiguration::default()
        },
        &[],
    );

    let mut avc1 = VisualSampleEntry::default();
    avc1.set_box_type(fourcc("avc1"));
    avc1.sample_entry.data_reference_index = 1;
    avc1.width = 1_920;
    avc1.height = 1_080;
    avc1.horizresolution = 0x0048_0000;
    avc1.vertresolution = 0x0048_0000;
    avc1.frame_count = 1;
    avc1.depth = 0x0018;
    avc1.pre_defined3 = -1;

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(&stsd, &encode_supported_box(&avc1, &avcc));

    let mut stts = Stts::default();
    stts.entry_count = 1;
    stts.entries = vec![SttsEntry {
        sample_count: 1,
        sample_delta: 1_000,
    }];
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 1;
    stsc.entries = vec![StscEntry {
        first_chunk: 1,
        samples_per_chunk: 1,
        sample_description_index: 1,
    }];
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_size = 8;
    stsz.sample_count = 1;
    let stsz = encode_supported_box(&stsz, &[]);

    let mut stco = Stco::default();
    stco.entry_count = 0;
    let stco = encode_supported_box(&stco, &[]);

    let stbl = encode_raw_box(fourcc("stbl"), &[stsd, stts, stsc, stsz, stco].concat());
    let minf = encode_raw_box(fourcc("minf"), &stbl);
    let mdia = encode_raw_box(
        fourcc("mdia"),
        &[encode_supported_box(&mdhd, &[]), minf].concat(),
    );
    encode_raw_box(
        fourcc("trak"),
        &[encode_supported_box(&tkhd, &[]), mdia].concat(),
    )
}

fn build_video_segment(base_media_decode_time: u32) -> Vec<u8> {
    let mut tfhd = Tfhd::default();
    tfhd.track_id = 1;
    tfhd.default_sample_duration = 1_000;
    tfhd.default_sample_size = 8;
    tfhd.set_flags(TFHD_DEFAULT_SAMPLE_DURATION_PRESENT | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT);

    let mut tfdt = Tfdt::default();
    tfdt.base_media_decode_time_v0 = base_media_decode_time;

    let mut trun = Trun::default();
    trun.sample_count = 1;
    let trun = encode_supported_box(&trun, &[]);
    let traf = encode_raw_box(
        fourcc("traf"),
        &[
            encode_supported_box(&tfhd, &[]),
            encode_supported_box(&tfdt, &[]),
            trun,
        ]
        .concat(),
    );
    let moof = encode_raw_box(fourcc("moof"), &traf);
    let mdat = encode_raw_box(fourcc("mdat"), &[0, 1, 2, 3, 4, 5, 6, 7]);
    [moof, mdat].concat()
}

fn sorted_file_names(path: &Path) -> Vec<String> {
    let mut names = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn probe_file(path: &Path) -> mp4forge::probe::ProbeInfo {
    let mut file = fs::File::open(path).unwrap();
    probe(&mut file).unwrap()
}

fn assert_segment_matches(expected: &mp4forge::probe::SegmentInfo, path: &Path) {
    let summary = probe_file(path);
    assert!(
        summary.tracks.is_empty(),
        "segment file should not contain trak boxes"
    );
    assert_eq!(summary.segments.len(), 1);
    let actual = &summary.segments[0];
    assert_eq!(actual.moof_offset, 0);
    assert_eq!(actual.track_id, expected.track_id);
    assert_eq!(
        actual.base_media_decode_time,
        expected.base_media_decode_time
    );
    assert_eq!(
        actual.default_sample_duration,
        expected.default_sample_duration
    );
    assert_eq!(actual.sample_count, expected.sample_count);
    assert_eq!(actual.duration, expected.duration);
    assert_eq!(
        actual.composition_time_offset,
        expected.composition_time_offset
    );
    assert_eq!(actual.size, expected.size);
}
