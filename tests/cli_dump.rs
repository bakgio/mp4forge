#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;

use mp4forge::boxes::iso14496_12::{Ftyp, Moov, Mvhd};
use mp4forge::cli::dump;

use support::{
    encode_raw_box, encode_supported_box, fixture_path, fourcc, normalize_text, read_golden,
    write_temp_file,
};

#[test]
fn dump_command_renders_supported_and_unsupported_boxes() {
    let path = write_temp_file("dump-cli", &build_dump_input_file());
    let args = vec![path.to_string_lossy().into_owned()];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = dump::run(&args, &mut stdout, &mut stderr);

    let _ = fs::remove_file(&path);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        concat!(
            "[ftyp] Size=20 MajorBrand=\"isom\" MinorVersion=512 CompatibleBrands=[{CompatibleBrand=\"isom\"}]\n",
            "[free] Size=12 Data=[...] (use \"-full free\" to show all)\n",
            "[zzzz] (unsupported box type) Size=11 Data=[...] (use \"-full zzzz\" to show all)\n",
            "[moov] Size=116\n",
            "  [mvhd] Size=108 ... (use \"-full mvhd\" to show all)\n"
        )
    );
}

#[test]
fn dump_command_matches_shared_fixture_goldens() {
    let fixture = fixture_path("sample.mp4");
    let cases: &[(&[&str], &str)] = &[
        (&[], "cli_dump/sample.txt"),
        (
            &["-full", "mvhd,loci"],
            "cli_dump/sample-full-mvhd-loci.txt",
        ),
        (&["-offset"], "cli_dump/sample-offset.txt"),
        (&["-hex"], "cli_dump/sample-hex.txt"),
    ];

    for (options, golden) in cases {
        let mut args = options
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        args.push(fixture.to_string_lossy().into_owned());

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = dump::run(&args, &mut stdout, &mut stderr);

        assert_eq!(exit_code, 0, "fixture dump failed for {golden}");
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
fn dump_command_accepts_go_style_long_options() {
    let fixture = fixture_path("sample.mp4");
    let args = vec![
        "--full".to_string(),
        "mvhd,loci".to_string(),
        "--offset".to_string(),
        "--hex".to_string(),
        fixture.to_string_lossy().into_owned(),
    ];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = dump::run(&args, &mut stdout, &mut stderr);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");

    let output = String::from_utf8(stdout).unwrap();
    assert!(output.contains("[ftyp] Offset=0x0 Size=0x20"));
    assert!(output.contains("[mvhd] Offset=0x1932 Size=0x6c Version=0 Flags=0x000000"));
    assert!(output.contains("Rate=1 Volume=256"));
    assert!(output.contains("[loci] (unsupported box type) Offset=0x2033 Size=0x23 Data=[0x00"));
}

#[test]
fn dump_command_reads_quicktime_wave_audio_children() {
    let args = vec![fixture_path("sample_qt.mp4").to_string_lossy().into_owned()];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = dump::run(&args, &mut stdout, &mut stderr);
    let stdout = String::from_utf8(stdout).unwrap();

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert!(stdout.contains("[wave] Size=79"));
    assert!(stdout.contains("[frma] Size=12 DataFormat=\"mp4a\""));
    assert!(stdout.contains("[mp4a] Size=12 QuickTimeData=[0x0, 0x0, 0x0, 0x0]"));
    assert!(stdout.contains(
        "[0x00000001] Size=29 Version=0 Flags=0x000000 ItemName=\"data\" Data={DataType=UTF8 DataLang=0 Data=\"1.0.0\"}"
    ));
}

fn build_dump_input_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 512,
            compatible_brands: vec![fourcc("isom")],
        },
        &[],
    );
    let free = encode_raw_box(fourcc("free"), &[0xaa, 0xbb, 0xcc, 0xdd]);
    let unknown = encode_raw_box(fourcc("zzzz"), &[1, 2, 3]);

    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 4_096;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    let moov = encode_supported_box(&Moov, &encode_supported_box(&mvhd, &[]));

    [ftyp, free, unknown, moov].concat()
}
