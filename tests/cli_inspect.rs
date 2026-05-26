#![cfg(feature = "mux")]

mod support;

use mp4forge::cli::inspect;

use support::write_test_ogg_speex_file;

#[test]
fn inspect_command_validates_argument_shape() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(inspect::run::<_, Vec<u8>>(&[], &mut stdout, &mut stderr), 1);
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), inspect_usage());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        inspect::run(
            &[
                "-format".to_string(),
                "toml".to_string(),
                "input.bin".to_string()
            ],
            &mut stdout,
            &mut stderr
        ),
        1
    );
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: unsupported inspect format: toml\n"
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        inspect::run(
            &[
                "-view".to_string(),
                "frames".to_string(),
                "input.bin".to_string()
            ],
            &mut stdout,
            &mut stderr
        ),
        1
    );
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: unsupported inspect view: frames\n"
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        inspect::run(
            &[
                "-format".to_string(),
                "nhnt".to_string(),
                "input.bin".to_string()
            ],
            &mut stdout,
            &mut stderr
        ),
        1
    );
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: NHNT output requires `-view packets`\n"
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        inspect::run(
            &[
                "-view".to_string(),
                "packets".to_string(),
                "-format".to_string(),
                "nhml".to_string(),
                "input.bin".to_string()
            ],
            &mut stdout,
            &mut stderr
        ),
        1
    );
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error [stage=request category=input]: NHML output requires `-view tracks`\n"
    );
}

#[test]
fn inspect_command_writes_real_json_report_for_path_first_ogg_speex_input() {
    let input = write_test_ogg_speex_file("cli-inspect-ogg-speex-input", &[b"abc", b"def"]);

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        inspect::run(
            &[
                "-format".to_string(),
                "json".to_string(),
                input.display().to_string()
            ],
            &mut stdout,
            &mut stderr
        ),
        0
    );
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output = String::from_utf8(stdout).unwrap();
    assert!(output.contains("\"SupportsFlatMux\": true"));
    assert!(output.contains("\"Kind\": \"raw\""));
    assert!(output.contains("\"Codec\": \"speex\""));
    assert!(output.contains("\"TrackCount\": 1"));
    assert!(output.contains("\"SampleCount\": 2"));
}

#[test]
fn inspect_command_writes_packet_view_when_requested() {
    let input = write_test_ogg_speex_file("cli-inspect-packets-ogg-speex-input", &[b"abc", b"def"]);

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        inspect::run(
            &[
                "-view".to_string(),
                "packets".to_string(),
                "-format".to_string(),
                "json".to_string(),
                input.display().to_string()
            ],
            &mut stdout,
            &mut stderr
        ),
        0
    );
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output = String::from_utf8(stdout).unwrap();
    assert!(output.contains("\"Packets\": ["));
    assert!(output.contains("\"PacketCount\": 2"));
    assert!(output.contains("\"TrackKind\": \"audio\""));
    assert!(output.contains("\"PacketIndex\": 1"));
    assert!(output.contains("\"PayloadCrc32\":"));
    assert!(output.contains("\"PreviousDecodeDelta\":"));
}

#[test]
fn inspect_command_can_emit_warning_mode_when_track_diagnostics_exist() {
    let input =
        write_test_ogg_speex_file("cli-inspect-warnings-ogg-speex-input", &[b"abc", b"def"]);

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        inspect::run(
            &[
                "-warnings".to_string(),
                "-format".to_string(),
                "json".to_string(),
                input.display().to_string()
            ],
            &mut stdout,
            &mut stderr
        ),
        0
    );
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Warning: track 1 (audio) changes decode duration 1 time(s)\n"
    );
    let output = String::from_utf8(stdout).unwrap();
    assert!(output.contains("\"SupportsFlatMux\": true"));
}

#[test]
fn inspect_command_writes_nhml_and_nhnt_sidecars() {
    let input =
        write_test_ogg_speex_file("cli-inspect-sidecars-ogg-speex-input", &[b"abc", b"def"]);

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        inspect::run(
            &[
                "-format".to_string(),
                "nhml".to_string(),
                input.display().to_string()
            ],
            &mut stdout,
            &mut stderr
        ),
        0
    );
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output = String::from_utf8(stdout).unwrap();
    assert!(output.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<nhml "));
    assert!(output.contains("codec=\"speex\""));
    assert!(output.contains("<track "));

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        inspect::run(
            &[
                "-view".to_string(),
                "packets".to_string(),
                "-format".to_string(),
                "nhnt".to_string(),
                input.display().to_string()
            ],
            &mut stdout,
            &mut stderr
        ),
        0
    );
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    let output = String::from_utf8(stdout).unwrap();
    assert!(output.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<nhnt "));
    assert!(output.contains("codec=\"speex\""));
    assert!(output.contains("<packet "));
    assert!(output.contains("payloadCrc32=\""));
}

fn inspect_usage() -> String {
    String::from(
        "USAGE: mp4forge inspect [OPTIONS] INPUT\n\nOPTIONS:\n  -format <json|yaml|nhml|nhnt>  Output format (default: json)\n  -view <tracks|packets>  Inspection view (default: tracks)\n  -warnings  Emit warning-grade diagnostics to stderr after a successful report\n",
    )
}
