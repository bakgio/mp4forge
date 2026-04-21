#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;

use mp4forge::boxes::iso14496_12::{Ftyp, Moov};
use mp4forge::boxes::iso23001_7::{Pssh, PsshKid};
use mp4forge::cli::pssh;
use mp4forge::codec::MutableBox;

use support::{
    encode_supported_box, fixture_path, fourcc, normalize_text, read_golden, write_temp_file,
};

#[test]
fn psshdump_command_renders_offsets_flags_and_base64() {
    let file = build_pssh_input_file();
    let path = write_temp_file("psshdump-cli", &file);
    let args = vec![path.to_string_lossy().into_owned()];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = pssh::run(&args, &mut stdout, &mut stderr);

    let _ = fs::remove_file(&path);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        concat!(
            "0:\n",
            "  offset: 28\n",
            "  size: 52\n",
            "  version: 1\n",
            "  flags: 0x000000\n",
            "  systemId: 1077efec-c0b2-4d02-ace3-3c1e52e2fb4b\n",
            "  dataSize: 0\n",
            "  base64: \"AAAANHBzc2gBAAAAEHfv7MCyTQKs4zweUuL7SwAAAAEBI0VniavN7wEjRWeJq83vAAAAAA==\"\n",
            "\n"
        )
    );
}

#[test]
fn psshdump_command_matches_shared_encrypted_init_fixtures() {
    for fixture_name in ["sample_init.encv.mp4", "sample_init.enca.mp4"] {
        let args = vec![fixture_path(fixture_name).to_string_lossy().into_owned()];

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = pssh::run(&args, &mut stdout, &mut stderr);

        assert_eq!(exit_code, 0, "fixture psshdump failed for {fixture_name}");
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            "",
            "stderr for {fixture_name}"
        );
        assert_eq!(
            normalize_text(&String::from_utf8(stdout).unwrap()),
            read_golden("cli_psshdump/sample_init.txt"),
            "golden mismatch for {fixture_name}"
        );
    }
}

fn build_pssh_input_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 1,
            compatible_brands: vec![fourcc("isom")],
        },
        &[],
    );

    let mut pssh = Pssh::default();
    pssh.system_id = [
        0x10, 0x77, 0xef, 0xec, 0xc0, 0xb2, 0x4d, 0x02, 0xac, 0xe3, 0x3c, 0x1e, 0x52, 0xe2, 0xfb,
        0x4b,
    ];
    pssh.kid_count = 1;
    pssh.kids = vec![PsshKid {
        kid: [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef,
        ],
    }];
    pssh.data_size = 0;
    pssh.data = Vec::new();
    pssh.set_version(1);

    let moov = encode_supported_box(&Moov, &encode_supported_box(&pssh, &[]));
    [ftyp, moov].concat()
}
