#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;
use std::io::Cursor;

use mp4forge::boxes::iso14496_12::{
    Ftyp, Meta, Moof, TFHD_DEFAULT_SAMPLE_DURATION_PRESENT, TFHD_DEFAULT_SAMPLE_SIZE_PRESENT, Tfdt,
    Tfhd, Traf,
};
use mp4forge::cli::edit;
use mp4forge::codec::MutableBox;
use mp4forge::extract::{extract_box, extract_box_as};
use mp4forge::probe::probe;
use mp4forge::walk::BoxPath;

use support::{encode_raw_box, encode_supported_box, fixture_path, fourcc, write_temp_file};

#[test]
fn edit_command_updates_tfdt_and_can_drop_boxes() {
    let input = build_edit_input_file();
    let input_path = write_temp_file("edit-input", &input);
    let output_path = write_temp_file("edit-output", &[]);
    let args = vec![
        "-base_media_decode_time".to_string(),
        "12345".to_string(),
        "-drop".to_string(),
        "free".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_path.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = edit::run(&args, &mut stderr);

    let output = fs::read(&output_path).unwrap();
    let mut reader = Cursor::new(output.clone());
    let summary = probe(&mut reader).unwrap();

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(summary.segments.len(), 1);
    assert_eq!(summary.segments[0].base_media_decode_time, 12_345);
    assert!(!output.windows(4).any(|window| window == b"free"));
}

#[test]
fn edit_command_validates_argument_shape() {
    let mut stderr = Vec::new();
    assert_eq!(edit::run(&[], &mut stderr), 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        concat!(
            "USAGE: mp4forge edit [OPTIONS] INPUT.mp4 OUTPUT.mp4\n",
            "\n",
            "OPTIONS:\n",
            "  -base_media_decode_time <value>    Replace tfdt base media decode times\n",
            "  -path <box/path>                   Limit supported typed rewrites to parsed slash-delimited box paths\n",
            "  -drop <type,type>                  Drop boxes by fourcc\n"
        )
    );
}

#[test]
fn edit_command_accepts_go_style_long_options() {
    let input = build_edit_input_file();
    let input_path = write_temp_file("edit-long-options-input", &input);
    let output_path = write_temp_file("edit-long-options-output", &[]);
    let args = vec![
        "--base_media_decode_time".to_string(),
        "12345".to_string(),
        "--drop".to_string(),
        "free".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_path.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = edit::run(&args, &mut stderr);

    let output = fs::read(&output_path).unwrap();
    let mut reader = Cursor::new(output.clone());
    let summary = probe(&mut reader).unwrap();

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(summary.segments.len(), 1);
    assert_eq!(summary.segments[0].base_media_decode_time, 12_345);
    assert!(!output.windows(4).any(|window| window == b"free"));
}

#[test]
fn edit_command_matches_shared_fragmented_fixture_behavior() {
    let input_path = fixture_path("sample_fragmented.mp4");
    let output_path = write_temp_file("edit-shared-fragmented", &[]);
    let args = vec![
        "-base_media_decode_time".to_string(),
        "123456".to_string(),
        "-drop".to_string(),
        "mfra".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_path.to_string_lossy().into_owned(),
    ];

    let original = fs::read(&input_path).unwrap();
    let mut original_reader = Cursor::new(original);
    let original_summary = probe(&mut original_reader).unwrap();

    let mut stderr = Vec::new();
    let exit_code = edit::run(&args, &mut stderr);

    let edited = fs::read(&output_path).unwrap();
    let mut edited_reader = Cursor::new(edited.clone());
    let edited_summary = probe(&mut edited_reader).unwrap();
    let mfra = extract_box(
        &mut Cursor::new(edited),
        None,
        BoxPath::from([fourcc("mfra")]),
    )
    .unwrap();

    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(edited_summary.tracks, original_summary.tracks);
    assert_eq!(
        edited_summary.segments.len(),
        original_summary.segments.len()
    );
    assert!(!edited_summary.segments.is_empty());
    assert!(
        original_summary
            .segments
            .iter()
            .any(|segment| segment.base_media_decode_time != 123_456)
    );

    for (edited_segment, original_segment) in edited_summary
        .segments
        .iter()
        .zip(original_summary.segments.iter())
    {
        assert_eq!(edited_segment.track_id, original_segment.track_id);
        assert_eq!(
            edited_segment.default_sample_duration,
            original_segment.default_sample_duration
        );
        assert_eq!(edited_segment.sample_count, original_segment.sample_count);
        assert_eq!(edited_segment.duration, original_segment.duration);
        assert_eq!(
            edited_segment.composition_time_offset,
            original_segment.composition_time_offset
        );
        assert_eq!(edited_segment.size, original_segment.size);
        assert_eq!(edited_segment.base_media_decode_time, 123_456);
    }

    assert!(mfra.is_empty());
}

#[test]
fn edit_command_scopes_tfdt_rewrites_to_matching_paths() {
    let input = build_edit_scoped_input_file();
    let input_path = write_temp_file("edit-scoped-input", &input);
    let output_path = write_temp_file("edit-scoped-output", &[]);
    let args = vec![
        "-path".to_string(),
        "moof/traf/tfdt".to_string(),
        "-base_media_decode_time".to_string(),
        "12345".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_path.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = edit::run(&args, &mut stderr);

    let output = fs::read(&output_path).unwrap();
    let scoped_tfdt = extract_box_as::<_, Tfdt>(
        &mut Cursor::new(output.clone()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
    )
    .unwrap();
    let untouched_tfdt = extract_box_as::<_, Tfdt>(
        &mut Cursor::new(output),
        None,
        BoxPath::from([fourcc("meta"), fourcc("tfdt")]),
    )
    .unwrap();

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(scoped_tfdt.len(), 1);
    assert_eq!(untouched_tfdt.len(), 1);
    assert_eq!(scoped_tfdt[0].base_media_decode_time_v0, 12_345);
    assert_eq!(untouched_tfdt[0].base_media_decode_time_v0, 54_321);
}

#[test]
fn edit_command_rejects_path_scoped_type_mismatches() {
    let input = build_edit_input_file();
    let input_path = write_temp_file("edit-path-type-mismatch-input", &input);
    let output_path = write_temp_file("edit-path-type-mismatch-output", &[]);
    let args = vec![
        "-path".to_string(),
        "moof/traf/tfhd".to_string(),
        "-base_media_decode_time".to_string(),
        "12345".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_path.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = edit::run(&args, &mut stderr);
    let stderr = String::from_utf8(stderr).unwrap();

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 1);
    assert!(
        stderr.contains(
            "Error: path-based -base_media_decode_time rewrites require tfdt boxes: matched moof/traf/tfhd (type=tfhd"
        ),
        "{stderr}"
    );
}

#[test]
fn edit_command_reports_invalid_path_arguments() {
    let mut stderr = Vec::new();
    let exit_code = edit::run(
        &[
            "-path".to_string(),
            "moof//tfdt".to_string(),
            "-base_media_decode_time".to_string(),
            "12345".to_string(),
            "input.mp4".to_string(),
            "output.mp4".to_string(),
        ],
        &mut stderr,
    );

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: invalid box path: box path segment 2 must not be empty\n"
    );
}

#[test]
fn edit_command_rejects_unsupported_path_only_rewrites() {
    let input = build_edit_input_file();
    let input_path = write_temp_file("edit-path-unsupported-input", &input);
    let output_path = write_temp_file("edit-path-unsupported-output", &[]);
    let args = vec![
        "-path".to_string(),
        "moof/traf/tfdt".to_string(),
        "-drop".to_string(),
        "free".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_path.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = edit::run(&args, &mut stderr);

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: edit -path currently supports only -base_media_decode_time rewrites\n"
    );
}

#[test]
fn edit_command_preserves_bytes_when_scoped_path_matches_nothing() {
    let input = build_edit_input_file();
    let input_path = write_temp_file("edit-path-noop-input", &input);
    let output_path = write_temp_file("edit-path-noop-output", &[]);
    let args = vec![
        "-path".to_string(),
        "moov/trak/tfdt".to_string(),
        "-base_media_decode_time".to_string(),
        "12345".to_string(),
        input_path.to_string_lossy().into_owned(),
        output_path.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = edit::run(&args, &mut stderr);
    let output = fs::read(&output_path).unwrap();

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(output, input);
}

fn build_edit_input_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("iso6"),
            minor_version: 1,
            compatible_brands: vec![fourcc("iso6"), fourcc("dash")],
        },
        &[],
    );
    let free = encode_raw_box(fourcc("free"), &[0xde, 0xad, 0xbe, 0xef]);

    let tfhd = {
        let mut tfhd = Tfhd::default();
        tfhd.track_id = 7;
        tfhd.default_sample_duration = 1_000;
        tfhd.default_sample_size = 8;
        tfhd.set_flags(TFHD_DEFAULT_SAMPLE_DURATION_PRESENT | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT);
        encode_supported_box(&tfhd, &[])
    };

    let tfdt = {
        let mut tfdt = Tfdt::default();
        tfdt.base_media_decode_time_v0 = 9_000;
        encode_supported_box(&tfdt, &[])
    };

    let traf = encode_supported_box(&Traf, &[tfhd, tfdt].concat());
    let moof = encode_supported_box(&Moof, &traf);
    let mdat = encode_raw_box(fourcc("mdat"), &[0, 1, 2, 3, 4, 5, 6, 7]);

    [ftyp, free, moof, mdat].concat()
}

fn build_edit_scoped_input_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("iso6"),
            minor_version: 1,
            compatible_brands: vec![fourcc("iso6"), fourcc("dash")],
        },
        &[],
    );

    let tfdt_in_fragment = {
        let mut tfdt = Tfdt::default();
        tfdt.base_media_decode_time_v0 = 9_000;
        encode_supported_box(&tfdt, &[])
    };
    let tfdt_in_meta = {
        let mut tfdt = Tfdt::default();
        tfdt.base_media_decode_time_v0 = 54_321;
        encode_supported_box(&tfdt, &[])
    };

    let traf = encode_supported_box(&Traf, &tfdt_in_fragment);
    let moof = encode_supported_box(&Moof, &traf);
    let meta = encode_supported_box(&Meta::default(), &tfdt_in_meta);
    let mdat = encode_raw_box(fourcc("mdat"), &[0, 1, 2, 3]);

    [ftyp, moof, meta, mdat].concat()
}
