#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;
use std::io::Cursor;

use mp4forge::boxes::iso14496_12::{Ftyp, Moov, Mvhd};
use mp4forge::cli::dump::{
    self, DumpPayloadStatus, FieldStructuredDumpBoxReport, FieldStructuredDumpReport,
    StructuredDumpBoxReport, StructuredDumpFieldReport, StructuredDumpFormat, StructuredDumpReport,
};
use mp4forge::codec::FieldValue;
use mp4forge::walk::BoxPath;

use support::{
    encode_raw_box, encode_supported_box, fixture_path, fourcc, normalize_text, read_golden,
    write_temp_file,
};

#[test]
fn structured_dump_report_renders_json_and_yaml_with_stable_field_order() {
    let report = StructuredDumpReport {
        boxes: vec![
            StructuredDumpBoxReport {
                box_type: "ftyp".to_string(),
                path: "ftyp".to_string(),
                offset: 0,
                size: 20,
                supported: true,
                payload_status: DumpPayloadStatus::Summary,
                payload_summary: Some(
                    "MajorBrand=\"isom\" MinorVersion=512 CompatibleBrands=[{CompatibleBrand=\"isom\"}]"
                        .to_string(),
                ),
                payload_bytes: None,
                children: Vec::new(),
            },
            StructuredDumpBoxReport {
                box_type: "moov".to_string(),
                path: "moov".to_string(),
                offset: 20,
                size: 116,
                supported: true,
                payload_status: DumpPayloadStatus::Empty,
                payload_summary: None,
                payload_bytes: None,
                children: vec![StructuredDumpBoxReport {
                    box_type: "mvhd".to_string(),
                    path: "moov/mvhd".to_string(),
                    offset: 28,
                    size: 108,
                    supported: true,
                    payload_status: DumpPayloadStatus::Omitted,
                    payload_summary: None,
                    payload_bytes: None,
                    children: Vec::new(),
                }],
            },
            StructuredDumpBoxReport {
                box_type: "zzzz".to_string(),
                path: "zzzz".to_string(),
                offset: 136,
                size: 11,
                supported: false,
                payload_status: DumpPayloadStatus::Bytes,
                payload_summary: None,
                payload_bytes: Some(vec![1, 2, 3]),
                children: Vec::new(),
            },
        ],
    };

    let mut json = Vec::new();
    dump::write_structured_report(&mut json, &report, StructuredDumpFormat::Json).unwrap();
    assert_eq!(
        String::from_utf8(json).unwrap(),
        concat!(
            "{\n",
            "  \"Boxes\": [\n",
            "    {\n",
            "      \"BoxType\": \"ftyp\",\n",
            "      \"Path\": \"ftyp\",\n",
            "      \"Offset\": 0,\n",
            "      \"Size\": 20,\n",
            "      \"Supported\": true,\n",
            "      \"PayloadStatus\": \"summary\",\n",
            "      \"PayloadSummary\": \"MajorBrand=\\\"isom\\\" MinorVersion=512 CompatibleBrands=[{CompatibleBrand=\\\"isom\\\"}]\",\n",
            "      \"Children\": [\n",
            "      ]\n",
            "    },\n",
            "    {\n",
            "      \"BoxType\": \"moov\",\n",
            "      \"Path\": \"moov\",\n",
            "      \"Offset\": 20,\n",
            "      \"Size\": 116,\n",
            "      \"Supported\": true,\n",
            "      \"PayloadStatus\": \"empty\",\n",
            "      \"Children\": [\n",
            "        {\n",
            "          \"BoxType\": \"mvhd\",\n",
            "          \"Path\": \"moov/mvhd\",\n",
            "          \"Offset\": 28,\n",
            "          \"Size\": 108,\n",
            "          \"Supported\": true,\n",
            "          \"PayloadStatus\": \"omitted\",\n",
            "          \"Children\": [\n",
            "          ]\n",
            "        }\n",
            "      ]\n",
            "    },\n",
            "    {\n",
            "      \"BoxType\": \"zzzz\",\n",
            "      \"Path\": \"zzzz\",\n",
            "      \"Offset\": 136,\n",
            "      \"Size\": 11,\n",
            "      \"Supported\": false,\n",
            "      \"PayloadStatus\": \"bytes\",\n",
            "      \"PayloadBytes\": [\n",
            "        1,\n",
            "        2,\n",
            "        3\n",
            "      ],\n",
            "      \"Children\": [\n",
            "      ]\n",
            "    }\n",
            "  ]\n",
            "}\n"
        )
    );

    let mut yaml = Vec::new();
    dump::write_structured_report(&mut yaml, &report, StructuredDumpFormat::Yaml).unwrap();
    assert_eq!(
        String::from_utf8(yaml).unwrap(),
        concat!(
            "boxes:\n",
            "- box_type: ftyp\n",
            "  path: ftyp\n",
            "  offset: 0\n",
            "  size: 20\n",
            "  supported: true\n",
            "  payload_status: summary\n",
            "  payload_summary: 'MajorBrand=\"isom\" MinorVersion=512 CompatibleBrands=[{CompatibleBrand=\"isom\"}]'\n",
            "  children: []\n",
            "- box_type: moov\n",
            "  path: moov\n",
            "  offset: 20\n",
            "  size: 116\n",
            "  supported: true\n",
            "  payload_status: empty\n",
            "  children:\n",
            "  - box_type: mvhd\n",
            "    path: moov/mvhd\n",
            "    offset: 28\n",
            "    size: 108\n",
            "    supported: true\n",
            "    payload_status: omitted\n",
            "    children: []\n",
            "- box_type: zzzz\n",
            "  path: zzzz\n",
            "  offset: 136\n",
            "  size: 11\n",
            "  supported: false\n",
            "  payload_status: bytes\n",
            "  payload_bytes:\n",
            "    - 1\n",
            "    - 2\n",
            "    - 3\n",
            "  children: []\n"
        )
    );
}

#[test]
fn field_structured_dump_report_renders_json_and_yaml_with_stable_field_order() {
    let report = FieldStructuredDumpReport {
        boxes: vec![FieldStructuredDumpBoxReport {
            box_type: "ftyp".to_string(),
            path: "ftyp".to_string(),
            offset: 0,
            size: 20,
            supported: true,
            payload_status: DumpPayloadStatus::Summary,
            payload_fields: vec![
                StructuredDumpFieldReport {
                    name: "MajorBrand".to_string(),
                    value: FieldValue::String("isom".to_string()),
                    display_value: None,
                },
                StructuredDumpFieldReport {
                    name: "CompatibleBrands".to_string(),
                    value: FieldValue::Bytes(vec![105, 115, 111, 109]),
                    display_value: Some("[{CompatibleBrand=\"isom\"}]".to_string()),
                },
            ],
            payload_summary: Some(
                "MajorBrand=\"isom\" CompatibleBrands=[{CompatibleBrand=\"isom\"}]".to_string(),
            ),
            payload_bytes: None,
            children: Vec::new(),
        }],
    };

    let mut json = Vec::new();
    dump::write_field_structured_report(&mut json, &report, StructuredDumpFormat::Json).unwrap();
    assert_eq!(
        String::from_utf8(json).unwrap(),
        concat!(
            "{\n",
            "  \"Boxes\": [\n",
            "    {\n",
            "      \"BoxType\": \"ftyp\",\n",
            "      \"Path\": \"ftyp\",\n",
            "      \"Offset\": 0,\n",
            "      \"Size\": 20,\n",
            "      \"Supported\": true,\n",
            "      \"PayloadStatus\": \"summary\",\n",
            "      \"PayloadFields\": [\n",
            "        {\n",
            "          \"Name\": \"MajorBrand\",\n",
            "          \"ValueKind\": \"string\",\n",
            "          \"Value\": \"isom\"\n",
            "        },\n",
            "        {\n",
            "          \"Name\": \"CompatibleBrands\",\n",
            "          \"ValueKind\": \"bytes\",\n",
            "          \"Value\": [\n",
            "            105,\n",
            "            115,\n",
            "            111,\n",
            "            109\n",
            "          ],\n",
            "          \"DisplayValue\": \"[{CompatibleBrand=\\\"isom\\\"}]\"\n",
            "        }\n",
            "      ],\n",
            "      \"PayloadSummary\": \"MajorBrand=\\\"isom\\\" CompatibleBrands=[{CompatibleBrand=\\\"isom\\\"}]\",\n",
            "      \"Children\": [\n",
            "      ]\n",
            "    }\n",
            "  ]\n",
            "}\n"
        )
    );

    let mut yaml = Vec::new();
    dump::write_field_structured_report(&mut yaml, &report, StructuredDumpFormat::Yaml).unwrap();
    assert_eq!(
        String::from_utf8(yaml).unwrap(),
        concat!(
            "boxes:\n",
            "- box_type: ftyp\n",
            "  path: ftyp\n",
            "  offset: 0\n",
            "  size: 20\n",
            "  supported: true\n",
            "  payload_status: summary\n",
            "  payload_fields:\n",
            "    - name: MajorBrand\n",
            "      value_kind: string\n",
            "      value: isom\n",
            "    - name: CompatibleBrands\n",
            "      value_kind: bytes\n",
            "      value:\n",
            "        - 105\n",
            "        - 115\n",
            "        - 111\n",
            "        - 109\n",
            "      display_value: '[{CompatibleBrand=\"isom\"}]'\n",
            "  payload_summary: 'MajorBrand=\"isom\" CompatibleBrands=[{CompatibleBrand=\"isom\"}]'\n",
            "  children: []\n"
        )
    );
}

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
        (&["-format", "json"], "cli_dump/sample.json"),
        (&["-format", "yaml"], "cli_dump/sample.yaml"),
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
fn structured_dump_report_respects_full_payload_controls() {
    let mut default_reader = Cursor::new(build_dump_input_file());
    let default_report =
        dump::build_structured_report(&mut default_reader, &dump::DumpOptions::default()).unwrap();

    assert_eq!(default_report.boxes.len(), 4);
    assert_eq!(
        default_report.boxes[0].payload_status,
        DumpPayloadStatus::Summary
    );
    assert_eq!(
        default_report.boxes[1].payload_status,
        DumpPayloadStatus::Omitted
    );
    assert_eq!(
        default_report.boxes[2].payload_status,
        DumpPayloadStatus::Omitted
    );
    assert_eq!(
        default_report.boxes[3].payload_status,
        DumpPayloadStatus::Empty
    );
    assert_eq!(
        default_report.boxes[3].children[0].payload_status,
        DumpPayloadStatus::Summary
    );

    let mut full_options = dump::DumpOptions::default();
    full_options.full_box_types.insert(fourcc("free"));
    full_options.full_box_types.insert(fourcc("zzzz"));
    full_options.full_box_types.insert(fourcc("mvhd"));

    let mut full_reader = Cursor::new(build_dump_input_file());
    let full_report = dump::build_structured_report(&mut full_reader, &full_options).unwrap();

    assert_eq!(
        full_report.boxes[1].payload_status,
        DumpPayloadStatus::Summary
    );
    assert!(
        full_report.boxes[1]
            .payload_summary
            .as_ref()
            .unwrap()
            .contains("Data=[0xaa, 0xbb, 0xcc, 0xdd]")
    );
    assert_eq!(
        full_report.boxes[2].payload_status,
        DumpPayloadStatus::Bytes
    );
    assert_eq!(
        full_report.boxes[2].payload_bytes.as_deref(),
        Some(&[1, 2, 3][..])
    );
    assert_eq!(
        full_report.boxes[3].children[0].payload_status,
        DumpPayloadStatus::Summary
    );
    assert!(
        full_report.boxes[3].children[0]
            .payload_summary
            .as_ref()
            .unwrap()
            .contains("Version=0")
    );
}

#[test]
fn field_structured_dump_report_prefers_supported_fields_over_legacy_leaf_omission() {
    let fixture = fixture_path("sample.mp4");
    let mut reader = fs::File::open(&fixture).unwrap();
    let report =
        dump::build_field_structured_report(&mut reader, &dump::DumpOptions::default()).unwrap();

    let mdat = find_field_box(&report.boxes, "mdat").unwrap();
    assert_eq!(mdat.payload_status, DumpPayloadStatus::Omitted);
    assert!(mdat.payload_fields.is_empty());

    let ctts = find_field_box(&report.boxes, "moov/trak/mdia/minf/stbl/ctts").unwrap();
    assert_eq!(ctts.payload_status, DumpPayloadStatus::Summary);
    assert!(ctts.payload_summary.is_some());
    assert_eq!(
        ctts.payload_fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Version", "Flags", "EntryCount", "Entries"]
    );
    let entries = &ctts.payload_fields[3];
    assert!(matches!(entries.value, FieldValue::Bytes(_)));
    assert!(
        entries
            .display_value
            .as_ref()
            .unwrap()
            .contains("SampleCount=1")
    );
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

#[test]
fn dump_command_scopes_text_output_to_selected_subtrees() {
    let path = write_temp_file("dump-cli-path", &build_dump_input_file());
    let args = vec![
        "--path".to_string(),
        "moov".to_string(),
        path.to_string_lossy().into_owned(),
    ];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = dump::run(&args, &mut stdout, &mut stderr);

    let _ = fs::remove_file(&path);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        concat!(
            "[moov] Size=116\n",
            "  [mvhd] Size=108 ... (use \"-full mvhd\" to show all)\n"
        )
    );
}

#[test]
fn dump_command_scopes_structured_output_to_selected_subtrees() {
    let path = write_temp_file("dump-cli-path-json", &build_dump_input_file());
    let args = vec![
        "--format".to_string(),
        "json".to_string(),
        "--path".to_string(),
        "moov".to_string(),
        path.to_string_lossy().into_owned(),
    ];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = dump::run(&args, &mut stdout, &mut stderr);

    let _ = fs::remove_file(&path);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");

    let output = String::from_utf8(stdout).unwrap();
    assert!(output.contains("\"Path\": \"moov\""));
    assert!(output.contains("\"Path\": \"moov/mvhd\""));
    assert!(!output.contains("\"Path\": \"ftyp\""));
    assert!(!output.contains("\"Path\": \"zzzz\""));
}

#[test]
fn structured_dump_report_paths_support_wildcards_and_exact_roots() {
    let fixture = fixture_path("sample.mp4");
    let mut reader = fs::File::open(&fixture).unwrap();
    let paths = vec![BoxPath::parse("moov/*/mdia/mdhd").unwrap()];
    let report =
        dump::build_structured_report_paths(&mut reader, &dump::DumpOptions::default(), &paths)
            .unwrap();

    assert_eq!(report.boxes.len(), 2);
    assert!(report.boxes.iter().all(|entry| entry.box_type == "mdhd"));
    assert!(
        report
            .boxes
            .iter()
            .all(|entry| entry.path == "moov/trak/mdia/mdhd")
    );
    assert!(report.boxes.iter().all(|entry| entry.children.is_empty()));
}

#[test]
fn dump_command_treats_root_path_like_full_dump() {
    let fixture = fixture_path("sample.mp4");
    let args = vec![
        "--path".to_string(),
        "<root>".to_string(),
        fixture.to_string_lossy().into_owned(),
    ];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = dump::run(&args, &mut stdout, &mut stderr);

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(
        normalize_text(&String::from_utf8(stdout).unwrap()),
        read_golden("cli_dump/sample.txt")
    );
}

#[test]
fn dump_command_returns_empty_output_for_unmatched_paths() {
    let fixture = fixture_path("sample.mp4");

    let mut text_stdout = Vec::new();
    let mut text_stderr = Vec::new();
    let text_exit_code = dump::run(
        &[
            "--path".to_string(),
            "moov/zzzz".to_string(),
            fixture.to_string_lossy().into_owned(),
        ],
        &mut text_stdout,
        &mut text_stderr,
    );

    assert_eq!(text_exit_code, 0);
    assert_eq!(String::from_utf8(text_stderr).unwrap(), "");
    assert_eq!(String::from_utf8(text_stdout).unwrap(), "");

    let mut json_stdout = Vec::new();
    let mut json_stderr = Vec::new();
    let json_exit_code = dump::run(
        &[
            "--format".to_string(),
            "json".to_string(),
            "--path".to_string(),
            "moov/zzzz".to_string(),
            fixture.to_string_lossy().into_owned(),
        ],
        &mut json_stdout,
        &mut json_stderr,
    );

    assert_eq!(json_exit_code, 0);
    assert_eq!(String::from_utf8(json_stderr).unwrap(), "");
    assert_eq!(
        String::from_utf8(json_stdout).unwrap(),
        concat!("{\n", "  \"Boxes\": [\n", "  ]\n", "}\n")
    );
}

#[test]
fn dump_command_rejects_invalid_path_arguments() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        dump::run(
            &[
                "--path".to_string(),
                "moov/trakk".to_string(),
                fixture_path("sample.mp4").to_string_lossy().into_owned(),
            ],
            &mut stdout,
            &mut stderr,
        ),
        1
    );
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: invalid box path: invalid box path segment 2 (\"trakk\"): fourcc values must be exactly 4 bytes, got 5\n"
    );
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

fn find_field_box<'a>(
    boxes: &'a [FieldStructuredDumpBoxReport],
    path: &str,
) -> Option<&'a FieldStructuredDumpBoxReport> {
    for entry in boxes {
        if entry.path == path {
            return Some(entry);
        }
        if let Some(found) = find_field_box(&entry.children, path) {
            return Some(found);
        }
    }
    None
}
