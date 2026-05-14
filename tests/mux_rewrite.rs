#![cfg(feature = "mux")]

mod support;

use mp4forge::bitio::BitWriter;
use mp4forge::boxes::iso14496_12::{AVCDecoderConfiguration, HEVCDecoderConfiguration};
use mp4forge::boxes::iso14496_15::VVCDecoderConfiguration;
use mp4forge::mux::rewrite::{
    AdtsRewriteError, AnnexBRewriteError, Av1AnnexBRewriteError, MhasRewriteError,
    rewrite_aac_sample_to_adts, rewrite_av1_sample_to_annex_b, rewrite_avc_sample_to_annex_b,
    rewrite_hevc_sample_to_annex_b, rewrite_mhas_samples_to_stream, rewrite_vvc_sample_to_annex_b,
};
use std::fs;

use support::{
    build_test_av1_sequence_header_obu, write_test_adts_file, write_test_av1_annex_b_file,
    write_test_mhas_file,
};

#[test]
fn rewrite_avc_sample_to_annex_b_rewrites_multiple_nalus() {
    let avcc = AVCDecoderConfiguration {
        length_size_minus_one: 3,
        ..Default::default()
    };
    let sample = [
        0x00, 0x00, 0x00, 0x02, 0x65, 0x88, 0x00, 0x00, 0x00, 0x01, 0x06,
    ];

    let rewritten = rewrite_avc_sample_to_annex_b(&sample, &avcc).unwrap();

    assert_eq!(
        rewritten,
        vec![
            0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x00, 0x00, 0x00, 0x01, 0x06
        ]
    );
}

#[test]
fn rewrite_hevc_sample_to_annex_b_rewrites_multiple_nalus() {
    let hvcc = HEVCDecoderConfiguration {
        length_size_minus_one: 1,
        ..Default::default()
    };
    let sample = [0x00, 0x03, 0x26, 0x01, 0xAA, 0x00, 0x02, 0x02, 0x01];

    let rewritten = rewrite_hevc_sample_to_annex_b(&sample, &hvcc).unwrap();

    assert_eq!(
        rewritten,
        vec![
            0x00, 0x00, 0x00, 0x01, 0x26, 0x01, 0xAA, 0x00, 0x00, 0x00, 0x01, 0x02, 0x01
        ]
    );
}

#[test]
fn rewrite_vvc_sample_to_annex_b_rewrites_multiple_nalus() {
    let vvcc = VVCDecoderConfiguration {
        decoder_configuration_record: vec![0xFE],
        ..Default::default()
    };
    let sample = [
        0x00, 0x00, 0x00, 0x03, 0x8A, 0x00, 0x55, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01,
    ];

    let rewritten = rewrite_vvc_sample_to_annex_b(&sample, &vvcc).unwrap();

    assert_eq!(
        rewritten,
        vec![
            0x00, 0x00, 0x00, 0x01, 0x8A, 0x00, 0x55, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01
        ]
    );
}

#[test]
fn rewrite_rejects_truncated_nal_payloads() {
    let avcc = AVCDecoderConfiguration {
        length_size_minus_one: 3,
        ..Default::default()
    };
    let sample = [0x00, 0x00, 0x00, 0x04, 0x65, 0x88];

    let error = rewrite_avc_sample_to_annex_b(&sample, &avcc).unwrap_err();

    assert_eq!(
        error,
        AnnexBRewriteError::TruncatedNalUnit {
            codec: "AVC",
            offset: 4,
            declared_size: 4,
            remaining_size: 2,
        }
    );
}

#[test]
fn rewrite_rejects_invalid_length_field_widths() {
    let avcc = AVCDecoderConfiguration {
        length_size_minus_one: 4,
        ..Default::default()
    };

    let error = rewrite_avc_sample_to_annex_b(&[0x00, 0x01, 0x09], &avcc).unwrap_err();

    assert_eq!(
        error,
        AnnexBRewriteError::InvalidLengthFieldWidth {
            codec: "AVC",
            width: 5,
        }
    );
}

#[test]
fn rewrite_av1_sample_to_annex_b_matches_expected_temporal_unit_shape() {
    let mut sample = build_test_av1_sequence_header_obu(640, 480);
    sample.extend_from_slice(&[0x32, 0x02, 0x80, 0xAA]);

    let expected_path =
        write_test_av1_annex_b_file("rewrite-av1-annex-b-expected", &[sample.as_slice()]);
    let expected = fs::read(expected_path).unwrap();

    let rewritten = rewrite_av1_sample_to_annex_b(&sample).unwrap();

    assert_eq!(rewritten, expected);
}

#[test]
fn rewrite_av1_rejects_missing_internal_obu_size_fields() {
    let error = rewrite_av1_sample_to_annex_b(&[0x08, 0xAA]).unwrap_err();

    assert_eq!(
        error,
        Av1AnnexBRewriteError::MissingObuSizeField { offset: 0 }
    );
}

#[test]
fn rewrite_aac_sample_to_adts_matches_fixture_header_shape() {
    let payload = b"abc";
    let expected_path = write_test_adts_file("rewrite-aac-adts-expected", &[payload.as_slice()]);
    let expected = fs::read(expected_path).unwrap();

    let rewritten = rewrite_aac_sample_to_adts(payload, &[0x12, 0x10]).unwrap();

    assert_eq!(rewritten, expected);
}

#[test]
fn rewrite_aac_rejects_unsupported_object_types_for_adts() {
    let error = rewrite_aac_sample_to_adts(b"abc", &[0x2A, 0x10]).unwrap_err();

    assert_eq!(
        error,
        AdtsRewriteError::UnsupportedAudioObjectType {
            audio_object_type: 5,
        }
    );
}

#[test]
fn rewrite_mhas_samples_to_stream_round_trips_valid_packetized_samples() {
    let expected_path = write_test_mhas_file("rewrite-mhas-stream-expected", &[b"abc", b"def"]);
    let expected = fs::read(expected_path).unwrap();

    let first_frame = build_test_mhas_frame_packet(b"abc");
    let second_frame = build_test_mhas_frame_packet(b"def");
    let first_sample = [
        build_test_mhas_packet(6, &[0xA5]),
        build_test_mhas_packet(1, &build_test_mhas_config_payload()),
        first_frame,
    ]
    .concat();
    let second_sample = second_frame;

    let rewritten =
        rewrite_mhas_samples_to_stream(&[first_sample.as_slice(), second_sample.as_slice()])
            .unwrap();

    assert_eq!(rewritten, expected);
}

#[test]
fn rewrite_mhas_rejects_samples_without_the_required_leading_sync_packet() {
    let sample = build_test_mhas_packet(1, &build_test_mhas_config_payload());

    let error = rewrite_mhas_samples_to_stream(&[sample.as_slice()]).unwrap_err();

    assert_eq!(error, MhasRewriteError::MissingLeadingSyncPacket);
}

fn build_test_mhas_frame_packet(payload: &[u8]) -> Vec<u8> {
    let mut frame_payload = Vec::with_capacity(payload.len() + 1);
    frame_payload.push(0x80);
    frame_payload.extend_from_slice(payload);
    build_test_mhas_packet(2, &frame_payload)
}

fn build_test_mhas_config_payload() -> Vec<u8> {
    let mut writer = BitWriter::new(Vec::new());
    write_test_bits_u64(&mut writer, 12, 8);
    write_test_bits_u64(&mut writer, 3, 5);
    write_test_bits_u64(&mut writer, 1, 3);
    writer.write_bit(false).unwrap();
    writer.write_bit(false).unwrap();
    write_test_bits_u64(&mut writer, 1, 2);
    write_test_mhas_escaped_value(&mut writer, 1, 5, 8, 16);
    align_test_bit_writer(&mut writer);
    writer.into_inner().unwrap()
}

fn build_test_mhas_packet(packet_type: u64, payload: &[u8]) -> Vec<u8> {
    let mut writer = BitWriter::new(Vec::new());
    write_test_mhas_escaped_value(&mut writer, packet_type, 3, 8, 8);
    write_test_mhas_escaped_value(&mut writer, 0, 2, 8, 32);
    write_test_mhas_escaped_value(
        &mut writer,
        u64::try_from(payload.len()).unwrap(),
        11,
        24,
        24,
    );
    align_test_bit_writer(&mut writer);
    let mut packet = writer.into_inner().unwrap();
    packet.extend_from_slice(payload);
    packet
}

fn write_test_mhas_escaped_value(
    writer: &mut BitWriter<Vec<u8>>,
    value: u64,
    first_width: usize,
    second_width: usize,
    third_width: usize,
) {
    let first_max = (1_u64 << first_width) - 1;
    if value < first_max {
        write_test_bits_u64(writer, value, first_width);
        return;
    }
    write_test_bits_u64(writer, first_max, first_width);
    let remainder = value - first_max;
    let second_max = (1_u64 << second_width) - 1;
    if remainder < second_max {
        write_test_bits_u64(writer, remainder, second_width);
        return;
    }
    write_test_bits_u64(writer, second_max, second_width);
    write_test_bits_u64(writer, remainder - second_max, third_width);
}

fn write_test_bits_u64(writer: &mut BitWriter<Vec<u8>>, value: u64, width: usize) {
    for shift in (0..width).rev() {
        writer.write_bit(((value >> shift) & 1) != 0).unwrap();
    }
}

fn align_test_bit_writer(writer: &mut BitWriter<Vec<u8>>) {
    while !writer.is_aligned() {
        writer.write_bit(false).unwrap();
    }
}
