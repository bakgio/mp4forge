#![cfg(feature = "mux")]

mod support;

use std::fs;
use std::io::Cursor;

use mp4forge::BoxInfo;
use mp4forge::boxes::iso14496_12::{
    AudioSampleEntry, Hdlr, Mdhd, Nmhd, SampleEntry, Sthd, VisualSampleEntry,
    XMLSubtitleSampleEntry,
};
use mp4forge::boxes::iso14496_30::{WVTTSampleEntry, WebVTTConfigurationBox, WebVTTSourceLabelBox};
use mp4forge::cli::{self, mux};
use mp4forge::mux::{MuxFileConfig, MuxTrackConfig};

use support::{
    TestMuxSample, encode_supported_box, fourcc, write_single_track_mp4_input, write_temp_file,
};

#[test]
fn mux_command_validates_argument_shape() {
    let mut stderr = Vec::new();
    assert_eq!(mux::run(&[], &mut stderr), 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        concat!(
            "USAGE: mp4forge mux --track <SPEC> [--track <SPEC> ...] [--layout <flat|fragmented>] [--segment_duration <SECONDS> | --fragment_duration <SECONDS>] OUTPUT\n",
            "\n",
            "OPTIONS:\n",
            "  --track <SPEC>                Add one mux input using the widened track-spec grammar\n",
            "                               Raw: <codec>:PATH[#key=value[,key=value...]]\n",
            "                               Some raw codecs require explicit layout parameters such as width/height or sample_rate/channel_count\n",
            "                               MP4: PATH.mp4#video, PATH.mp4#audio, PATH.mp4#audio:N, PATH.mp4#text, PATH.mp4#text:N, PATH.mp4#track:ID\n",
            "  --segment_duration <SECONDS> Set one target segment duration for supported single-input jobs\n",
            "  --fragment_duration <SECONDS> Set one target fragment duration for supported single-input jobs\n",
            "  --layout <flat|fragmented>   Choose the output container layout; defaults to flat\n",
            "\n",
            "The current mux command supports at most one video track plus one or more audio and text/subtitle tracks and always writes one explicit output MP4 file. Flat output rejects duration modes. Fragmented output currently requires exactly one duration mode.\n",
        )
    );
}

#[test]
fn mux_command_rejects_invalid_track_specs() {
    let output = write_temp_file("mux-cli-invalid-output", &[]);
    let args = vec![
        "--track".to_string(),
        "bad-spec".to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: invalid mux track spec `bad-spec`: expected `<codec>:PATH[#key=value[,key=value...]]` or `PATH.mp4#video`, `PATH.mp4#audio`, `PATH.mp4#audio:N`, `PATH.mp4#text`, `PATH.mp4#text:N`, or `PATH.mp4#track:ID`\n"
    );
}

#[test]
fn mux_command_rejects_conflicting_duration_flags() {
    let output = write_temp_file("mux-cli-conflict-output", &[]);
    let args = vec![
        "--track".to_string(),
        "aac:input.aac".to_string(),
        "--segment_duration".to_string(),
        "4".to_string(),
        "--fragment_duration".to_string(),
        "2".to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: --segment_duration and --fragment_duration may not be used together\n"
    );
}

#[test]
fn mux_command_rejects_duration_flags_for_flat_layout() {
    let output = write_temp_file("mux-cli-flat-layout-output", &[]);
    let args = vec![
        "--track".to_string(),
        "aac:input.aac".to_string(),
        "--fragment_duration".to_string(),
        "2".to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: invalid mux layout `flat`: flat output does not support `--fragment_duration`; use `--layout fragmented` instead\n"
    );
}

#[test]
fn mux_command_rejects_fragmented_layout_without_duration() {
    let output = write_temp_file("mux-cli-fragmented-missing-duration-output", &[]);
    let args = vec![
        "--track".to_string(),
        "aac:input.aac".to_string(),
        "--layout".to_string(),
        "fragmented".to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: invalid mux layout `fragmented`: fragmented output requires exactly one of `--segment_duration` or `--fragment_duration`\n"
    );
}

#[test]
fn mux_command_rejects_multiple_video_tracks() {
    let output = write_temp_file("mux-cli-multi-video-output", &[]);
    let args = vec![
        "--track".to_string(),
        "h264:first.h264".to_string(),
        "--track".to_string(),
        "h264:second.h264".to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: the current mux surface supports at most one video track per job, but 2 were requested\n"
    );
}

#[test]
fn mux_command_rejects_fragmented_multi_track_jobs() {
    let output = write_temp_file("mux-cli-fragmented-multi-track-output", &[]);
    let args = vec![
        "--track".to_string(),
        "aac:first.aac".to_string(),
        "--track".to_string(),
        "h264:second.h264".to_string(),
        "--layout".to_string(),
        "fragmented".to_string(),
        "--fragment_duration".to_string(),
        "1.0".to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: invalid mux layout `fragmented`: the current fragmented mux follow-on only supports single-track jobs\n"
    );
}

#[test]
fn mux_command_writes_real_mp4_output_from_mp4_tracks() {
    let audio_input = build_audio_input_file("mux-cli-audio-input", fourcc("dash"));
    let video_input = build_video_input_file("mux-cli-video-input", fourcc("isom"));
    let output = write_temp_file("mux-cli-output", &[]);
    let args = vec![
        "--track".to_string(),
        format!("{}#audio", audio_input.display()),
        "--track".to_string(),
        format!("{}#video", video_input.display()),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        root_boxes.iter().map(BoxInfo::box_type).collect::<Vec<_>>(),
        vec![fourcc("ftyp"), fourcc("moov"), fourcc("mdat")]
    );
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"audvideo");
}

#[test]
fn mux_command_writes_real_mp4_output_from_text_track_selectors() {
    let text_input = build_text_input_file("mux-cli-text-input", fourcc("isom"));
    let output = write_temp_file("mux-cli-text-output", &[]);
    let args = vec![
        "--track".to_string(),
        format!("{}#text", text_input.display()),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"wvtt");

    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
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
        mp4forge::walk::BoxPath::from([
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
fn mux_command_writes_fragmented_output_when_requested() {
    let audio_input = build_audio_input_file("mux-cli-fragmented-audio-input", fourcc("isom"));
    let output = write_temp_file("mux-cli-fragmented-output", &[]);
    let args = vec![
        "--track".to_string(),
        format!("{}#audio", audio_input.display()),
        "--layout".to_string(),
        "fragmented".to_string(),
        "--fragment_duration".to_string(),
        "0.015".to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        root_boxes.iter().map(BoxInfo::box_type).collect::<Vec<_>>(),
        vec![
            fourcc("ftyp"),
            fourcc("moov"),
            fourcc("sidx"),
            fourcc("moof"),
            fourcc("mdat"),
        ]
    );
}

#[test]
fn mux_command_writes_mixed_video_audio_subtitle_output_and_preserves_track_metadata() {
    let video_input = build_video_input_file_with_metadata(
        "mux-cli-mixed-video-input",
        fourcc("isom"),
        "avc1",
        *b"und",
        "PrimaryVideoHandler",
        b"video",
    );
    let audio_input = build_audio_input_file_with_metadata(
        "mux-cli-mixed-audio-input",
        fourcc("dash"),
        "mp4a",
        *b"eng",
        "EnglishAudioHandler",
        b"aud",
    );
    let text_input = build_mixed_text_input_file("mux-cli-mixed-text-input", fourcc("mp42"));
    let output = write_temp_file("mux-cli-mixed-output", &[]);
    let args = vec![
        "--track".to_string(),
        format!("{}#video", video_input.display()),
        "--track".to_string(),
        format!("{}#audio", audio_input.display()),
        "--track".to_string(),
        format!("{}#text", text_input.display()),
        "--track".to_string(),
        format!("{}#text:2", text_input.display()),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        b"videoaudwvttstpp"
    );

    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );
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
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    assert_eq!(
        mdhd_boxes
            .iter()
            .map(|box_value| decode_mdhd_language(box_value.language))
            .collect::<Vec<_>>(),
        vec![*b"und", *b"eng", *b"eng", *b"fra"]
    );

    let nmhd_boxes = extract_boxes::<Nmhd>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("nmhd"),
        ]),
    );
    assert_eq!(nmhd_boxes.len(), 1);

    let sthd_boxes = extract_boxes::<Sthd>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
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
fn mux_command_writes_real_mp4_output_from_broader_codec_track_selectors() {
    let audio_input =
        build_audio_input_file_with_type("mux-cli-alac-input", fourcc("dash"), "alac");
    let video_input =
        build_video_input_file_with_type("mux-cli-dvh1-input", fourcc("isom"), "dvh1");
    let output = write_temp_file("mux-cli-broader-output", &[]);
    let args = vec![
        "--track".to_string(),
        format!("{}#audio", audio_input.display()),
        "--track".to_string(),
        format!("{}#video", video_input.display()),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"alacdvh1");

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("alac"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("alac"));

    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("dvh1"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].sample_entry.box_type, fourcc("dvh1"));
}

#[test]
fn mux_command_writes_real_mp4_output_from_broader_raw_track_specs() {
    let audio_input = write_temp_file("mux-cli-raw-alac-input", b"alac");
    let video_input = write_temp_file("mux-cli-raw-av1-input", b"av01");
    let output = write_temp_file("mux-cli-raw-broader-output", &[]);
    let args = vec![
        "--track".to_string(),
        format!(
            "alac:{}#sample_rate=48000,channel_count=2,sample_duration=1024",
            audio_input.display()
        ),
        "--track".to_string(),
        format!(
            "av1:{}#width=640,height=360,timescale=1000,sample_duration=1000",
            video_input.display()
        ),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"alacav01");

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("alac"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("alac"));

    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("av01"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].sample_entry.box_type, fourcc("av01"));
}

#[test]
fn dispatch_routes_mux_command() {
    let audio_input = build_audio_input_file("mux-dispatch-audio-input", fourcc("dash"));
    let video_input = build_video_input_file("mux-dispatch-video-input", fourcc("isom"));
    let output = write_temp_file("mux-dispatch-output", &[]);
    let args = vec![
        "mux".to_string(),
        "--track".to_string(),
        format!("{}#audio", audio_input.display()),
        "--track".to_string(),
        format!("{}#video", video_input.display()),
        output.to_string_lossy().into_owned(),
    ];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = cli::dispatch(&args, &mut stdout, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        root_boxes.iter().map(BoxInfo::box_type).collect::<Vec<_>>(),
        vec![fourcc("ftyp"), fourcc("moov"), fourcc("mdat")]
    );
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"audvideo");
}

fn build_audio_input_file(prefix: &str, major_brand: mp4forge::FourCc) -> std::path::PathBuf {
    write_single_track_mp4_input(
        prefix,
        &MuxFileConfig::new(1_000)
            .with_major_brand(major_brand)
            .with_compatible_brand(fourcc("mp42")),
        MuxTrackConfig::new_audio(1, 1_000, audio_sample_entry_box()),
        &[TestMuxSample {
            bytes: b"aud",
            duration: 4,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    )
}

fn build_audio_input_file_with_type(
    prefix: &str,
    major_brand: mp4forge::FourCc,
    sample_entry_type: &str,
) -> std::path::PathBuf {
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
        &[TestMuxSample {
            bytes: sample_entry_type.as_bytes(),
            duration: 4,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    )
}

fn build_video_input_file(prefix: &str, major_brand: mp4forge::FourCc) -> std::path::PathBuf {
    write_single_track_mp4_input(
        prefix,
        &MuxFileConfig::new(1_000)
            .with_major_brand(major_brand)
            .with_compatible_brand(fourcc("mp42")),
        MuxTrackConfig::new_video(1, 1_000, 640, 360, video_sample_entry_box()),
        &[TestMuxSample {
            bytes: b"video",
            duration: 4,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    )
}

fn build_video_input_file_with_type(
    prefix: &str,
    major_brand: mp4forge::FourCc,
    sample_entry_type: &str,
) -> std::path::PathBuf {
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
        &[TestMuxSample {
            bytes: sample_entry_type.as_bytes(),
            duration: 4,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    )
}

fn build_text_input_file(prefix: &str, major_brand: mp4forge::FourCc) -> std::path::PathBuf {
    write_single_track_mp4_input(
        prefix,
        &MuxFileConfig::new(1_000)
            .with_major_brand(major_brand)
            .with_compatible_brand(fourcc("mp42")),
        MuxTrackConfig::new_text(1, 1_000, 0, 0, text_sample_entry_box()),
        &[TestMuxSample {
            bytes: b"wvtt",
            duration: 4,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    )
}

fn build_audio_input_file_with_metadata(
    prefix: &str,
    major_brand: mp4forge::FourCc,
    sample_entry_type: &str,
    language: [u8; 3],
    handler_name: &str,
    payload: &[u8],
) -> std::path::PathBuf {
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
        &[TestMuxSample {
            bytes: payload,
            duration: 4,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    )
}

fn build_video_input_file_with_metadata(
    prefix: &str,
    major_brand: mp4forge::FourCc,
    sample_entry_type: &str,
    language: [u8; 3],
    handler_name: &str,
    payload: &[u8],
) -> std::path::PathBuf {
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
        &[TestMuxSample {
            bytes: payload,
            duration: 4,
            composition_time_offset: 0,
            is_sync_sample: true,
        }],
    )
}

fn build_mixed_text_input_file(prefix: &str, major_brand: mp4forge::FourCc) -> std::path::PathBuf {
    let first_source = write_temp_file(&format!("{prefix}-source-text"), b"wvtt");
    let second_source = write_temp_file(&format!("{prefix}-source-subtitle"), b"stpp");
    let output_path = write_temp_file(prefix, &[]);
    let plan = mp4forge::mux::plan_staged_media_items(
        vec![
            mp4forge::mux::MuxStagedMediaItem::new(0, 1, 0, 10, 0, 4).with_sync_sample(true),
            mp4forge::mux::MuxStagedMediaItem::new(1, 2, 0, 10, 0, 4).with_sync_sample(true),
        ],
        mp4forge::mux::MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();
    let file_config = MuxFileConfig::new(1_000)
        .with_major_brand(major_brand)
        .with_compatible_brand(fourcc("mp42"));
    let track_configs = vec![
        MuxTrackConfig::new_text(1, 1_000, 0, 0, text_sample_entry_box())
            .with_language(*b"eng")
            .with_handler_name("EnglishCaptionHandler"),
        MuxTrackConfig::new_subtitle(2, 1_000, 0, 0, subtitle_sample_entry_box())
            .with_language(*b"fra")
            .with_handler_name("FrenchSubtitleHandler"),
    ];

    mp4forge::mux::write_mp4_mux_to_path(
        &[&first_source, &second_source],
        &output_path,
        &file_config,
        &track_configs,
        &plan,
    )
    .unwrap();
    output_path
}

fn audio_sample_entry_box() -> Vec<u8> {
    audio_sample_entry_box_with_type("mp4a")
}

fn audio_sample_entry_box_with_type(box_type: &str) -> Vec<u8> {
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
        &[],
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

fn text_sample_entry_box() -> Vec<u8> {
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

fn subtitle_sample_entry_box() -> Vec<u8> {
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

fn decode_mdhd_language(encoded: [u8; 3]) -> [u8; 3] {
    [encoded[0] + b'`', encoded[1] + b'`', encoded[2] + b'`']
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

fn extract_boxes<T>(bytes: &[u8], path: mp4forge::walk::BoxPath) -> Vec<T>
where
    T: mp4forge::codec::CodecBox + Clone + 'static,
{
    let mut reader = Cursor::new(bytes);
    mp4forge::extract::extract_box_as::<_, T>(&mut reader, None, path).unwrap()
}
