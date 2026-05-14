#![cfg(feature = "mux")]

mod support;

use mp4forge::mux::{MuxError, MuxRequest, MuxTrackSpec, mux_to_path};

use support::{
    write_temp_file, write_temp_file_with_extension, write_test_avi_audio_tag_file,
    write_test_saf_remote_url_file,
};

#[test]
fn mux_to_path_rejects_non_core_dts_family_with_actionable_message() {
    let dts_input = write_temp_file("mux-diagnostics-dtshd-input", b"DTSHDHDRdemo");
    let output_path = write_temp_file("mux-diagnostics-dtshd-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&dts_input)]);

    let error = mux_to_path(&request, &output_path).unwrap_err();
    let message = error.to_string();

    assert!(message.contains("non-core DTS-family audio"), "{message}");
    assert!(
        message.contains("expose one contiguous core substream"),
        "{message}"
    );
    assert!(
        message.contains("little-endian core DTS sync frames"),
        "{message}"
    );
    assert!(
        message.contains("transformed 14-bit core DTS sync frames"),
        "{message}"
    );
    assert!(
        message
            .contains("import this family from an MP4 source with `#audio` or `#track:ID` instead"),
        "{message}"
    );
}

#[test]
fn mux_to_path_rejects_unknown_avi_audio_tags_with_context() {
    let avi_input = write_test_avi_audio_tag_file(
        "mux-diagnostics-avi-tag-input",
        0x7777,
        8_000,
        1,
        4,
        &[b"\x12\x34\x56\x78"],
    );
    let output_path = write_temp_file("mux-diagnostics-avi-tag-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&avi_input)]);

    let error = mux_to_path(&request, &output_path).unwrap_err();
    let message = error.to_string();

    assert!(
        message.contains("unsupported WAVE format tag 0x7777"),
        "{message}"
    );
    assert!(message.contains("channels=1"), "{message}");
    assert!(message.contains("sample_rate=8000"), "{message}");
    assert!(message.contains("bits_per_sample=4"), "{message}");
    assert!(message.contains("currently accepts"), "{message}");
    assert!(message.contains("IBM CVSD"), "{message}");
    assert!(message.contains("OKI ADPCM"), "{message}");
    assert!(message.contains("DIGISTD"), "{message}");
    assert!(message.contains("Yamaha ADPCM"), "{message}");
    assert!(message.contains("DSP TrueSpeech"), "{message}");
    assert!(message.contains("GSM 610"), "{message}");
    assert!(message.contains("IBM ADPCM"), "{message}");
    assert!(message.contains("AAC ADTS"), "{message}");
}

#[test]
fn mux_to_path_rejects_saf_remote_url_declarations_with_actionable_message() {
    let saf_input = write_test_saf_remote_url_file("mux-diagnostics-saf-remote-input");
    let output_path = write_temp_file("mux-diagnostics-saf-remote-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&saf_input)]);

    let error = mux_to_path(&request, &output_path).unwrap_err();
    let message = error.to_string();

    assert!(
        message
            .contains("remote URL declarations are outside the current path-only import contract"),
        "{message}"
    );
}

#[test]
fn mux_to_path_rejects_gsf_serialized_transport_sources_with_actionable_message() {
    let gsf_input =
        write_temp_file_with_extension("mux-diagnostics-gsf-input", "gsf", b"GS5F\x01demo");
    let output_path = write_temp_file("mux-diagnostics-gsf-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&gsf_input)]);

    let error = mux_to_path(&request, &output_path).unwrap_err();
    let message = error.to_string();

    assert!(
        message.contains("GSF is a serialized multi-PID transport surface"),
        "{message}"
    );
    assert!(
        message.contains("import the authored files or authored MP4 tracks directly instead"),
        "{message}"
    );
}

#[test]
fn mux_to_path_rejects_ghi_segment_index_sources_with_actionable_message() {
    let ghi_input = write_temp_file_with_extension("mux-diagnostics-ghi-input", "ghi", b"GHIDdemo");
    let output_path = write_temp_file("mux-diagnostics-ghi-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path(&ghi_input)]);

    let error = mux_to_path(&request, &output_path).unwrap_err();
    let message = error.to_string();

    assert!(
        message.contains("GHI is a segment-index or manifest transport surface"),
        "{message}"
    );
    assert!(
        message.contains("import the authored media files or local MPD inputs directly instead"),
        "{message}"
    );
}

#[test]
fn mux_to_path_reports_missing_input_path_with_context() {
    let output_path = write_temp_file("mux-diagnostics-missing-input-output", &[]);
    let request = MuxRequest::new(vec![MuxTrackSpec::path("this-file-does-not-exist.bin")]);

    let error = mux_to_path(&request, &output_path).unwrap_err();
    let message = error.to_string();

    assert!(message.contains("failed to open mux input"), "{message}");
    assert!(
        message.contains("this-file-does-not-exist.bin"),
        "{message}"
    );
}

#[test]
fn mux_errors_report_stable_category_and_stage_metadata() {
    let request_error = MuxError::InvalidOutputLayout {
        layout: "fragmented",
        message: "demo".to_string(),
    };
    assert_eq!(request_error.category(), "input");
    assert_eq!(request_error.stage(), "request");

    let unsupported = MuxError::UnsupportedTrackImport {
        spec: "demo".to_string(),
        message: "not supported".to_string(),
    };
    assert_eq!(unsupported.category(), "unsupported");
    assert_eq!(unsupported.stage(), "import");
}
