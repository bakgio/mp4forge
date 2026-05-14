#![cfg(feature = "mux")]

#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;
use std::path::Path;

use mp4forge::boxes::AnyTypeBox;
use mp4forge::boxes::av1::AV1CodecConfiguration;
use mp4forge::boxes::etsi_ts_102_366::Dac3;
use mp4forge::boxes::iso14496_12::{
    AVCDecoderConfiguration, AudioSampleEntry, Frma, Ftyp, HEVCDecoderConfiguration, Mdhd,
    SampleEntry, Schm, Sinf, Stco, Stsc, StscEntry, Stsd, Stsz, Stts, SttsEntry,
    TFHD_DEFAULT_SAMPLE_DURATION_PRESENT, TFHD_DEFAULT_SAMPLE_SIZE_PRESENT,
    TRUN_SAMPLE_DURATION_PRESENT, TRUN_SAMPLE_SIZE_PRESENT, Tfdt, Tfhd, Tkhd, Trun, TrunEntry,
    VisualSampleEntry, XMLSubtitleSampleEntry,
};
use mp4forge::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    Esds,
};
use mp4forge::boxes::iso23001_5::PcmC;
use mp4forge::boxes::opus::DOps;
use mp4forge::boxes::vp::VpCodecConfiguration;
use mp4forge::cli::divide;
use mp4forge::codec::MutableBox;
use mp4forge::mux::{MuxRequest, MuxTrackSpec, mux_to_path};
use mp4forge::probe::{TrackCodec, probe, probe_detailed};

use support::{
    encode_raw_box, encode_supported_box, fixture_path, fourcc, read_golden, read_text,
    temp_output_dir, write_temp_file,
};

const DIVIDE_SCOPE_MESSAGE: &str = "divide currently supports fragmented inputs with at most one video track from AVC, HEVC, Dolby Vision on HEVC, AV1, VP8, or VP9 and one or more audio tracks from MP4A-based audio, Opus, AC-3, E-AC-3, AC-4, ALAC, DTS-family entries, FLAC, IAMF, MPEG-H, or PCM; subtitle and text tracks remain unsupported";

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
    let manifest = fs::read_to_string(output_dir.join("manifest.mpd")).unwrap();
    let init = fs::read(output_dir.join("video").join("init.mp4")).unwrap();
    let segment0 = fs::read(output_dir.join("video").join("0.mp4")).unwrap();
    let segment1 = fs::read(output_dir.join("video").join("1.mp4")).unwrap();

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);

    assert_eq!(
        master_playlist,
        concat!(
            "#EXTM3U\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=128,CODECS=\"avc1.64001f\",RESOLUTION=1920x1080\n",
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
    assert!(manifest.contains("<MPD "));
    assert!(manifest.contains("contentType=\"video\""));
    assert!(manifest.contains("media=\"video/$Number$.mp4\""));
    assert!(init.windows(4).any(|window| window == b"ftyp"));
    assert!(init.windows(4).any(|window| window == b"moov"));
    assert!(segment0.windows(4).any(|window| window == b"moof"));
    assert!(segment0.windows(4).any(|window| window == b"mdat"));
    assert!(segment1.windows(4).any(|window| window == b"moof"));
    assert!(segment1.windows(4).any(|window| window == b"mdat"));
}

#[test]
fn divide_command_writes_hls_only_when_requested() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-hls-only-input", &input);
    let output_dir = temp_output_dir("divide-hls-only-output");
    let args = vec![
        "-manifest".to_string(),
        "hls".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert!(output_dir.join("playlist.m3u8").is_file());
    assert!(!output_dir.join("manifest.mpd").exists());

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_writes_hls_with_base_url_event_type_and_start_offset() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-hls-base-url-input", &input);
    let output_dir = temp_output_dir("divide-hls-base-url-output");
    let args = vec![
        "-manifest".to_string(),
        "hls".to_string(),
        "-hls-base-url".to_string(),
        "https://cdn.example.invalid/hls/".to_string(),
        "-hls-playlist-type".to_string(),
        "event".to_string(),
        "-hls-start-time-offset".to_string(),
        "-3.5".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(
        read_text(&output_dir.join("playlist.m3u8")),
        concat!(
            "#EXTM3U\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=128,CODECS=\"avc1.64001f\",RESOLUTION=1920x1080\n",
            "https://cdn.example.invalid/hls/video/playlist.m3u8\n"
        )
    );
    assert_eq!(
        read_text(&output_dir.join("video").join("playlist.m3u8")),
        concat!(
            "#EXTM3U\n",
            "#EXT-X-VERSION:7\n",
            "#EXT-X-TARGETDURATION:1\n",
            "#EXT-X-PLAYLIST-TYPE:EVENT\n",
            "#EXT-X-START:TIME-OFFSET=-3.500000\n",
            "#EXT-X-MAP:URI=\"https://cdn.example.invalid/hls/video/init.mp4\"\n",
            "#EXTINF:1.000000,\n",
            "https://cdn.example.invalid/hls/video/0.mp4\n",
            "#EXTINF:1.000000,\n",
            "https://cdn.example.invalid/hls/video/1.mp4\n"
        )
    );
    assert!(!output_dir.join("manifest.mpd").exists());

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_writes_hls_program_date_time_when_requested() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-hls-program-date-time-input", &input);
    let output_dir = temp_output_dir("divide-hls-program-date-time-output");
    let args = vec![
        "-manifest".to_string(),
        "hls".to_string(),
        "-hls-program-date-time".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");

    let media_playlist = read_text(&output_dir.join("video").join("playlist.m3u8"));
    let lines = media_playlist.lines().collect::<Vec<_>>();
    assert!(lines[5].starts_with("#EXT-X-PROGRAM-DATE-TIME:"));
    assert_eq!(lines[6], "#EXTINF:1.000000,");
    assert!(lines[5].ends_with('Z'));

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_writes_manifest_name_overrides() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-manifest-name-input", &input);
    let output_dir = temp_output_dir("divide-manifest-name-output");
    let args = vec![
        "-hls-master-playlist-name".to_string(),
        "master.m3u8".to_string(),
        "-hls-media-playlist-name".to_string(),
        "media.m3u8".to_string(),
        "-dash-manifest-name".to_string(),
        "stream.mpd".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");

    assert!(output_dir.join("master.m3u8").is_file());
    assert!(!output_dir.join("playlist.m3u8").exists());
    assert!(output_dir.join("stream.mpd").is_file());
    assert!(!output_dir.join("manifest.mpd").exists());
    assert!(output_dir.join("video").join("media.m3u8").is_file());
    assert!(!output_dir.join("video").join("playlist.m3u8").exists());

    assert_eq!(
        read_text(&output_dir.join("master.m3u8")),
        concat!(
            "#EXTM3U\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=128,CODECS=\"avc1.64001f\",RESOLUTION=1920x1080\n",
            "video/media.m3u8\n"
        )
    );
    let manifest = read_text(&output_dir.join("stream.mpd"));
    assert!(manifest.contains("initialization=\"video/init.mp4\""));

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_writes_dynamic_dash_only_manifest_when_requested() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-dynamic-dash-input", &input);
    let output_dir = temp_output_dir("divide-dynamic-dash-output");
    let args = vec![
        "-manifest".to_string(),
        "dash".to_string(),
        "-dash-mode".to_string(),
        "dynamic".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let manifest = fs::read_to_string(output_dir.join("manifest.mpd")).unwrap();
    assert!(manifest.contains("type=\"dynamic\""));
    assert!(manifest.contains("minimumUpdatePeriod=\"PT5S\""));
    assert!(manifest.contains("availabilityStartTime=\""));
    assert!(manifest.contains("publishTime=\""));
    assert!(!manifest.contains("availabilityStartTime=\"1970-01-01T00:00:00Z\""));
    assert!(!manifest.contains("publishTime=\"1970-01-01T00:00:00Z\""));
    assert!(!manifest.contains("timeShiftBufferDepth="));
    assert!(!manifest.contains("suggestedPresentationDelay="));
    assert!(!manifest.contains("mediaPresentationDuration="));
    assert!(!output_dir.join("playlist.m3u8").exists());

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_writes_dynamic_dash_manifest_with_defaults_and_repeated_base_urls() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-dynamic-dash-defaults-input", &input);
    let output_dir = temp_output_dir("divide-dynamic-dash-defaults-output");
    let args = vec![
        "-manifest".to_string(),
        "dash".to_string(),
        "-dash-mode".to_string(),
        "dynamic".to_string(),
        "-dash-profile".to_string(),
        "live".to_string(),
        "-dash-base-url".to_string(),
        "https://cdn.example.invalid/root/".to_string(),
        "-dash-base-url".to_string(),
        "https://cdn-backup.example.invalid/root/".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let manifest = fs::read_to_string(output_dir.join("manifest.mpd")).unwrap();
    assert!(manifest.contains("type=\"dynamic\""));
    assert!(manifest.contains("profiles=\"urn:mpeg:dash:profile:isoff-live:2011\""));
    assert!(manifest.contains("<BaseURL>https://cdn.example.invalid/root/</BaseURL>"));
    assert!(manifest.contains("<BaseURL>https://cdn-backup.example.invalid/root/</BaseURL>"));
    assert!(manifest.contains("minBufferTime=\"PT2S\""));
    assert!(manifest.contains("minimumUpdatePeriod=\"PT5S\""));
    assert!(manifest.contains("availabilityStartTime=\""));
    assert!(manifest.contains("publishTime=\""));
    assert!(manifest.contains("<Period start=\"PT0S\">"));
    assert!(!manifest.contains("<Location>"));
    assert!(!manifest.contains("<UTCTiming "));
    assert!(!manifest.contains("suggestedPresentationDelay="));
    assert!(!manifest.contains("timeShiftBufferDepth="));
    assert!(!manifest.contains("availabilityStartTime=\"1970-01-01T00:00:00Z\""));
    assert!(!manifest.contains("publishTime=\"1970-01-01T00:00:00Z\""));

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_saves_and_reloads_dash_session_state() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-session-input", &input);
    let first_output_dir = temp_output_dir("divide-session-first-output");
    let session_path = first_output_dir.join("dash.session");
    let first_args = vec![
        "-manifest".to_string(),
        "dash".to_string(),
        "-dash-mode".to_string(),
        "dynamic".to_string(),
        "-dash-layout".to_string(),
        "list".to_string(),
        "-dash-profile".to_string(),
        "live".to_string(),
        "-dash-base-url".to_string(),
        "https://cdn.example.invalid/root/".to_string(),
        "-dash-base-url".to_string(),
        "https://cdn-backup.example.invalid/root/".to_string(),
        "-dash-session-save".to_string(),
        session_path.to_string_lossy().into_owned(),
        input_path.to_string_lossy().into_owned(),
        first_output_dir.to_string_lossy().into_owned(),
    ];

    let mut first_stderr = Vec::new();
    let first_exit_code = divide::run(&first_args, &mut first_stderr);
    assert_eq!(
        first_exit_code,
        0,
        "{}",
        String::from_utf8_lossy(&first_stderr)
    );
    assert_eq!(String::from_utf8(first_stderr).unwrap(), "");
    let session_text = fs::read_to_string(&session_path).unwrap();
    assert!(session_text.contains("manifest_selection=dash"));
    assert!(session_text.contains("dash_manifest_mode=dynamic"));
    assert!(session_text.contains("dash_manifest_layout=list"));
    assert!(session_text.contains("dash_manifest_profile=live"));
    assert!(session_text.contains("dash_base_url=https://cdn.example.invalid/root/"));
    assert!(session_text.contains("dash_base_url=https://cdn-backup.example.invalid/root/"));
    assert!(session_text.contains("next_period_start_micros=2000000"));
    assert!(session_text.contains("next_segment_index_track_1=2"));

    let second_output_dir = temp_output_dir("divide-session-second-output");
    let second_args = vec![
        "-dash-session-load".to_string(),
        session_path.to_string_lossy().into_owned(),
        input_path.to_string_lossy().into_owned(),
        second_output_dir.to_string_lossy().into_owned(),
    ];

    let mut second_stderr = Vec::new();
    let second_exit_code = divide::run(&second_args, &mut second_stderr);
    assert_eq!(
        second_exit_code,
        0,
        "{}",
        String::from_utf8_lossy(&second_stderr)
    );
    assert_eq!(String::from_utf8(second_stderr).unwrap(), "");
    let manifest = fs::read_to_string(second_output_dir.join("manifest.mpd")).unwrap();
    assert!(manifest.contains("type=\"dynamic\""));
    assert!(manifest.contains("profiles=\"urn:mpeg:dash:profile:isoff-live:2011\""));
    assert!(manifest.contains("<Period start=\"PT2S\">"));
    assert!(manifest.contains("<BaseURL>https://cdn.example.invalid/root/</BaseURL>"));
    assert!(manifest.contains("<BaseURL>https://cdn-backup.example.invalid/root/</BaseURL>"));
    assert!(manifest.contains("<SegmentList>"));
    assert!(manifest.contains("<SegmentURL media=\"video/2.mp4\" />"));
    assert!(manifest.contains("<SegmentURL media=\"video/3.mp4\" />"));
    assert!(second_output_dir.join("video").join("2.mp4").is_file());
    assert!(second_output_dir.join("video").join("3.mp4").is_file());
    assert!(!second_output_dir.join("video").join("0.mp4").exists());
    assert!(!second_output_dir.join("playlist.m3u8").exists());
    assert!(
        !second_output_dir
            .join("video")
            .join("playlist.m3u8")
            .exists()
    );

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&first_output_dir);
    let _ = fs::remove_dir_all(&second_output_dir);
}

#[test]
fn divide_command_session_reload_still_allows_manifest_override_with_continuity() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-session-override-input", &input);
    let first_output_dir = temp_output_dir("divide-session-override-first-output");
    let session_path = first_output_dir.join("dash.session");
    let first_args = vec![
        "-manifest".to_string(),
        "dash".to_string(),
        "-dash-mode".to_string(),
        "dynamic".to_string(),
        "-dash-layout".to_string(),
        "list".to_string(),
        "-dash-profile".to_string(),
        "live".to_string(),
        "-dash-session-save".to_string(),
        session_path.to_string_lossy().into_owned(),
        input_path.to_string_lossy().into_owned(),
        first_output_dir.to_string_lossy().into_owned(),
    ];

    let mut first_stderr = Vec::new();
    let first_exit_code = divide::run(&first_args, &mut first_stderr);
    assert_eq!(
        first_exit_code,
        0,
        "{}",
        String::from_utf8_lossy(&first_stderr)
    );
    assert_eq!(String::from_utf8(first_stderr).unwrap(), "");

    let second_output_dir = temp_output_dir("divide-session-override-second-output");
    let second_args = vec![
        "-manifest".to_string(),
        "both".to_string(),
        "-dash-session-load".to_string(),
        session_path.to_string_lossy().into_owned(),
        input_path.to_string_lossy().into_owned(),
        second_output_dir.to_string_lossy().into_owned(),
    ];

    let mut second_stderr = Vec::new();
    let second_exit_code = divide::run(&second_args, &mut second_stderr);
    assert_eq!(
        second_exit_code,
        0,
        "{}",
        String::from_utf8_lossy(&second_stderr)
    );
    assert_eq!(String::from_utf8(second_stderr).unwrap(), "");
    let manifest = fs::read_to_string(second_output_dir.join("manifest.mpd")).unwrap();
    let video_playlist =
        fs::read_to_string(second_output_dir.join("video").join("playlist.m3u8")).unwrap();
    assert!(manifest.contains("type=\"dynamic\""));
    assert!(manifest.contains("<Period start=\"PT2S\">"));
    assert!(manifest.contains("<SegmentList>"));
    assert!(manifest.contains("<SegmentURL media=\"video/2.mp4\" />"));
    assert!(manifest.contains("<SegmentURL media=\"video/3.mp4\" />"));
    assert!(video_playlist.contains("#EXT-X-MEDIA-SEQUENCE:2"));
    assert!(video_playlist.contains("\n2.mp4\n"));
    assert!(video_playlist.contains("\n3.mp4\n"));
    assert!(second_output_dir.join("playlist.m3u8").is_file());

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&first_output_dir);
    let _ = fs::remove_dir_all(&second_output_dir);
}

#[test]
fn divide_command_writes_dash_segment_list_when_requested() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-dash-list-input", &input);
    let output_dir = temp_output_dir("divide-dash-list-output");
    let args = vec![
        "-manifest".to_string(),
        "dash".to_string(),
        "-dash-layout".to_string(),
        "list".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let manifest = fs::read_to_string(output_dir.join("manifest.mpd")).unwrap();
    assert!(manifest.contains("<SegmentList>"));
    assert!(manifest.contains("<Initialization sourceURL=\"video/init.mp4\" />"));
    assert!(manifest.contains("<SegmentURL media=\"video/0.mp4\" />"));
    assert!(!manifest.contains("<SegmentTemplate"));
    assert!(!output_dir.join("playlist.m3u8").exists());

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_validates_argument_shape() {
    let mut stderr = Vec::new();
    assert_eq!(divide::run(&[], &mut stderr), 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        concat!(
            "USAGE: mp4forge divide [OPTIONS] INPUT.mp4 OUTPUT_DIR\n",
            "       mp4forge divide -validate INPUT.mp4\n",
            "\n",
            "OPTIONS:\n",
            "  -validate    Validate the fragmented divide layout without writing output files\n",
            "  -warnings    Emit warning-grade diagnostics to stderr after a successful run\n",
            "  -manifest <hls|dash|both>  Manifest families to write (default: both)\n",
            "  -default-language <tag>  Prefer this audio language in HLS defaults and DASH main-role signaling\n",
            "  -hls-base-url <url>  Prefix HLS playlist, init, and media segment URIs\n",
            "  -hls-playlist-type <vod|event|live>  HLS playlist style (default: vod)\n",
            "  -hls-start-time-offset <seconds>  Add EXT-X-START with a signed seconds offset\n",
            "  -hls-program-date-time  Add EXT-X-PROGRAM-DATE-TIME to HLS media playlists\n",
            "  -hls-master-playlist-name <name.m3u8>  Override the root HLS master playlist file name\n",
            "  -hls-media-playlist-name <name.m3u8>  Override per-track HLS media playlist file names\n",
            "  -dash-mode <static|dynamic>  DASH manifest mode (default: static)\n",
            "  -dash-layout <template|list>  DASH manifest layout (default: template)\n",
            "  -dash-profile <main|live>  DASH profile signaling (default: main)\n",
            "  -dash-base-url <url>  Add one DASH BaseURL element (repeatable)\n",
            "  -dash-manifest-name <name.mpd>  Override the root DASH manifest file name\n",
            "  -dash-session-load <path>  Reload saved DASH session controls and next-period continuity\n",
            "  -dash-session-save <path>  Save DASH session controls and next-period continuity\n",
            "\n",
            "Successful output writes the selected retained HLS playlist tree and/or additive MPD manifest.\n",
            "DASH metadata such as Period ids, timing descriptors, and dynamic refresh attributes use built-in defaults.\n",
            "\n",
            "Currently supports fragmented inputs with up to one video track from AVC, HEVC, Dolby Vision on HEVC, AV1, VP8, or VP9\n",
            "and one or more audio tracks from MP4A-based audio, Opus, AC-3, E-AC-3, AC-4, ALAC, DTS-family entries, FLAC, IAMF, MPEG-H, or PCM,\n",
            "including encrypted wrappers that preserve those original sample-entry formats. Subtitle and text tracks remain unsupported.\n",
        )
    );
}

#[test]
fn divide_command_rejects_removed_dash_override_options() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-removed-dash-option-input", &input);
    let output_dir = temp_output_dir("divide-removed-dash-option-output");
    let args = vec![
        "-manifest".to_string(),
        "dash".to_string(),
        "-dash-minimum-update-period".to_string(),
        "5".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: divide option `-dash-minimum-update-period` was removed; mp4forge now uses built-in DASH minimumUpdatePeriod defaults\n"
    );

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_rejects_dash_only_options_when_manifest_selection_is_hls() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-hls-dash-option-invalid-input", &input);
    let output_dir = temp_output_dir("divide-hls-dash-option-invalid-output");
    let args = vec![
        "-manifest".to_string(),
        "hls".to_string(),
        "-dash-mode".to_string(),
        "dynamic".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: divide manifest selection `hls` does not support `-dash-mode`; use `-manifest dash` or `-manifest both`\n"
    );

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_rejects_hls_only_options_when_manifest_selection_is_dash() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-dash-hls-option-invalid-input", &input);
    let output_dir = temp_output_dir("divide-dash-hls-option-invalid-output");
    let args = vec![
        "-manifest".to_string(),
        "dash".to_string(),
        "-hls-base-url".to_string(),
        "https://cdn.example.invalid/hls/".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: divide manifest selection `dash` does not support `-hls-base-url`; use `-manifest hls` or `-manifest both`\n"
    );

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_rejects_dynamic_only_dash_options_in_static_mode() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-static-dynamic-option-invalid-input", &input);
    let output_dir = temp_output_dir("divide-static-dynamic-option-invalid-output");
    let args = vec![
        "-manifest".to_string(),
        "dash".to_string(),
        "-dash-profile".to_string(),
        "live".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: divide DASH profile `live` requires `-dash-mode dynamic`\n"
    );

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_rejects_session_state_options_in_validate_mode() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-validate-session-invalid-input", &input);
    let session_path = temp_output_dir("divide-validate-session-state").join("dash.session");
    let args = vec![
        "-validate".to_string(),
        "-dash-session-save".to_string(),
        session_path.to_string_lossy().into_owned(),
        input_path.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: divide validation mode does not support `-dash-session-save`\n"
    );

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(session_path.parent().unwrap());
}

#[test]
fn divide_command_rejects_reusing_the_same_session_state_path() {
    let input = build_divide_input_file();
    let input_path = write_temp_file("divide-session-reuse-invalid-input", &input);
    let output_dir = temp_output_dir("divide-session-reuse-invalid-output");
    let session_path = output_dir.join("dash.session");
    let args = vec![
        "-manifest".to_string(),
        "dash".to_string(),
        "-dash-session-load".to_string(),
        session_path.to_string_lossy().into_owned(),
        "-dash-session-save".to_string(),
        session_path.to_string_lossy().into_owned(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        format!(
            "Error [stage=request category=input]: divide DASH session load and save paths must differ: `{}`\n",
            session_path.display()
        )
    );

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_derives_master_playlist_signaling_from_probe_metadata() {
    let input = build_video_and_audio_divide_input_file();
    let input_path = write_temp_file("divide-signaling-input", &input);
    let output_dir = temp_output_dir("divide-signaling-output");
    let args = vec![
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(
        read_text(&output_dir.join("playlist.m3u8")),
        concat!(
            "#EXTM3U\n",
            "#EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio/playlist.m3u8\",GROUP-ID=\"audio\",NAME=\"audio\",AUTOSELECT=YES,CHANNELS=\"6\"\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=128,CODECS=\"avc1.4d401f,mp4a.40.5\",RESOLUTION=640x360,AUDIO=\"audio\"\n",
            "video/playlist.m3u8\n"
        )
    );
    let manifest = read_text(&output_dir.join("manifest.mpd"));
    assert!(manifest.contains("contentType=\"video\""));
    assert!(manifest.contains("contentType=\"audio\""));
    assert!(manifest.contains("codecs=\"avc1.4d401f\""));
    assert!(manifest.contains("codecs=\"mp4a.40.5\""));
    assert!(manifest.contains("audioSamplingRate=\"48000\""));
    assert!(manifest.contains("initialization=\"video/init.mp4\""));
    assert!(manifest.contains("initialization=\"audio/init.mp4\""));

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_writes_multi_audio_group_outputs() {
    let input = build_avc_with_aac_and_ac3_divide_input_file();
    let input_path = write_temp_file("divide-multi-audio-input", &input);
    let output_dir = temp_output_dir("divide-multi-audio-output");
    let args = vec![
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(
        read_text(&output_dir.join("playlist.m3u8")),
        concat!(
            "#EXTM3U\n",
            "#EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio_2/playlist.m3u8\",GROUP-ID=\"audio\",NAME=\"audio-2\",AUTOSELECT=YES,DEFAULT=YES,CHANNELS=\"2\"\n",
            "#EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio_3/playlist.m3u8\",GROUP-ID=\"audio\",NAME=\"audio-3\",AUTOSELECT=YES,DEFAULT=NO,CHANNELS=\"6\"\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=128,CODECS=\"avc1.4d401f,mp4a.40.2,ac-3\",RESOLUTION=640x360,AUDIO=\"audio\"\n",
            "video/playlist.m3u8\n"
        )
    );
    assert_eq!(
        read_text(&output_dir.join("audio_2").join("playlist.m3u8")),
        concat!(
            "#EXTM3U\n",
            "#EXT-X-VERSION:7\n",
            "#EXT-X-TARGETDURATION:1\n",
            "#EXT-X-PLAYLIST-TYPE:VOD\n",
            "#EXT-X-MAP:URI=\"init.mp4\"\n",
            "#EXTINF:1.000000,\n",
            "0.mp4\n",
            "#EXT-X-ENDLIST\n"
        )
    );
    assert_eq!(
        read_text(&output_dir.join("audio_3").join("playlist.m3u8")),
        concat!(
            "#EXTM3U\n",
            "#EXT-X-VERSION:7\n",
            "#EXT-X-TARGETDURATION:1\n",
            "#EXT-X-PLAYLIST-TYPE:VOD\n",
            "#EXT-X-MAP:URI=\"init.mp4\"\n",
            "#EXTINF:1.000000,\n",
            "0.mp4\n",
            "#EXT-X-ENDLIST\n"
        )
    );
    let manifest = read_text(&output_dir.join("manifest.mpd"));
    assert!(manifest.contains("contentType=\"video\""));
    assert!(manifest.contains("contentType=\"audio\""));
    assert!(manifest.contains("codecs=\"mp4a.40.2\""));
    assert!(manifest.contains("codecs=\"ac-3\""));
    assert!(manifest.contains("initialization=\"audio_2/init.mp4\""));
    assert!(manifest.contains("initialization=\"audio_3/init.mp4\""));

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_prefers_default_language_for_hls_and_dash() {
    let input = build_avc_with_aac_and_ac3_divide_input_file_with_languages(*b"eng", *b"fra");
    let input_path = write_temp_file("divide-default-language-input", &input);
    let output_dir = temp_output_dir("divide-default-language-output");
    let args = vec![
        "-default-language".to_string(),
        "fra".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(
        read_text(&output_dir.join("playlist.m3u8")),
        concat!(
            "#EXTM3U\n",
            "#EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio_2/playlist.m3u8\",GROUP-ID=\"audio\",NAME=\"audio-2\",AUTOSELECT=YES,DEFAULT=NO,LANGUAGE=\"eng\",CHANNELS=\"2\"\n",
            "#EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio_3/playlist.m3u8\",GROUP-ID=\"audio\",NAME=\"audio-3\",AUTOSELECT=YES,DEFAULT=YES,LANGUAGE=\"fra\",CHANNELS=\"6\"\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=128,CODECS=\"avc1.4d401f,mp4a.40.2,ac-3\",RESOLUTION=640x360,AUDIO=\"audio\"\n",
            "video/playlist.m3u8\n"
        )
    );
    let manifest = read_text(&output_dir.join("manifest.mpd"));
    assert!(!manifest.contains(concat!(
        "<AdaptationSet id=\"2\" contentType=\"audio\" mimeType=\"audio/mp4\" lang=\"eng\">\n",
        "      <Role schemeIdUri=\"urn:mpeg:dash:role:2011\" value=\"main\" />\n"
    )));
    assert!(manifest.contains(concat!(
        "<AdaptationSet id=\"3\" contentType=\"audio\" mimeType=\"audio/mp4\" lang=\"fra\">\n",
        "      <Role schemeIdUri=\"urn:mpeg:dash:role:2011\" value=\"main\" />\n"
    )));

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_command_rejects_multiple_video_tracks_with_clear_message() {
    let input = build_two_video_track_divide_input_file();
    let input_path = write_temp_file("divide-multi-video-input", &input);
    let output_dir = temp_output_dir("divide-multi-video-output");
    let args = vec![
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        format!(
            "Error [stage=request category=input]: {DIVIDE_SCOPE_MESSAGE}; found multiple fragmented video tracks (1 and 2).\n"
        )
    );

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);
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

#[test]
fn divide_dash_list_manifest_round_trips_through_mux_input() {
    let input = build_video_and_audio_divide_input_file();
    let input_path = write_temp_file("divide-dash-list-import-input", &input);
    let output_dir = temp_output_dir("divide-dash-list-import-output");
    let args = vec![
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
        "-manifest".to_string(),
        "dash".to_string(),
        "-dash-layout".to_string(),
        "list".to_string(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));

    let manifest_path = output_dir.join("manifest.mpd");
    let output_path = write_temp_file("divide-dash-list-import-remux-output", &[]);
    mux_to_path(
        &MuxRequest::new(vec![MuxTrackSpec::path(&manifest_path)]),
        &output_path,
    )
    .unwrap();

    let remuxed = probe_file(&output_path);
    assert_eq!(remuxed.segments.len(), 0);
    assert_eq!(remuxed.tracks.len(), 2);
    assert_eq!(remuxed.tracks[0].codec, TrackCodec::Avc1);
    assert_eq!(remuxed.tracks[1].codec, TrackCodec::Mp4a);

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_dash_template_manifest_round_trips_through_mux_input() {
    let input = build_video_and_audio_divide_input_file();
    let input_path = write_temp_file("divide-dash-template-import-input", &input);
    let output_dir = temp_output_dir("divide-dash-template-import-output");
    let args = vec![
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
        "-manifest".to_string(),
        "dash".to_string(),
        "-dash-layout".to_string(),
        "template".to_string(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));

    let manifest_path = output_dir.join("manifest.mpd");
    let output_path = write_temp_file("divide-dash-template-import-remux-output", &[]);
    mux_to_path(
        &MuxRequest::new(vec![MuxTrackSpec::path(&manifest_path)]),
        &output_path,
    )
    .unwrap();

    let remuxed = probe_file(&output_path);
    assert_eq!(remuxed.segments.len(), 0);
    assert_eq!(remuxed.tracks.len(), 2);
    assert_eq!(remuxed.tracks[0].codec, TrackCodec::Avc1);
    assert_eq!(remuxed.tracks[1].codec, TrackCodec::Mp4a);

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_dir_all(&output_dir);
}

#[test]
fn divide_validate_reports_supported_layout_without_writing_files() {
    let input = build_video_and_audio_divide_input_file();
    let input_path = write_temp_file("divide-validate-supported-input", &input);
    let args = vec![
        "-validate".to_string(),
        input_path.to_string_lossy().into_owned(),
    ];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = divide::run_with_output(&args, &mut stdout, &mut stderr);

    let _ = fs::remove_file(&input_path);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        concat!(
            "supported fragmented divide layout\n",
            "track 1: role=video codec=avc1 segments=1\n",
            "track 2: role=audio codec=mp4a segments=1\n",
        )
    );
}

#[test]
fn validate_divide_reader_accepts_supported_broader_video_families() {
    let cases = [
        ("hevc", build_hevc_divide_input_file(), "hvc1"),
        ("av1", build_av1_divide_input_file(), "av01"),
        ("vp8", build_vp8_divide_input_file(), "vp08"),
        ("vp9", build_vp9_divide_input_file(), "vp09"),
        ("dvh1", build_dvh1_divide_input_file(), "dvh1"),
        ("dvhe", build_dvhe_divide_input_file(), "dvhe"),
    ];

    for (name, input, sample_entry_type) in cases {
        let report = divide::validate_divide_reader(&mut std::io::Cursor::new(input)).unwrap();
        assert_eq!(report.tracks.len(), 1, "case={name}");
        assert_eq!(report.tracks[0].track_id, 1, "case={name}");
        assert_eq!(
            report.tracks[0].role,
            divide::DivideTrackRole::Video,
            "case={name}"
        );
        assert_eq!(
            report.tracks[0].sample_entry_type,
            Some(fourcc(sample_entry_type)),
            "case={name}"
        );
        assert_eq!(report.tracks[0].segment_count, 1, "case={name}");
    }
}

#[test]
fn validate_divide_reader_accepts_supported_broader_audio_families() {
    let cases = [
        ("mp3", build_mp3_divide_input_file(), "mp4a"),
        ("opus", build_opus_divide_input_file(), "Opus"),
        ("ac3", build_ac3_divide_input_file(), "ac-3"),
        ("ec3", build_ec3_divide_input_file(), "ec-3"),
        ("ac4", build_ac4_divide_input_file(), "ac-4"),
        ("alac", build_alac_divide_input_file(), "alac"),
        ("dtsc", build_dtsc_divide_input_file(), "dtsc"),
        ("dtse", build_dtse_divide_input_file(), "dtse"),
        ("dtsh", build_dtsh_divide_input_file(), "dtsh"),
        ("dtsl", build_dtsl_divide_input_file(), "dtsl"),
        ("dtsm", build_dtsm_divide_input_file(), "dtsm"),
        ("dtsx", build_dtsx_divide_input_file(), "dtsx"),
        ("dtsy", build_dtsy_divide_input_file(), "dtsy"),
        ("flac", build_flac_divide_input_file(), "fLaC"),
        ("iamf", build_iamf_divide_input_file(), "iamf"),
        ("mha1", build_mha1_divide_input_file(), "mha1"),
        ("mhm1", build_mhm1_divide_input_file(), "mhm1"),
        ("pcm", build_pcm_divide_input_file(), "ipcm"),
    ];

    for (name, input, sample_entry_type) in cases {
        let report = divide::validate_divide_reader(&mut std::io::Cursor::new(input)).unwrap();
        assert_eq!(report.tracks.len(), 1, "case={name}");
        assert_eq!(report.tracks[0].track_id, 1, "case={name}");
        assert_eq!(
            report.tracks[0].role,
            divide::DivideTrackRole::Audio,
            "case={name}"
        );
        assert_eq!(
            report.tracks[0].sample_entry_type,
            Some(fourcc(sample_entry_type)),
            "case={name}"
        );
        assert_eq!(report.tracks[0].segment_count, 1, "case={name}");
    }
}

#[test]
fn validate_divide_reader_accepts_encrypted_hevc_tracks_with_original_format() {
    let report = divide::validate_divide_reader(&mut std::io::Cursor::new(
        build_encrypted_hevc_divide_input_file(),
    ))
    .unwrap();

    assert_eq!(report.tracks.len(), 1);
    assert_eq!(report.tracks[0].track_id, 1);
    assert_eq!(report.tracks[0].role, divide::DivideTrackRole::Video);
    assert!(report.tracks[0].encrypted);
    assert_eq!(report.tracks[0].sample_entry_type, Some(fourcc("encv")));
    assert_eq!(report.tracks[0].original_format, Some(fourcc("hvc1")));
    assert_eq!(report.tracks[0].segment_count, 1);
}

#[test]
fn divide_command_derives_master_playlist_signaling_from_broader_codec_metadata() {
    let cases = [
        (
            "hevc-opus",
            build_hevc_and_opus_divide_input_file(),
            "hvc1,Opus",
            2_u16,
        ),
        (
            "avc-mp3",
            build_avc_and_mp3_divide_input_file(),
            "avc1.4d401f,mp4a.6b",
            2_u16,
        ),
        (
            "avc-ec3",
            build_avc_and_ec3_divide_input_file(),
            "avc1.4d401f,ec-3",
            6_u16,
        ),
        (
            "avc-ac4",
            build_avc_and_ac4_divide_input_file(),
            "avc1.4d401f,ac-4",
            2_u16,
        ),
        (
            "avc-alac",
            build_avc_and_alac_divide_input_file(),
            "avc1.4d401f,alac",
            2_u16,
        ),
        (
            "avc-dtsc",
            build_avc_and_dtsc_divide_input_file(),
            "avc1.4d401f,dtsc",
            6_u16,
        ),
        (
            "avc-flac",
            build_avc_and_flac_divide_input_file(),
            "avc1.4d401f,fLaC",
            2_u16,
        ),
        (
            "avc-iamf",
            build_avc_and_iamf_divide_input_file(),
            "avc1.4d401f,iamf",
            2_u16,
        ),
        (
            "avc-mha1",
            build_avc_and_mha1_divide_input_file(),
            "avc1.4d401f,mha1",
            2_u16,
        ),
        (
            "avc-mhm1",
            build_avc_and_mhm1_divide_input_file(),
            "avc1.4d401f,mhm1",
            2_u16,
        ),
    ];

    for (name, input, codecs, channels) in cases {
        let input_path = write_temp_file(&format!("divide-{name}-signaling-input"), &input);
        let output_dir = temp_output_dir(&format!("divide-{name}-signaling-output"));
        let args = vec![
            input_path.to_string_lossy().into_owned(),
            output_dir.to_string_lossy().into_owned(),
        ];

        let mut stderr = Vec::new();
        let exit_code = divide::run(&args, &mut stderr);

        assert_eq!(
            exit_code,
            0,
            "case={name}: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(String::from_utf8(stderr).unwrap(), "", "case={name}");
        assert_eq!(
            read_text(&output_dir.join("playlist.m3u8")),
            format!(
                "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio/playlist.m3u8\",GROUP-ID=\"audio\",NAME=\"audio\",AUTOSELECT=YES,CHANNELS=\"{channels}\"\n#EXT-X-STREAM-INF:BANDWIDTH=128,CODECS=\"{codecs}\",RESOLUTION=640x360,AUDIO=\"audio\"\nvideo/playlist.m3u8\n"
            ),
            "case={name}"
        );

        let _ = fs::remove_file(&input_path);
        let _ = fs::remove_dir_all(&output_dir);
    }
}

#[test]
fn divide_command_writes_supported_broader_video_family_outputs() {
    let cases = [
        ("hevc", build_hevc_divide_input_file(), "hvc1"),
        ("av1", build_av1_divide_input_file(), "av01"),
        ("vp8", build_vp8_divide_input_file(), "vp08"),
        ("vp9", build_vp9_divide_input_file(), "vp09"),
        ("dvh1", build_dvh1_divide_input_file(), "dvh1"),
        ("dvhe", build_dvhe_divide_input_file(), "dvhe"),
    ];

    for (name, input, codec) in cases {
        let mut input_summary_reader = std::io::Cursor::new(input.clone());
        let input_summary = probe(&mut input_summary_reader).unwrap();
        let input_path = write_temp_file(&format!("divide-{name}-video-input"), &input);
        let output_dir = temp_output_dir(&format!("divide-{name}-video-output"));
        let args = vec![
            input_path.to_string_lossy().into_owned(),
            output_dir.to_string_lossy().into_owned(),
        ];

        let mut stderr = Vec::new();
        let exit_code = divide::run(&args, &mut stderr);

        assert_eq!(
            exit_code,
            0,
            "case={name}: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(String::from_utf8(stderr).unwrap(), "", "case={name}");
        assert_eq!(
            read_text(&output_dir.join("playlist.m3u8")),
            format!(
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=128,CODECS=\"{codec}\",RESOLUTION=640x360\nvideo/playlist.m3u8\n"
            ),
            "case={name}"
        );
        assert_eq!(
            read_text(&output_dir.join("video").join("playlist.m3u8")),
            concat!(
                "#EXTM3U\n",
                "#EXT-X-VERSION:7\n",
                "#EXT-X-TARGETDURATION:1\n",
                "#EXT-X-PLAYLIST-TYPE:VOD\n",
                "#EXT-X-MAP:URI=\"init.mp4\"\n",
                "#EXTINF:1.000000,\n",
                "0.mp4\n",
                "#EXT-X-ENDLIST\n"
            ),
            "case={name}"
        );

        let init = probe_detailed_file(&output_dir.join("video").join("init.mp4"));
        assert_eq!(init.tracks.len(), 1, "case={name}");
        assert_eq!(init.tracks[0].summary.track_id, 1, "case={name}");
        assert_eq!(
            init.tracks[0].sample_entry_type,
            Some(fourcc(codec)),
            "case={name}"
        );
        assert!(init.segments.is_empty(), "case={name}");
        assert_segment_matches(
            &input_summary.segments[0],
            &output_dir.join("video").join("0.mp4"),
        );

        let _ = fs::remove_file(&input_path);
        let _ = fs::remove_dir_all(&output_dir);
    }
}

#[test]
fn divide_command_writes_supported_broader_audio_family_outputs() {
    let cases = [
        ("mp3", build_mp3_divide_input_file(), "mp4a"),
        ("opus", build_opus_divide_input_file(), "Opus"),
        ("ac3", build_ac3_divide_input_file(), "ac-3"),
        ("ec3", build_ec3_divide_input_file(), "ec-3"),
        ("ac4", build_ac4_divide_input_file(), "ac-4"),
        ("alac", build_alac_divide_input_file(), "alac"),
        ("dtsc", build_dtsc_divide_input_file(), "dtsc"),
        ("dtse", build_dtse_divide_input_file(), "dtse"),
        ("dtsh", build_dtsh_divide_input_file(), "dtsh"),
        ("dtsl", build_dtsl_divide_input_file(), "dtsl"),
        ("dtsm", build_dtsm_divide_input_file(), "dtsm"),
        ("dtsx", build_dtsx_divide_input_file(), "dtsx"),
        ("dtsy", build_dtsy_divide_input_file(), "dtsy"),
        ("flac", build_flac_divide_input_file(), "fLaC"),
        ("iamf", build_iamf_divide_input_file(), "iamf"),
        ("mha1", build_mha1_divide_input_file(), "mha1"),
        ("mhm1", build_mhm1_divide_input_file(), "mhm1"),
        ("pcm", build_pcm_divide_input_file(), "ipcm"),
    ];

    for (name, input, codec) in cases {
        let mut input_summary_reader = std::io::Cursor::new(input.clone());
        let input_summary = probe(&mut input_summary_reader).unwrap();
        let input_path = write_temp_file(&format!("divide-{name}-audio-input"), &input);
        let output_dir = temp_output_dir(&format!("divide-{name}-audio-output"));
        let args = vec![
            input_path.to_string_lossy().into_owned(),
            output_dir.to_string_lossy().into_owned(),
        ];

        let mut stderr = Vec::new();
        let exit_code = divide::run(&args, &mut stderr);

        assert_eq!(
            exit_code,
            0,
            "case={name}: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(String::from_utf8(stderr).unwrap(), "", "case={name}");
        assert!(!output_dir.join("playlist.m3u8").exists(), "case={name}");
        assert_eq!(
            read_text(&output_dir.join("audio").join("playlist.m3u8")),
            concat!(
                "#EXTM3U\n",
                "#EXT-X-VERSION:7\n",
                "#EXT-X-TARGETDURATION:1\n",
                "#EXT-X-PLAYLIST-TYPE:VOD\n",
                "#EXT-X-MAP:URI=\"init.mp4\"\n",
                "#EXTINF:1.000000,\n",
                "0.mp4\n",
                "#EXT-X-ENDLIST\n"
            ),
            "case={name}"
        );

        let init = probe_detailed_file(&output_dir.join("audio").join("init.mp4"));
        assert_eq!(init.tracks.len(), 1, "case={name}");
        assert_eq!(init.tracks[0].summary.track_id, 1, "case={name}");
        assert_eq!(
            init.tracks[0].sample_entry_type,
            Some(fourcc(codec)),
            "case={name}"
        );
        assert!(init.segments.is_empty(), "case={name}");
        assert_segment_matches(
            &input_summary.segments[0],
            &output_dir.join("audio").join("0.mp4"),
        );

        let _ = fs::remove_file(&input_path);
        let _ = fs::remove_dir_all(&output_dir);
    }
}

#[test]
fn divide_validate_rejects_duplicate_video_layouts_before_writing_output() {
    let input = build_two_video_track_divide_input_file();
    let input_path = write_temp_file("divide-validate-duplicate-video-input", &input);
    let args = vec![
        "--validate".to_string(),
        input_path.to_string_lossy().into_owned(),
    ];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = divide::run_with_output(&args, &mut stdout, &mut stderr);

    let _ = fs::remove_file(&input_path);

    assert_eq!(exit_code, 1);
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        format!(
            "Error [stage=request category=input]: {DIVIDE_SCOPE_MESSAGE}; found multiple fragmented video tracks (1 and 2).\n"
        )
    );
}

#[test]
fn divide_validate_rejects_subtitle_layout_with_clear_message() {
    let input = build_stpp_divide_input_file();
    let input_path = write_temp_file("divide-validate-stpp-input", &input);
    let args = vec![
        "-validate".to_string(),
        input_path.to_string_lossy().into_owned(),
    ];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = divide::run_with_output(&args, &mut stdout, &mut stderr);

    let _ = fs::remove_file(&input_path);

    assert_eq!(exit_code, 1);
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        format!(
            "Error [stage=request category=input]: track 1 uses unsupported codec `stpp`; {DIVIDE_SCOPE_MESSAGE}\n"
        )
    );
}

#[test]
fn divide_command_can_emit_warning_mode_for_audio_only_outputs() {
    let input = build_opus_divide_input_file();
    let input_path = write_temp_file("divide-warning-audio-only-input", &input);
    let output_dir = temp_output_dir("divide-warning-audio-only-output");
    let args = vec![
        "-warnings".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Warning: divide output is audio-only; no fragmented video track was selected\n"
    );
}

#[test]
fn divide_command_warning_mode_reports_fragmented_decode_gap_and_duration_shift() {
    let input = build_fragmented_input_file(
        vec![build_video_trak_with_profile(1, 640, 360, 0x64, 0x00, 0x1f)],
        vec![
            build_track_segment(1, 0, 1_000, 8),
            build_track_segment(1, 3_000, 500, 8),
        ],
    );
    let input_path = write_temp_file("divide-warning-gap-input", &input);
    let output_dir = temp_output_dir("divide-warning-gap-output");
    let args = vec![
        "-warnings".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        concat!(
            "Warning: track 1 changes segment duration 1 time(s)\n",
            "Warning: track 1 fragmented segment duration spans 0.500000s to 1.000000s\n",
            "Warning: track 1 changes authored fragmented sample duration 1 time(s)\n",
            "Warning: track 1 authored fragmented sample duration spans 0.500000s (500 tick(s)) to 1.000000s (1000 tick(s))\n",
            "Warning: track 1 changes average fragmented sample duration 1 time(s)\n",
            "Warning: track 1 fragmented average sample duration spans 0.500000s to 1.000000s\n",
            "Warning: track 1 has 1 fragmented decode-timeline gap(s)\n",
            "Warning: track 1 has a largest fragmented decode-timeline gap of 2.000000s (2000 tick(s))\n",
        )
    );
}

#[test]
fn divide_command_warning_mode_reports_fragmented_decode_regression_and_zero_duration_samples() {
    let input = build_fragmented_input_file(
        vec![build_video_trak_with_profile(1, 640, 360, 0x64, 0x00, 0x1f)],
        vec![
            build_track_segment(1, 0, 1_000, 8),
            build_track_segment(1, 500, 0, 8),
        ],
    );
    let input_path = write_temp_file("divide-warning-regression-input", &input);
    let output_dir = temp_output_dir("divide-warning-regression-output");
    let args = vec![
        "-warnings".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        concat!(
            "Warning: track 1 has 1 zero-duration fragmented segment(s)\n",
            "Warning: track 1 changes segment duration 1 time(s)\n",
            "Warning: track 1 fragmented segment duration spans 0.000000s to 1.000000s\n",
            "Warning: track 1 carries 1 sample(s) inside zero-duration fragmented segment(s)\n",
            "Warning: track 1 carries 1 zero-duration fragmented sample(s)\n",
            "Warning: track 1 changes average fragmented sample duration 1 time(s)\n",
            "Warning: track 1 fragmented average sample duration spans 0.000000s to 1.000000s\n",
            "Warning: track 1 has 1 fragmented decode-timeline regression(s)\n",
            "Warning: track 1 has a largest fragmented decode-timeline regression of 0.500000s (500 tick(s))\n",
        )
    );
}

#[test]
fn divide_command_warning_mode_reports_zero_duration_samples_inside_nonzero_duration_segment() {
    let input = build_fragmented_input_file(
        vec![build_video_trak_with_profile(1, 640, 360, 0x64, 0x00, 0x1f)],
        vec![build_track_segment_with_explicit_sample_durations(
            1,
            0,
            &[0, 1_000],
            8,
        )],
    );
    let input_path = write_temp_file("divide-warning-zero-duration-sample-input", &input);
    let output_dir = temp_output_dir("divide-warning-zero-duration-sample-output");
    let args = vec![
        "-warnings".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Warning: track 1 carries 1 zero-duration fragmented sample(s)\n"
    );
}

#[test]
fn divide_command_warning_mode_reports_authored_fragmented_sample_duration_jitter() {
    let input = build_fragmented_input_file(
        vec![build_video_trak_with_profile(1, 640, 360, 0x64, 0x00, 0x1f)],
        vec![build_track_segment_with_explicit_sample_durations(
            1,
            0,
            &[500, 1_000],
            8,
        )],
    );
    let input_path = write_temp_file("divide-warning-sample-jitter-input", &input);
    let output_dir = temp_output_dir("divide-warning-sample-jitter-output");
    let args = vec![
        "-warnings".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = divide::run(&args, &mut stderr);

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_dir_all(&output_dir);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        concat!(
            "Warning: track 1 changes authored fragmented sample duration 1 time(s)\n",
            "Warning: track 1 authored fragmented sample duration spans 0.500000s (500 tick(s)) to 1.000000s (1000 tick(s))\n",
        )
    );
}

#[test]
fn validate_divide_reader_reports_supported_tracks() {
    let input = build_video_and_audio_divide_input_file();
    let report = divide::validate_divide_reader(&mut std::io::Cursor::new(input)).unwrap();

    assert_eq!(report.tracks.len(), 2);
    assert_eq!(report.tracks[0].track_id, 1);
    assert_eq!(report.tracks[0].role, divide::DivideTrackRole::Video);
    assert_eq!(report.tracks[0].sample_entry_type, Some(fourcc("avc1")));
    assert_eq!(report.tracks[0].segment_count, 1);
    assert_eq!(report.tracks[1].track_id, 2);
    assert_eq!(report.tracks[1].role, divide::DivideTrackRole::Audio);
    assert_eq!(report.tracks[1].sample_entry_type, Some(fourcc("mp4a")));
    assert_eq!(report.tracks[1].segment_count, 1);
}

fn build_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_video_trak_with_profile(
            1, 1_920, 1_080, 0x64, 0x00, 0x1f,
        )],
        vec![
            build_track_segment(1, 0, 1_000, 8),
            build_track_segment(1, 1_000, 1_000, 8),
        ],
    )
}

fn build_video_and_audio_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![
            build_video_trak_with_profile(1, 640, 360, 0x4d, 0x40, 0x1f),
            build_audio_trak(2, 6, 0x40, &[0x10, 0x02, 0xb7, 0x2c, 0x00]),
        ],
        vec![
            build_track_segment(1, 0, 1_000, 8),
            build_track_segment(2, 0, 1_000, 6),
        ],
    )
}

fn build_two_video_track_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![
            build_video_trak_with_profile(1, 640, 360, 0x64, 0x00, 0x1f),
            build_video_trak_with_profile(2, 320, 180, 0x42, 0x00, 0x1e),
        ],
        vec![
            build_track_segment(1, 0, 1_000, 8),
            build_track_segment(2, 0, 1_000, 8),
        ],
    )
}

fn build_hevc_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_hevc_trak(1, 640, 360)],
        vec![build_track_segment(1, 0, 1_000, 8)],
    )
}

fn build_av1_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_av1_trak(1, 640, 360)],
        vec![build_track_segment(1, 0, 1_000, 8)],
    )
}

fn build_vp8_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_vp8_trak(1, 640, 360)],
        vec![build_track_segment(1, 0, 1_000, 8)],
    )
}

fn build_vp9_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_vp9_trak(1, 640, 360)],
        vec![build_track_segment(1, 0, 1_000, 8)],
    )
}

fn build_hevc_and_opus_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_hevc_trak(1, 640, 360), build_opus_trak(2, 2)],
        vec![
            build_track_segment(1, 0, 1_000, 8),
            build_track_segment(2, 0, 1_000, 6),
        ],
    )
}

fn build_avc_and_mp3_divide_input_file() -> Vec<u8> {
    build_video_and_custom_audio_divide_input_file(build_mp3_trak(2, 2))
}

fn build_avc_and_ec3_divide_input_file() -> Vec<u8> {
    build_video_and_custom_audio_divide_input_file(build_ec3_trak(2, 6))
}

fn build_avc_and_ac4_divide_input_file() -> Vec<u8> {
    build_video_and_custom_audio_divide_input_file(build_ac4_trak(2, 2))
}

fn build_avc_and_alac_divide_input_file() -> Vec<u8> {
    build_video_and_custom_audio_divide_input_file(build_alac_trak(2, 2))
}

fn build_avc_and_dtsc_divide_input_file() -> Vec<u8> {
    build_video_and_custom_audio_divide_input_file(build_dtsc_trak(2, 6))
}

fn build_avc_and_flac_divide_input_file() -> Vec<u8> {
    build_video_and_custom_audio_divide_input_file(build_flac_trak(2, 2))
}

fn build_avc_and_iamf_divide_input_file() -> Vec<u8> {
    build_video_and_custom_audio_divide_input_file(build_iamf_trak(2, 2))
}

fn build_avc_and_mha1_divide_input_file() -> Vec<u8> {
    build_video_and_custom_audio_divide_input_file(build_mha1_trak(2, 2))
}

fn build_avc_and_mhm1_divide_input_file() -> Vec<u8> {
    build_video_and_custom_audio_divide_input_file(build_mhm1_trak(2, 2))
}

fn build_video_and_custom_audio_divide_input_file(audio_trak: Vec<u8>) -> Vec<u8> {
    build_video_and_custom_audio_tracks_divide_input_file(vec![audio_trak])
}

fn build_avc_with_aac_and_ac3_divide_input_file() -> Vec<u8> {
    build_video_and_custom_audio_tracks_divide_input_file(vec![
        build_audio_trak(2, 2, 0x40, &[0x11, 0x90]),
        build_ac3_trak(3, 6),
    ])
}

fn build_avc_with_aac_and_ac3_divide_input_file_with_languages(
    aac_language: [u8; 3],
    ac3_language: [u8; 3],
) -> Vec<u8> {
    build_video_and_custom_audio_tracks_divide_input_file(vec![
        build_audio_trak_with_language(2, 2, 0x40, &[0x11, 0x90], aac_language),
        build_ac3_trak_with_language(3, 6, ac3_language),
    ])
}

fn build_video_and_custom_audio_tracks_divide_input_file(audio_traks: Vec<Vec<u8>>) -> Vec<u8> {
    let mut traks = vec![build_video_trak_with_profile(1, 640, 360, 0x4d, 0x40, 0x1f)];
    traks.extend(audio_traks);

    let mut segments = vec![build_track_segment(1, 0, 1_000, 8)];
    for track_id in 2..=traks.len() as u32 {
        segments.push(build_track_segment(track_id, 0, 1_000, 6));
    }

    build_fragmented_input_file(traks, segments)
}

fn build_opus_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_opus_trak(1, 2)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_mp3_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_mp3_trak(1, 2)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_ac3_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_ac3_trak(1, 6)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_ac4_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_ac4_trak(1, 2)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_alac_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_alac_trak(1, 2)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_pcm_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_pcm_trak(1, 2)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_ec3_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_ec3_trak(1, 6)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_encrypted_hevc_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_encrypted_hevc_trak(1, 640, 360)],
        vec![build_track_segment(1, 0, 1_000, 8)],
    )
}

fn build_dvh1_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_dvh1_trak(1, 640, 360)],
        vec![build_track_segment(1, 0, 1_000, 8)],
    )
}

fn build_dvhe_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_dvhe_trak(1, 640, 360)],
        vec![build_track_segment(1, 0, 1_000, 8)],
    )
}

fn build_dtsc_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_dtsc_trak(1, 6)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_dtse_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_dtse_trak(1, 6)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_dtsh_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_dtsh_trak(1, 6)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_dtsl_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_dtsl_trak(1, 6)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_dtsm_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_dtsm_trak(1, 6)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_dtsx_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_dtsx_trak(1, 6)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_dtsy_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_dtsy_trak(1, 6)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_flac_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_flac_trak(1, 2)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_iamf_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_iamf_trak(1, 2)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_mha1_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_mha1_trak(1, 2)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_mhm1_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_mhm1_trak(1, 2)],
        vec![build_track_segment(1, 0, 1_000, 6)],
    )
}

fn build_stpp_divide_input_file() -> Vec<u8> {
    build_fragmented_input_file(
        vec![build_stpp_trak(1)],
        vec![build_track_segment(1, 0, 1_000, 5)],
    )
}

fn build_fragmented_input_file(traks: Vec<Vec<u8>>, segments: Vec<Vec<u8>>) -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("iso6"),
            minor_version: 1,
            compatible_brands: vec![fourcc("iso6"), fourcc("dash")],
        },
        &[],
    );
    let moov = encode_raw_box(fourcc("moov"), &traks.concat());

    let mut file = [ftyp, moov].concat();
    for segment in segments {
        file.extend_from_slice(&segment);
    }
    file
}

fn build_video_trak_with_profile(
    track_id: u32,
    width: u16,
    height: u16,
    profile: u8,
    profile_compatibility: u8,
    level: u8,
) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = track_id;
    tkhd.width = u32::from(width) << 16;
    tkhd.height = u32::from(height) << 16;

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.duration_v0 = 1_000;

    let avcc = encode_supported_box(
        &AVCDecoderConfiguration {
            configuration_version: 1,
            profile,
            profile_compatibility,
            level,
            length_size_minus_one: 3,
            ..AVCDecoderConfiguration::default()
        },
        &[],
    );

    let mut avc1 = VisualSampleEntry::default();
    avc1.set_box_type(fourcc("avc1"));
    avc1.sample_entry.data_reference_index = 1;
    avc1.width = width;
    avc1.height = height;
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

fn build_audio_trak(
    track_id: u32,
    channel_count: u16,
    object_type_indication: u8,
    decoder_specific_info: &[u8],
) -> Vec<u8> {
    build_audio_trak_with_language(
        track_id,
        channel_count,
        object_type_indication,
        decoder_specific_info,
        *b"und",
    )
}

fn build_audio_trak_with_language(
    track_id: u32,
    channel_count: u16,
    object_type_indication: u8,
    decoder_specific_info: &[u8],
    language: [u8; 3],
) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = track_id;

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.duration_v0 = 1_000;
    mdhd.language = encode_mdhd_language(language);

    let mut mp4a = AudioSampleEntry::default();
    mp4a.set_box_type(fourcc("mp4a"));
    mp4a.sample_entry = SampleEntry {
        box_type: fourcc("mp4a"),
        data_reference_index: 1,
    };
    mp4a.channel_count = channel_count;
    mp4a.sample_size = 16;
    mp4a.sample_rate = 48_000_u32 << 16;

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let mp4a = encode_supported_box(
        &mp4a,
        &encode_supported_box(
            &aac_profile_esds(object_type_indication, decoder_specific_info),
            &[],
        ),
    );
    let stsd = encode_supported_box(&stsd, &mp4a);

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
    stsz.sample_size = 6;
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

fn build_hevc_trak(track_id: u32, width: u16, height: u16) -> Vec<u8> {
    build_video_trak_with_type_and_children(
        track_id,
        width,
        height,
        "hvc1",
        &encode_supported_box(
            &HEVCDecoderConfiguration {
                configuration_version: 1,
                general_profile_idc: 1,
                length_size_minus_one: 3,
                ..HEVCDecoderConfiguration::default()
            },
            &[],
        ),
    )
}

fn build_av1_trak(track_id: u32, width: u16, height: u16) -> Vec<u8> {
    build_video_trak_with_type_and_children(
        track_id,
        width,
        height,
        "av01",
        &encode_supported_box(&av1_config(), &[]),
    )
}

fn build_vp8_trak(track_id: u32, width: u16, height: u16) -> Vec<u8> {
    build_video_trak_with_type_and_children(
        track_id,
        width,
        height,
        "vp08",
        &encode_supported_box(&vp8_config(), &[]),
    )
}

fn build_vp9_trak(track_id: u32, width: u16, height: u16) -> Vec<u8> {
    build_video_trak_with_type_and_children(
        track_id,
        width,
        height,
        "vp09",
        &encode_supported_box(&vp9_config(), &[]),
    )
}

fn build_encrypted_hevc_trak(track_id: u32, width: u16, height: u16) -> Vec<u8> {
    let mut schm = Schm::default();
    schm.set_version(0);
    schm.scheme_type = fourcc("cenc");
    schm.scheme_version = 0x0001_0000;
    let sinf = encode_supported_box(
        &Sinf,
        &[
            encode_supported_box(
                &Frma {
                    data_format: fourcc("hvc1"),
                },
                &[],
            ),
            encode_supported_box(&schm, &[]),
        ]
        .concat(),
    );
    build_video_trak_with_type_and_children(
        track_id,
        width,
        height,
        "encv",
        &[
            encode_supported_box(
                &HEVCDecoderConfiguration {
                    configuration_version: 1,
                    general_profile_idc: 1,
                    length_size_minus_one: 3,
                    ..HEVCDecoderConfiguration::default()
                },
                &[],
            ),
            sinf,
        ]
        .concat(),
    )
}

fn build_dvh1_trak(track_id: u32, width: u16, height: u16) -> Vec<u8> {
    build_video_trak_with_type_and_children(
        track_id,
        width,
        height,
        "dvh1",
        &encode_supported_box(
            &HEVCDecoderConfiguration {
                configuration_version: 1,
                general_profile_idc: 1,
                length_size_minus_one: 3,
                ..HEVCDecoderConfiguration::default()
            },
            &[],
        ),
    )
}

fn build_dvhe_trak(track_id: u32, width: u16, height: u16) -> Vec<u8> {
    build_video_trak_with_type_and_children(
        track_id,
        width,
        height,
        "dvhe",
        &encode_supported_box(
            &HEVCDecoderConfiguration {
                configuration_version: 1,
                general_profile_idc: 1,
                length_size_minus_one: 3,
                ..HEVCDecoderConfiguration::default()
            },
            &[],
        ),
    )
}

fn build_opus_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(
        track_id,
        "Opus",
        channel_count,
        48_000,
        6,
        &encode_supported_box(&opus_config(), &[]),
    )
}

fn build_mp3_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak(track_id, channel_count, 0x6b, &[])
}

fn build_ac3_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(
        track_id,
        "ac-3",
        channel_count,
        48_000,
        6,
        &encode_supported_box(&ac3_config(), &[]),
    )
}

fn build_ac3_trak_with_language(track_id: u32, channel_count: u16, language: [u8; 3]) -> Vec<u8> {
    build_audio_trak_with_type_and_children_and_language(
        track_id,
        "ac-3",
        channel_count,
        48_000,
        6,
        &encode_supported_box(&ac3_config(), &[]),
        language,
    )
}

fn build_ac4_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "ac-4", channel_count, 48_000, 6, &[])
}

fn build_alac_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "alac", channel_count, 48_000, 6, &[])
}

fn build_pcm_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(
        track_id,
        "ipcm",
        channel_count,
        48_000,
        6,
        &encode_supported_box(&pcm_config(), &[]),
    )
}

fn build_ec3_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "ec-3", channel_count, 48_000, 6, &[])
}

fn build_dtsc_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "dtsc", channel_count, 48_000, 6, &[])
}

fn build_dtse_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "dtse", channel_count, 48_000, 6, &[])
}

fn build_dtsh_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "dtsh", channel_count, 48_000, 6, &[])
}

fn build_dtsl_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "dtsl", channel_count, 48_000, 6, &[])
}

fn build_dtsm_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "dtsm", channel_count, 48_000, 6, &[])
}

fn build_dtsx_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "dtsx", channel_count, 48_000, 6, &[])
}

fn build_dtsy_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "dtsy", channel_count, 48_000, 6, &[])
}

fn build_flac_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "fLaC", channel_count, 48_000, 6, &[])
}

fn build_iamf_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "iamf", channel_count, 48_000, 6, &[])
}

fn build_mha1_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "mha1", channel_count, 48_000, 6, &[])
}

fn build_mhm1_trak(track_id: u32, channel_count: u16) -> Vec<u8> {
    build_audio_trak_with_type_and_children(track_id, "mhm1", channel_count, 48_000, 6, &[])
}

fn build_stpp_trak(track_id: u32) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = track_id;

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.duration_v0 = 1_000;

    let mut stpp = XMLSubtitleSampleEntry::default();
    stpp.sample_entry.data_reference_index = 1;
    stpp.namespace = "urn:ttml".to_string();
    stpp.auxiliary_mime_types = "application/ttml+xml".to_string();

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(&stsd, &encode_supported_box(&stpp, &[]));

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
    stsz.sample_size = 5;
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

fn build_track_segment(
    track_id: u32,
    base_media_decode_time: u32,
    sample_duration: u32,
    sample_size: u32,
) -> Vec<u8> {
    let mut tfhd = Tfhd::default();
    tfhd.track_id = track_id;
    tfhd.default_sample_duration = sample_duration;
    tfhd.default_sample_size = sample_size;
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
    let mdat = encode_raw_box(fourcc("mdat"), &vec![0_u8; sample_size as usize]);
    [moof, mdat].concat()
}

fn build_track_segment_with_explicit_sample_durations(
    track_id: u32,
    base_media_decode_time: u32,
    sample_durations: &[u32],
    sample_size: u32,
) -> Vec<u8> {
    let mut tfhd = Tfhd::default();
    tfhd.track_id = track_id;
    tfhd.default_sample_size = sample_size;
    tfhd.set_flags(TFHD_DEFAULT_SAMPLE_SIZE_PRESENT);

    let mut tfdt = Tfdt::default();
    tfdt.base_media_decode_time_v0 = base_media_decode_time;

    let mut trun = Trun::default();
    trun.sample_count = sample_durations.len() as u32;
    trun.entries = sample_durations
        .iter()
        .copied()
        .map(|sample_duration| TrunEntry {
            sample_duration,
            sample_size,
            ..TrunEntry::default()
        })
        .collect();
    trun.set_flags(TRUN_SAMPLE_DURATION_PRESENT | TRUN_SAMPLE_SIZE_PRESENT);

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
    let mdat = encode_raw_box(
        fourcc("mdat"),
        &vec![0_u8; sample_size as usize * sample_durations.len()],
    );
    [moof, mdat].concat()
}

fn aac_profile_esds(object_type_indication: u8, decoder_specific_info: &[u8]) -> Esds {
    let mut esds = Esds::default();
    esds.descriptors = vec![
        Descriptor {
            tag: DECODER_CONFIG_DESCRIPTOR_TAG,
            size: 13,
            decoder_config_descriptor: Some(DecoderConfigDescriptor {
                object_type_indication,
                stream_type: 5,
                reserved: true,
                ..DecoderConfigDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_SPECIFIC_INFO_TAG,
            size: decoder_specific_info.len() as u32,
            data: decoder_specific_info.to_vec(),
            ..Descriptor::default()
        },
    ];
    esds
}

fn encode_mdhd_language(language: [u8; 3]) -> [u8; 3] {
    [language[0] - b'`', language[1] - b'`', language[2] - b'`']
}

fn build_video_trak_with_type_and_children(
    track_id: u32,
    width: u16,
    height: u16,
    sample_entry_type: &str,
    sample_entry_children: &[u8],
) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = track_id;
    tkhd.width = u32::from(width) << 16;
    tkhd.height = u32::from(height) << 16;

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.duration_v0 = 1_000;

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(
        &stsd,
        &encode_supported_box(
            &video_sample_entry_with_type(sample_entry_type, width, height),
            sample_entry_children,
        ),
    );

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

fn build_audio_trak_with_type_and_children(
    track_id: u32,
    sample_entry_type: &str,
    channel_count: u16,
    sample_rate: u16,
    sample_size: u32,
    sample_entry_children: &[u8],
) -> Vec<u8> {
    build_audio_trak_with_type_and_children_and_language(
        track_id,
        sample_entry_type,
        channel_count,
        sample_rate,
        sample_size,
        sample_entry_children,
        *b"und",
    )
}

fn build_audio_trak_with_type_and_children_and_language(
    track_id: u32,
    sample_entry_type: &str,
    channel_count: u16,
    sample_rate: u16,
    sample_size: u32,
    sample_entry_children: &[u8],
    language: [u8; 3],
) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = track_id;

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.duration_v0 = 1_000;
    mdhd.language = encode_mdhd_language(language);

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(
        &stsd,
        &encode_supported_box(
            &audio_sample_entry_with_type(sample_entry_type, channel_count, sample_rate),
            sample_entry_children,
        ),
    );

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
    stsz.sample_size = sample_size;
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

fn video_sample_entry_with_type(
    sample_entry_type: &str,
    width: u16,
    height: u16,
) -> VisualSampleEntry {
    let mut sample_entry = VisualSampleEntry::default();
    sample_entry.set_box_type(fourcc(sample_entry_type));
    sample_entry.sample_entry.data_reference_index = 1;
    sample_entry.width = width;
    sample_entry.height = height;
    sample_entry.horizresolution = 0x0048_0000;
    sample_entry.vertresolution = 0x0048_0000;
    sample_entry.frame_count = 1;
    sample_entry.depth = 0x0018;
    sample_entry.pre_defined3 = -1;
    sample_entry
}

fn audio_sample_entry_with_type(
    sample_entry_type: &str,
    channel_count: u16,
    sample_rate: u16,
) -> AudioSampleEntry {
    let mut sample_entry = AudioSampleEntry::default();
    sample_entry.set_box_type(fourcc(sample_entry_type));
    sample_entry.sample_entry = SampleEntry {
        box_type: fourcc(sample_entry_type),
        data_reference_index: 1,
    };
    sample_entry.channel_count = channel_count;
    sample_entry.sample_size = 16;
    sample_entry.sample_rate = u32::from(sample_rate) << 16;
    sample_entry
}

fn av1_config() -> AV1CodecConfiguration {
    AV1CodecConfiguration {
        seq_profile: 0,
        seq_level_idx_0: 13,
        seq_tier_0: 1,
        high_bitdepth: 1,
        twelve_bit: 0,
        monochrome: 0,
        chroma_subsampling_x: 1,
        chroma_subsampling_y: 0,
        chroma_sample_position: 2,
        initial_presentation_delay_present: 1,
        initial_presentation_delay_minus_one: 3,
        config_obus: vec![0x12, 0x34, 0x56],
    }
}

fn vp8_config() -> VpCodecConfiguration {
    let mut config = VpCodecConfiguration::default();
    config.profile = 0;
    config.level = 10;
    config.bit_depth = 8;
    config.chroma_subsampling = 1;
    config.video_full_range_flag = 0;
    config.colour_primaries = 1;
    config.transfer_characteristics = 1;
    config.matrix_coefficients = 1;
    config
}

fn vp9_config() -> VpCodecConfiguration {
    let mut config = VpCodecConfiguration::default();
    config.profile = 2;
    config.level = 31;
    config.bit_depth = 10;
    config.chroma_subsampling = 1;
    config.video_full_range_flag = 1;
    config.colour_primaries = 9;
    config.transfer_characteristics = 16;
    config.matrix_coefficients = 9;
    config.codec_initialization_data_size = 3;
    config.codec_initialization_data = vec![0x01, 0x02, 0x03];
    config
}

fn opus_config() -> DOps {
    DOps {
        version: 0,
        output_channel_count: 2,
        pre_skip: 312,
        input_sample_rate: 48_000,
        output_gain: 0,
        channel_mapping_family: 1,
        stream_count: 2,
        coupled_count: 1,
        channel_mapping: vec![0, 1],
    }
}

fn ac3_config() -> Dac3 {
    Dac3 {
        fscod: 1,
        bsid: 8,
        bsmod: 3,
        acmod: 7,
        lfe_on: 1,
        bit_rate_code: 10,
    }
}

fn pcm_config() -> PcmC {
    let mut config = PcmC::default();
    config.format_flags = 1;
    config.pcm_sample_size = 24;
    config
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

fn probe_detailed_file(path: &Path) -> mp4forge::probe::DetailedProbeInfo {
    let mut file = fs::File::open(path).unwrap();
    probe_detailed(&mut file).unwrap()
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
