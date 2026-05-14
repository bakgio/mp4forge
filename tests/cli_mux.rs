#![cfg(feature = "mux")]

mod support;

use std::fs;
use std::io::Cursor;

use mp4forge::BoxInfo;
use mp4forge::boxes::avs3::Av3c;
use mp4forge::boxes::dolby::Dmlp;
use mp4forge::boxes::iamf::Iacb;
use mp4forge::boxes::iso14496_12::{
    AudioSampleEntry, Btrt, DvsC, GenericMediaSampleEntry, Hdlr, Mdhd, Nmhd, SampleEntry, Sthd,
    Stss, VisualSampleEntry, XMLSubtitleSampleEntry,
};
use mp4forge::boxes::iso14496_14::Esds;
use mp4forge::boxes::iso14496_15::VVCDecoderConfiguration;
use mp4forge::boxes::iso14496_30::{WVTTSampleEntry, WebVTTConfigurationBox, WebVTTSourceLabelBox};
use mp4forge::boxes::threegpp::{Damr, Dqcp};
use mp4forge::boxes::vp::VpCodecConfiguration;
use mp4forge::cli::{self, mux};
use mp4forge::mux::{MuxFileConfig, MuxTrackConfig};

use support::{
    TestAviAvc1Stream, TestAviH264Stream, TestAviMp4vStream, TestAviPcmStream, TestMuxSample,
    TestQcpCodecKind, build_test_av1_sequence_header_obu, build_test_mp4v_decoder_specific_info,
    build_test_vp10_keyframe, encode_supported_box, fixture_path, fourcc,
    write_single_track_mp4_input, write_temp_file, write_test_ac4_file, write_test_adts_file,
    write_test_aifc_pcm_file, write_test_aiff_pcm_file, write_test_amr_file,
    write_test_amr_wb_file, write_test_av1_annex_b_file, write_test_av1_ivf_file,
    write_test_av1_obu_file, write_test_avi_ac3_file, write_test_avi_avc1_file,
    write_test_avi_h263_file, write_test_avi_h264_file, write_test_avi_jpeg_file,
    write_test_avi_mp3_file, write_test_avi_mp4v_file, write_test_avi_pcm_file,
    write_test_avi_png_file, write_test_caf_alac_file, write_test_caf_alac_variable_packet_file,
    write_test_dts_file, write_test_dts_little_endian_file, write_test_flac_file,
    write_test_h263_file, write_test_h265_annexb_file, write_test_iamf_file, write_test_jpeg_file,
    write_test_latm_file, write_test_mhas_file, write_test_mp3_file, write_test_mp4v_file,
    write_test_ogg_flac_file, write_test_ogg_flac_mapping_file, write_test_ogg_opus_file,
    write_test_ogg_speex_file, write_test_ogg_theora_file, write_test_ogg_vorbis_file,
    write_test_png_file, write_test_program_stream_ac3_file, write_test_program_stream_h264_file,
    write_test_program_stream_h264_open_ended_file, write_test_program_stream_h265_file,
    write_test_program_stream_lpcm_file, write_test_program_stream_mp3_file,
    write_test_program_stream_mp4v_file, write_test_program_stream_mpeg2v_file,
    write_test_program_stream_vobsub_file, write_test_program_stream_vvc_file,
    write_test_qcp_constant_file, write_test_transport_stream_ac3_file,
    write_test_transport_stream_ac4_file, write_test_transport_stream_av1_file,
    write_test_transport_stream_avs3_file, write_test_transport_stream_dts_file,
    write_test_transport_stream_dvb_subtitle_file, write_test_transport_stream_dvb_teletext_file,
    write_test_transport_stream_eac3_file, write_test_transport_stream_h264_file,
    write_test_transport_stream_h265_file, write_test_transport_stream_latm_file,
    write_test_transport_stream_mhas_file, write_test_transport_stream_mp3_file,
    write_test_transport_stream_mp4v_file, write_test_transport_stream_mpeg2v_file,
    write_test_transport_stream_truehd_file, write_test_transport_stream_vvc_file,
    write_test_truehd_file, write_test_usac_latm_file, write_test_vobsub_files,
    write_test_vp10_ivf_file, write_test_wave_pcm_file, write_test_wrapped_dts_file_with_tail,
};

#[test]
fn mux_command_validates_argument_shape() {
    let mut stderr = Vec::new();
    assert_eq!(mux::run(&[], &mut stderr), 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        concat!(
            "USAGE: mp4forge mux --track <SPEC> [--track <SPEC> ...] [--layout <flat|fragmented>] [--segment_duration <SECONDS> | --fragment_duration <SECONDS>] [--out <PATH>] [DEST]\n",
            "\n",
            "OPTIONS:\n",
            "  --track <SPEC>                Add one mux input using the path-first track-spec grammar\n",
            "                               Path only: PATH\n",
            "                               Select one MP4 track when needed with: PATH#video, PATH#audio, PATH#audio:N, PATH#text, PATH#text:N, PATH#track:ID\n",
            "                               Current path-only auto-detection covers MP4, VobSub, supported AVI audio streams plus H.263/JPEG/PNG/MPEG-4 Part 2/H.264/AVC1 video streams, supported MPEG-PS MPEG audio streams plus LPCM audio plus MPEG-4 Part 2/H.264/H.265/VVC video streams, supported MPEG-TS MPEG audio streams plus AAC LATM/MHAS plus AC-3/E-AC-3/AC-4/DTS/TrueHD audio plus MPEG-2/AV1/AVS3/MPEG-4 Part 2/H.264/H.265/VVC video streams, AAC ADTS, AAC LATM, MP3, AC-3, E-AC-3, AC-4, AMR, AMR-WB, QCP voice audio, DTS-family core audio, Dolby TrueHD, leading-sync MHAS MPEG-H, IAMF, H.263 elementary video, MPEG-2 elementary video, MPEG-4 Part 2 elementary video, H.264 Annex B, H.265 Annex B, VVC Annex B, raw AV1 OBU, raw AV1 Annex B, IVF AV1/VP8/VP9/VP10, JPEG still images, PNG still images, WAVE/AIFF/AIFC PCM, native FLAC, Ogg FLAC, Ogg Opus, Ogg Vorbis, Ogg Speex, Ogg Theora, and CAF ALAC\n",
            "                               Broader DTS-family sample-entry variants remain supported through MP4 track import\n",
            "  --segment_duration <SECONDS> Set one target segment duration for supported single-input jobs\n",
            "  --fragment_duration <SECONDS> Set one target fragment duration for supported single-input jobs\n",
            "  --layout <flat|fragmented>   Choose the output container layout; defaults to flat\n",
            "  --out <PATH>                 Force one newly created output destination at PATH\n",
            "  -warnings                    Emit warning-grade diagnostics to stderr after a successful run\n",
            "\n",
            "The current mux command supports at most one video track plus one or more audio and text/subtitle tracks. One positional DEST path follows the update-or-create destination flow: if DEST is an existing MP4, its current tracks are preserved and the requested tracks are imported into it; otherwise DEST is treated as the newly created output file. `--out PATH` is the explicit force-new path. Flat output rejects duration modes. Fragmented output currently requires exactly one duration mode and should be paired with `--out PATH`. Path-only MP4 inputs import all supported tracks unless you add one selector suffix.\n",
        )
    );
}

#[test]
fn mux_command_rejects_positional_dest_when_out_is_present() {
    let video_input = build_video_input_file("mux-cli-out-conflict-input", fourcc("isom"));
    let args = vec![
        "--out".to_string(),
        "fresh-output.mp4".to_string(),
        "--track".to_string(),
        video_input.to_string_lossy().into_owned(),
        "dest.mp4".to_string(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: --out <PATH> may not be used together with a positional DEST path\n"
    );
}

#[test]
fn mux_command_updates_the_positional_destination_mp4() {
    let destination = build_video_input_file("mux-cli-destination-video-input", fourcc("isom"));
    let audio_input = write_test_adts_file("mux-cli-destination-audio-input", &[b"aud"]);
    let args = vec![
        "--track".to_string(),
        audio_input.to_string_lossy().into_owned(),
        destination.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");

    let output_bytes = fs::read(&destination).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        root_boxes.iter().map(BoxInfo::box_type).collect::<Vec<_>>(),
        vec![
            fourcc("ftyp"),
            fourcc("moov"),
            fourcc("mdat"),
            fourcc("free"),
        ]
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
    assert_eq!(hdlr_boxes.len(), 2);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_dts_input() {
    let dts_input = write_test_dts_file("mux-cli-path-only-dts-input", 2);
    let expected_payload = fs::read(&dts_input).unwrap();
    let output = write_temp_file("mux-cli-path-only-dts-output", &[]);
    let args = vec![
        "--track".to_string(),
        dts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_little_endian_dts_input() {
    let dts_input = write_test_dts_little_endian_file("mux-cli-path-only-dts-le-input", 2);
    let expected_payload = fs::read(&dts_input).unwrap();
    let output = write_temp_file("mux-cli-path-only-dts-le-output", &[]);
    let args = vec![
        "--track".to_string(),
        dts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);
}

#[test]
fn mux_command_writes_real_mp4_output_from_wrapped_core_dts_input_with_trailing_family_tail() {
    let dts_input = write_test_wrapped_dts_file_with_tail(
        "mux-cli-path-only-dts-wrapped-tail-input",
        2,
        b"DTSHDTRAILER",
    );
    let expected_payload = fs::read(&dts_input).unwrap();
    let output = write_temp_file("mux-cli-path-only-dts-wrapped-tail-output", &[]);
    let args = vec![
        "--track".to_string(),
        dts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_avi_pcm_input() {
    let chunk = [0_u8, 0, 0, 0, 1, 0, 1, 0];
    let avi_input = write_test_avi_pcm_file(
        "mux-cli-path-only-avi-input",
        &[TestAviPcmStream {
            sample_rate: 48_000,
            channel_count: 2,
            bits_per_sample: 16,
            chunks: &[&chunk],
        }],
    );
    let output = write_temp_file("mux-cli-path-only-avi-output", &[]);
    let args = vec![
        "--track".to_string(),
        avi_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
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
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_mp4v_input() {
    let decoder_specific_info = build_test_mp4v_decoder_specific_info(320, 180);
    let intra_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x00, 0xAA, 0xBB];
    let predictive_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x40, 0xCC, 0xDD];
    let mut elementary = decoder_specific_info;
    elementary.extend_from_slice(&intra_frame);
    elementary.extend_from_slice(&predictive_frame);
    let mp4v_input = write_test_mp4v_file("mux-cli-path-only-mp4v-input", &elementary);
    let output = write_temp_file("mux-cli-path-only-mp4v-output", &[]);
    let args = vec![
        "--track".to_string(),
        mp4v_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
        ]),
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
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 320);
    assert_eq!(video_entries[0].height, 180);
    assert_eq!(video_entries[0].compressorname[0], 0);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 25_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_avi_mp4v_input() {
    let decoder_specific_info = [0x00_u8, 0x00, 0x01, 0x20, 0x11, 0x22];
    let intra_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x00, 0xAA, 0xBB];
    let predictive_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x40, 0xCC, 0xDD];
    let avi_input = write_test_avi_mp4v_file(
        "mux-cli-path-only-avi-mp4v-input",
        &TestAviMp4vStream {
            width: 320,
            height: 180,
            frame_scale: 1,
            frame_rate: 25,
            compression: *b"MP4V",
            decoder_specific_info: &decoder_specific_info,
            frames: &[&intra_frame, &predictive_frame],
        },
    );
    let output = write_temp_file("mux-cli-path-only-avi-mp4v-output", &[]);
    let args = vec![
        "--track".to_string(),
        avi_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
        ]),
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
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 320);
    assert_eq!(video_entries[0].height, 180);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 25_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_avi_h264_input() {
    let avi_input = write_test_avi_h264_file(
        "mux-cli-path-only-avi-h264-input",
        &TestAviH264Stream {
            width: 320,
            height: 180,
            frame_scale: 1,
            frame_rate: 25,
            compression: *b"H264",
            sample_payloads: &[b"\xAA\xBB", b"\xCC\xDD"],
        },
    );
    let output = write_temp_file("mux-cli-path-only-avi-h264-output", &[]);
    let args = vec![
        "--track".to_string(),
        avi_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("avc1"),
        ]),
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
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 320);
    assert_eq!(video_entries[0].height, 180);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 25_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_avi_avc1_input() {
    let avi_input = write_test_avi_avc1_file(
        "cli-mux-avi-avc1-input",
        &TestAviAvc1Stream {
            width: 320,
            height: 180,
            frame_scale: 1,
            frame_rate: 25,
            sample_payloads: &[b"\xAA\xBB", b"\xCC\xDD"],
        },
    );
    let output_path = write_temp_file("cli-mux-avi-avc1-output", &[]);
    let args = vec![
        "--track".to_string(),
        avi_input.to_string_lossy().into_owned(),
        "--out".to_string(),
        output_path.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    assert_eq!(
        mux::run(&args, &mut stderr),
        0,
        "{}",
        String::from_utf8_lossy(&stderr)
    );

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("avc1"),
        ]),
    );
    let handlers = extract_boxes::<Hdlr>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );

    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 320);
    assert_eq!(video_entries[0].height, 180);
    assert_eq!(handlers.len(), 1);
    assert_eq!(handlers[0].name, "VideoHandler");
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_avi_mp3_input() {
    let avi_input = write_test_avi_mp3_file(
        "mux-cli-path-only-avi-mp3-input",
        48_000,
        2,
        &[b"avi-mp3-a", b"avi-mp3-b"],
    );
    let output = write_temp_file("mux-cli-path-only-avi-mp3-output", &[]);
    let args = vec![
        "--track".to_string(),
        avi_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc(".mp3"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc(".mp3"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_avi_ac3_input() {
    let avi_input = write_test_avi_ac3_file(
        "mux-cli-path-only-avi-ac3-input",
        48_000,
        2,
        &[b"avi-ac3-a", b"avi-ac3-b"],
    );
    let output = write_temp_file("mux-cli-path-only-avi-ac3-output", &[]);
    let args = vec![
        "--track".to_string(),
        avi_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ac-3"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ac-3"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_avi_h263_input() {
    let avi_input = write_test_avi_h263_file(
        "mux-cli-path-only-avi-h263-input",
        176,
        144,
        1,
        25,
        &[b"\xAA\xBB", b"\xCC\xDD"],
    );
    let output = write_temp_file("mux-cli-path-only-avi-h263-output", &[]);
    let args = vec![
        "--track".to_string(),
        avi_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("H263"),
        ]),
    );
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("H263"),
            fourcc("btrt"),
        ]),
    );
    let stss_boxes = extract_boxes::<Stss>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stss"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 176);
    assert_eq!(video_entries[0].height, 144);
    assert_eq!(btrt_boxes.len(), 1);
    assert!(btrt_boxes[0].buffer_size_db > 0);
    assert!(btrt_boxes[0].max_bitrate > 0);
    assert!(btrt_boxes[0].avg_bitrate > 0);
    assert_eq!(stss_boxes.len(), 1);
    assert_eq!(stss_boxes[0].entry_count, 0);
    assert!(stss_boxes[0].sample_number.is_empty());
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_avi_jpeg_input() {
    let jpeg_frame = fs::read(fixture_path("generated-1x1.jpg")).unwrap();
    let avi_input = write_test_avi_jpeg_file(
        "mux-cli-path-only-avi-jpeg-input",
        1,
        1,
        1,
        25,
        &[&jpeg_frame],
    );
    let output = write_temp_file("mux-cli-path-only-avi-jpeg-output", &[]);
    let args = vec![
        "--track".to_string(),
        avi_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("MJPG"),
        ]),
    );
    let stss_boxes = extract_boxes::<Stss>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stss"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 1);
    assert_eq!(video_entries[0].height, 1);
    assert_eq!(stss_boxes.len(), 1);
    assert_eq!(stss_boxes[0].entry_count, 0);
    assert!(stss_boxes[0].sample_number.is_empty());
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_avi_png_input() {
    let png_frame_path = write_test_png_file("mux-cli-path-only-avi-png-frame");
    let png_frame = fs::read(png_frame_path).unwrap();
    let avi_input = write_test_avi_png_file(
        "mux-cli-path-only-avi-png-input",
        1,
        1,
        1,
        25,
        &[&png_frame],
    );
    let output = write_temp_file("mux-cli-path-only-avi-png-output", &[]);
    let args = vec![
        "--track".to_string(),
        avi_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("PNG "),
        ]),
    );
    let stss_boxes = extract_boxes::<Stss>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stss"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 1);
    assert_eq!(video_entries[0].height, 1);
    assert_eq!(stss_boxes.len(), 1);
    assert_eq!(stss_boxes[0].entry_count, 0);
    assert!(stss_boxes[0].sample_number.is_empty());
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_program_stream_mp4v_input() {
    let decoder_specific_info = build_test_mp4v_decoder_specific_info(320, 180);
    let intra_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x00, 0xAA, 0xBB];
    let predictive_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x40, 0xCC, 0xDD];
    let first_payload = [&decoder_specific_info[..], &intra_frame[..]].concat();
    let ps_input = write_test_program_stream_mp4v_file(
        "mux-cli-path-only-program-stream-mp4v-input",
        &[&first_payload, &predictive_frame],
    );
    let output = write_temp_file("mux-cli-path-only-program-stream-mp4v-output", &[]);
    let args = vec![
        "--track".to_string(),
        ps_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_program_stream_mpeg2v_input() {
    let ps_input = write_test_program_stream_mpeg2v_file(
        "mux-cli-path-only-program-stream-mpeg2v-input",
        &[b"mpeg2v-a", b"mpeg2v-b"],
    );
    let output = write_temp_file("mux-cli-path-only-program-stream-mpeg2v-output", &[]);
    let args = vec![
        "--track".to_string(),
        ps_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_program_stream_input() {
    let ps_input = write_test_program_stream_mp3_file(
        "mux-cli-path-only-program-stream-input",
        &[&[0x21; 96]],
    );
    let output = write_temp_file("mux-cli-path-only-program-stream-output", &[]);
    let args = vec![
        "--track".to_string(),
        ps_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc(".mp3"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_program_stream_ac3_input() {
    let ps_input =
        write_test_program_stream_ac3_file("mux-cli-path-only-program-stream-ac3-input", &[b"ac3"]);
    let output = write_temp_file("mux-cli-path-only-program-stream-ac3-output", &[]);
    let args = vec![
        "--track".to_string(),
        ps_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ac-3"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_program_stream_mp3_input() {
    let ps_input = write_test_program_stream_mp3_file(
        "mux-cli-path-only-program-stream-mp3-input",
        &[b"mp3-a", b"mp3-b"],
    );
    let output = write_temp_file("mux-cli-path-only-program-stream-mp3-output", &[]);
    let args = vec![
        "--track".to_string(),
        ps_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc(".mp3"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc(".mp3"));
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_program_stream_lpcm_input() {
    let sample_a = [0x00_u8, 0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04];
    let sample_b = [0x00_u8, 0x05, 0x00, 0x06, 0x00, 0x07, 0x00, 0x08];
    let ps_input = write_test_program_stream_lpcm_file(
        "mux-cli-path-only-program-stream-lpcm-input",
        &[&sample_a, &sample_b],
    );
    let output = write_temp_file("mux-cli-path-only-program-stream-lpcm-output", &[]);
    let args = vec![
        "--track".to_string(),
        ps_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ipcm"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ipcm"));
    assert_eq!(audio_entries[0].channel_count, 2);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_program_stream_h264_input() {
    let ps_input = write_test_program_stream_h264_file(
        "mux-cli-path-only-program-stream-h264-input",
        &[b"idr"],
    );
    let output = write_temp_file("mux-cli-path-only-program-stream-h264-output", &[]);
    let args = vec![
        "--track".to_string(),
        ps_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("avc1"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_program_stream_h264_open_ended_input() {
    let ps_input = write_test_program_stream_h264_open_ended_file(
        "mux-cli-path-only-program-stream-h264-open-ended-input",
        &[b"idr", b"p-frame"],
    );
    let output = write_temp_file(
        "mux-cli-path-only-program-stream-h264-open-ended-output",
        &[],
    );
    let args = vec![
        "--track".to_string(),
        ps_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("avc1"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_program_stream_h265_input() {
    let ps_input = write_test_program_stream_h265_file(
        "mux-cli-path-only-program-stream-h265-input",
        &[b"hevc"],
    );
    let output = write_temp_file("mux-cli-path-only-program-stream-h265-output", &[]);
    let args = vec![
        "--track".to_string(),
        ps_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("hvc1"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_mp4v_input() {
    let decoder_specific_info = build_test_mp4v_decoder_specific_info(320, 180);
    let intra_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x00, 0xAA, 0xBB];
    let predictive_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x40, 0xCC, 0xDD];
    let first_payload = [&decoder_specific_info[..], &intra_frame[..]].concat();
    let ts_input = write_test_transport_stream_mp4v_file(
        "mux-cli-path-only-transport-stream-mp4v-input",
        &[&first_payload, &predictive_frame],
    );
    let output = write_temp_file("mux-cli-path-only-transport-stream-mp4v-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_mpeg2v_input() {
    let ts_input = write_test_transport_stream_mpeg2v_file(
        "mux-cli-path-only-transport-stream-mpeg2v-input",
        &[b"mpeg2v-a", b"mpeg2v-b"],
    );
    let output = write_temp_file("mux-cli-path-only-transport-stream-mpeg2v-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_av1_input() {
    let frame_a = build_test_av1_sequence_header_obu(320, 240);
    let frame_b = build_test_av1_sequence_header_obu(320, 240);
    let ts_input = write_test_transport_stream_av1_file(
        "mux-cli-path-only-transport-stream-av1-input",
        &[&frame_a, &frame_b],
    );
    let output = write_temp_file("mux-cli-path-only-transport-stream-av1-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
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
    assert_eq!(video_entries[0].width, 320);
    assert_eq!(video_entries[0].height, 240);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_avs3_input() {
    let ts_input = write_test_transport_stream_avs3_file(
        "mux-cli-path-only-transport-stream-avs3-input",
        &[b"avs3-a", b"avs3-b"],
    );
    let output = write_temp_file("mux-cli-path-only-transport-stream-avs3-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("avs3"),
        ]),
    );
    let av3c_boxes = extract_boxes::<Av3c>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("avs3"),
            fourcc("av3c"),
        ]),
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
    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("avs3"),
            fourcc("btrt"),
        ]),
    );
    let stss_boxes = extract_boxes::<Stss>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stss"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 0);
    assert_eq!(video_entries[0].height, 0);
    assert_eq!(av3c_boxes.len(), 1);
    assert_eq!(
        av3c_boxes[0].sequence_header,
        vec![0x00, 0x00, 0x01, 0xB0, 0x20, 0x10]
    );
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 90_000);
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].name, "VideoHandler");
    assert_eq!(btrt_boxes.len(), 1);
    assert_eq!(stss_boxes.len(), 1);
    assert_eq!(stss_boxes[0].entry_count, 0);
    assert!(stss_boxes[0].sample_number.is_empty());
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_h264_input() {
    let ts_input = write_test_transport_stream_h264_file(
        "mux-cli-path-only-transport-stream-h264-input",
        &[b"idr"],
    );
    let output = write_temp_file("mux-cli-path-only-transport-stream-h264-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("avc1"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_h265_input() {
    let ts_input = write_test_transport_stream_h265_file(
        "mux-cli-path-only-transport-stream-h265-input",
        &[b"hevc"],
    );
    let output = write_temp_file("mux-cli-path-only-transport-stream-h265-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("hvc1"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_program_stream_vvc_input() {
    let ps_input =
        write_test_program_stream_vvc_file("mux-cli-path-only-program-stream-vvc-input", &[]);
    let output = write_temp_file("mux-cli-path-only-program-stream-vvc-output", &[]);
    let args = vec![
        "--track".to_string(),
        ps_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("vvc1"),
        ]),
    );
    let vvc_boxes = extract_boxes::<VVCDecoderConfiguration>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("vvc1"),
            fourcc("vvcC"),
        ]),
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
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 1280);
    assert_eq!(video_entries[0].height, 720);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 25);
    assert_eq!(mdhd_boxes[0].duration(), 2);
    assert_eq!(vvc_boxes.len(), 1);
    assert!(!vvc_boxes[0].decoder_configuration_record.is_empty());
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_vvc_input() {
    let ts_input =
        write_test_transport_stream_vvc_file("mux-cli-path-only-transport-stream-vvc-input", &[]);
    let output = write_temp_file("mux-cli-path-only-transport-stream-vvc-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("vvc1"),
        ]),
    );
    let vvc_boxes = extract_boxes::<VVCDecoderConfiguration>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("vvc1"),
            fourcc("vvcC"),
        ]),
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
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 1280);
    assert_eq!(video_entries[0].height, 720);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 90_000);
    assert_eq!(mdhd_boxes[0].duration(), 0);
    assert_eq!(vvc_boxes.len(), 1);
    assert!(!vvc_boxes[0].decoder_configuration_record.is_empty());
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_ac3_input() {
    let ts_input = write_test_transport_stream_ac3_file(
        "mux-cli-path-only-transport-stream-ac3-input",
        &[b"ac3"],
    );
    let output = write_temp_file("mux-cli-path-only-transport-stream-ac3-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ac-3"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_latm_input() {
    let ts_input = write_test_transport_stream_latm_file(
        "mux-cli-path-only-transport-stream-latm-input",
        &[b"abc", b"defg"],
    );
    let output = write_temp_file("mux-cli-path-only-transport-stream-latm-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4a"),
        ]),
    );
    let esds_boxes = extract_boxes::<Esds>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4a"),
            fourcc("esds"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(esds_boxes.len(), 1);
    assert_eq!(
        esds_boxes[0]
            .decoder_config_descriptor()
            .unwrap()
            .object_type_indication,
        0x40
    );
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_mhas_input() {
    let ts_input = write_test_transport_stream_mhas_file(
        "mux-cli-path-only-transport-stream-mhas-input",
        &[b"frame-one", b"frame-two"],
    );
    let output = write_temp_file("mux-cli-path-only-transport-stream-mhas-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mhm1"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_eac3_input() {
    let ts_input = write_test_transport_stream_eac3_file(
        "mux-cli-path-only-transport-stream-eac3-input",
        &[b"ec3"],
    );
    let output = write_temp_file("mux-cli-path-only-transport-stream-eac3-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ec-3"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_ac4_input() {
    let ts_input =
        write_test_transport_stream_ac4_file("mux-cli-path-only-transport-stream-ac4-input", 2);
    let output = write_temp_file("mux-cli-path-only-transport-stream-ac4-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ac-4"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_truehd_input() {
    let ts_input = write_test_transport_stream_truehd_file(
        "mux-cli-path-only-transport-stream-truehd-input",
        &[b"abcdefgh", b"ijklmnop"],
    );
    let output = write_temp_file("mux-cli-path-only-transport-stream-truehd-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mlpa"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_dts_input() {
    let ts_input =
        write_test_transport_stream_dts_file("mux-cli-path-only-transport-stream-dts-input", 2);
    let output = write_temp_file("mux-cli-path-only-transport-stream-dts-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("dtsx"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_dvb_subtitle_input() {
    let ts_input = write_test_transport_stream_dvb_subtitle_file(
        "mux-cli-path-only-transport-stream-dvb-subtitle-input",
        &[b"\x20cli-subtitle"],
    );
    let output = write_temp_file(
        "mux-cli-path-only-transport-stream-dvb-subtitle-output",
        &[],
    );
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let subtitle_entries = extract_boxes::<GenericMediaSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("dvbs"),
        ]),
    );
    let dvsc_boxes = extract_boxes::<DvsC>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("dvbs"),
            fourcc("dvsC"),
        ]),
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
    assert_eq!(subtitle_entries.len(), 1);
    assert_eq!(subtitle_entries[0].sample_entry.box_type, fourcc("dvbs"));
    assert_eq!(dvsc_boxes.len(), 1);
    assert_eq!(dvsc_boxes[0].composition_page_id, 0x0123);
    assert_eq!(dvsc_boxes[0].ancillary_page_id, 0x0456);
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].handler_type, fourcc("subt"));
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_dvb_teletext_input() {
    let ts_input = write_test_transport_stream_dvb_teletext_file(
        "mux-cli-path-only-transport-stream-dvb-teletext-input",
        &[b"\x10cli-text"],
    );
    let output = write_temp_file(
        "mux-cli-path-only-transport-stream-dvb-teletext-output",
        &[],
    );
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let subtitle_entries = extract_boxes::<GenericMediaSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("dvbt"),
        ]),
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
    assert_eq!(subtitle_entries.len(), 1);
    assert_eq!(subtitle_entries[0].sample_entry.box_type, fourcc("dvbt"));
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].handler_type, fourcc("subt"));
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_vobsub_sub_input() {
    let (_idx_input, sub_input) =
        write_test_vobsub_files("mux-cli-path-only-vobsub-sub-input", &[0], &[b"\x11\x22"]);
    let output = write_temp_file("mux-cli-path-only-vobsub-sub-output", &[]);
    let args = vec![
        "--track".to_string(),
        sub_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let subtitle_entries = extract_boxes::<GenericMediaSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4s"),
        ]),
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
    assert_eq!(subtitle_entries.len(), 1);
    assert_eq!(subtitle_entries[0].sample_entry.box_type, fourcc("mp4s"));
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].handler_type, fourcc("subp"));
    assert_eq!(hdlr_boxes[0].name, "SubtitleHandler");
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_program_stream_vobsub_input() {
    let ps_input = write_test_program_stream_vobsub_file(
        "mux-cli-path-only-program-stream-vobsub-input",
        &[0],
        &[b"\x11\x22"],
    );
    let output = write_temp_file("mux-cli-path-only-program-stream-vobsub-output", &[]);
    let args = vec![
        "--track".to_string(),
        ps_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let subtitle_entries = extract_boxes::<GenericMediaSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4s"),
        ]),
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
    assert_eq!(subtitle_entries.len(), 1);
    assert_eq!(subtitle_entries[0].sample_entry.box_type, fourcc("mp4s"));
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].handler_type, fourcc("subp"));
    assert_eq!(hdlr_boxes[0].name, "SubtitleHandler");
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_vvc_input() {
    let vvc_input = fixture_path("mux/raw_vvc_idr.vvc");
    let output = write_temp_file("mux-cli-path-only-vvc-output", &[]);
    let args = vec![
        "--track".to_string(),
        vvc_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("vvc1"),
        ]),
    );
    let vvc_boxes = extract_boxes::<VVCDecoderConfiguration>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("vvc1"),
            fourcc("vvcC"),
        ]),
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
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 1280);
    assert_eq!(video_entries[0].height, 720);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 25);
    assert_eq!(mdhd_boxes[0].duration(), 2);
    assert_eq!(vvc_boxes.len(), 1);
    assert!(!vvc_boxes[0].decoder_configuration_record.is_empty());
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_only_transport_stream_input() {
    let ts_input = write_test_transport_stream_mp3_file(
        "mux-cli-path-only-transport-stream-input",
        &[&[0x31; 320]],
    );
    let output = write_temp_file("mux-cli-path-only-transport-stream-output", &[]);
    let args = vec![
        "--track".to_string(),
        ts_input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc(".mp3"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
}

#[test]
fn mux_command_rejects_invalid_track_specs() {
    let output = write_temp_file("mux-cli-invalid-output", &[]);
    let args = vec![
        "--track".to_string(),
        "input.bin#width=640".to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: invalid mux track spec `input.bin#width=640`: public mux track specs only allow selector suffixes such as `#video`, `#audio`, `#text`, or `#track:ID`; raw `#name=value` parameters are no longer accepted\n"
    );
}

#[test]
fn mux_command_rejects_conflicting_duration_flags() {
    let output = write_temp_file("mux-cli-conflict-output", &[]);
    let args = vec![
        "--track".to_string(),
        "input.aac".to_string(),
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
        "Error [stage=request category=input]: --segment_duration and --fragment_duration may not be used together\n"
    );
}

#[test]
fn mux_command_rejects_duration_flags_for_flat_layout() {
    let output = write_temp_file("mux-cli-flat-layout-output", &[]);
    let args = vec![
        "--track".to_string(),
        "input.aac".to_string(),
        "--fragment_duration".to_string(),
        "2".to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: invalid mux layout `flat`: flat output does not support `--fragment_duration`; use `--layout fragmented` instead\n"
    );
}

#[test]
fn mux_command_rejects_fragmented_layout_without_duration() {
    let output = write_temp_file("mux-cli-fragmented-missing-duration-output", &[]);
    let args = vec![
        "--track".to_string(),
        "input.aac".to_string(),
        "--layout".to_string(),
        "fragmented".to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: invalid mux layout `fragmented`: fragmented output requires exactly one of `--segment_duration` or `--fragment_duration`\n"
    );
}

#[test]
fn mux_command_rejects_multiple_video_tracks() {
    let output = write_temp_file("mux-cli-multi-video-output", &[]);
    let args = vec![
        "--track".to_string(),
        "first.mp4#video".to_string(),
        "--track".to_string(),
        "second.mp4#video".to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: the current mux surface supports at most one video track per job, but 2 were requested\n"
    );
}

#[test]
fn mux_command_rejects_fragmented_multi_track_jobs() {
    let output = write_temp_file("mux-cli-fragmented-multi-track-output", &[]);
    let args = vec![
        "--track".to_string(),
        "first.mp4#audio".to_string(),
        "--track".to_string(),
        "second.mp4#video".to_string(),
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
        "Error [stage=request category=input]: invalid mux layout `fragmented`: the current fragmented mux follow-on only supports single-track jobs\n"
    );
}

#[test]
fn mux_command_rejects_fragmented_destination_path_mode_before_execution() {
    let destination =
        build_audio_input_file("mux-cli-fragmented-destination-output", fourcc("isom"));
    let audio_input = write_test_adts_file("mux-cli-fragmented-destination-audio-input", &[b"aud"]);
    let args = vec![
        "--track".to_string(),
        audio_input.to_string_lossy().into_owned(),
        "--layout".to_string(),
        "fragmented".to_string(),
        "--fragment_duration".to_string(),
        "2".to_string(),
        destination.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: invalid mux destination mode `update-or-create-destination`: the current destination-path mux mode only supports flat output; use `--out PATH` for create-new fragmented output\n"
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
fn mux_command_can_emit_warning_mode_for_fragmented_audio_only_output() {
    let audio_input =
        build_audio_input_file("mux-cli-fragmented-warning-audio-input", fourcc("isom"));
    let output = write_temp_file("mux-cli-fragmented-warning-output", &[]);
    let args = vec![
        "-warnings".to_string(),
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
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Warning: divide output is audio-only; no fragmented video track was selected\n"
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
fn mux_command_writes_real_mp4_output_from_path_first_ivf_tracks() {
    let av1_frame_a = build_test_av1_sequence_header_obu(640, 360);
    let av1_frame_b = build_test_av1_sequence_header_obu(640, 360);
    let video_input = write_test_av1_ivf_file(
        "mux-cli-raw-av1-input",
        640,
        360,
        &[0, 1],
        &[av1_frame_a.as_slice(), av1_frame_b.as_slice()],
    );
    let output = write_temp_file("mux-cli-raw-broader-output", &[]);
    let args = vec![
        "--track".to_string(),
        video_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        [av1_frame_a, av1_frame_b].concat()
    );

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
    assert_eq!(video_entries[0].width, 640);
    assert_eq!(video_entries[0].height, 360);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_raw_av1_obu_tracks() {
    let av1_frame_a = build_test_av1_sequence_header_obu(640, 360);
    let av1_frame_b = build_test_av1_sequence_header_obu(640, 360);
    let video_input =
        write_test_av1_obu_file("mux-cli-raw-av1-obu-input", &[&av1_frame_a, &av1_frame_b]);
    let output = write_temp_file("mux-cli-raw-av1-obu-output", &[]);
    let args = vec![
        "--track".to_string(),
        video_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        [av1_frame_a, av1_frame_b].concat()
    );
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_raw_av1_annexb_tracks() {
    let av1_frame_a = build_test_av1_sequence_header_obu(640, 360);
    let av1_frame_b = build_test_av1_sequence_header_obu(640, 360);
    let video_input = write_test_av1_annex_b_file(
        "mux-cli-raw-av1-annexb-input",
        &[av1_frame_a.as_slice(), av1_frame_b.as_slice()],
    );
    let output = write_temp_file("mux-cli-raw-av1-annexb-output", &[]);
    let args = vec![
        "--track".to_string(),
        video_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        [av1_frame_a, av1_frame_b].concat()
    );
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_vp10_tracks() {
    let frame_a = build_test_vp10_keyframe(640, 360, 0);
    let frame_b = build_test_vp10_keyframe(640, 360, 0);
    let video_input = write_test_vp10_ivf_file(
        "mux-cli-raw-vp10-input",
        640,
        360,
        &[0, 1],
        &[frame_a.as_slice(), frame_b.as_slice()],
    );
    let output = write_temp_file("mux-cli-raw-vp10-output", &[]);
    let args = vec![
        "--track".to_string(),
        video_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        [frame_a, frame_b].concat()
    );

    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("vp10"),
        ]),
    );
    let vpcc = extract_boxes::<VpCodecConfiguration>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("vp10"),
            fourcc("vpcC"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].sample_entry.box_type, fourcc("vp10"));
    assert_eq!(video_entries[0].width, 640);
    assert_eq!(video_entries[0].height, 360);
    assert_eq!(vpcc.len(), 1);
    assert_eq!(vpcc[0].profile, 1);
    assert_eq!(vpcc[0].level, 10);
    assert_eq!(vpcc[0].bit_depth, 8);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_ac4_tracks() {
    let audio_input = write_test_ac4_file("mux-cli-raw-ac4-input", 2);
    let output = write_temp_file("mux-cli-raw-ac4-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ac-4"),
        ]),
    );
    let stts_boxes = extract_boxes::<Mdhd>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ac-4"));
    assert!(audio_entries[0].channel_count > 0);
    assert_eq!(stts_boxes.len(), 1);
    assert!(stts_boxes[0].timescale > 0);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_amr_tracks() {
    let audio_input = write_test_amr_file("mux-cli-raw-amr-input", &[b"one", b"two"]);
    let output = write_temp_file("mux-cli-raw-amr-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("samr"),
        ]),
    );
    let damr_boxes = extract_boxes::<Damr>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("samr"),
            fourcc("damr"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("samr"));
    assert_eq!(audio_entries[0].channel_count, 1);
    assert_eq!(damr_boxes.len(), 1);
    assert_eq!(damr_boxes[0].vendor, 0);
    assert_eq!(damr_boxes[0].frames_per_sample, 1);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 8_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_amr_wb_tracks() {
    let audio_input = write_test_amr_wb_file("mux-cli-raw-amr-wb-input", &[b"wide", b"band"]);
    let output = write_temp_file("mux-cli-raw-amr-wb-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("sawb"),
        ]),
    );
    let damr_boxes = extract_boxes::<Damr>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("sawb"),
            fourcc("damr"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("sawb"));
    assert_eq!(audio_entries[0].channel_count, 1);
    assert_eq!(damr_boxes.len(), 1);
    assert_eq!(damr_boxes[0].vendor, 0);
    assert_eq!(damr_boxes[0].frames_per_sample, 1);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 16_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_qcp_tracks() {
    let audio_input = write_test_qcp_constant_file(
        "mux-cli-raw-qcp-input",
        TestQcpCodecKind::Qcelp,
        &[&b"QCP1"[..], &b"QCP2"[..]],
    );
    let output = write_temp_file("mux-cli-raw-qcp-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("sqcp"),
        ]),
    );
    let dqcp_boxes = extract_boxes::<Dqcp>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("sqcp"),
            fourcc("dqcp"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("sqcp"));
    assert_eq!(dqcp_boxes.len(), 1);
    assert_eq!(dqcp_boxes[0].vendor, 0);
    assert_eq!(dqcp_boxes[0].frames_per_sample, 1);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 8_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_mp3_tracks() {
    let audio_input = write_test_mp3_file("mux-cli-raw-mp3-input", &[&b"abc"[..], &b"defg"[..]]);
    let expected_payload = fs::read(&audio_input).unwrap();
    let output = write_temp_file("mux-cli-raw-mp3-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc(".mp3"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc(".mp3"));
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_latm_tracks() {
    let audio_input = write_test_latm_file("mux-cli-raw-latm-input", &[b"abc", b"defg"]);
    let output = write_temp_file("mux-cli-raw-latm-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"abcdefg");

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4a"),
        ]),
    );
    let esds_boxes = extract_boxes::<Esds>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4a"),
            fourcc("esds"),
        ]),
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
    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("mp4a"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(esds_boxes.len(), 1);
    assert_eq!(
        esds_boxes[0]
            .decoder_config_descriptor()
            .unwrap()
            .object_type_indication,
        0x40
    );
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].name, "SoundHandler");
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_usac_latm_tracks() {
    let audio_input =
        write_test_usac_latm_file("mux-cli-raw-usac-latm-input", &[b"\x80abc", b"\x00defg"]);
    let output = write_temp_file("mux-cli-raw-usac-latm-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        b"\x80abc\x00defg"
    );

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4a"),
        ]),
    );
    let esds_boxes = extract_boxes::<Esds>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4a"),
            fourcc("esds"),
        ]),
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

    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("mp4a"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(esds_boxes.len(), 1);
    assert_eq!(esds_boxes[0].decoder_specific_info().unwrap().len(), 3);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_truehd_tracks() {
    let audio_input =
        write_test_truehd_file("mux-cli-raw-truehd-input", &[b"abcdefgh", b"ijklmnop"]);
    let expected_payload = fs::read(&audio_input).unwrap();
    let output = write_temp_file("mux-cli-raw-truehd-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mlpa"),
        ]),
    );
    let dmlp_boxes = extract_boxes::<Dmlp>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mlpa"),
            fourcc("dmlp"),
        ]),
    );
    let btrt_boxes = extract_boxes::<mp4forge::boxes::iso14496_12::Btrt>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mlpa"),
            fourcc("btrt"),
        ]),
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
    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("mlpa"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(audio_entries[0].sample_rate, 48_000);
    assert_eq!(dmlp_boxes.len(), 1);
    assert_eq!(dmlp_boxes[0].format_info, 0);
    assert_eq!(dmlp_boxes[0].peak_data_rate, 0);
    assert_eq!(btrt_boxes.len(), 1);
    assert_eq!(btrt_boxes[0].buffer_size_db, 40);
    assert_eq!(btrt_boxes[0].max_bitrate, 384_000);
    assert_eq!(btrt_boxes[0].avg_bitrate, 384_000);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].name, "SoundHandler");
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_mhas_tracks() {
    let audio_input = write_test_mhas_file("mux-cli-raw-mhas-input", &[b"frame-one", b"frame-two"]);
    let expected_payload = fs::read(&audio_input).unwrap();
    let output = write_temp_file("mux-cli-raw-mhas-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mhm1"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("mhm1"));
    assert_eq!(audio_entries[0].channel_count, 0);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_flac_tracks() {
    let audio_input = write_test_flac_file("mux-cli-raw-flac-input", b"flac-frame");
    let output = write_temp_file("mux-cli-raw-flac-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let input_bytes = fs::read(&audio_input).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        &input_bytes[42..]
    );

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("fLaC"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("fLaC"));
    assert_eq!(audio_entries[0].channel_count, 2);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_ogg_flac_tracks() {
    let audio_input = write_test_ogg_flac_file("mux-cli-raw-ogg-flac-input", &[b"abc", b"def"]);
    let output = write_temp_file("mux-cli-raw-ogg-flac-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("fLaC"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("fLaC"));
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_ogg_flac_mapping_tracks() {
    let audio_input =
        write_test_ogg_flac_mapping_file("mux-cli-raw-ogg-flac-mapping-input", &[b"abc", b"def"]);
    let output = write_temp_file("mux-cli-raw-ogg-flac-mapping-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("fLaC"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("fLaC"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_ogg_opus_tracks() {
    let audio_input = write_test_ogg_opus_file("mux-cli-raw-opus-input", &[b"abc", b"def"]);
    let output = write_temp_file("mux-cli-raw-opus-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"\0abc\0def");

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("Opus"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("Opus"));
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_wave_pcm_tracks() {
    let audio_input = write_test_wave_pcm_file(
        "mux-cli-raw-wave-pcm-input",
        &[[-1_000, 1_000], [2_000, -2_000]],
    );
    let output = write_temp_file("mux-cli-raw-wave-pcm-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    let expected_payload = fs::read(&audio_input).unwrap()[44..].to_vec();
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ipcm"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ipcm"));
    assert_eq!(audio_entries[0].channel_count, 2);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_aiff_pcm_tracks() {
    let audio_input = write_test_aiff_pcm_file(
        "mux-cli-raw-aiff-pcm-input",
        &[[-1_000, 1_000], [2_000, -2_000]],
    );
    let output = write_temp_file("mux-cli-raw-aiff-pcm-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    let expected_payload = vec![0xFC, 0x18, 0x03, 0xE8, 0x07, 0xD0, 0xF8, 0x30];
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ipcm"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ipcm"));
    assert_eq!(audio_entries[0].channel_count, 2);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_aifc_pcm_tracks() {
    let audio_input = write_test_aifc_pcm_file(
        "mux-cli-raw-aifc-pcm-input",
        &[[-1_000, 1_000], [2_000, -2_000]],
    );
    let output = write_temp_file("mux-cli-raw-aifc-pcm-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    let expected_payload = vec![0xFC, 0x18, 0x03, 0xE8, 0x07, 0xD0, 0xF8, 0x30];
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ipcm"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ipcm"));
    assert_eq!(audio_entries[0].channel_count, 2);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_ogg_vorbis_tracks() {
    let audio_input = write_test_ogg_vorbis_file("mux-cli-raw-vorbis-input", &[b"abc", b"def"]);
    let output = write_temp_file("mux-cli-raw-vorbis-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4a"),
        ]),
    );
    let esds_boxes = extract_boxes::<Esds>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4a"),
            fourcc("esds"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("mp4a"));
    assert_eq!(
        esds_boxes[0]
            .decoder_config_descriptor()
            .unwrap()
            .object_type_indication,
        0xDD
    );
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_ogg_speex_tracks() {
    let audio_input = write_test_ogg_speex_file("mux-cli-raw-speex-input", &[b"abc", b"def"]);
    let output = write_temp_file("mux-cli-raw-speex-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("spex"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("spex"));
    assert_eq!(audio_entries[0].channel_count, 0);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_ogg_theora_tracks() {
    let video_input =
        write_test_ogg_theora_file("mux-cli-raw-theora-input", &[b"frame-a", b"frame-b"]);
    let output = write_temp_file("mux-cli-raw-theora-output", &[]);
    let args = vec![
        "--track".to_string(),
        video_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
        ]),
    );
    let esds_boxes = extract_boxes::<Esds>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
            fourcc("esds"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].sample_entry.box_type, fourcc("mp4v"));
    assert_eq!(video_entries[0].width, 320);
    assert_eq!(video_entries[0].height, 240);
    assert_eq!(
        esds_boxes[0]
            .decoder_config_descriptor()
            .unwrap()
            .object_type_indication,
        0xDF
    );
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_jpeg_tracks() {
    let image_input = write_test_jpeg_file("mux-cli-raw-jpeg-input");
    let output = write_temp_file("mux-cli-raw-jpeg-output", &[]);
    let args = vec![
        "--track".to_string(),
        image_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let input_bytes = fs::read(&image_input).unwrap();
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), input_bytes);

    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("jpeg"),
        ]),
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
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].sample_entry.box_type, fourcc("jpeg"));
    assert_eq!(video_entries[0].width, 1);
    assert_eq!(video_entries[0].height, 1);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(video_entries[0].horizresolution, 72);
    assert_eq!(video_entries[0].vertresolution, 72);
    assert_eq!(mdhd_boxes[0].timescale, 1_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_h263_tracks() {
    let video_input = write_test_h263_file("mux-cli-raw-h263-input", &[b"frame-a", b"frame-b"]);
    let output = write_temp_file("mux-cli-raw-h263-output", &[]);
    let args = vec![
        "--track".to_string(),
        video_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let input_bytes = fs::read(&video_input).unwrap();
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), input_bytes);

    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("s263"),
        ]),
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
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].sample_entry.box_type, fourcc("s263"));
    assert_eq!(video_entries[0].width, 176);
    assert_eq!(video_entries[0].height, 144);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 15_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_png_tracks() {
    let image_input = write_test_png_file("mux-cli-raw-png-input");
    let output = write_temp_file("mux-cli-raw-png-output", &[]);
    let args = vec![
        "--track".to_string(),
        image_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("png "),
        ]),
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
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].sample_entry.box_type, fourcc("png "));
    assert_eq!(video_entries[0].width, 1);
    assert_eq!(video_entries[0].height, 1);
    assert_eq!(video_entries[0].horizresolution, 72);
    assert_eq!(video_entries[0].vertresolution, 72);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 1_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_iamf_tracks() {
    let audio_input = write_test_iamf_file("mux-cli-raw-iamf-input", &[b"frame-one", b"frame-two"]);
    let output = write_temp_file("mux-cli-raw-iamf-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("iamf"),
        ]),
    );
    let iacb_boxes = extract_boxes::<Iacb>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("iamf"),
            fourcc("iacb"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("iamf"));
    assert_eq!(audio_entries[0].channel_count, 0);
    assert_eq!(audio_entries[0].sample_size, 0);
    assert_eq!(audio_entries[0].sample_rate, 0);
    assert_eq!(iacb_boxes.len(), 1);
    assert_eq!(iacb_boxes[0].configuration_version, 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_caf_alac_tracks() {
    let audio_input = write_test_caf_alac_file("mux-cli-raw-alac-input", &[b"ABCD", b"EFGH"]);
    let output = write_temp_file("mux-cli-raw-alac-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"ABCDEFGH");

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
    assert_eq!(audio_entries[0].channel_count, 2);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_variable_packet_caf_alac_tracks() {
    let packet_a = vec![b'A'; 1_977];
    let packet_b = vec![b'B'; 254];
    let audio_input = write_test_caf_alac_variable_packet_file(
        "mux-cli-raw-alac-variable-input",
        &[packet_a.as_slice(), packet_b.as_slice()],
    );
    let output = write_temp_file("mux-cli-raw-alac-variable-output", &[]);
    let args = vec![
        "--track".to_string(),
        audio_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    let payload = mdat_payload(&output_bytes, root_boxes[2]);
    assert_eq!(payload.len(), packet_a.len() + packet_b.len());
    assert_eq!(&payload[..packet_a.len()], packet_a.as_slice());
    assert_eq!(&payload[packet_a.len()..], packet_b.as_slice());

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
    let mdhd_boxes = extract_boxes::<Mdhd>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("alac"));
    assert_eq!(audio_entries[0].channel_count, 1);
    assert_eq!(mdhd_boxes[0].timescale, 44_100);
}

#[test]
fn mux_command_writes_real_mp4_output_from_path_first_h265_tracks() {
    let video_input = write_test_h265_annexb_file("mux-cli-raw-h265-input", &[b"hevc"]);
    let output = write_temp_file("mux-cli-raw-h265-output", &[]);
    let args = vec![
        "--track".to_string(),
        video_input.display().to_string(),
        output.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = mux::run(&args, &mut stderr);

    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output_bytes = fs::read(output).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        mp4forge::walk::BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("hvc1"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].sample_entry.box_type, fourcc("hvc1"));
    assert_eq!(video_entries[0].width, 1920);
    assert_eq!(video_entries[0].height, 1080);
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
