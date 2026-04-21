#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;
use std::io::Cursor;

use mp4forge::BoxInfo;
use mp4forge::boxes::iso14496_12::{Ftyp, Moov, Mvhd};
use mp4forge::cli::extract;

use support::{encode_supported_box, fixture_path, fourcc, write_temp_file};

#[test]
fn extract_command_writes_matching_raw_boxes() {
    let file = build_extract_input_file();
    let path = write_temp_file("extract-cli", &file);
    let args = vec!["mvhd".to_string(), path.to_string_lossy().into_owned()];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = extract::run(&args, &mut stdout, &mut stderr);

    let _ = fs::remove_file(&path);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(stdout.len(), 108);
    assert_eq!(&stdout[4..8], b"mvhd");
}

#[test]
fn extract_command_rejects_invalid_arguments() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(extract::run(&[], &mut stdout, &mut stderr), 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "USAGE: mp4forge extract BOX_TYPE INPUT.mp4\n"
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        extract::run(
            &["xxxxx".to_string(), "missing.mp4".to_string()],
            &mut stdout,
            &mut stderr,
        ),
        1
    );
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: invalid box type: xxxxx\n"
    );
}

#[test]
fn extract_command_matches_shared_fixture_reference_sizes() {
    let cases = [
        ("sample.mp4", "ftyp", fourcc("ftyp"), 1_usize, 32_usize),
        ("sample.mp4", "mdhd", fourcc("mdhd"), 2, 64),
        ("sample_fragmented.mp4", "trun", fourcc("trun"), 8, 452),
    ];

    for (file_name, box_type, expected_type, expected_count, expected_len) in cases {
        let args = vec![
            box_type.to_string(),
            fixture_path(file_name).to_string_lossy().into_owned(),
        ];

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = extract::run(&args, &mut stdout, &mut stderr);

        assert_eq!(exit_code, 0, "fixture={file_name} type={box_type}");
        assert_eq!(String::from_utf8(stderr).unwrap(), "");
        assert_eq!(
            stdout.len(),
            expected_len,
            "fixture={file_name} type={box_type}"
        );

        let infos = parse_box_stream(&stdout);
        assert_eq!(
            infos.len(),
            expected_count,
            "fixture={file_name} type={box_type}"
        );
        assert!(infos.iter().all(|info| info.box_type() == expected_type));
        assert_eq!(
            infos.iter().map(BoxInfo::size).sum::<u64>() as usize,
            expected_len
        );
    }
}

fn build_extract_input_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 512,
            compatible_brands: vec![fourcc("isom")],
        },
        &[],
    );

    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 2_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    let moov = encode_supported_box(&Moov, &encode_supported_box(&mvhd, &[]));

    [ftyp, moov].concat()
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
