#![cfg(feature = "mux")]

mod support;

use std::fs;
use std::io::Cursor;
use std::str::FromStr;

use mp4forge::BoxInfo;
use mp4forge::boxes::av1::AV1CodecConfiguration;
use mp4forge::boxes::dolby::Dmlp;
use mp4forge::boxes::dts::Ddts;
use mp4forge::boxes::etsi_ts_103_190::Dac4;
use mp4forge::boxes::flac::DfLa;
use mp4forge::boxes::iamf::Iacb;
use mp4forge::boxes::iso14496_12::{
    AVCDecoderConfiguration, AudioSampleEntry, Btrt, Chnl, Co64, Colr, Ctts, Dinf, Dref, DvsC,
    Edts, Elst, Ftyp, GenericMediaSampleEntry, Hdlr, Mdhd, Mdia, Mehd, Meta, Minf, Moov, Mvex,
    Mvhd, Nmhd, Pasp, SampleEntry, Sbgp, Sgpd, Sidx, Smhd, Stbl, Stco, Sthd, Stsc, StscEntry, Stsd,
    Stss, Stsz, Stts, SttsEntry, Tfdt, Tfhd, Tkhd, Trak, Trex, Trun, Url, VisualSampleEntry, Vmhd,
    XMLSubtitleSampleEntry,
};
use mp4forge::boxes::iso14496_14::Esds;
use mp4forge::boxes::iso14496_14::Iods;
use mp4forge::boxes::iso14496_15::VVCDecoderConfiguration;
use mp4forge::boxes::iso14496_30::{WVTTSampleEntry, WebVTTConfigurationBox, WebVTTSourceLabelBox};
use mp4forge::boxes::iso23001_5::PcmC;
use mp4forge::boxes::metadata::Id32;
use mp4forge::boxes::mpeg_h::MhaC;
use mp4forge::boxes::threegpp::{D263, Damr, Devc, Dqcp, Dsmv};
use mp4forge::boxes::vp::VpCodecConfiguration;
use mp4forge::codec::{ImmutableBox, MutableBox};
use mp4forge::extract::{extract_box_as, extract_box_bytes};
#[cfg(feature = "async")]
use mp4forge::mux::mux_to_path_async;
use mp4forge::mux::{
    MuxDurationMode, MuxError, MuxFileConfig, MuxInterleavePolicy, MuxMp4TrackSelector,
    MuxOutputLayout, MuxRequest, MuxStagedMediaItem, MuxTrackConfig, MuxTrackKind, MuxTrackSpec,
    copy_planned_payloads, copy_planned_payloads_async, copy_planned_payloads_async_progressive,
    copy_planned_payloads_progressive, copy_planned_payloads_to_path,
    copy_planned_payloads_to_path_async, mux_into_path, mux_to_path, plan_staged_media_items,
    write_mp4_mux, write_mp4_mux_to_path, write_mp4_mux_to_path_async,
};
use mp4forge::probe::{TrackCodecDetails, probe_codec_detailed_bytes};
use mp4forge::walk::BoxPath;
#[cfg(feature = "async")]
use tokio::io::AsyncWriteExt;

use support::{
    TestAviAvc1Stream, TestAviH264Stream, TestAviMp4vStream, TestAviPcmStream, TestMuxSample,
    TestQcpCodecKind, build_test_av1_sequence_header_obu, build_test_mp4v_decoder_specific_info,
    build_test_vp8_keyframe, build_test_vp9_keyframe, build_test_vp10_keyframe, encode_raw_box,
    encode_supported_box, fixture_path, fourcc, write_single_track_mp4_input, write_temp_file,
    write_test_ac3_44100_file, write_test_ac3_file, write_test_ac4_file, write_test_adts_file,
    write_test_aifc_pcm_file, write_test_aiff_pcm_file, write_test_amr_file,
    write_test_amr_wb_file, write_test_av1_ivf_file, write_test_avi_ac3_file,
    write_test_avi_avc1_file, write_test_avi_h263_file, write_test_avi_h264_file,
    write_test_avi_jpeg_file, write_test_avi_mp3_file, write_test_avi_mp4v_file,
    write_test_avi_pcm_file, write_test_avi_png_file, write_test_caf_alac_file,
    write_test_caf_alac_variable_packet_file, write_test_dts_file, write_test_eac3_file,
    write_test_flac_file, write_test_flac_file_with_frames,
    write_test_flac_file_with_frames_and_block_size, write_test_h263_file,
    write_test_h264_annexb_file, write_test_h265_annexb_file,
    write_test_h265_annexb_file_with_timing, write_test_iamf_file, write_test_jpeg_file,
    write_test_latm_file, write_test_mhas_file, write_test_mp3_44100_file, write_test_mp3_file,
    write_test_mp3_file_with_leading_id3_tag, write_test_mp4v_file, write_test_ogg_flac_file,
    write_test_ogg_flac_mapping_file, write_test_ogg_opus_file, write_test_ogg_speex_file,
    write_test_ogg_theora_file, write_test_ogg_vorbis_file, write_test_png_file,
    write_test_program_stream_ac3_file, write_test_program_stream_h264_file,
    write_test_program_stream_h265_file, write_test_program_stream_mp3_file,
    write_test_program_stream_mp4v_file, write_test_program_stream_vobsub_file,
    write_test_program_stream_vvc_file, write_test_qcp_constant_file, write_test_qcp_variable_file,
    write_test_transport_stream_ac3_file, write_test_transport_stream_dvb_subtitle_file,
    write_test_transport_stream_dvb_teletext_file, write_test_transport_stream_eac3_file,
    write_test_transport_stream_h264_file, write_test_transport_stream_h265_file,
    write_test_transport_stream_mp3_file, write_test_transport_stream_mp4v_file,
    write_test_transport_stream_vvc_file, write_test_truehd_file, write_test_usac_latm_file,
    write_test_vobsub_files, write_test_vp8_ivf_file, write_test_vp9_ivf_file,
    write_test_vp10_ivf_file, write_test_wave_pcm_file,
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
fn mux_track_spec_from_str_accepts_the_path_first_public_grammar() {
    assert_eq!(
        MuxTrackSpec::from_str("path/to/video.h264").unwrap(),
        MuxTrackSpec::path("path/to/video.h264")
    );
    assert_eq!(
        MuxTrackSpec::from_str("path/to/audio.aac").unwrap(),
        MuxTrackSpec::path("path/to/audio.aac")
    );
    assert_eq!(
        MuxTrackSpec::from_str("path/to/file.mp4#video").unwrap(),
        MuxTrackSpec::selected("path/to/file.mp4", MuxMp4TrackSelector::Video)
    );
    assert_eq!(
        MuxTrackSpec::from_str("path/to/file.mp4#audio").unwrap(),
        MuxTrackSpec::selected(
            "path/to/file.mp4",
            MuxMp4TrackSelector::Audio { occurrence: 1 }
        )
    );
    assert_eq!(
        MuxTrackSpec::from_str("path/to/file.mp4#audio:2").unwrap(),
        MuxTrackSpec::selected(
            "path/to/file.mp4",
            MuxMp4TrackSelector::Audio { occurrence: 2 }
        )
    );
    assert_eq!(
        MuxTrackSpec::from_str("path/to/file.mp4#text").unwrap(),
        MuxTrackSpec::selected(
            "path/to/file.mp4",
            MuxMp4TrackSelector::Text { occurrence: 1 }
        )
    );
    assert_eq!(
        MuxTrackSpec::from_str("path/to/file.mp4#track:7").unwrap(),
        MuxTrackSpec::selected(
            "path/to/file.mp4",
            MuxMp4TrackSelector::TrackId { track_id: 7 }
        )
    );
}

#[test]
fn mux_track_spec_from_str_rejects_public_parameter_suffixes() {
    let error = MuxTrackSpec::from_str("path/to/video.h265#sample_entry=hvc1").unwrap_err();
    assert!(matches!(error, MuxError::InvalidTrackSpec { .. }));
    assert!(
        error
            .to_string()
            .contains("public mux track specs only allow selector suffixes"),
        "{error}"
    );
}

#[test]
fn mux_to_path_imports_path_only_raw_dts_inputs() {
    let dts_input = write_test_dts_file("mux-raw-dts-input", 2);
    let expected_payload = fs::read(&dts_input).unwrap();
    let output_path = write_temp_file("mux-raw-dts-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&dts_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("dtsc"),
        ]),
    );
    let ddts_boxes = extract_boxes::<Ddts>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("dtsc"),
            fourcc("ddts"),
        ]),
    );
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("dtsc"),
            fourcc("btrt"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("dtsc"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert!(ddts_boxes.is_empty());
    assert_eq!(btrt_boxes.len(), 1);
    assert_eq!(btrt_boxes[0].buffer_size_db, 2_048);
    assert_eq!(btrt_boxes[0].max_bitrate, 768_000);
    assert_eq!(btrt_boxes[0].avg_bitrate, 768_000);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 90_000);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1_920);
}

#[test]
fn mux_to_path_imports_path_only_avi_pcm_inputs() {
    let chunk = [0_u8, 0, 0, 0, 1, 0, 1, 0];
    let avi_input = write_test_avi_pcm_file(
        "mux-avi-pcm-input",
        &[TestAviPcmStream {
            sample_rate: 48_000,
            channel_count: 2,
            bits_per_sample: 16,
            chunks: &[&chunk],
        }],
    );
    let output_path = write_temp_file("mux-avi-pcm-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&avi_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ipcm"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ipcm"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(audio_entries[0].sample_rate, 48_000 << 16);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(
        stts_boxes[0].entries,
        vec![SttsEntry {
            sample_count: 1,
            sample_delta: 2,
        }]
    );
}

#[test]
fn mux_to_path_imports_path_only_mp4v_inputs() {
    let decoder_specific_info = build_test_mp4v_decoder_specific_info(320, 180);
    let intra_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x00, 0xAA, 0xBB];
    let predictive_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x40, 0xCC, 0xDD];
    let mut elementary = decoder_specific_info.clone();
    elementary.extend_from_slice(&intra_frame);
    elementary.extend_from_slice(&predictive_frame);
    let mp4v_input = write_test_mp4v_file("mux-mp4v-input", &elementary);
    let output_path = write_temp_file("mux-mp4v-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&mp4v_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    let pasp_boxes = extract_boxes::<Pasp>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
            fourcc("pasp"),
        ]),
    );
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
            fourcc("btrt"),
        ]),
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
    let stss_boxes = extract_boxes::<Stss>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stss"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].sample_entry.box_type, fourcc("mp4v"));
    assert_eq!(video_entries[0].width, 320);
    assert_eq!(video_entries[0].height, 180);
    assert_eq!(video_entries[0].compressorname[0], 0);
    assert_eq!(esds_boxes.len(), 1);
    assert_eq!(
        esds_boxes[0].decoder_specific_info().unwrap(),
        decoder_specific_info
    );
    assert_eq!(pasp_boxes.len(), 1);
    assert_eq!(pasp_boxes[0].h_spacing, 1);
    assert_eq!(pasp_boxes[0].v_spacing, 1);
    assert!(btrt_boxes.is_empty());
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 25_000);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(
        stts_boxes[0].entries,
        vec![SttsEntry {
            sample_count: 2,
            sample_delta: 1_000,
        }]
    );
    assert_eq!(stss_boxes.len(), 1);
    assert_eq!(stss_boxes[0].sample_number, vec![1]);
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        [&intra_frame[..], &predictive_frame[..]].concat()
    );
}

#[test]
fn mux_to_path_imports_path_only_avi_mp4v_inputs() {
    let decoder_specific_info = [0x00_u8, 0x00, 0x01, 0x20, 0x11, 0x22];
    let intra_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x00, 0xAA, 0xBB];
    let predictive_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x40, 0xCC, 0xDD];
    let avi_input = write_test_avi_mp4v_file(
        "mux-avi-mp4v-input",
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
    let expected_payload = [&intra_frame[..], &predictive_frame[..]].concat();
    let output_path = write_temp_file("mux-avi-mp4v-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&avi_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    let stss_boxes = extract_boxes::<Stss>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stss"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].sample_entry.box_type, fourcc("mp4v"));
    assert_eq!(video_entries[0].width, 320);
    assert_eq!(video_entries[0].height, 180);
    assert_eq!(esds_boxes.len(), 1);
    assert_eq!(
        esds_boxes[0]
            .decoder_config_descriptor()
            .unwrap()
            .object_type_indication,
        0x20
    );
    assert_eq!(
        esds_boxes[0].decoder_specific_info().unwrap(),
        decoder_specific_info
    );
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 25_000);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(
        stts_boxes[0].entries,
        vec![SttsEntry {
            sample_count: 2,
            sample_delta: 1_000,
        }]
    );
    assert_eq!(stss_boxes.len(), 1);
    assert_eq!(stss_boxes[0].sample_number, vec![1]);
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);
}

#[test]
fn mux_to_path_imports_path_only_avi_h264_inputs() {
    let avi_input = write_test_avi_h264_file(
        "mux-avi-h264-input",
        &TestAviH264Stream {
            width: 320,
            height: 180,
            frame_scale: 1,
            frame_rate: 25,
            compression: *b"H264",
            sample_payloads: &[b"\xAA\xBB", b"\xCC\xDD"],
        },
    );
    let output_path = write_temp_file("mux-avi-h264-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&avi_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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

    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 320);
    assert_eq!(video_entries[0].height, 180);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 25_000);
    assert_eq!(
        stts_boxes[0].entries,
        vec![SttsEntry {
            sample_count: 2,
            sample_delta: 1_000,
        }]
    );
}

#[test]
fn mux_to_path_imports_path_only_avi_avc1_inputs() {
    let avi_input = write_test_avi_avc1_file(
        "mux-avi-avc1-input",
        &TestAviAvc1Stream {
            width: 320,
            height: 180,
            frame_scale: 1,
            frame_rate: 25,
            sample_payloads: &[b"\xAA\xBB", b"\xCC\xDD"],
        },
    );
    let output_path = write_temp_file("mux-avi-avc1-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&avi_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let mdhd_boxes = extract_boxes::<Mdhd>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    let stss_boxes = extract_boxes::<Stss>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stss"),
        ]),
    );
    let colr_boxes = extract_boxes::<Colr>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("avc1"),
            fourcc("colr"),
        ]),
    );
    let avcc_boxes = extract_boxes::<AVCDecoderConfiguration>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("avc1"),
            fourcc("avcC"),
        ]),
    );

    assert!(output_bytes.windows(4).any(|bytes| bytes == b"avc1"));
    assert!(output_bytes.windows(4).any(|bytes| bytes == b"avcC"));
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 25_000);
    assert_eq!(stss_boxes.len(), 1);
    assert!(stss_boxes[0].sample_number.is_empty());
    assert!(colr_boxes.is_empty());
    assert_eq!(avcc_boxes.len(), 1);
    assert!(avcc_boxes[0].high_profile_fields_enabled);
    assert_eq!(avcc_boxes[0].chroma_format, 0);
    assert_eq!(avcc_boxes[0].num_of_sequence_parameter_set_ext, 0);
}

#[test]
fn mux_to_path_imports_path_only_avi_mp3_inputs() {
    let avi_input = write_test_avi_mp3_file(
        "mux-avi-mp3-input",
        48_000,
        2,
        &[b"avi-mp3-a", b"avi-mp3-b"],
    );
    let output_path = write_temp_file("mux-avi-mp3-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&avi_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
fn mux_to_path_imports_path_only_avi_ac3_inputs() {
    let avi_input = write_test_avi_ac3_file(
        "mux-avi-ac3-input",
        48_000,
        2,
        &[b"avi-ac3-a", b"avi-ac3-b"],
    );
    let output_path = write_temp_file("mux-avi-ac3-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&avi_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
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
    let mdhd_boxes = extract_boxes::<Mdhd>(
        &output_bytes,
        BoxPath::from([
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
fn mux_to_path_imports_path_only_avi_h263_inputs() {
    let avi_input = write_test_avi_h263_file(
        "mux-avi-h263-input",
        176,
        144,
        1,
        25,
        &[b"\xAA\xBB", b"\xCC\xDD"],
    );
    let output_path = write_temp_file("mux-avi-h263-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&avi_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
        BoxPath::from([
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
fn mux_to_path_imports_path_only_avi_jpeg_inputs() {
    let jpeg_frame = fs::read(fixture_path("generated-1x1.jpg")).unwrap();
    let avi_input = write_test_avi_jpeg_file("mux-avi-jpeg-input", 1, 1, 1, 25, &[&jpeg_frame]);
    let output_path = write_temp_file("mux-avi-jpeg-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&avi_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
fn mux_to_path_imports_path_only_avi_png_inputs() {
    let png_frame_path = write_test_png_file("mux-avi-png-frame");
    let png_frame = fs::read(png_frame_path).unwrap();
    let avi_input = write_test_avi_png_file("mux-avi-png-input", 1, 1, 1, 25, &[&png_frame]);
    let output_path = write_temp_file("mux-avi-png-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&avi_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
fn mux_to_path_imports_path_only_program_stream_mp4v_inputs() {
    let decoder_specific_info = build_test_mp4v_decoder_specific_info(320, 180);
    let intra_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x00, 0xAA, 0xBB];
    let predictive_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x40, 0xCC, 0xDD];
    let first_payload = [&decoder_specific_info[..], &intra_frame[..]].concat();
    let ps_input = write_test_program_stream_mp4v_file(
        "mux-program-stream-mp4v-input",
        &[&first_payload, &predictive_frame],
    );
    let output_path = write_temp_file("mux-program-stream-mp4v-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ps_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
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
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].sample_entry.box_type, fourcc("mp4v"));
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
}

#[test]
fn mux_to_path_imports_path_only_program_stream_mp3_inputs() {
    let ps_input = write_test_program_stream_mp3_file(
        "mux-program-stream-mp3-input",
        &[&[0x11; 96], &[0x22; 96]],
    );
    let output_path = write_temp_file("mux-program-stream-mp3-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ps_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc(".mp3"),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc(".mp3"));
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
}

#[test]
fn mux_to_path_imports_path_only_program_stream_ac3_inputs() {
    let raw_input = write_test_ac3_file("mux-program-stream-ac3-raw-input", &[b"ps", b"ac3"]);
    let expected_payload = fs::read(&raw_input).unwrap();
    let ps_input =
        write_test_program_stream_ac3_file("mux-program-stream-ac3-input", &[b"ps", b"ac3"]);
    let output_path = write_temp_file("mux-program-stream-ac3-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ps_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ac-3"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 90_000);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(
        stts_boxes[0].entries,
        vec![SttsEntry {
            sample_count: 2,
            sample_delta: 2_880,
        }]
    );
}

#[test]
fn mux_to_path_imports_path_only_program_stream_h264_inputs() {
    let ps_input =
        write_test_program_stream_h264_file("mux-program-stream-h264-input", &[b"idr-sample"]);
    let output_path = write_temp_file("mux-program-stream-h264-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ps_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    assert_eq!(mdhd_boxes[0].timescale, 20);
}

#[test]
fn mux_to_path_imports_path_only_program_stream_h265_inputs() {
    let ps_input =
        write_test_program_stream_h265_file("mux-program-stream-h265-input", &[b"hevc-sample"]);
    let output_path = write_temp_file("mux-program-stream-h265-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ps_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
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
    let mdhd_boxes = extract_boxes::<Mdhd>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 1920);
    assert_eq!(video_entries[0].height, 1080);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 30);
}

#[test]
fn mux_to_path_imports_path_only_program_stream_vvc_inputs() {
    let ps_input = write_test_program_stream_vvc_file("mux-program-stream-vvc-input", &[]);
    let output_path = write_temp_file("mux-program-stream-vvc-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ps_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 1280);
    assert_eq!(video_entries[0].height, 720);
    assert_eq!(vvc_boxes.len(), 1);
    assert!(!vvc_boxes[0].decoder_configuration_record.is_empty());
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 25);
    assert_eq!(mdhd_boxes[0].duration(), 2);
}

#[test]
fn mux_to_path_imports_path_only_transport_stream_mp4v_inputs() {
    let decoder_specific_info = build_test_mp4v_decoder_specific_info(320, 180);
    let intra_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x00, 0xAA, 0xBB];
    let predictive_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x40, 0xCC, 0xDD];
    let first_payload = [&decoder_specific_info[..], &intra_frame[..]].concat();
    let ts_input = write_test_transport_stream_mp4v_file(
        "mux-transport-stream-mp4v-input",
        &[&first_payload, &predictive_frame],
    );
    let output_path = write_temp_file("mux-transport-stream-mp4v-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ts_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
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
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].sample_entry.box_type, fourcc("mp4v"));
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
}

#[test]
fn mux_to_path_imports_path_only_transport_stream_h264_inputs() {
    let ts_input =
        write_test_transport_stream_h264_file("mux-transport-stream-h264-input", &[b"idr-sample"]);
    let output_path = write_temp_file("mux-transport-stream-h264-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ts_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    assert_eq!(mdhd_boxes[0].timescale, 20);
}

#[test]
fn mux_to_path_imports_path_only_transport_stream_h265_inputs() {
    let ts_input =
        write_test_transport_stream_h265_file("mux-transport-stream-h265-input", &[b"hevc-sample"]);
    let output_path = write_temp_file("mux-transport-stream-h265-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ts_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
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
    let mdhd_boxes = extract_boxes::<Mdhd>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 1920);
    assert_eq!(video_entries[0].height, 1080);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 30);
}

#[test]
fn mux_to_path_imports_path_only_transport_stream_vvc_inputs() {
    let ts_input = write_test_transport_stream_vvc_file("mux-transport-stream-vvc-input", &[]);
    let output_path = write_temp_file("mux-transport-stream-vvc-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ts_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );

    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 1280);
    assert_eq!(video_entries[0].height, 720);
    assert_eq!(vvc_boxes.len(), 1);
    assert!(!vvc_boxes[0].decoder_configuration_record.is_empty());
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 25);
    assert_eq!(mdhd_boxes[0].duration(), 2);
}

#[test]
fn mux_to_path_imports_path_only_transport_stream_ac3_inputs() {
    let raw_input = write_test_ac3_file("mux-transport-stream-ac3-raw-input", &[b"ac3", b"ts"]);
    let expected_payload = fs::read(&raw_input).unwrap();
    let ts_input =
        write_test_transport_stream_ac3_file("mux-transport-stream-ac3-input", &[b"ac3", b"ts"]);
    let output_path = write_temp_file("mux-transport-stream-ac3-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ts_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
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
    let mdhd_boxes = extract_boxes::<Mdhd>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ac-3"));
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
}

#[test]
fn mux_to_path_imports_path_only_transport_stream_eac3_inputs() {
    let raw_input = write_test_eac3_file("mux-transport-stream-eac3-raw-input", &[b"ec3", b"ts"]);
    let expected_payload = fs::read(&raw_input).unwrap();
    let ts_input =
        write_test_transport_stream_eac3_file("mux-transport-stream-eac3-input", &[b"ec3", b"ts"]);
    let output_path = write_temp_file("mux-transport-stream-eac3-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ts_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
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
    let mdhd_boxes = extract_boxes::<Mdhd>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ec-3"));
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
}

#[test]
fn mux_to_path_imports_path_only_transport_stream_dvb_subtitle_inputs() {
    let ts_input = write_test_transport_stream_dvb_subtitle_file(
        "mux-transport-stream-dvb-subtitle-input",
        &[b"\x20sub-1", b"\x21sub-2"],
    );
    let output_path = write_temp_file("mux-transport-stream-dvb-subtitle-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ts_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let subtitle_entries = extract_boxes::<GenericMediaSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    let mdhd_boxes = extract_boxes::<Mdhd>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
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
    let root_boxes = read_root_boxes(&output_bytes);

    assert_eq!(subtitle_entries.len(), 1);
    assert_eq!(subtitle_entries[0].sample_entry.box_type, fourcc("dvbs"));
    assert_eq!(dvsc_boxes.len(), 1);
    assert_eq!(dvsc_boxes[0].composition_page_id, 0x0123);
    assert_eq!(dvsc_boxes[0].ancillary_page_id, 0x0456);
    assert_eq!(dvsc_boxes[0].subtitle_type, 0x10);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 1_000);
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].handler_type, fourcc("subt"));
    assert_eq!(hdlr_boxes[0].name, "SubtitleHandler");
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        b"\x20sub-1\x21sub-2"
    );
}

#[test]
fn mux_to_path_imports_path_only_transport_stream_dvb_teletext_inputs() {
    let ts_input = write_test_transport_stream_dvb_teletext_file(
        "mux-transport-stream-dvb-teletext-input",
        &[b"\x10text-1", b"\x11text-2"],
    );
    let output_path = write_temp_file("mux-transport-stream-dvb-teletext-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ts_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let subtitle_entries = extract_boxes::<GenericMediaSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("dvbt"),
        ]),
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
    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );
    let root_boxes = read_root_boxes(&output_bytes);

    assert_eq!(subtitle_entries.len(), 1);
    assert_eq!(subtitle_entries[0].sample_entry.box_type, fourcc("dvbt"));
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 1_000);
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].handler_type, fourcc("subt"));
    assert_eq!(hdlr_boxes[0].name, "SubtitleHandler");
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        b"\x10text-1\x11text-2"
    );
}

#[test]
fn mux_to_path_imports_path_only_vobsub_idx_inputs() {
    let (idx_input, _sub_input) = write_test_vobsub_files(
        "mux-vobsub-idx-input",
        &[0, 1_000],
        &[b"\xAA\xBB", b"\xCC\xDD"],
    );
    let output_path = write_temp_file("mux-vobsub-idx-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&idx_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let subtitle_entries = extract_boxes::<GenericMediaSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4s"),
        ]),
    );
    let esds_boxes = extract_boxes::<Esds>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4s"),
            fourcc("esds"),
        ]),
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
    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
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
    let iods_boxes = extract_boxes::<Iods>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
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
    let stco_boxes = extract_boxes::<Stco>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stco"),
        ]),
    );

    assert_eq!(subtitle_entries.len(), 1);
    assert_eq!(subtitle_entries[0].sample_entry.box_type, fourcc("mp4s"));
    assert_eq!(esds_boxes.len(), 1);
    let decoder_config = esds_boxes[0].decoder_config_descriptor().unwrap();
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 90_000);
    assert_eq!(mdhd_boxes[0].duration_v0, 90_000);
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].handler_type, fourcc("subp"));
    assert_eq!(hdlr_boxes[0].name, "SubtitleHandler");
    assert_eq!(nmhd_boxes.len(), 1);
    assert_eq!(sthd_boxes.len(), 0);
    assert_eq!(iods_boxes.len(), 1);
    let iods_descriptor = iods_boxes[0].initial_object_descriptor().unwrap();
    assert_eq!(iods_descriptor.audio_profile_level_indication, 0xff);
    assert_eq!(iods_descriptor.visual_profile_level_indication, 0xff);
    assert_eq!(stsz_boxes.len(), 1);
    assert_eq!(stsz_boxes[0].sample_count, 2);
    let expected_buffer_size = stsz_boxes[0].sample_size;
    let expected_bitrate = expected_buffer_size
        .checked_mul(stsz_boxes[0].sample_count)
        .and_then(|value| value.checked_mul(8))
        .unwrap();
    assert_eq!(decoder_config.buffer_size_db, expected_buffer_size);
    assert_eq!(decoder_config.max_bitrate, expected_bitrate);
    assert_eq!(decoder_config.avg_bitrate, expected_bitrate);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries.len(), 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 90_000);
    assert_eq!(stts_boxes[0].entries[1].sample_delta, 0);
    assert_eq!(stsc_boxes.len(), 1);
    assert_eq!(stsc_boxes[0].entries.len(), 2);
    assert_eq!(stsc_boxes[0].entries[0].first_chunk, 1);
    assert_eq!(stsc_boxes[0].entries[0].samples_per_chunk, 1);
    assert_eq!(stsc_boxes[0].entries[1].first_chunk, 2);
    assert_eq!(stsc_boxes[0].entries[1].samples_per_chunk, 1);
    assert_eq!(stco_boxes.len(), 1);
    assert_eq!(stco_boxes[0].entry_count, 2);
}

#[test]
fn mux_to_path_imports_path_only_program_stream_vobsub_inputs() {
    let ps_input = write_test_program_stream_vobsub_file(
        "mux-program-stream-vobsub-input",
        &[0, 1_000],
        &[b"\xAA\xBB", b"\xCC\xDD"],
    );
    let output_path = write_temp_file("mux-program-stream-vobsub-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ps_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let subtitle_entries = extract_boxes::<GenericMediaSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4s"),
        ]),
    );
    let esds_boxes = extract_boxes::<Esds>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4s"),
            fourcc("esds"),
        ]),
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
    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
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
    let iods_boxes = extract_boxes::<Iods>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
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
    let stco_boxes = extract_boxes::<Stco>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stco"),
        ]),
    );

    assert_eq!(subtitle_entries.len(), 1);
    assert_eq!(subtitle_entries[0].sample_entry.box_type, fourcc("mp4s"));
    assert_eq!(esds_boxes.len(), 1);
    let decoder_config = esds_boxes[0].decoder_config_descriptor().unwrap();
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 90_000);
    assert_eq!(mdhd_boxes[0].duration_v0, 90_000);
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].handler_type, fourcc("subp"));
    assert_eq!(hdlr_boxes[0].name, "SubtitleHandler");
    assert_eq!(nmhd_boxes.len(), 1);
    assert_eq!(sthd_boxes.len(), 0);
    assert_eq!(iods_boxes.len(), 1);
    let iods_descriptor = iods_boxes[0].initial_object_descriptor().unwrap();
    assert_eq!(iods_descriptor.audio_profile_level_indication, 0xff);
    assert_eq!(iods_descriptor.visual_profile_level_indication, 0xff);
    assert_eq!(stsz_boxes.len(), 1);
    assert_eq!(stsz_boxes[0].sample_count, 2);
    let expected_buffer_size = stsz_boxes[0].sample_size;
    let expected_bitrate = expected_buffer_size
        .checked_mul(stsz_boxes[0].sample_count)
        .and_then(|value| value.checked_mul(8))
        .unwrap();
    assert_eq!(decoder_config.buffer_size_db, expected_buffer_size);
    assert_eq!(decoder_config.max_bitrate, expected_bitrate);
    assert_eq!(decoder_config.avg_bitrate, expected_bitrate);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries.len(), 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 90_000);
    assert_eq!(stts_boxes[0].entries[1].sample_delta, 0);
    assert_eq!(stsc_boxes.len(), 1);
    assert_eq!(stsc_boxes[0].entries.len(), 2);
    assert_eq!(stsc_boxes[0].entries[0].first_chunk, 1);
    assert_eq!(stsc_boxes[0].entries[0].samples_per_chunk, 1);
    assert_eq!(stsc_boxes[0].entries[1].first_chunk, 2);
    assert_eq!(stsc_boxes[0].entries[1].samples_per_chunk, 1);
    assert_eq!(stco_boxes.len(), 1);
    assert_eq!(stco_boxes[0].entry_count, 2);
}

#[test]
fn mux_to_path_imports_path_only_transport_stream_mp3_inputs() {
    let ts_input = write_test_transport_stream_mp3_file(
        "mux-transport-stream-mp3-input",
        &[&[0x33; 320], &[0x44; 320]],
    );
    let output_path = write_temp_file("mux-transport-stream-mp3-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ts_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc(".mp3"),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc(".mp3"));
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
}

#[test]
fn mux_to_path_selects_one_audio_track_from_avi_inputs() {
    let first_chunk = [0_u8, 0, 0, 0, 1, 0, 1, 0];
    let second_chunk = [2_u8, 0, 2, 0, 3, 0, 3, 0];
    let avi_input = write_test_avi_pcm_file(
        "mux-avi-select-input",
        &[
            TestAviPcmStream {
                sample_rate: 48_000,
                channel_count: 2,
                bits_per_sample: 16,
                chunks: &[&first_chunk],
            },
            TestAviPcmStream {
                sample_rate: 48_000,
                channel_count: 2,
                bits_per_sample: 16,
                chunks: &[&second_chunk],
            },
        ],
    );
    let output_path = write_temp_file("mux-avi-select-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::selected(
        &avi_input,
        MuxMp4TrackSelector::Audio { occurrence: 2 },
    )]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), second_chunk);
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
fn mux_into_path_preserves_an_existing_mp4_destination() {
    let destination =
        build_video_input_file("mux-destination-video-input", fourcc("isom"), &[b"video"]);
    let audio_input = write_test_adts_file("mux-destination-audio-input", &[b"aud"]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(audio_input)]);

    mux_into_path(&request, &destination).unwrap();

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
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );
    assert_eq!(hdlr_boxes.len(), 2);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn mux_into_path_async_preserves_an_existing_mp4_destination() {
    let destination = build_video_input_file(
        "mux-destination-async-video-input",
        fourcc("isom"),
        &[b"video"],
    );
    let audio_input = write_test_adts_file("mux-destination-async-audio-input", &[b"aud"]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(audio_input)]);

    mp4forge::mux::mux_into_path_async(&request, &destination)
        .await
        .unwrap();

    let output_bytes = fs::read(&destination).unwrap();
    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );
    assert_eq!(hdlr_boxes.len(), 2);
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
fn mux_to_path_flat_mode_preserves_imported_edit_media_time() {
    let samples = std::iter::repeat_n(
        TestMuxSample {
            bytes: b"aaaa",
            duration: 1_024,
            composition_time_offset: 0,
            is_sync_sample: true,
        },
        3,
    )
    .collect::<Vec<_>>();
    let input = build_imported_track_input_file_with_edit_media_time(
        "mux-flat-edit-media-time",
        &MuxFileConfig::new(44_100)
            .with_major_brand(fourcc("isom"))
            .with_compatible_brand(fourcc("mp42")),
        &MuxTrackConfig::new_audio(1, 44_100, audio_sample_entry_box()),
        2_048,
        1_024,
        &samples,
    );
    let output_path = write_temp_file("mux-flat-edit-media-time-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        input,
        MuxMp4TrackSelector::Audio { occurrence: 1 },
    )]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let mvhd_boxes = extract_boxes::<Mvhd>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("mvhd")]),
    );
    let tkhd_boxes = extract_boxes::<Tkhd>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("tkhd")]),
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
    let elst_boxes = extract_boxes::<Elst>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("edts"),
            fourcc("elst"),
        ]),
    );
    assert_eq!(mvhd_boxes.len(), 1);
    assert_eq!(mvhd_boxes[0].duration_v0, 2_048);
    assert_eq!(tkhd_boxes.len(), 1);
    assert_eq!(tkhd_boxes[0].duration_v0, 2_048);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].duration_v0, 3_072);
    assert_eq!(elst_boxes.len(), 1);
    assert_eq!(elst_boxes[0].entries.len(), 1);
    assert_eq!(elst_boxes[0].entries[0].segment_duration_v0, 2_048);
    assert_eq!(elst_boxes[0].entries[0].media_time_v0, 1_024);
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
    let mehd_boxes = extract_boxes::<Mehd>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("mvex"), fourcc("mehd")]),
    );
    let trun_boxes = extract_boxes::<Trun>(
        &output_bytes,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("trun")]),
    );
    let tfdt_boxes = extract_boxes::<Tfdt>(
        &output_bytes,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
    );
    assert_eq!(mehd_boxes.len(), 1);
    assert_eq!(mehd_boxes[0].fragment_duration_v0, 122_880);
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
fn mux_to_path_fragmented_video_mehd_uses_presentation_duration_for_imported_edits() {
    let samples = std::iter::repeat_n(
        TestMuxSample {
            bytes: b"v001",
            duration: 1_000,
            composition_time_offset: 0,
            is_sync_sample: true,
        },
        3,
    )
    .collect::<Vec<_>>();
    let input = build_imported_track_input_file_with_edit_media_time(
        "mux-fragment-video-edit-duration",
        &MuxFileConfig::new(1_000)
            .with_major_brand(fourcc("isom"))
            .with_compatible_brand(fourcc("mp42")),
        &MuxTrackConfig::new_video(1, 1_000, 640, 360, video_sample_entry_box()),
        2_500,
        500,
        &samples,
    );
    let output_path = write_temp_file("mux-fragment-video-edit-duration-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(input, MuxMp4TrackSelector::Video)])
        .with_output_layout(MuxOutputLayout::Fragmented)
        .with_duration_mode(MuxDurationMode::Fragment { seconds: 10.0 });

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let mehd_boxes = extract_boxes::<Mehd>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("mvex"), fourcc("mehd")]),
    );
    assert_eq!(mehd_boxes.len(), 1);
    assert_eq!(mehd_boxes[0].fragment_duration_v0, 2_500);
}

#[test]
fn mux_to_path_fragmented_direct_inputs_use_generic_handler_names() {
    let vp8_input = write_test_vp8_ivf_file(
        "mux-fragmented-direct-vp8-input",
        640,
        360,
        &[0, 1],
        &[
            &build_test_vp8_keyframe(640, 360, 1, b"vp8-a"),
            &build_test_vp8_keyframe(640, 360, 1, b"vp8-b"),
        ],
    );
    let ac3_input = write_test_ac3_file("mux-fragmented-direct-ac3-input", &[b"ac3"]);

    for (label, input, duration_mode, expected_handler_name) in [
        (
            "vp8",
            vp8_input.as_path(),
            MuxDurationMode::Fragment { seconds: 1.0 },
            "VideoHandler",
        ),
        (
            "ac3",
            ac3_input.as_path(),
            MuxDurationMode::Segment { seconds: 1.0 },
            "SoundHandler",
        ),
    ] {
        let output_path = write_temp_file(&format!("mux-fragmented-direct-{label}-output"), &[]);
        let request = MuxRequest::new(vec![MuxTrackSpec::path(input)])
            .with_output_layout(MuxOutputLayout::Fragmented)
            .with_duration_mode(duration_mode);

        mux_to_path(&request, &output_path).unwrap();

        let output_bytes = fs::read(output_path).unwrap();
        let hdlr_boxes = extract_boxes::<Hdlr>(
            &output_bytes,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("hdlr"),
            ]),
        );
        assert_eq!(hdlr_boxes.len(), 1, "{label}");
        assert_eq!(hdlr_boxes[0].name, expected_handler_name, "{label}");
    }
}

#[test]
fn mux_to_path_fragmented_imported_vp8_empty_stss_stays_sync() {
    let vp8_input = write_test_vp8_ivf_file(
        "mux-fragmented-imported-vp8-input",
        640,
        360,
        &[0],
        &[&build_test_vp8_keyframe(640, 360, 1, b"vp8-keyframe")],
    );
    let flat_source = write_temp_file("mux-fragmented-imported-vp8-source", &[]);
    mux_to_path(
        &MuxRequest::new(vec![MuxTrackSpec::path(&vp8_input)]),
        &flat_source,
    )
    .unwrap();

    let output_path = write_temp_file("mux-fragmented-imported-vp8-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        flat_source,
        MuxMp4TrackSelector::Video,
    )])
    .with_output_layout(MuxOutputLayout::Fragmented)
    .with_duration_mode(MuxDurationMode::Fragment { seconds: 10.0 });

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let sidx_boxes = extract_boxes::<Sidx>(&output_bytes, BoxPath::from([fourcc("sidx")]));
    let tfhd_boxes = extract_boxes::<Tfhd>(
        &output_bytes,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfhd")]),
    );
    assert_eq!(sidx_boxes.len(), 1);
    assert_eq!(sidx_boxes[0].references.len(), 1);
    assert!(sidx_boxes[0].references[0].starts_with_sap);
    assert_eq!(sidx_boxes[0].references[0].sap_type, 1);
    assert_eq!(tfhd_boxes.len(), 1);
    assert_eq!(tfhd_boxes[0].default_sample_flags, 0);
}

#[test]
fn mux_to_path_fragmented_imported_opus_uses_track_timescale() {
    let opus_input =
        write_test_ogg_opus_file("mux-fragmented-imported-opus-input", &[b"abc", b"def"]);
    let flat_source = write_temp_file("mux-fragmented-imported-opus-source", &[]);
    mux_to_path(
        &MuxRequest::new(vec![MuxTrackSpec::path(&opus_input)]),
        &flat_source,
    )
    .unwrap();

    let output_path = write_temp_file("mux-fragmented-imported-opus-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        flat_source,
        MuxMp4TrackSelector::Audio { occurrence: 1 },
    )])
    .with_output_layout(MuxOutputLayout::Fragmented)
    .with_duration_mode(MuxDurationMode::Fragment { seconds: 10.0 });

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let mvhd_boxes = extract_boxes::<Mvhd>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("mvhd")]),
    );
    let mehd_boxes = extract_boxes::<Mehd>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("mvex"), fourcc("mehd")]),
    );
    let sidx_boxes = extract_boxes::<Sidx>(&output_bytes, BoxPath::from([fourcc("sidx")]));
    assert_eq!(mvhd_boxes.len(), 1);
    assert_eq!(mvhd_boxes[0].timescale, 48_000);
    assert_eq!(mehd_boxes.len(), 1);
    assert_eq!(mehd_boxes[0].fragment_duration_v0, 960);
    assert_eq!(sidx_boxes.len(), 1);
    assert_eq!(sidx_boxes[0].timescale, 48_000);
    assert_eq!(sidx_boxes[0].references.len(), 1);
    assert_eq!(sidx_boxes[0].references[0].subsegment_duration, 648);
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
        "dtsy", "fLaC", "Opus", "iamf", "mha1", "mhm1",
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
    let request = MuxRequest::new(vec![MuxTrackSpec::path(aac_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"abcdefg");

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
    assert_eq!(hdlr_boxes[0].name, "SoundHandler");
}

#[test]
fn mux_to_path_flat_auto_profile_interleaves_long_raw_aac_inputs() {
    let payloads = (0..45).map(|_| b"abcdef".as_slice()).collect::<Vec<_>>();
    let aac_input = write_test_adts_file("mux-raw-aac-interleaved-input", &payloads);
    let output_path = write_temp_file("mux-raw-aac-interleaved-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(aac_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let esds_boxes = extract_boxes::<Esds>(
        &output_bytes,
        BoxPath::from([
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
    let stco_boxes = extract_boxes::<Stco>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stco"),
        ]),
    );

    assert_eq!(esds_boxes.len(), 1);
    let decoder_config = esds_boxes[0].decoder_config_descriptor().unwrap();
    assert_eq!(decoder_config.buffer_size_db, 6);
    assert_eq!(decoder_config.max_bitrate, 2_160);
    assert_eq!(decoder_config.avg_bitrate, 2_067);
    assert_eq!(stsc_boxes.len(), 1);
    assert_eq!(stco_boxes.len(), 1);
    assert_eq!(stsc_boxes[0].entries.len(), 2);
    assert_eq!(stsc_boxes[0].entries[0].first_chunk, 1);
    assert_eq!(stsc_boxes[0].entries[0].samples_per_chunk, 21);
    assert_eq!(stsc_boxes[0].entries[1].first_chunk, 3);
    assert_eq!(stsc_boxes[0].entries[1].samples_per_chunk, 3);
    assert_eq!(stco_boxes[0].entry_count, 3);
}

#[test]
fn mux_to_path_flat_auto_profile_interleaves_long_raw_mp3_inputs() {
    let payloads = (0..43).map(|_| b"abcdef".as_slice()).collect::<Vec<_>>();
    let mp3_input = write_test_mp3_file("mux-raw-mp3-interleaved-input", &payloads);
    let output_path = write_temp_file("mux-raw-mp3-interleaved-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(mp3_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    let stco_boxes = extract_boxes::<Stco>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stco"),
        ]),
    );

    assert_eq!(stsc_boxes.len(), 1);
    assert_eq!(stco_boxes.len(), 1);
    assert_eq!(stsc_boxes[0].entries.len(), 2);
    assert_eq!(stsc_boxes[0].entries[0].first_chunk, 1);
    assert_eq!(stsc_boxes[0].entries[0].samples_per_chunk, 20);
    assert_eq!(stsc_boxes[0].entries[1].first_chunk, 3);
    assert_eq!(stsc_boxes[0].entries[1].samples_per_chunk, 3);
    assert_eq!(stco_boxes[0].entry_count, 3);
}

#[test]
fn mux_to_path_flat_auto_profile_authors_avc_plus_mp3_import_style_iods_profiles() {
    let h264_input = write_test_h264_annexb_file("mux-flat-h264-mp3-iods-h264-input", &[b"idr"]);
    let mp3_input = write_test_mp3_file("mux-flat-h264-mp3-iods-mp3-input", &[b"abcdef"]);
    let output_path = write_temp_file("mux-flat-h264-mp3-iods-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(&h264_input),
        MuxTrackSpec::path(&mp3_input),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let iods_boxes = extract_boxes::<Iods>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
    );
    assert_eq!(iods_boxes.len(), 1);
    let descriptor = iods_boxes[0].initial_object_descriptor().unwrap();
    assert_eq!(descriptor.audio_profile_level_indication, 0xff);
    assert_eq!(descriptor.visual_profile_level_indication, 0x15);
}

#[test]
fn mux_to_path_flat_auto_profile_keeps_avc_plus_aac_visual_profile_at_7f() {
    let h264_input = write_test_h264_annexb_file("mux-flat-h264-aac-iods-h264-input", &[b"idr"]);
    let aac_input = write_test_adts_file("mux-flat-h264-aac-iods-aac-input", &[b"abcdef"]);
    let output_path = write_temp_file("mux-flat-h264-aac-iods-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(&h264_input),
        MuxTrackSpec::path(&aac_input),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let iods_boxes = extract_boxes::<Iods>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
    );
    assert_eq!(iods_boxes.len(), 1);
    let descriptor = iods_boxes[0].initial_object_descriptor().unwrap();
    assert_eq!(descriptor.audio_profile_level_indication, 0x29);
    assert_eq!(descriptor.visual_profile_level_indication, 0x7f);
}

#[test]
fn mux_to_path_flat_auto_profile_keeps_avc_plus_aac_plus_ac3_visual_profile_at_7f() {
    let h264_input =
        write_test_h264_annexb_file("mux-flat-h264-aac-ac3-iods-h264-input", &[b"idr"]);
    let aac_input = write_test_adts_file("mux-flat-h264-aac-ac3-iods-aac-input", &[b"abcdef"]);
    let ac3_input = write_test_ac3_file("mux-flat-h264-aac-ac3-iods-ac3-input", &[b"ac3"]);
    let output_path = write_temp_file("mux-flat-h264-aac-ac3-iods-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(&h264_input),
        MuxTrackSpec::path(&aac_input),
        MuxTrackSpec::path(&ac3_input),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let iods_boxes = extract_boxes::<Iods>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
    );
    assert_eq!(iods_boxes.len(), 1);
    let descriptor = iods_boxes[0].initial_object_descriptor().unwrap();
    assert_eq!(descriptor.audio_profile_level_indication, 0x29);
    assert_eq!(descriptor.visual_profile_level_indication, 0x7f);
}

#[test]
fn mux_to_path_flat_auto_profile_authors_avc_plus_speex_import_style_iods_profiles() {
    let h264_input = write_test_h264_annexb_file("mux-flat-h264-speex-iods-h264-input", &[b"idr"]);
    let speex_input = write_test_ogg_speex_file("mux-flat-h264-speex-iods-speex-input", &[b"abc"]);
    let output_path = write_temp_file("mux-flat-h264-speex-iods-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(&h264_input),
        MuxTrackSpec::path(&speex_input),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let iods_boxes = extract_boxes::<Iods>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
    );
    assert_eq!(iods_boxes.len(), 1);
    let descriptor = iods_boxes[0].initial_object_descriptor().unwrap();
    assert_eq!(descriptor.audio_profile_level_indication, 0xff);
    assert_eq!(descriptor.visual_profile_level_indication, 0x15);
}

#[test]
fn mux_to_path_flat_auto_profile_authors_direct_mp4v_import_style_iods_profiles() {
    let decoder_specific_info = build_test_mp4v_decoder_specific_info(320, 180);
    let intra_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x00, 0xAA, 0xBB];
    let predictive_frame = [0x00_u8, 0x00, 0x01, 0xB6, 0x40, 0xCC, 0xDD];
    let mut elementary = decoder_specific_info;
    elementary.extend_from_slice(&intra_frame);
    elementary.extend_from_slice(&predictive_frame);
    let mp4v_input = write_test_mp4v_file("mux-flat-mp4v-iods-input", &elementary);
    let output_path = write_temp_file("mux-flat-mp4v-iods-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&mp4v_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let iods_boxes = extract_boxes::<Iods>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
    );
    assert_eq!(iods_boxes.len(), 1);
    let descriptor = iods_boxes[0].initial_object_descriptor().unwrap();
    assert_eq!(descriptor.audio_profile_level_indication, 0xff);
    assert_eq!(descriptor.visual_profile_level_indication, 0x01);
}

#[test]
fn mux_to_path_flat_auto_profile_authors_direct_ogg_theora_import_style_iods_profiles() {
    let theora_input =
        write_test_ogg_theora_file("mux-flat-theora-iods-input", &[b"frame-a", b"frame-b"]);
    let output_path = write_temp_file("mux-flat-theora-iods-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&theora_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let iods_boxes = extract_boxes::<Iods>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
    );
    assert_eq!(iods_boxes.len(), 1);
    let descriptor = iods_boxes[0].initial_object_descriptor().unwrap();
    assert_eq!(descriptor.audio_profile_level_indication, 0xff);
    assert_eq!(descriptor.visual_profile_level_indication, 0xfe);
}

#[test]
fn mux_to_path_flat_auto_profile_authors_avc_plus_amr_import_style_iods_profiles() {
    let h264_input = write_test_h264_annexb_file("mux-flat-h264-amr-iods-h264-input", &[b"idr"]);
    let amr_input = write_test_amr_file("mux-flat-h264-amr-iods-amr-input", &[b"abc", b"def"]);
    let output_path = write_temp_file("mux-flat-h264-amr-iods-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(&h264_input),
        MuxTrackSpec::path(&amr_input),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let iods_boxes = extract_boxes::<Iods>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
    );
    assert_eq!(iods_boxes.len(), 1);
    let descriptor = iods_boxes[0].initial_object_descriptor().unwrap();
    assert_eq!(descriptor.audio_profile_level_indication, 0xfe);
    assert_eq!(descriptor.visual_profile_level_indication, 0x15);
}

#[test]
fn mux_to_path_flat_auto_profile_authors_theora_plus_aac_import_style_iods_profiles() {
    let theora_input = write_test_ogg_theora_file(
        "mux-flat-theora-aac-iods-theora-input",
        &[b"frame-a", b"frame-b"],
    );
    let aac_input = write_test_adts_file("mux-flat-theora-aac-iods-aac-input", &[b"abcdef"]);
    let output_path = write_temp_file("mux-flat-theora-aac-iods-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(&theora_input),
        MuxTrackSpec::path(&aac_input),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let iods_boxes = extract_boxes::<Iods>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
    );
    assert_eq!(iods_boxes.len(), 1);
    let descriptor = iods_boxes[0].initial_object_descriptor().unwrap();
    assert_eq!(descriptor.audio_profile_level_indication, 0x29);
    assert_eq!(descriptor.visual_profile_level_indication, 0xfe);
}

#[test]
fn mux_to_path_flat_auto_profile_authors_avc_plus_qcp_import_style_iods_profiles() {
    let h264_input = write_test_h264_annexb_file("mux-flat-h264-qcp-iods-h264-input", &[b"idr"]);
    let qcp_input = write_test_qcp_constant_file(
        "mux-flat-h264-qcp-iods-qcp-input",
        TestQcpCodecKind::Qcelp,
        &[b"abc", b"def"],
    );
    let output_path = write_temp_file("mux-flat-h264-qcp-iods-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(&h264_input),
        MuxTrackSpec::path(&qcp_input),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let iods_boxes = extract_boxes::<Iods>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
    );
    assert_eq!(iods_boxes.len(), 1);
    let descriptor = iods_boxes[0].initial_object_descriptor().unwrap();
    assert_eq!(descriptor.audio_profile_level_indication, 0xfe);
    assert_eq!(descriptor.visual_profile_level_indication, 0x15);
}

#[test]
fn mux_to_path_flat_auto_profile_authors_avc_plus_mhas_import_style_iods_profiles() {
    let h264_input = write_test_h264_annexb_file("mux-flat-h264-mhas-iods-h264-input", &[b"idr"]);
    let mhas_input = write_test_mhas_file("mux-flat-h264-mhas-iods-mhas-input", &[b"frame-one"]);
    let output_path = write_temp_file("mux-flat-h264-mhas-iods-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(&h264_input),
        MuxTrackSpec::path(&mhas_input),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let iods_boxes = extract_boxes::<Iods>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
    );
    assert_eq!(iods_boxes.len(), 1);
    let descriptor = iods_boxes[0].initial_object_descriptor().unwrap();
    assert_eq!(descriptor.audio_profile_level_indication, 0xfe);
    assert_eq!(descriptor.visual_profile_level_indication, 0x15);
}

#[test]
fn mux_to_path_flat_auto_profile_preserves_terminal_mp3_chunk_run_boundary() {
    let payloads = (0..171).map(|_| b"abcdef".as_slice()).collect::<Vec<_>>();
    let mp3_input = write_test_mp3_44100_file("mux-raw-mp3-terminal-run-input", &payloads);
    let output_path = write_temp_file("mux-raw-mp3-terminal-run-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(mp3_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    let stco_boxes = extract_boxes::<Stco>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stco"),
        ]),
    );

    assert_eq!(stsc_boxes.len(), 1);
    assert_eq!(stco_boxes.len(), 1);
    assert_eq!(stsc_boxes[0].entries.len(), 2);
    assert_eq!(stsc_boxes[0].entries[0].first_chunk, 1);
    assert_eq!(stsc_boxes[0].entries[0].samples_per_chunk, 19);
    assert_eq!(stsc_boxes[0].entries[1].first_chunk, 9);
    assert_eq!(stsc_boxes[0].entries[1].samples_per_chunk, 19);
    assert_eq!(stco_boxes[0].entry_count, 9);
}

#[test]
fn mux_to_path_imports_path_only_latm_inputs() {
    let latm_input = write_test_latm_file("mux-raw-latm-input", &[b"abc", b"defg"]);
    let output_path = write_temp_file("mux-raw-latm-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&latm_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"abcdefg");

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        BoxPath::from([
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
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1_024);
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].name, "SoundHandler");
}

#[test]
fn mux_to_path_imports_path_only_usac_latm_inputs() {
    let first_payload = b"\x80abc";
    let second_payload = b"\x00defg";
    let latm_input = write_test_usac_latm_file(
        "mux-raw-usac-latm-input",
        &[first_payload.as_slice(), second_payload.as_slice()],
    );
    let output_path = write_temp_file("mux-raw-usac-latm-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&latm_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        [first_payload.as_slice(), second_payload.as_slice()].concat()
    );

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    assert_eq!(esds_boxes[0].decoder_specific_info().unwrap().len(), 3);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(
        stts_boxes[0].entries,
        vec![SttsEntry {
            sample_count: 2,
            sample_delta: 1_024,
        }]
    );

    let probed = probe_codec_detailed_bytes(&output_bytes).unwrap();
    assert_eq!(probed.tracks.len(), 1);
    match &probed.tracks[0].codec_details {
        TrackCodecDetails::Mp4Audio(details) => {
            assert_eq!(details.object_type_indication, 0x40);
            assert_eq!(details.audio_object_type, 42);
            assert_eq!(details.channel_count, 2);
            assert_eq!(details.sample_rate, Some(48_000));
        }
        other => panic!("expected mp4 audio codec details, found {other:?}"),
    }
}

#[test]
fn mux_to_path_imports_path_only_truehd_inputs() {
    let truehd_input = write_test_truehd_file("mux-raw-truehd-input", &[b"abcdefgh", b"ijklmnop"]);
    let output_path = write_temp_file("mux-raw-truehd-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&truehd_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let expected_payload = fs::read(&truehd_input).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        root_boxes.iter().map(BoxInfo::box_type).collect::<Vec<_>>(),
        vec![
            fourcc("ftyp"),
            fourcc("moov"),
            fourcc("mdat"),
            fourcc("free")
        ]
    );
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        BoxPath::from([
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
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
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
    assert_eq!(btrt_boxes.len(), 1);
    assert_eq!(btrt_boxes[0].buffer_size_db, 40);
    assert_eq!(btrt_boxes[0].max_bitrate, 384_000);
    assert_eq!(btrt_boxes[0].avg_bitrate, 384_000);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 40);
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].name, "SoundHandler");
}

#[test]
fn mux_to_path_imports_path_only_raw_ac4_inputs() {
    let ac4_input = write_test_ac4_file("mux-raw-ac4-input", 2);
    let output_path = write_temp_file("mux-raw-ac4-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ac4_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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

    let audio_entries = extract_boxes::<AudioSampleEntry>(
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
    let dac4_boxes = extract_boxes::<Dac4>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ac-4"),
            fourcc("dac4"),
        ]),
    );
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ac-4"),
            fourcc("btrt"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ac-4"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(dac4_boxes.len(), 1);
    assert_eq!(dac4_boxes[0].data.len(), 29);
    assert_eq!(btrt_boxes.len(), 1);
    assert_eq!(btrt_boxes[0].buffer_size_db, 348);
    assert_eq!(btrt_boxes[0].max_bitrate, 83_432);
    assert_eq!(btrt_boxes[0].avg_bitrate, 83_432);
    assert!(mdhd_boxes[0].timescale > 0);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert!(stts_boxes[0].entries[0].sample_delta > 0);
}

#[test]
fn mux_to_path_imports_path_only_raw_amr_inputs() {
    let amr_input = write_test_amr_file("mux-raw-amr-input", &[b"one", b"two"]);
    let input_bytes = fs::read(&amr_input).unwrap();
    let output_path = write_temp_file("mux-raw-amr-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&amr_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        &input_bytes[6..]
    );

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("samr"));
    assert_eq!(audio_entries[0].channel_count, 1);
    assert_eq!(damr_boxes.len(), 1);
    assert_eq!(damr_boxes[0].vendor, 0x4750_4143);
    assert_eq!(damr_boxes[0].frames_per_sample, 1);
    assert_ne!(damr_boxes[0].mode_set, 0);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 8_000);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 160);
}

#[test]
fn mux_to_path_imports_path_only_raw_amr_wb_inputs() {
    let amr_input = write_test_amr_wb_file("mux-raw-amr-wb-input", &[b"wide", b"band"]);
    let input_bytes = fs::read(&amr_input).unwrap();
    let output_path = write_temp_file("mux-raw-amr-wb-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&amr_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        &input_bytes[9..]
    );

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("sawb"));
    assert_eq!(audio_entries[0].channel_count, 1);
    assert_eq!(damr_boxes.len(), 1);
    assert_eq!(damr_boxes[0].vendor, 0x4750_4143);
    assert_eq!(damr_boxes[0].frames_per_sample, 1);
    assert_ne!(damr_boxes[0].mode_set, 0);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 16_000);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 320);
}

#[test]
fn mux_to_path_imports_path_only_qcelp_qcp_inputs() {
    let packet_one = b"QCP1";
    let packet_two = b"QCP2";
    let qcp_input = write_test_qcp_constant_file(
        "mux-raw-qcelp-input",
        TestQcpCodecKind::Qcelp,
        &[&packet_one[..], &packet_two[..]],
    );
    let output_path = write_temp_file("mux-raw-qcelp-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&qcp_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        [packet_one.as_slice(), packet_two.as_slice()].concat()
    );
    let ftyp_boxes = extract_boxes::<Ftyp>(&output_bytes, BoxPath::from([fourcc("ftyp")]));

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    assert_eq!(ftyp_boxes.len(), 1);
    assert_eq!(ftyp_boxes[0].major_brand, fourcc("3g2a"));
    assert_eq!(ftyp_boxes[0].minor_version, 65_536);
    assert_eq!(
        ftyp_boxes[0].compatible_brands,
        vec![fourcc("isom"), fourcc("3g2a")]
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("sqcp"));
    assert_eq!(audio_entries[0].channel_count, 1);
    assert_eq!(dqcp_boxes.len(), 1);
    assert_eq!(dqcp_boxes[0].vendor, 0x4750_4143);
    assert_eq!(dqcp_boxes[0].frames_per_sample, 1);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 8_000);
    assert_eq!(
        stts_boxes[0].entries,
        vec![SttsEntry {
            sample_count: 2,
            sample_delta: 160
        }]
    );
}

#[test]
fn mux_to_path_imports_path_only_evrc_qcp_inputs() {
    let packet_one = (3_u8, &b"EVR"[..]);
    let packet_two = (7_u8, &b"C12X"[..]);
    let qcp_input = write_test_qcp_variable_file(
        "mux-raw-evrc-input",
        TestQcpCodecKind::Evrc,
        &[packet_one, packet_two],
    );
    let output_path = write_temp_file("mux-raw-evrc-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&qcp_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        [
            &[packet_one.0][..],
            packet_one.1,
            &[packet_two.0][..],
            packet_two.1
        ]
        .concat()
    );

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("sevc"),
        ]),
    );
    let devc_boxes = extract_boxes::<Devc>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("sevc"),
            fourcc("devc"),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("sevc"));
    assert_eq!(devc_boxes.len(), 1);
    assert_eq!(devc_boxes[0].vendor, 0x4750_4143);
    assert_eq!(devc_boxes[0].frames_per_sample, 1);
    assert_eq!(
        stts_boxes[0].entries,
        vec![SttsEntry {
            sample_count: 2,
            sample_delta: 160
        }]
    );
}

#[test]
fn mux_to_path_imports_path_only_smv_qcp_inputs() {
    let packet_one = b"SMVA";
    let packet_two = b"SMVB";
    let qcp_input = write_test_qcp_constant_file(
        "mux-raw-smv-input",
        TestQcpCodecKind::Smv,
        &[&packet_one[..], &packet_two[..]],
    );
    let output_path = write_temp_file("mux-raw-smv-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&qcp_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        [packet_one.as_slice(), packet_two.as_slice()].concat()
    );

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ssmv"),
        ]),
    );
    let dsmv_boxes = extract_boxes::<Dsmv>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ssmv"),
            fourcc("dsmv"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ssmv"));
    assert_eq!(dsmv_boxes.len(), 1);
    assert_eq!(dsmv_boxes[0].vendor, 0x4750_4143);
    assert_eq!(dsmv_boxes[0].frames_per_sample, 1);
}

#[test]
fn mux_to_path_flat_auto_profile_authors_avc_plus_qcp_import_style_brands() {
    let h264_input = write_test_h264_annexb_file("mux-flat-h264-qcp-brand-h264-input", &[b"idr"]);
    let qcp_input = write_test_qcp_constant_file(
        "mux-flat-h264-qcp-brand-qcp-input",
        TestQcpCodecKind::Qcelp,
        &[&b"QCP1"[..]],
    );
    let output_path = write_temp_file("mux-flat-h264-qcp-brand-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(&h264_input),
        MuxTrackSpec::path(&qcp_input),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let ftyp_boxes = extract_boxes::<Ftyp>(&output_bytes, BoxPath::from([fourcc("ftyp")]));
    assert_eq!(ftyp_boxes.len(), 1);
    assert_eq!(ftyp_boxes[0].major_brand, fourcc("3g2a"));
    assert_eq!(ftyp_boxes[0].minor_version, 65_536);
    assert_eq!(
        ftyp_boxes[0].compatible_brands,
        vec![fourcc("isom"), fourcc("avc1"), fourcc("3g2a")]
    );
}

#[test]
fn mux_to_path_imports_path_only_mhas_inputs() {
    let mhas_input = write_test_mhas_file("mux-raw-mhas-input", &[b"frame-one", b"frame-two"]);
    let expected_payload = fs::read(&mhas_input).unwrap();
    let output_path = write_temp_file("mux-raw-mhas-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&mhas_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mhm1"),
        ]),
    );
    let mhac_boxes = extract_boxes::<MhaC>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mhm1"),
            fourcc("mhaC"),
        ]),
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
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mhm1"),
            fourcc("btrt"),
        ]),
    );
    let stss_boxes = extract_boxes::<Stss>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stss"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("mhm1"));
    assert_eq!(audio_entries[0].channel_count, 0);
    assert!(mhac_boxes.is_empty());
    assert_eq!(btrt_boxes.len(), 1);
    assert!(btrt_boxes[0].buffer_size_db > 0);
    assert!(btrt_boxes[0].max_bitrate > 0);
    assert!(btrt_boxes[0].avg_bitrate > 0);
    assert_eq!(stss_boxes.len(), 1);
    assert_eq!(stss_boxes[0].entry_count, 1);
    assert_eq!(stss_boxes[0].sample_number, vec![1]);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1_024);
}

#[test]
fn mux_to_path_imports_path_only_raw_flac_inputs() {
    let flac_input = write_test_flac_file("mux-raw-flac-input", b"flac-frame");
    let output_path = write_temp_file("mux-raw-flac-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&flac_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let input_bytes = fs::read(&flac_input).unwrap();
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
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        &input_bytes[42..]
    );

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("fLaC"),
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
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("fLaC"),
            fourcc("btrt"),
        ]),
    );
    let dfla_boxes = extract_boxes::<DfLa>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("fLaC"),
            fourcc("dfLa"),
        ]),
    );
    let dfla_box_bytes = extract_box_bytes(
        &mut Cursor::new(&output_bytes),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("fLaC"),
            fourcc("dfLa"),
        ]),
    )
    .unwrap();
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("fLaC"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(dfla_boxes.len(), 1);
    assert_eq!(dfla_box_bytes.len(), 1);
    assert_eq!(dfla_boxes[0].metadata_blocks.len(), 1);
    assert_eq!(dfla_boxes[0].metadata_blocks[0].block_type, 0);
    assert_eq!(dfla_boxes[0].metadata_blocks[0].length, 34);
    assert_eq!(dfla_box_bytes[0][12], 0x00);
    assert_eq!(btrt_boxes.len(), 1);
    assert!(btrt_boxes[0].buffer_size_db > 0);
    assert!(btrt_boxes[0].max_bitrate > 0);
    assert!(btrt_boxes[0].avg_bitrate > 0);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 1);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1_024);
}

#[test]
fn mux_to_path_imports_path_only_multi_frame_raw_flac_inputs() {
    let flac_input = write_test_flac_file_with_frames(
        "mux-raw-flac-multi-input",
        &[b"frame-a", b"frame-b", b"frame-c"],
    );
    let output_path = write_temp_file("mux-raw-flac-multi-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&flac_input)]);

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
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 3);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1_024);
    assert_eq!(stsz_boxes.len(), 1);
    assert_eq!(stsz_boxes[0].sample_count, 3);
    assert_eq!(stsc_boxes.len(), 1);
    assert_eq!(stsc_boxes[0].entry_count, 1);
    assert_eq!(stsc_boxes[0].entries.len(), 1);
    assert_eq!(
        stsc_boxes[0].entries[0],
        StscEntry {
            first_chunk: 1,
            samples_per_chunk: 3,
            sample_description_index: 1,
        }
    );
}

#[test]
fn mux_to_path_flat_auto_profile_preserves_terminal_flac_chunk_run_boundary_in_multi_audio_merge() {
    let h264_input =
        write_test_h264_annexb_file("mux-flat-multi-audio-flac-h264-input", &[b"h264-sample"]);
    let flac_frames = [
        b"frame-00".as_slice(),
        b"frame-01".as_slice(),
        b"frame-02".as_slice(),
        b"frame-03".as_slice(),
        b"frame-04".as_slice(),
        b"frame-05".as_slice(),
        b"frame-06".as_slice(),
        b"frame-07".as_slice(),
        b"frame-08".as_slice(),
        b"frame-09".as_slice(),
    ];
    let flac_input = write_test_flac_file_with_frames_and_block_size(
        "mux-flat-multi-audio-flac-audio-input",
        48_000,
        5_880,
        &flac_frames,
    );
    let opus_input =
        write_test_ogg_opus_file("mux-flat-multi-audio-opus-input", &[b"opus-a", b"opus-b"]);
    let output_path = write_temp_file("mux-flat-multi-audio-flac-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(&h264_input),
        MuxTrackSpec::path(&flac_input),
        MuxTrackSpec::path(&opus_input),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let hdlr_boxes = extract_boxes::<Hdlr>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
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
    assert_eq!(
        hdlr_boxes
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["VideoHandler", "SoundHandler", "SoundHandler"]
    );
    let flac_track_index = 1;
    assert_eq!(stsc_boxes.len(), 3);
    assert_eq!(stsc_boxes[flac_track_index].entry_count, 3);
    assert_eq!(
        stsc_boxes[flac_track_index].entries,
        vec![
            StscEntry {
                first_chunk: 1,
                samples_per_chunk: 4,
                sample_description_index: 1,
            },
            StscEntry {
                first_chunk: 2,
                samples_per_chunk: 3,
                sample_description_index: 1,
            },
            StscEntry {
                first_chunk: 3,
                samples_per_chunk: 3,
                sample_description_index: 1,
            },
        ]
    );
}

#[test]
fn mux_to_path_imports_path_only_ogg_flac_inputs() {
    let flac_input = write_test_ogg_flac_file("mux-raw-ogg-flac-input", &[b"abc", b"def"]);
    let output_path = write_temp_file("mux-raw-ogg-flac-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&flac_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
    let dfla_boxes = extract_boxes::<DfLa>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("fLaC"),
            fourcc("dfLa"),
        ]),
    );
    let dfla_box_bytes = extract_box_bytes(
        &mut Cursor::new(&output_bytes),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("fLaC"),
            fourcc("dfLa"),
        ]),
    )
    .unwrap();
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("fLaC"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(dfla_boxes.len(), 1);
    assert_eq!(dfla_box_bytes.len(), 1);
    assert_eq!(dfla_boxes[0].metadata_blocks.len(), 1);
    assert_eq!(dfla_boxes[0].metadata_blocks[0].block_type, 0);
    assert_eq!(dfla_box_bytes[0][12], 0x00);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1_024);
}

#[test]
fn mux_to_path_imports_path_only_ogg_flac_mapping_header_inputs() {
    let flac_input =
        write_test_ogg_flac_mapping_file("mux-raw-ogg-flac-mapping-input", &[b"abc", b"def"]);
    let output_path = write_temp_file("mux-raw-ogg-flac-mapping-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&flac_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
    let dfla_boxes = extract_boxes::<DfLa>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("fLaC"),
            fourcc("dfLa"),
        ]),
    );
    let dfla_box_bytes = extract_box_bytes(
        &mut Cursor::new(&output_bytes),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("fLaC"),
            fourcc("dfLa"),
        ]),
    )
    .unwrap();
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("fLaC"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(dfla_boxes.len(), 1);
    assert_eq!(dfla_box_bytes.len(), 1);
    assert_eq!(dfla_boxes[0].metadata_blocks.len(), 1);
    assert_eq!(dfla_boxes[0].metadata_blocks[0].block_type, 0);
    assert_eq!(dfla_box_bytes[0][12], 0x00);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1_024);
}

#[test]
fn mux_to_path_imports_path_only_ogg_opus_inputs() {
    let opus_input = write_test_ogg_opus_file("mux-raw-opus-input", &[b"abc", b"def"]);
    let output_path = write_temp_file("mux-raw-opus-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&opus_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"\0abc\0def");

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("Opus"),
            fourcc("btrt"),
        ]),
    );
    let elst_boxes = extract_boxes::<Elst>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("edts"),
            fourcc("elst"),
        ]),
    );
    let sgpd_boxes = extract_boxes::<Sgpd>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("sgpd"),
        ]),
    );
    let sbgp_boxes = extract_boxes::<Sbgp>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("sbgp"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("Opus"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(btrt_boxes.len(), 1);
    assert!(btrt_boxes[0].buffer_size_db > 0);
    assert!(btrt_boxes[0].max_bitrate > 0);
    assert!(btrt_boxes[0].avg_bitrate > 0);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(mdhd_boxes[0].duration_v0, 960);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 480);
    assert_eq!(elst_boxes.len(), 1);
    assert_eq!(elst_boxes[0].entries.len(), 1);
    assert_eq!(elst_boxes[0].entries[0].segment_duration_v0, 8);
    assert_eq!(elst_boxes[0].entries[0].media_time_v0, 312);
    assert_eq!(sgpd_boxes.len(), 1);
    assert_eq!(sgpd_boxes[0].grouping_type, fourcc("roll"));
    assert_eq!(sgpd_boxes[0].default_length, 2);
    assert_eq!(sgpd_boxes[0].entry_count, 1);
    assert_eq!(sgpd_boxes[0].roll_distances, vec![3_840]);
    assert_eq!(sbgp_boxes.len(), 1);
    assert_eq!(sbgp_boxes[0].grouping_type, u32::from_be_bytes(*b"roll"));
    assert_eq!(sbgp_boxes[0].entry_count, 1);
    assert_eq!(sbgp_boxes[0].entries.len(), 1);
    assert_eq!(sbgp_boxes[0].entries[0].sample_count, 2);
    assert_eq!(sbgp_boxes[0].entries[0].group_description_index, 1);
}

#[test]
fn mux_to_path_imports_path_only_wave_pcm_inputs() {
    let pcm_input = write_test_wave_pcm_file(
        "mux-raw-wave-pcm-input",
        &[[-1_000, 1_000], [2_000, -2_000], [3_000, -3_000]],
    );
    let output_path = write_temp_file("mux-raw-wave-pcm-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&pcm_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(&output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    let expected_payload = fs::read(&pcm_input).unwrap()[44..].to_vec();
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ipcm"),
        ]),
    );
    let pcm_configs = extract_boxes::<PcmC>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ipcm"),
            fourcc("pcmC"),
        ]),
    );
    let chnl_boxes = extract_boxes::<Chnl>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ipcm"),
            fourcc("chnl"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ipcm"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(pcm_configs.len(), 1);
    assert_eq!(pcm_configs[0].format_flags, 1);
    assert_eq!(pcm_configs[0].pcm_sample_size, 16);
    assert_eq!(chnl_boxes.len(), 1);
    assert_eq!(
        chnl_boxes[0].data,
        vec![0, 0, 0, 0, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 3);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1);
    assert_eq!(stsz_boxes.len(), 1);
    assert_eq!(stsz_boxes[0].sample_count, 3);
    assert_eq!(stsz_boxes[0].sample_size, 4);
}

#[test]
fn mux_to_path_imports_path_only_aiff_pcm_inputs() {
    let frames = [[-1_000, 1_000], [2_000, -2_000], [3_000, -3_000]];
    let pcm_input = write_test_aiff_pcm_file("mux-raw-aiff-pcm-input", &frames);
    let output_path = write_temp_file("mux-raw-aiff-pcm-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&pcm_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(&output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    let expected_payload = frames
        .into_iter()
        .flat_map(|frame| frame.into_iter().flat_map(i16::to_be_bytes))
        .collect::<Vec<_>>();
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ipcm"),
        ]),
    );
    let pcm_configs = extract_boxes::<PcmC>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ipcm"),
            fourcc("pcmC"),
        ]),
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("ipcm"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(pcm_configs.len(), 1);
    assert_eq!(pcm_configs[0].format_flags, 0);
    assert_eq!(pcm_configs[0].pcm_sample_size, 16);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 3);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1);
    assert_eq!(stsz_boxes.len(), 1);
    assert_eq!(stsz_boxes[0].sample_count, 3);
    assert_eq!(stsz_boxes[0].sample_size, 4);
}

#[test]
fn mux_to_path_imports_path_only_aifc_pcm_inputs() {
    let frames = [[-1_000, 1_000], [2_000, -2_000]];
    let pcm_input = write_test_aifc_pcm_file("mux-raw-aifc-pcm-input", &frames);
    let output_path = write_temp_file("mux-raw-aifc-pcm-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&pcm_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(&output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    let expected_payload = frames
        .into_iter()
        .flat_map(|frame| frame.into_iter().flat_map(i16::to_be_bytes))
        .collect::<Vec<_>>();
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), expected_payload);

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
    let pcm_configs = extract_boxes::<PcmC>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("ipcm"),
            fourcc("pcmC"),
        ]),
    );
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(pcm_configs.len(), 1);
    assert_eq!(pcm_configs[0].format_flags, 0);
    assert_eq!(pcm_configs[0].pcm_sample_size, 16);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1);
    assert_eq!(stsz_boxes.len(), 1);
    assert_eq!(stsz_boxes[0].sample_count, 2);
    assert_eq!(stsz_boxes[0].sample_size, 4);
}

#[test]
fn mux_to_path_imports_path_only_ogg_vorbis_inputs() {
    let vorbis_input = write_test_ogg_vorbis_file("mux-raw-vorbis-input", &[b"abc", b"def"]);
    let output_path = write_temp_file("mux-raw-vorbis-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&vorbis_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(&output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        b"\x02abc\x02def"
    );

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("mp4a"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(esds_boxes.len(), 1);
    assert!(esds_boxes[0].es_descriptor().is_some());
    assert_eq!(
        esds_boxes[0]
            .decoder_config_descriptor()
            .unwrap()
            .object_type_indication,
        0xDD
    );
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 64);
}

#[test]
fn mux_to_path_imports_path_only_ogg_speex_inputs() {
    let speex_input = write_test_ogg_speex_file("mux-raw-speex-input", &[b"abc", b"def"]);
    let output_path = write_temp_file("mux-raw-speex-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&speex_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(&output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"abcdef");

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("spex"),
        ]),
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
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("spex"),
            fourcc("btrt"),
        ]),
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
            fourcc("spex"),
        ]),
    )
    .unwrap();
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("spex"));
    assert_eq!(audio_entries[0].channel_count, 0);
    assert_eq!(sample_entry_boxes.len(), 1);
    assert_eq!(&sample_entry_boxes[0][20..24], b"mp4f");
    assert_eq!(btrt_boxes.len(), 1);
    assert!(btrt_boxes[0].buffer_size_db > 0);
    assert!(btrt_boxes[0].max_bitrate > 0);
    assert!(btrt_boxes[0].avg_bitrate > 0);
    assert_eq!(mdhd_boxes[0].timescale, 16_000);
    assert_eq!(stts_boxes[0].entries.len(), 2);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 1);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1);
    assert_eq!(stts_boxes[0].entries[1].sample_count, 1);
    assert_eq!(stts_boxes[0].entries[1].sample_delta, 320);
}

#[test]
fn mux_to_path_imports_path_only_ogg_theora_inputs() {
    let theora_input =
        write_test_ogg_theora_file("mux-raw-theora-input", &[b"frame-a", b"frame-b"]);
    let output_path = write_temp_file("mux-raw-theora-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&theora_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(&output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        b"\x00frame-a\x00frame-b"
    );

    let visual_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    let pasp_boxes = extract_boxes::<Pasp>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("mp4v"),
            fourcc("pasp"),
        ]),
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
    assert_eq!(visual_entries.len(), 1);
    assert_eq!(visual_entries[0].sample_entry.box_type, fourcc("mp4v"));
    assert_eq!(visual_entries[0].width, 320);
    assert_eq!(visual_entries[0].height, 240);
    assert_eq!(esds_boxes.len(), 1);
    assert!(esds_boxes[0].es_descriptor().is_some());
    assert_eq!(
        esds_boxes[0]
            .decoder_config_descriptor()
            .unwrap()
            .object_type_indication,
        0xDF
    );
    assert_eq!(pasp_boxes.len(), 1);
    assert_eq!(pasp_boxes[0].h_spacing, 4);
    assert_eq!(pasp_boxes[0].v_spacing, 3);
    assert_eq!(mdhd_boxes[0].timescale, 30_000);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1_001);
}

#[test]
fn mux_to_path_imports_path_only_jpeg_inputs() {
    let jpeg_input = write_test_jpeg_file("mux-raw-jpeg-input");
    let output_path = write_temp_file("mux-raw-jpeg-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&jpeg_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let input_bytes = fs::read(&jpeg_input).unwrap();
    let output_bytes = fs::read(&output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), input_bytes);
    let ftyp_boxes = extract_boxes::<Ftyp>(&output_bytes, BoxPath::from([fourcc("ftyp")]));

    let visual_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
    assert_eq!(ftyp_boxes.len(), 1);
    assert_eq!(ftyp_boxes[0].major_brand, fourcc("isom"));
    assert_eq!(ftyp_boxes[0].compatible_brands, vec![fourcc("isom")]);
    assert_eq!(visual_entries.len(), 1);
    assert_eq!(visual_entries[0].sample_entry.box_type, fourcc("jpeg"));
    assert_eq!(visual_entries[0].width, 1);
    assert_eq!(visual_entries[0].height, 1);
    assert_eq!(visual_entries[0].horizresolution, 72);
    assert_eq!(visual_entries[0].vertresolution, 72);
    assert_eq!(mdhd_boxes[0].timescale, 1_000);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 1);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1_000);
}

#[test]
fn mux_to_path_imports_path_only_h263_inputs() {
    let h263_input = write_test_h263_file("mux-raw-h263-input", &[b"frame-a", b"frame-b"]);
    let output_path = write_temp_file("mux-raw-h263-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&h263_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let input_bytes = fs::read(&h263_input).unwrap();
    let output_bytes = fs::read(&output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), input_bytes);

    let visual_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("s263"),
        ]),
    );
    let d263_boxes = extract_boxes::<D263>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("s263"),
            fourcc("d263"),
        ]),
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
    let ftyp_boxes = extract_boxes::<Ftyp>(&output_bytes, BoxPath::from([fourcc("ftyp")]));
    assert_eq!(ftyp_boxes.len(), 1);
    assert_eq!(ftyp_boxes[0].major_brand, fourcc("isom"));
    assert_eq!(
        ftyp_boxes[0].compatible_brands,
        vec![fourcc("isom"), fourcc("3gg6"), fourcc("3gg5")]
    );
    assert_eq!(visual_entries.len(), 1);
    assert_eq!(visual_entries[0].sample_entry.box_type, fourcc("s263"));
    assert_eq!(visual_entries[0].width, 176);
    assert_eq!(visual_entries[0].height, 144);
    assert_eq!(visual_entries[0].compressorname[0], 0);
    assert_eq!(d263_boxes.len(), 1);
    assert_eq!(d263_boxes[0].vendor, 0x4750_4143);
    assert_eq!(d263_boxes[0].decoder_version, 0);
    assert_eq!(d263_boxes[0].h263_level, 10);
    assert_eq!(d263_boxes[0].h263_profile, 0);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 15_000);
    assert_eq!(
        stts_boxes[0].entries,
        vec![SttsEntry {
            sample_count: 2,
            sample_delta: 1_000,
        }]
    );
}

#[test]
fn mux_to_path_imports_path_only_png_inputs() {
    let png_input = write_test_png_file("mux-raw-png-input");
    let output_path = write_temp_file("mux-raw-png-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&png_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let input_bytes = fs::read(&png_input).unwrap();
    let output_bytes = fs::read(&output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), input_bytes);

    let visual_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
    assert_eq!(visual_entries.len(), 1);
    assert_eq!(visual_entries[0].sample_entry.box_type, fourcc("png "));
    assert_eq!(visual_entries[0].width, 1);
    assert_eq!(visual_entries[0].height, 1);
    assert_eq!(visual_entries[0].horizresolution, 72);
    assert_eq!(visual_entries[0].vertresolution, 72);
    assert_eq!(mdhd_boxes[0].timescale, 1_000);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 1);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1_000);
}

#[test]
fn mux_to_path_imports_path_only_iamf_inputs() {
    let iamf_input = write_test_iamf_file("mux-raw-iamf-input", &[b"frame-one", b"frame-two"]);
    let output_path = write_temp_file("mux-raw-iamf-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&iamf_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
        BoxPath::from([
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("iamf"));
    assert_eq!(audio_entries[0].channel_count, 0);
    assert_eq!(audio_entries[0].sample_size, 0);
    assert_eq!(audio_entries[0].sample_rate, 0);
    assert_eq!(iacb_boxes.len(), 1);
    assert_eq!(iacb_boxes[0].configuration_version, 1);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(mdhd_boxes[0].duration(), 4_294_967_296);
    assert_eq!(stts_boxes[0].entries.len(), 2);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 1);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1);
    assert_eq!(stts_boxes[0].entries[1].sample_count, 1);
    assert_eq!(stts_boxes[0].entries[1].sample_delta, u32::MAX);
}

#[test]
fn mux_to_path_imports_path_only_caf_alac_inputs() {
    let alac_input = write_test_caf_alac_file("mux-raw-alac-input", &[b"ABCD", b"EFGH"]);
    let output_path = write_temp_file("mux-raw-alac-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&alac_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    assert_eq!(mdat_payload(&output_bytes, root_boxes[2]), b"ABCDEFGH");

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("alac"),
            fourcc("btrt"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("alac"));
    assert_eq!(audio_entries[0].channel_count, 2);
    assert_eq!(btrt_boxes.len(), 1);
    assert!(btrt_boxes[0].buffer_size_db > 0);
    assert!(btrt_boxes[0].max_bitrate > 0);
    assert!(btrt_boxes[0].avg_bitrate > 0);
    assert_eq!(mdhd_boxes[0].timescale, 48_000);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1_024);
}

#[test]
fn mux_to_path_imports_path_only_variable_packet_caf_alac_inputs() {
    let packet_a = vec![b'A'; 1_977];
    let packet_b = vec![b'B'; 254];
    let alac_input = write_test_caf_alac_variable_packet_file(
        "mux-raw-alac-variable-input",
        &[packet_a.as_slice(), packet_b.as_slice()],
    );
    let output_path = write_temp_file("mux-raw-alac-variable-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&alac_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    let payload = mdat_payload(&output_bytes, root_boxes[2]);
    assert_eq!(payload.len(), packet_a.len() + packet_b.len());
    assert_eq!(&payload[..packet_a.len()], packet_a.as_slice());
    assert_eq!(&payload[packet_a.len()..], packet_b.as_slice());

    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
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
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc("alac"));
    assert_eq!(audio_entries[0].channel_count, 1);
    assert_eq!(mdhd_boxes[0].timescale, 44_100);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 4_096);
}

#[test]
fn mux_to_path_imports_path_only_raw_h265_annexb_inputs() {
    let h265_input = write_test_h265_annexb_file("mux-raw-h265-input", &[b"hevc"]);
    let output_path = write_temp_file("mux-raw-h265-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(h265_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    assert_eq!(hvc1[0].width, 1920);
    assert_eq!(hvc1[0].height, 1080);

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
    assert_eq!(hdlr_boxes[0].name, "VideoHandler");

    let pasp_boxes = extract_boxes::<Pasp>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("hvc1"),
            fourcc("pasp"),
        ]),
    );
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("hvc1"),
            fourcc("btrt"),
        ]),
    );
    assert_eq!(pasp_boxes.len(), 1);
    assert_eq!(pasp_boxes[0].h_spacing, 1);
    assert_eq!(pasp_boxes[0].v_spacing, 1);
    assert_eq!(btrt_boxes.len(), 1);
}

#[test]
fn mux_to_path_imports_multisample_h265_inputs_with_stream_timing() {
    let h265_input = write_test_h265_annexb_file_with_timing(
        "mux-raw-h265-timed-input",
        &[b"\x80hevc", b"\x80tail"],
    );
    let output_path = write_temp_file("mux-raw-h265-timed-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(h265_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
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
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stsc_boxes.len(), 1);
    assert_eq!(stsz_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 24);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 2);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1);
    assert_eq!(stsc_boxes[0].entries.len(), 1);
    assert_eq!(stsc_boxes[0].entries[0].first_chunk, 1);
    assert_eq!(stsc_boxes[0].entries[0].samples_per_chunk, 2);
    assert_eq!(stsz_boxes[0].sample_count, 2);
    assert!(stsz_boxes[0].sample_size > 0);
    assert!(stsz_boxes[0].entry_size.is_empty());

    let pasp_boxes = extract_boxes::<Pasp>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("hvc1"),
            fourcc("pasp"),
        ]),
    );
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("hvc1"),
            fourcc("btrt"),
        ]),
    );
    let tkhd_boxes = extract_boxes::<Tkhd>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("tkhd")]),
    );
    assert_eq!(pasp_boxes.len(), 1);
    assert_eq!(pasp_boxes[0].h_spacing, 855);
    assert_eq!(pasp_boxes[0].v_spacing, 857);
    assert_eq!(btrt_boxes.len(), 1);
    assert!(btrt_boxes[0].buffer_size_db > 0);
    assert!(btrt_boxes[0].max_bitrate > 0);
    assert!(btrt_boxes[0].avg_bitrate > 0);
    assert_eq!(tkhd_boxes.len(), 1);
    assert_eq!(tkhd_boxes[0].width >> 16, 1277);
    assert_eq!(tkhd_boxes[0].height >> 16, 570);
}

#[test]
fn mux_to_path_flat_auto_profile_collapses_mixed_direct_video_tracks_into_one_chunk() {
    let h265_input = write_test_h265_annexb_file_with_timing(
        "mux-flat-mixed-h265-input",
        &[b"\x80hevc", b"\x80tail"],
    );
    let aac_input = write_test_adts_file("mux-flat-mixed-aac-input", &[b"abc", b"defg"]);
    let output_path = write_temp_file("mux-flat-mixed-h265-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(h265_input),
        MuxTrackSpec::path(aac_input),
    ]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(&output_path).unwrap();
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
    let stco_boxes = extract_boxes::<Stco>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stco"),
        ]),
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
    assert_eq!(stsc_boxes.len(), 2);
    assert_eq!(stco_boxes.len(), 2);
    let video_index = hdlr_boxes
        .iter()
        .position(|hdlr| hdlr.handler_type == fourcc("vide"))
        .unwrap();
    assert_eq!(
        stsc_boxes[video_index].entries,
        vec![StscEntry {
            first_chunk: 1,
            samples_per_chunk: 2,
            sample_description_index: 1,
        }]
    );
    assert_eq!(stco_boxes[video_index].entry_count, 1);
}

#[test]
fn mux_to_path_imports_real_h265_bframes_with_edit_list_and_ctts() {
    let h265_input = fixture_path("mux/raw_h265_bframes.h265");
    let output_path = write_temp_file("mux-raw-h265-bframes-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&h265_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let tkhd_boxes = extract_boxes::<Tkhd>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("tkhd")]),
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
    let ctts_boxes = extract_boxes::<Ctts>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("ctts"),
        ]),
    );
    let edts_boxes = extract_boxes::<Edts>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("edts")]),
    );
    let elst_boxes = extract_boxes::<Elst>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("edts"),
            fourcc("elst"),
        ]),
    );
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("hvc1"),
            fourcc("btrt"),
        ]),
    );

    assert_eq!(tkhd_boxes.len(), 1);
    assert_eq!(tkhd_boxes[0].width >> 16, 1277);
    assert_eq!(tkhd_boxes[0].height >> 16, 570);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 24);
    assert_eq!(mdhd_boxes[0].duration(), 8);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entries.len(), 1);
    assert_eq!(stts_boxes[0].entries[0].sample_count, 6);
    assert_eq!(stts_boxes[0].entries[0].sample_delta, 1);
    assert_eq!(ctts_boxes.len(), 1);
    assert_eq!(ctts_boxes[0].entry_count, 5);
    assert_eq!(ctts_boxes[0].entries[0].sample_count, 1);
    assert_eq!(ctts_boxes[0].sample_offset(0), 2);
    assert_eq!(ctts_boxes[0].entries[1].sample_count, 1);
    assert_eq!(ctts_boxes[0].sample_offset(1), 6);
    assert_eq!(ctts_boxes[0].entries[2].sample_count, 1);
    assert_eq!(ctts_boxes[0].sample_offset(2), 3);
    assert_eq!(ctts_boxes[0].entries[3].sample_count, 2);
    assert_eq!(ctts_boxes[0].sample_offset(3), 0);
    assert_eq!(ctts_boxes[0].entries[4].sample_count, 1);
    assert_eq!(ctts_boxes[0].sample_offset(4), 1);
    assert_eq!(edts_boxes.len(), 1);
    assert_eq!(elst_boxes.len(), 1);
    assert_eq!(elst_boxes[0].entry_count, 1);
    assert_eq!(elst_boxes[0].segment_duration(0), 150);
    assert_eq!(elst_boxes[0].media_time(0), 2);
    assert_eq!(btrt_boxes.len(), 1);
    assert_eq!(btrt_boxes[0].buffer_size_db, 10_985);
    assert_eq!(btrt_boxes[0].max_bitrate, 271_536);
    assert_eq!(btrt_boxes[0].avg_bitrate, 271_536);
}

#[test]
fn mux_to_path_imports_real_single_sample_vvc_annex_b_input() {
    let vvc_input = fixture_path("mux/raw_vvc_idr.vvc");
    let output_path = write_temp_file("mux-raw-vvc-idr-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&vvc_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let tkhd_boxes = extract_boxes::<Tkhd>(
        &output_bytes,
        BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("tkhd")]),
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
    let vvc_boxes = extract_boxes::<VVCDecoderConfiguration>(
        &output_bytes,
        BoxPath::from([
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
    let video_entries = extract_boxes::<VisualSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("vvc1"),
        ]),
    );
    let ctts_boxes = extract_boxes::<Ctts>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("ctts"),
        ]),
    );
    let elst_boxes = extract_boxes::<Elst>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("edts"),
            fourcc("elst"),
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

    assert_eq!(tkhd_boxes.len(), 1);
    assert_eq!(tkhd_boxes[0].width >> 16, 1280);
    assert_eq!(tkhd_boxes[0].height >> 16, 720);
    assert_eq!(video_entries.len(), 1);
    assert_eq!(video_entries[0].width, 1280);
    assert_eq!(video_entries[0].height, 720);
    assert_eq!(mdhd_boxes.len(), 1);
    assert_eq!(mdhd_boxes[0].timescale, 25);
    assert_eq!(mdhd_boxes[0].duration(), 2);
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(
        stts_boxes[0].entries,
        vec![SttsEntry {
            sample_count: 1,
            sample_delta: 1
        }]
    );
    assert_eq!(ctts_boxes.len(), 1);
    assert_eq!(ctts_boxes[0].entry_count, 1);
    assert_eq!(ctts_boxes[0].sample_offset(0), 1);
    assert_eq!(elst_boxes.len(), 1);
    assert_eq!(elst_boxes[0].entry_count, 1);
    assert_eq!(elst_boxes[0].segment_duration(0), 24);
    assert_eq!(elst_boxes[0].media_time(0), 1);
    assert_eq!(vvc_boxes.len(), 1);
    assert_eq!(vvc_boxes[0].version, 0);
    assert!(!vvc_boxes[0].decoder_configuration_record.is_empty());
    assert_eq!(
        &vvc_boxes[0].decoder_configuration_record[..4],
        &[0xFF, 0x00, 0x65, 0x5F]
    );
}

#[test]
fn mux_to_path_imports_path_first_ivf_video_inputs() {
    for (sample_entry_type, prefix, frame_payloads, writer) in [
        (
            "av01",
            "mux-raw-av1",
            vec![
                build_test_av1_sequence_header_obu(640, 360),
                build_test_av1_sequence_header_obu(640, 360),
            ],
            write_test_av1_ivf_file as fn(&str, u16, u16, &[u64], &[&[u8]]) -> std::path::PathBuf,
        ),
        (
            "vp08",
            "mux-raw-vp8",
            vec![
                build_test_vp8_keyframe(640, 360, 1, b"vp8-a"),
                build_test_vp8_keyframe(640, 360, 1, b"vp8-b"),
            ],
            write_test_vp8_ivf_file as fn(&str, u16, u16, &[u64], &[&[u8]]) -> std::path::PathBuf,
        ),
        (
            "vp09",
            "mux-raw-vp9",
            vec![
                build_test_vp9_keyframe(640, 360, 0),
                build_test_vp9_keyframe(640, 360, 0),
            ],
            write_test_vp9_ivf_file as fn(&str, u16, u16, &[u64], &[&[u8]]) -> std::path::PathBuf,
        ),
        (
            "vp10",
            "mux-raw-vp10",
            vec![
                build_test_vp10_keyframe(640, 360, 0),
                build_test_vp10_keyframe(640, 360, 0),
            ],
            write_test_vp10_ivf_file as fn(&str, u16, u16, &[u64], &[&[u8]]) -> std::path::PathBuf,
        ),
    ] {
        let frame_refs = frame_payloads.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let input = writer(prefix, 640, 360, &[0, 1], &frame_refs);
        let output_path = write_temp_file(&format!("{prefix}-output"), &[]);
        let request = MuxRequest::new(vec![MuxTrackSpec::path(&input)]);

        mux_to_path(&request, &output_path).unwrap();

        let output_bytes = fs::read(output_path).unwrap();
        let root_boxes = read_root_boxes(&output_bytes);
        assert_eq!(
            mdat_payload(&output_bytes, root_boxes[2]),
            frame_payloads.concat(),
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
        if matches!(sample_entry_type, "vp08" | "vp09" | "vp10") {
            let visible_len = usize::from(entries[0].compressorname[0]).min(31);
            assert_eq!(
                &entries[0].compressorname[1..1 + visible_len],
                b"VPC Coding",
                "{sample_entry_type}"
            );
        }

        let sample_sizes = extract_boxes::<Stsz>(
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
        assert_eq!(sample_sizes.len(), 1, "{sample_entry_type}");
        assert_eq!(sample_sizes[0].sample_count, 2, "{sample_entry_type}");
        if frame_payloads[0].len() == frame_payloads[1].len() {
            assert_eq!(
                sample_sizes[0].sample_size,
                u32::try_from(frame_payloads[0].len()).unwrap(),
                "{sample_entry_type}"
            );
            assert!(sample_sizes[0].entry_size.is_empty(), "{sample_entry_type}");
        } else {
            assert_eq!(sample_sizes[0].sample_size, 0, "{sample_entry_type}");
            assert_eq!(
                sample_sizes[0].entry_size,
                frame_payloads
                    .iter()
                    .map(|payload| u64::try_from(payload.len()).unwrap())
                    .collect::<Vec<_>>(),
                "{sample_entry_type}"
            );
        }

        let sample_times = extract_boxes::<Stts>(
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
        assert_eq!(sample_times.len(), 1, "{sample_entry_type}");
        assert_eq!(
            sample_times[0].entries,
            vec![SttsEntry {
                sample_count: 2,
                sample_delta: 1,
            }],
            "{sample_entry_type}"
        );

        match sample_entry_type {
            "av01" => {
                let av1c = extract_boxes::<AV1CodecConfiguration>(
                    &output_bytes,
                    BoxPath::from([
                        fourcc("moov"),
                        fourcc("trak"),
                        fourcc("mdia"),
                        fourcc("minf"),
                        fourcc("stbl"),
                        fourcc("stsd"),
                        fourcc("av01"),
                        fourcc("av1C"),
                    ]),
                );
                assert_eq!(av1c.len(), 1);
                assert!(!av1c[0].config_obus.is_empty());

                let colr = extract_boxes::<Colr>(
                    &output_bytes,
                    BoxPath::from([
                        fourcc("moov"),
                        fourcc("trak"),
                        fourcc("mdia"),
                        fourcc("minf"),
                        fourcc("stbl"),
                        fourcc("stsd"),
                        fourcc("av01"),
                        fourcc("colr"),
                    ]),
                );
                assert_eq!(colr.len(), 1);
                assert_eq!(colr[0].colour_type, fourcc("nclx"));
                assert_eq!(colr[0].colour_primaries, 2);
                assert_eq!(colr[0].transfer_characteristics, 2);
                assert_eq!(colr[0].matrix_coefficients, 2);
            }
            "vp08" | "vp09" | "vp10" => {
                let vpcc = extract_boxes::<VpCodecConfiguration>(
                    &output_bytes,
                    BoxPath::from([
                        fourcc("moov"),
                        fourcc("trak"),
                        fourcc("mdia"),
                        fourcc("minf"),
                        fourcc("stbl"),
                        fourcc("stsd"),
                        fourcc(sample_entry_type),
                        fourcc("vpcC"),
                    ]),
                );
                assert_eq!(vpcc.len(), 1);
                assert_eq!(vpcc[0].version(), 1);
                if sample_entry_type == "vp08" {
                    assert_eq!(vpcc[0].profile, 1);
                    assert_eq!(vpcc[0].level, 10);
                } else if sample_entry_type == "vp09" {
                    assert_eq!(vpcc[0].profile, 0);
                    assert_eq!(vpcc[0].level, 0);
                    assert_eq!(vpcc[0].colour_primaries, 5);
                    assert_eq!(vpcc[0].transfer_characteristics, 5);
                    assert_eq!(vpcc[0].matrix_coefficients, 6);
                } else {
                    assert_eq!(vpcc[0].profile, 1);
                    assert_eq!(vpcc[0].level, 10);
                    assert_eq!(vpcc[0].bit_depth, 8);
                    assert_eq!(vpcc[0].colour_primaries, 0);
                    assert_eq!(vpcc[0].transfer_characteristics, 0);
                    assert_eq!(vpcc[0].matrix_coefficients, 0);
                }

                let stss = extract_boxes::<Stss>(
                    &output_bytes,
                    BoxPath::from([
                        fourcc("moov"),
                        fourcc("trak"),
                        fourcc("mdia"),
                        fourcc("minf"),
                        fourcc("stbl"),
                        fourcc("stss"),
                    ]),
                );
                if sample_entry_type == "vp08" {
                    assert_eq!(stss.len(), 1);
                    assert_eq!(stss[0].entry_count, 0);
                    assert!(stss[0].sample_number.is_empty());
                } else {
                    assert!(stss.is_empty());
                }
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn mux_to_path_imports_single_sample_ivf_video_inputs_with_zero_duration() {
    for (sample_entry_type, prefix, frame_payloads, writer) in [
        (
            "av01",
            "mux-raw-single-av1",
            vec![build_test_av1_sequence_header_obu(640, 360)],
            write_test_av1_ivf_file as fn(&str, u16, u16, &[u64], &[&[u8]]) -> std::path::PathBuf,
        ),
        (
            "vp08",
            "mux-raw-single-vp8",
            vec![build_test_vp8_keyframe(640, 360, 1, b"vp8-a")],
            write_test_vp8_ivf_file as fn(&str, u16, u16, &[u64], &[&[u8]]) -> std::path::PathBuf,
        ),
        (
            "vp09",
            "mux-raw-single-vp9",
            vec![build_test_vp9_keyframe(640, 360, 0)],
            write_test_vp9_ivf_file as fn(&str, u16, u16, &[u64], &[&[u8]]) -> std::path::PathBuf,
        ),
        (
            "vp10",
            "mux-raw-single-vp10",
            vec![build_test_vp10_keyframe(640, 360, 0)],
            write_test_vp10_ivf_file as fn(&str, u16, u16, &[u64], &[&[u8]]) -> std::path::PathBuf,
        ),
    ] {
        let frame_refs = frame_payloads.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let input = writer(prefix, 640, 360, &[0], &frame_refs);
        let output_path = write_temp_file(&format!("{prefix}-output"), &[]);
        let request = MuxRequest::new(vec![MuxTrackSpec::path(&input)]);

        mux_to_path(&request, &output_path).unwrap();

        let output_bytes = fs::read(output_path).unwrap();
        let media_headers = extract_boxes::<Mdhd>(
            &output_bytes,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("mdhd"),
            ]),
        );
        let sample_times = extract_boxes::<Stts>(
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
        assert_eq!(media_headers.len(), 1, "{sample_entry_type}");
        assert_eq!(media_headers[0].duration(), 0, "{sample_entry_type}");
        assert_eq!(sample_times.len(), 1, "{sample_entry_type}");
        assert_eq!(
            sample_times[0].entries,
            vec![SttsEntry {
                sample_count: 1,
                sample_delta: 0,
            }],
            "{sample_entry_type}"
        );
    }
}

#[test]
fn mux_to_path_strips_leading_temporal_delimiter_obus_from_direct_av1_samples() {
    let mut frame = vec![0x12, 0x00];
    frame.extend_from_slice(&build_test_av1_sequence_header_obu(320, 240));
    let input = write_test_av1_ivf_file("mux-av1-temporal-delimiter", 320, 240, &[0], &[&frame]);
    let output_path = write_temp_file("mux-av1-temporal-delimiter-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        build_test_av1_sequence_header_obu(320, 240)
    );
}

#[test]
fn mux_to_path_imports_raw_mp3_inputs() {
    let mp3_input = write_test_mp3_file("mux-raw-mp3-input", &[b"abc", b"defg"]);
    let expected = fs::read(&mp3_input).unwrap();
    let output_path = write_temp_file("mux-raw-mp3-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(mp3_input)]);

    mux_to_path(&request, &output_path).unwrap();

    let output_bytes = fs::read(output_path).unwrap();
    let root_boxes = read_root_boxes(&output_bytes);
    assert_eq!(
        mdat_payload(&output_bytes, root_boxes[2]),
        expected.as_slice()
    );
    let audio_entries = extract_boxes::<AudioSampleEntry>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc(".mp3"),
        ]),
    );
    let btrt_boxes = extract_boxes::<Btrt>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc(".mp3"),
            fourcc("btrt"),
        ]),
    );
    assert_eq!(audio_entries.len(), 1);
    assert_eq!(audio_entries[0].sample_entry.box_type, fourcc(".mp3"));
    assert_eq!(btrt_boxes.len(), 1);
    assert_eq!(btrt_boxes[0].buffer_size_db, 384);
    assert_eq!(btrt_boxes[0].max_bitrate, 128_000);
    assert_eq!(btrt_boxes[0].avg_bitrate, 128_000);
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
    let request = MuxRequest::new(vec![MuxTrackSpec::path(mp3_input)]);

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
    let request = MuxRequest::new(vec![MuxTrackSpec::path(mp3_input)]);

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
    let request = MuxRequest::new(vec![MuxTrackSpec::path(ac3_input)]);

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
    let request = MuxRequest::new(vec![MuxTrackSpec::path(ac3_input)]);

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
    let request = MuxRequest::new(vec![MuxTrackSpec::path(eac3_input)]);

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
fn mux_to_path_reimports_hevc_outputs_with_decoder_configuration() {
    let h265_input = write_test_h265_annexb_file("mux-hevc-reimport-source", &[b"hevc"]);
    let intermediate = write_temp_file("mux-hevc-reimport-intermediate", &[]);
    let final_output = write_temp_file("mux-hevc-reimport-output", &[]);
    let first_request = MuxRequest::new(vec![MuxTrackSpec::path(&h265_input)]);
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
    let stco_boxes = extract_boxes::<mp4forge::boxes::iso14496_12::Stco>(
        &output_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stco"),
        ]),
    );
    assert_eq!(stts_boxes.len(), 1);
    assert_eq!(stts_boxes[0].entry_count, 0);
    assert_eq!(stsc_boxes.len(), 1);
    assert_eq!(stsc_boxes[0].entry_count, 0);
    assert_eq!(stsz_boxes.len(), 1);
    assert_eq!(stsz_boxes[0].sample_count, 0);
    assert_eq!(stco_boxes.len(), 1);
    assert_eq!(stco_boxes[0].entry_count, 0);
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
    assert_eq!(tkhds[0].alternate_group, 1);
    assert_eq!(tkhds[0].volume, 0x0100);
    assert_eq!(tkhds[1].track_id, 2);
    assert_eq!(tkhds[1].duration(), 14);
    assert_eq!(tkhds[1].alternate_group, 1);
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
    assert_eq!(stsz_boxes[0].sample_size, 4);
    assert!(stsz_boxes[0].entry_size.is_empty());
    assert_eq!(stsz_boxes[1].sample_count, 2);
    assert_eq!(stsz_boxes[1].entry_size, vec![5, 2]);

    let stco_boxes = extract_boxes::<mp4forge::boxes::iso14496_12::Stco>(
        &bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stco"),
        ]),
    );
    let mdat_data_start = root_boxes[2].offset() + root_boxes[2].header_size();
    assert_eq!(stco_boxes.len(), 2);
    assert_eq!(stco_boxes[0].chunk_offset, vec![mdat_data_start]);
    assert_eq!(
        stco_boxes[1].chunk_offset,
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
async fn mux_to_path_async_matches_sync_path_first_track_output() {
    let audio_input = write_test_adts_file("mux-async-audio-input", &[b"abc", b"defg"]);
    let av1_frame_a = build_test_av1_sequence_header_obu(640, 360);
    let av1_frame_b = build_test_av1_sequence_header_obu(640, 360);
    let video_input = write_test_av1_ivf_file(
        "mux-async-video-input",
        640,
        360,
        &[0, 1],
        &[av1_frame_a.as_slice(), av1_frame_b.as_slice()],
    );
    let sync_output = write_temp_file("mux-async-sync-output", &[]);
    let async_output = write_temp_file("mux-async-async-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(audio_input),
        MuxTrackSpec::path(video_input),
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
async fn mux_to_path_async_matches_sync_program_stream_output() {
    let ps_input =
        write_test_program_stream_mp3_file("mux-async-program-stream-input", &[&[0x55; 96]]);
    let sync_output = write_temp_file("mux-async-program-stream-sync-output", &[]);
    let async_output = write_temp_file("mux-async-program-stream-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(ps_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_program_stream_ac3_output() {
    let ps_input =
        write_test_program_stream_ac3_file("mux-async-program-stream-ac3-input", &[b"ac3"]);
    let sync_output = write_temp_file("mux-async-program-stream-ac3-sync-output", &[]);
    let async_output = write_temp_file("mux-async-program-stream-ac3-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(ps_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_transport_stream_output() {
    let ts_input =
        write_test_transport_stream_mp3_file("mux-async-transport-stream-input", &[&[0x66; 320]]);
    let sync_output = write_temp_file("mux-async-transport-stream-sync-output", &[]);
    let async_output = write_temp_file("mux-async-transport-stream-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(ts_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_transport_stream_vvc_output() {
    let ts_input =
        write_test_transport_stream_vvc_file("mux-async-transport-stream-vvc-input", &[]);
    let sync_output = write_temp_file("mux-async-transport-stream-vvc-sync-output", &[]);
    let async_output = write_temp_file("mux-async-transport-stream-vvc-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(ts_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_program_stream_vvc_output() {
    let ps_input = write_test_program_stream_vvc_file("mux-async-program-stream-vvc-input", &[]);
    let sync_output = write_temp_file("mux-async-program-stream-vvc-sync-output", &[]);
    let async_output = write_temp_file("mux-async-program-stream-vvc-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(ps_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_transport_stream_ac3_output() {
    let ts_input =
        write_test_transport_stream_ac3_file("mux-async-transport-stream-ac3-input", &[b"ac3"]);
    let sync_output = write_temp_file("mux-async-transport-stream-ac3-sync-output", &[]);
    let async_output = write_temp_file("mux-async-transport-stream-ac3-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(ts_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_transport_stream_eac3_output() {
    let ts_input =
        write_test_transport_stream_eac3_file("mux-async-transport-stream-eac3-input", &[b"ec3"]);
    let sync_output = write_temp_file("mux-async-transport-stream-eac3-sync-output", &[]);
    let async_output = write_temp_file("mux-async-transport-stream-eac3-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(ts_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_transport_stream_dvb_subtitle_output() {
    let ts_input = write_test_transport_stream_dvb_subtitle_file(
        "mux-async-transport-stream-dvb-subtitle-input",
        &[b"\x20async-sub"],
    );
    let sync_output = write_temp_file("mux-async-transport-stream-dvb-subtitle-sync-output", &[]);
    let async_output = write_temp_file("mux-async-transport-stream-dvb-subtitle-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(ts_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_vobsub_sub_output() {
    let (_idx_input, sub_input) =
        write_test_vobsub_files("mux-async-vobsub-sub-input", &[1_000], &[b"\xDE\xAD"]);
    let sync_output = write_temp_file("mux-async-vobsub-sync-output", &[]);
    let async_output = write_temp_file("mux-async-vobsub-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&sub_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    let sync_bytes = fs::read(sync_output).unwrap();
    let async_bytes = fs::read(async_output).unwrap();
    assert_eq!(sync_bytes, async_bytes);

    let hdlr_boxes = extract_boxes::<Hdlr>(
        &async_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );
    let stsz_boxes = extract_boxes::<Stsz>(
        &async_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsz"),
        ]),
    );
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].handler_type, fourcc("subp"));
    assert_eq!(stsz_boxes.len(), 1);
    assert_eq!(stsz_boxes[0].sample_count, 2);
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_program_stream_vobsub_output() {
    let ps_input = write_test_program_stream_vobsub_file(
        "mux-async-program-stream-vobsub-input",
        &[1_000],
        &[b"\xDE\xAD"],
    );
    let sync_output = write_temp_file("mux-async-program-stream-vobsub-sync-output", &[]);
    let async_output = write_temp_file("mux-async-program-stream-vobsub-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ps_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    let sync_bytes = fs::read(sync_output).unwrap();
    let async_bytes = fs::read(async_output).unwrap();
    assert_eq!(sync_bytes, async_bytes);

    let hdlr_boxes = extract_boxes::<Hdlr>(
        &async_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("hdlr"),
        ]),
    );
    let stsz_boxes = extract_boxes::<Stsz>(
        &async_bytes,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsz"),
        ]),
    );
    assert_eq!(hdlr_boxes.len(), 1);
    assert_eq!(hdlr_boxes[0].handler_type, fourcc("subp"));
    assert_eq!(stsz_boxes.len(), 1);
    assert_eq!(stsz_boxes[0].sample_count, 1);
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_transformed_raw_track_output() {
    let audio_input = write_test_adts_file("mux-async-adts-input", &[b"abc", b"defg"]);
    let video_input = write_test_h265_annexb_file("mux-async-h265-input", &[b"hevc"]);
    let sync_output = write_temp_file("mux-async-transformed-sync-output", &[]);
    let async_output = write_temp_file("mux-async-transformed-async-output", &[]);
    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(audio_input),
        MuxTrackSpec::path(video_input),
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
    let request = MuxRequest::new(vec![MuxTrackSpec::path(eac3_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_raw_dts_output() {
    let dts_input = write_test_dts_file("mux-async-dts-input", 2);
    let sync_output = write_temp_file("mux-async-dts-sync-output", &[]);
    let async_output = write_temp_file("mux-async-dts-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(dts_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_raw_ac4_output() {
    let ac4_input = write_test_ac4_file("mux-async-ac4-input", 2);
    let sync_output = write_temp_file("mux-async-ac4-sync-output", &[]);
    let async_output = write_temp_file("mux-async-ac4-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(ac4_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_raw_amr_output() {
    let amr_input = write_test_amr_file("mux-async-amr-input", &[b"one", b"two"]);
    let sync_output = write_temp_file("mux-async-amr-sync-output", &[]);
    let async_output = write_temp_file("mux-async-amr-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(amr_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_raw_amr_wb_output() {
    let amr_input = write_test_amr_wb_file("mux-async-amr-wb-input", &[b"wide", b"band"]);
    let sync_output = write_temp_file("mux-async-amr-wb-sync-output", &[]);
    let async_output = write_temp_file("mux-async-amr-wb-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(amr_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_raw_latm_output() {
    let latm_input = write_test_latm_file("mux-async-latm-input", &[b"abc", b"defg"]);
    let sync_output = write_temp_file("mux-async-latm-sync-output", &[]);
    let async_output = write_temp_file("mux-async-latm-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(latm_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_raw_usac_latm_output() {
    let latm_input =
        write_test_usac_latm_file("mux-async-usac-latm-input", &[b"\x80abc", b"\x00defg"]);
    let sync_output = write_temp_file("mux-async-usac-latm-sync-output", &[]);
    let async_output = write_temp_file("mux-async-usac-latm-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(latm_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_raw_truehd_output() {
    let truehd_input =
        write_test_truehd_file("mux-async-truehd-input", &[b"abcdefgh", b"ijklmnop"]);
    let sync_output = write_temp_file("mux-async-truehd-sync-output", &[]);
    let async_output = write_temp_file("mux-async-truehd-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(truehd_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_raw_flac_output() {
    let flac_input = write_test_flac_file("mux-async-flac-input", b"flac-frame");
    let sync_output = write_temp_file("mux-async-flac-sync-output", &[]);
    let async_output = write_temp_file("mux-async-flac-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(flac_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_ogg_flac_output() {
    let flac_input = write_test_ogg_flac_file("mux-async-ogg-flac-input", &[b"abc", b"def"]);
    let sync_output = write_temp_file("mux-async-ogg-flac-sync-output", &[]);
    let async_output = write_temp_file("mux-async-ogg-flac-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(flac_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_ogg_flac_mapping_output() {
    let flac_input =
        write_test_ogg_flac_mapping_file("mux-async-ogg-flac-mapping-input", &[b"abc", b"def"]);
    let sync_output = write_temp_file("mux-async-ogg-flac-mapping-sync-output", &[]);
    let async_output = write_temp_file("mux-async-ogg-flac-mapping-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(flac_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_mhas_output() {
    let mhas_input = write_test_mhas_file("mux-async-mhas-input", &[b"frame-one", b"frame-two"]);
    let sync_output = write_temp_file("mux-async-mhas-sync-output", &[]);
    let async_output = write_temp_file("mux-async-mhas-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(mhas_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_iamf_output() {
    let iamf_input = write_test_iamf_file("mux-async-iamf-input", &[b"frame-one", b"frame-two"]);
    let sync_output = write_temp_file("mux-async-iamf-sync-output", &[]);
    let async_output = write_temp_file("mux-async-iamf-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(iamf_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_ogg_opus_output() {
    let opus_input = write_test_ogg_opus_file("mux-async-opus-input", &[b"abc", b"def"]);
    let sync_output = write_temp_file("mux-async-opus-sync-output", &[]);
    let async_output = write_temp_file("mux-async-opus-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(opus_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_wave_pcm_output() {
    let pcm_input = write_test_wave_pcm_file(
        "mux-async-wave-pcm-input",
        &[[-1_000, 1_000], [2_000, -2_000]],
    );
    let sync_output = write_temp_file("mux-async-wave-pcm-sync-output", &[]);
    let async_output = write_temp_file("mux-async-wave-pcm-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(pcm_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_aiff_pcm_output() {
    let pcm_input = write_test_aiff_pcm_file(
        "mux-async-aiff-pcm-input",
        &[[-1_000, 1_000], [2_000, -2_000]],
    );
    let sync_output = write_temp_file("mux-async-aiff-pcm-sync-output", &[]);
    let async_output = write_temp_file("mux-async-aiff-pcm-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(pcm_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_ogg_vorbis_output() {
    let vorbis_input = write_test_ogg_vorbis_file("mux-async-vorbis-input", &[b"abc", b"def"]);
    let sync_output = write_temp_file("mux-async-vorbis-sync-output", &[]);
    let async_output = write_temp_file("mux-async-vorbis-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(vorbis_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_ogg_speex_output() {
    let speex_input = write_test_ogg_speex_file("mux-async-speex-input", &[b"abc", b"def"]);
    let sync_output = write_temp_file("mux-async-speex-sync-output", &[]);
    let async_output = write_temp_file("mux-async-speex-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(speex_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_ogg_theora_output() {
    let theora_input =
        write_test_ogg_theora_file("mux-async-theora-input", &[b"frame-a", b"frame-b"]);
    let sync_output = write_temp_file("mux-async-theora-sync-output", &[]);
    let async_output = write_temp_file("mux-async-theora-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(theora_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_caf_alac_output() {
    let alac_input = write_test_caf_alac_file("mux-async-alac-input", &[b"ABCD", b"EFGH"]);
    let sync_output = write_temp_file("mux-async-alac-sync-output", &[]);
    let async_output = write_temp_file("mux-async-alac-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(alac_input)]);

    mux_to_path(&request, &sync_output).unwrap();
    mux_to_path_async(&request, &async_output).await.unwrap();

    assert_eq!(
        fs::read(sync_output).unwrap(),
        fs::read(async_output).unwrap()
    );
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mux_to_path_async_matches_sync_variable_packet_caf_alac_output() {
    let packet_a = vec![b'A'; 1_977];
    let packet_b = vec![b'B'; 254];
    let alac_input = write_test_caf_alac_variable_packet_file(
        "mux-async-alac-variable-input",
        &[packet_a.as_slice(), packet_b.as_slice()],
    );
    let sync_output = write_temp_file("mux-async-alac-variable-sync-output", &[]);
    let async_output = write_temp_file("mux-async-alac-variable-async-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(alac_input)]);

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
    stbl_children.extend([stsc_bytes, stsz_bytes, co64_bytes]);

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
