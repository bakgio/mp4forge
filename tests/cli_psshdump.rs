#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;
use std::io::Cursor;

use mp4forge::boxes::iso14496_12::{Ftyp, Moof, Moov};
use mp4forge::boxes::iso23001_7::{Pssh, PsshKid};
use mp4forge::cli::pssh::{self, PsshDumpFormat, PsshEntryReport, PsshReport, PsshReportFilter};
use mp4forge::codec::MutableBox;
use mp4forge::walk::BoxPath;

use support::{
    encode_supported_box, fixture_path, fourcc, normalize_text, read_golden, write_temp_file,
};

const PRIMARY_SYSTEM_ID: [u8; 16] = [
    0x10, 0x77, 0xef, 0xec, 0xc0, 0xb2, 0x4d, 0x02, 0xac, 0xe3, 0x3c, 0x1e, 0x52, 0xe2, 0xfb, 0x4b,
];
const PRIMARY_KID: [u8; 16] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
];
const SECONDARY_SYSTEM_ID: [u8; 16] = [
    0xed, 0xef, 0x8b, 0xa9, 0x79, 0xd6, 0x4a, 0xce, 0xa3, 0xc8, 0x27, 0xdc, 0xd5, 0x1d, 0x21, 0xed,
];
const SECONDARY_KID: [u8; 16] = [
    0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
];

#[test]
fn pssh_report_renders_json_and_yaml_with_stable_field_order() {
    let report = PsshReport {
        entries: vec![PsshEntryReport {
            index: 0,
            path: "moov/pssh".to_string(),
            offset: 28,
            size: 54,
            version: 1,
            flags: 0,
            system_id: "1077efec-c0b2-4d02-ace3-3c1e52e2fb4b".to_string(),
            kid_count: 1,
            kids: vec!["01234567-89ab-cdef-0123-456789abcdef".to_string()],
            data_size: 2,
            data_bytes: vec![170, 187],
            raw_box_base64: "AAA=".to_string(),
        }],
    };

    let mut json = Vec::new();
    pssh::write_pssh_report(&mut json, &report, PsshDumpFormat::Json).unwrap();
    assert_eq!(
        String::from_utf8(json).unwrap(),
        concat!(
            "{\n",
            "  \"Entries\": [\n",
            "    {\n",
            "      \"Index\": 0,\n",
            "      \"Path\": \"moov/pssh\",\n",
            "      \"Offset\": 28,\n",
            "      \"Size\": 54,\n",
            "      \"Version\": 1,\n",
            "      \"Flags\": 0,\n",
            "      \"SystemId\": \"1077efec-c0b2-4d02-ace3-3c1e52e2fb4b\",\n",
            "      \"KidCount\": 1,\n",
            "      \"Kids\": [\n",
            "        \"01234567-89ab-cdef-0123-456789abcdef\"\n",
            "      ],\n",
            "      \"DataSize\": 2,\n",
            "      \"DataBytes\": [\n",
            "        170,\n",
            "        187\n",
            "      ],\n",
            "      \"RawBoxBase64\": \"AAA=\"\n",
            "    }\n",
            "  ]\n",
            "}\n"
        )
    );

    let mut yaml = Vec::new();
    pssh::write_pssh_report(&mut yaml, &report, PsshDumpFormat::Yaml).unwrap();
    assert_eq!(
        String::from_utf8(yaml).unwrap(),
        concat!(
            "entries:\n",
            "- index: 0\n",
            "  path: moov/pssh\n",
            "  offset: 28\n",
            "  size: 54\n",
            "  version: 1\n",
            "  flags: 0\n",
            "  system_id: 1077efec-c0b2-4d02-ace3-3c1e52e2fb4b\n",
            "  kid_count: 1\n",
            "  kids:\n",
            "    - 01234567-89ab-cdef-0123-456789abcdef\n",
            "  data_size: 2\n",
            "  data_bytes:\n",
            "    - 170\n",
            "    - 187\n",
            "  raw_box_base64: 'AAA='\n"
        )
    );
}

#[test]
fn build_pssh_report_extracts_kids_data_and_raw_box_base64() {
    let file = build_pssh_input_file(&[0xaa, 0xbb]);
    let report = pssh::build_pssh_report(&mut Cursor::new(file)).unwrap();

    assert_eq!(
        report,
        PsshReport {
            entries: vec![PsshEntryReport {
                index: 0,
                path: "moov/pssh".to_string(),
                offset: 28,
                size: 54,
                version: 1,
                flags: 0,
                system_id: "1077efec-c0b2-4d02-ace3-3c1e52e2fb4b".to_string(),
                kid_count: 1,
                kids: vec!["01234567-89ab-cdef-0123-456789abcdef".to_string()],
                data_size: 2,
                data_bytes: vec![0xaa, 0xbb],
                raw_box_base64:
                    "AAAANnBzc2gBAAAAEHfv7MCyTQKs4zweUuL7SwAAAAEBI0VniavN7wEjRWeJq83vAAAAAqq7"
                        .to_string(),
            }],
        }
    );
}

#[test]
fn build_pssh_report_with_filters_scopes_by_path_system_id_and_kid() {
    let file = build_filtered_pssh_input_file();

    let moof_only = pssh::build_pssh_report_with_filters(
        &mut Cursor::new(&file),
        &PsshReportFilter {
            paths: vec![BoxPath::parse("moof").unwrap()],
            ..PsshReportFilter::default()
        },
    )
    .unwrap();
    assert_eq!(moof_only.entries.len(), 1);
    assert_eq!(moof_only.entries[0].index, 1);
    assert_eq!(moof_only.entries[0].path, "moof/pssh");
    assert_eq!(
        moof_only.entries[0].system_id,
        "edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"
    );

    let system_filtered = pssh::build_pssh_report_with_filters(
        &mut Cursor::new(&file),
        &PsshReportFilter {
            system_ids: vec![SECONDARY_SYSTEM_ID],
            ..PsshReportFilter::default()
        },
    )
    .unwrap();
    assert_eq!(system_filtered.entries.len(), 1);
    assert_eq!(system_filtered.entries[0].index, 1);
    assert_eq!(system_filtered.entries[0].path, "moof/pssh");

    let kid_filtered = pssh::build_pssh_report_with_filters(
        &mut Cursor::new(&file),
        &PsshReportFilter {
            kids: vec![PRIMARY_KID],
            ..PsshReportFilter::default()
        },
    )
    .unwrap();
    assert_eq!(kid_filtered.entries.len(), 1);
    assert_eq!(kid_filtered.entries[0].index, 0);
    assert_eq!(kid_filtered.entries[0].path, "moov/pssh");
    assert_eq!(
        kid_filtered.entries[0].kids,
        vec!["01234567-89ab-cdef-0123-456789abcdef".to_string()]
    );
}

#[test]
fn psshdump_command_renders_offsets_flags_and_base64() {
    let file = build_pssh_input_file(&[]);
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
fn psshdump_command_filters_text_output_by_path() {
    let file = build_filtered_pssh_input_file();
    let path = write_temp_file("psshdump-cli-filter-path", &file);
    let args = vec![
        "-path".to_string(),
        "moof".to_string(),
        path.to_string_lossy().into_owned(),
    ];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = pssh::run(&args, &mut stdout, &mut stderr);

    let _ = fs::remove_file(&path);

    let stdout = String::from_utf8(stdout).unwrap();
    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert!(stdout.contains("1:\n"));
    assert!(stdout.contains("systemId: edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"));
    assert!(!stdout.contains("1077efec-c0b2-4d02-ace3-3c1e52e2fb4b"));
}

#[test]
fn psshdump_command_filters_structured_output_with_stable_goldens() {
    let file = build_filtered_pssh_input_file();
    let path = write_temp_file("psshdump-cli-filtered", &file);
    let cases: &[(&[&str], &str)] = &[
        (
            &["-path", "moof", "-format", "json"],
            "cli_psshdump/filtered_path.json",
        ),
        (
            &[
                "-system-id",
                "edef8ba9-79d6-4ace-a3c8-27dcd51d21ed",
                "-format",
                "json",
            ],
            "cli_psshdump/filtered_system_id.json",
        ),
        (
            &[
                "-kid",
                "fedcba98-7654-3210-fedc-ba9876543210",
                "-format",
                "yaml",
            ],
            "cli_psshdump/filtered_kid.yaml",
        ),
    ];

    for (options, golden) in cases {
        let mut args = options
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        args.push(path.to_string_lossy().into_owned());

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = pssh::run(&args, &mut stdout, &mut stderr);

        assert_eq!(exit_code, 0, "filtered psshdump failed for {golden}");
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

    let _ = fs::remove_file(&path);
}

#[test]
fn psshdump_command_rejects_invalid_filter_values() {
    let file = build_filtered_pssh_input_file();
    let path = write_temp_file("psshdump-cli-invalid-filter", &file);
    let cases: &[(&[&str], &str)] = &[
        (
            &["-path", "moov//pssh"],
            "Error: box path segment 2 must not be empty\n",
        ),
        (
            &["-system-id", "not-a-uuid"],
            "Error: invalid system ID: expected 32 hexadecimal digits with optional hyphens\n",
        ),
        (
            &["-kid", "xyz"],
            "Error: invalid KID: expected 32 hexadecimal digits with optional hyphens\n",
        ),
    ];

    for (options, expected_stderr) in cases {
        let mut args = options
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        args.push(path.to_string_lossy().into_owned());

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = pssh::run(&args, &mut stdout, &mut stderr);

        assert_eq!(exit_code, 1, "invalid filter unexpectedly succeeded");
        assert_eq!(String::from_utf8(stdout).unwrap(), "");
        assert_eq!(String::from_utf8(stderr).unwrap(), *expected_stderr);
    }

    let _ = fs::remove_file(&path);
}

#[test]
fn psshdump_command_returns_empty_reports_for_empty_matches() {
    let file = build_filtered_pssh_input_file();
    let path = write_temp_file("psshdump-cli-empty-filter", &file);

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let text_exit = pssh::run(
        &[
            "-system-id".to_string(),
            "ffffffff-ffff-ffff-ffff-ffffffffffff".to_string(),
            path.to_string_lossy().into_owned(),
        ],
        &mut stdout,
        &mut stderr,
    );
    assert_eq!(text_exit, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(String::from_utf8(stdout).unwrap(), "");

    let mut json_stdout = Vec::new();
    let mut json_stderr = Vec::new();
    let json_exit = pssh::run(
        &[
            "-system-id".to_string(),
            "ffffffff-ffff-ffff-ffff-ffffffffffff".to_string(),
            "-format".to_string(),
            "json".to_string(),
            path.to_string_lossy().into_owned(),
        ],
        &mut json_stdout,
        &mut json_stderr,
    );
    assert_eq!(json_exit, 0);
    assert_eq!(String::from_utf8(json_stderr).unwrap(), "");
    assert_eq!(
        String::from_utf8(json_stdout).unwrap(),
        "{\n  \"Entries\": [\n  ]\n}\n"
    );

    let _ = fs::remove_file(&path);
}

#[test]
fn psshdump_command_matches_shared_encrypted_init_fixtures() {
    let cases: &[(&[&str], &str)] = &[
        (&[], "cli_psshdump/sample_init.txt"),
        (&["-format", "json"], "cli_psshdump/sample_init.json"),
        (&["-format", "yaml"], "cli_psshdump/sample_init.yaml"),
    ];

    for fixture_name in ["sample_init.encv.mp4", "sample_init.enca.mp4"] {
        for (options, golden) in cases {
            let mut args = options
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>();
            args.push(fixture_path(fixture_name).to_string_lossy().into_owned());

            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let exit_code = pssh::run(&args, &mut stdout, &mut stderr);

            assert_eq!(
                exit_code, 0,
                "fixture psshdump failed for {fixture_name} {golden}"
            );
            assert_eq!(
                String::from_utf8(stderr).unwrap(),
                "",
                "stderr for {fixture_name} {golden}"
            );
            assert_eq!(
                normalize_text(&String::from_utf8(stdout).unwrap()),
                read_golden(golden),
                "golden mismatch for {fixture_name} {golden}"
            );
        }
    }
}

fn build_pssh_input_file(data: &[u8]) -> Vec<u8> {
    let moov = encode_supported_box(
        &Moov,
        &encode_supported_box(&build_pssh_box(PRIMARY_SYSTEM_ID, PRIMARY_KID, data), &[]),
    );
    [build_ftyp_box(), moov].concat()
}

fn build_filtered_pssh_input_file() -> Vec<u8> {
    let moov = encode_supported_box(
        &Moov,
        &encode_supported_box(
            &build_pssh_box(PRIMARY_SYSTEM_ID, PRIMARY_KID, &[0xaa]),
            &[],
        ),
    );
    let moof = encode_supported_box(
        &Moof,
        &encode_supported_box(
            &build_pssh_box(SECONDARY_SYSTEM_ID, SECONDARY_KID, &[0xbb, 0xcc]),
            &[],
        ),
    );
    [build_ftyp_box(), moov, moof].concat()
}

fn build_ftyp_box() -> Vec<u8> {
    encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 1,
            compatible_brands: vec![fourcc("isom")],
        },
        &[],
    )
}

fn build_pssh_box(system_id: [u8; 16], kid: [u8; 16], data: &[u8]) -> Pssh {
    let mut pssh = Pssh::default();
    pssh.system_id = system_id;
    pssh.kid_count = 1;
    pssh.kids = vec![PsshKid { kid }];
    pssh.data_size = u32::try_from(data.len()).unwrap();
    pssh.data = data.to_vec();
    pssh.set_version(1);
    pssh
}
