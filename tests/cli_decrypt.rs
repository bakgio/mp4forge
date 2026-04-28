#![cfg(feature = "decrypt")]

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;

use mp4forge::cli::{self, decrypt};
use mp4forge::extract::extract_box_payload_bytes;
use mp4forge::probe::probe_detailed;
use mp4forge::walk::BoxPath;

use support::{
    ProtectedMovieTopologyFixture, RetainedDecryptFileFixture, RetainedFragmentedDecryptFixture,
    build_decrypt_rewrite_fixture, build_iaec_broader_movie_fixture,
    build_iaec_sample_description_index_unsupported_movie_fixture,
    build_marlin_ipmp_acbc_broader_movie_fixture,
    build_marlin_ipmp_acbc_sample_description_index_movie_fixture,
    build_marlin_ipmp_acgk_broader_movie_fixture,
    build_marlin_ipmp_acgk_sample_description_index_movie_fixture,
    build_multi_sample_entry_decrypt_fixture, build_oma_dcf_broader_movie_fixture,
    build_oma_dcf_sample_description_index_unsupported_movie_fixture,
    build_zero_kid_multi_sample_entry_decrypt_fixture, common_encryption_fragment_fixture,
    common_encryption_multi_track_fixture, fourcc, isma_iaec_fixture, marlin_ipmp_acbc_fixture,
    marlin_ipmp_acgk_fixture, oma_dcf_cbc_fixture, oma_dcf_cbc_grpi_fixture, oma_dcf_ctr_fixture,
    oma_dcf_ctr_grpi_fixture, piff_cbc_fixture, piff_cbc_segment_fixture, piff_ctr_fixture,
    piff_ctr_segment_fixture, write_temp_file,
};

#[test]
fn decrypt_command_writes_clear_output_via_dispatch() {
    let fixture = build_decrypt_rewrite_fixture();
    let input_path = write_temp_file("cli-decrypt-input", &fixture.single_file);
    let output_path = write_temp_file("cli-decrypt-output", &[]);
    let args = vec![
        "decrypt".to_string(),
        "--key".to_string(),
        fixture.all_keys[0].to_spec(),
        "--key".to_string(),
        fixture.all_keys[1].to_spec(),
        input_path.to_string_lossy().into_owned(),
        output_path.to_string_lossy().into_owned(),
    ];

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = cli::dispatch(&args, &mut stdout, &mut stderr);

    let output = fs::read(&output_path).unwrap();
    let detailed = probe_detailed(&mut Cursor::new(output)).unwrap();
    let tracks = detailed
        .tracks
        .into_iter()
        .map(|track| (track.summary.track_id, track))
        .collect::<BTreeMap<_, _>>();

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 0, "stderr={}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    for track_id in [fixture.first_track_id, fixture.second_track_id] {
        let track = tracks.get(&track_id).unwrap();
        assert!(!track.summary.encrypted);
        assert_eq!(track.sample_entry_type, Some(fourcc("avc1")));
        assert!(track.protection_scheme.is_none());
    }
}

#[test]
fn decrypt_command_supports_fragments_info_files() {
    let fixture = build_decrypt_rewrite_fixture();
    let init_path = write_temp_file("cli-decrypt-init", &fixture.init_segment);
    let input_path = write_temp_file("cli-decrypt-media", &fixture.media_segment);
    let output_path = write_temp_file("cli-decrypt-media-output", &[]);
    let args = vec![
        "--key".to_string(),
        fixture.all_keys[0].to_spec(),
        "--key".to_string(),
        fixture.all_keys[1].to_spec(),
        "--fragments-info".to_string(),
        init_path.to_string_lossy().into_owned(),
        input_path.to_string_lossy().into_owned(),
        output_path.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = decrypt::run(&args, &mut stderr);

    let output = fs::read(&output_path).unwrap();
    let mdat_payloads = extract_box_payload_bytes(
        &mut Cursor::new(output),
        None,
        BoxPath::from([fourcc("mdat")]),
    )
    .unwrap();

    let _ = fs::remove_file(&init_path);
    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 0, "stderr={}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(mdat_payloads.len(), 1);
    assert_eq!(
        mdat_payloads[0],
        [
            fixture.first_track_plaintext,
            fixture.second_track_plaintext
        ]
        .concat()
    );
}

#[test]
fn decrypt_command_writes_stable_progress_lines() {
    let fixture = build_decrypt_rewrite_fixture();
    let input_path = write_temp_file("cli-decrypt-progress-input", &fixture.single_file);
    let output_path = write_temp_file("cli-decrypt-progress-output", &[]);
    let args = vec![
        "--show-progress".to_string(),
        "--key".to_string(),
        fixture.all_keys[0].to_spec(),
        "--key".to_string(),
        fixture.all_keys[1].to_spec(),
        input_path.to_string_lossy().into_owned(),
        output_path.to_string_lossy().into_owned(),
    ];

    let mut stderr = Vec::new();
    let exit_code = decrypt::run(&args, &mut stderr);

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 0, "stderr={}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        concat!(
            "OpenInput 0/1\n",
            "OpenInput 1/1\n",
            "InspectStructure 0/1\n",
            "InspectStructure 1/1\n",
            "ProcessSamples 0/1\n",
            "ProcessSamples 1/1\n",
            "OpenOutput 0/1\n",
            "OpenOutput 1/1\n",
            "FinalizeOutput 0/1\n",
            "FinalizeOutput 1/1\n",
        )
    );
}

#[test]
fn decrypt_command_rejects_invalid_arguments() {
    let mut stderr = Vec::new();
    assert_eq!(decrypt::run(&[], &mut stderr), 1);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        concat!(
            "USAGE: mp4forge decrypt --key <ID:KEY> [--key <ID:KEY> ...] [--fragments-info FILE] [--show-progress] INPUT OUTPUT\n",
            "\n",
            "OPTIONS:\n",
            "  --key <ID:KEY>             Add one decryption key addressed by decimal track ID or 128-bit KID\n",
            "  --fragments-info <FILE>    Read matching initialization-segment bytes for standalone media-segment decrypt\n",
            "  --show-progress            Write coarse decrypt progress snapshots to stderr\n",
            "\n",
            "Key syntax:\n",
            "  --key <id>:<key>\n",
            "      <id> is either a track ID in decimal or a 128-bit KID in hex\n",
            "      <key> is a 128-bit decryption key in hex\n",
            "      note: --fragments-info is typically the init segment when decrypting fragmented media segments\n",
        )
    );

    let mut stderr = Vec::new();
    assert_eq!(
        decrypt::run(
            &["input.mp4".to_string(), "output.mp4".to_string(),],
            &mut stderr,
        ),
        1
    );
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: at least one --key <ID:KEY> is required\n"
    );

    let mut stderr = Vec::new();
    assert_eq!(
        decrypt::run(
            &[
                "--key".to_string(),
                "bad".to_string(),
                "input.mp4".to_string(),
                "output.mp4".to_string(),
            ],
            &mut stderr,
        ),
        1
    );
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: invalid decryption key spec \"bad\": expected <id>:<key>\n"
    );
}

fn assert_retained_file_fixture_cli_decrypts(
    fixture: &RetainedDecryptFileFixture,
    temp_prefix: &str,
) {
    let output_path = write_temp_file(temp_prefix, &[]);
    let expected = fs::read(&fixture.decrypted_path).unwrap();
    let mut args = vec!["decrypt".to_string()];
    for key in &fixture.keys {
        args.push("--key".to_string());
        args.push(key.to_spec());
    }
    args.push(fixture.encrypted_path.to_string_lossy().into_owned());
    args.push(output_path.to_string_lossy().into_owned());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = cli::dispatch(&args, &mut stdout, &mut stderr);
    let output = fs::read(&output_path).unwrap();

    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 0, "stderr={}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(output, expected);
}

fn assert_retained_fragmented_fixture_cli_decrypts(
    fixture: &RetainedFragmentedDecryptFixture,
    temp_prefix: &str,
) {
    let output_path = write_temp_file(temp_prefix, &[]);
    let expected = fs::read(&fixture.clear_segment_path).unwrap();
    let mut args = vec!["decrypt".to_string()];
    for key in &fixture.keys {
        args.push("--key".to_string());
        args.push(key.to_spec());
    }
    args.push("--fragments-info".to_string());
    args.push(fixture.fragments_info_path.to_string_lossy().into_owned());
    args.push(
        fixture
            .encrypted_segment_path
            .to_string_lossy()
            .into_owned(),
    );
    args.push(output_path.to_string_lossy().into_owned());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = cli::dispatch(&args, &mut stdout, &mut stderr);
    let output = fs::read(&output_path).unwrap();

    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 0, "stderr={}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(output, expected);
}

fn assert_generated_topology_fixture_cli_decrypts(
    fixture: ProtectedMovieTopologyFixture,
    temp_prefix: &str,
) {
    let input_path = write_temp_file(temp_prefix, &fixture.encrypted);
    let output_path = write_temp_file(&format!("{temp_prefix}-output"), &[]);
    let mut args = vec!["decrypt".to_string()];
    for key in &fixture.keys {
        args.push("--key".to_string());
        args.push(key.to_spec());
    }
    args.push(input_path.to_string_lossy().into_owned());
    args.push(output_path.to_string_lossy().into_owned());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = cli::dispatch(&args, &mut stdout, &mut stderr);
    let output = fs::read(&output_path).unwrap();

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 0, "stderr={}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), "");
    assert_eq!(output, fixture.decrypted);
}

fn assert_generated_topology_fixture_cli_rejects_first_sample_description_limit(
    fixture: ProtectedMovieTopologyFixture,
    temp_prefix: &str,
) {
    let input_path = write_temp_file(temp_prefix, &fixture.encrypted);
    let output_path = write_temp_file(&format!("{temp_prefix}-output"), &[]);
    let mut args = vec!["decrypt".to_string()];
    for key in &fixture.keys {
        args.push("--key".to_string());
        args.push(key.to_spec());
    }
    args.push(input_path.to_string_lossy().into_owned());
    args.push(output_path.to_string_lossy().into_owned());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = cli::dispatch(&args, &mut stdout, &mut stderr);
    let stderr_text = String::from_utf8(stderr).unwrap();

    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    assert_eq!(exit_code, 1, "stderr={stderr_text}");
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert!(
        stderr_text.contains("only supports the first protected sample description"),
        "unexpected stderr: {stderr_text}"
    );
}

macro_rules! common_encryption_fragment_cli_case {
    ($test_name:ident, $directory:literal, $track:literal, $prefix:literal) => {
        #[test]
        fn $test_name() {
            let fixture = common_encryption_fragment_fixture($directory, $track);
            assert_retained_fragmented_fixture_cli_decrypts(&fixture, $prefix);
        }
    };
}

#[test]
fn decrypt_command_supports_retained_oma_dcf_ctr_movie_files() {
    assert_retained_file_fixture_cli_decrypts(&oma_dcf_ctr_fixture(), "cli-decrypt-oma-ctr-output");
}

#[test]
fn decrypt_command_supports_broader_oma_dcf_movie_layouts() {
    assert_generated_topology_fixture_cli_decrypts(
        build_oma_dcf_broader_movie_fixture(),
        "cli-decrypt-oma-broader-input",
    );
}

#[test]
fn decrypt_command_rejects_oma_dcf_movie_sample_description_indices_beyond_the_first_entry() {
    assert_generated_topology_fixture_cli_rejects_first_sample_description_limit(
        build_oma_dcf_sample_description_index_unsupported_movie_fixture(),
        "cli-decrypt-oma-sample-description-index-input",
    );
}

#[test]
fn decrypt_command_supports_retained_piff_ctr_movie_files() {
    assert_retained_file_fixture_cli_decrypts(&piff_ctr_fixture(), "cli-decrypt-piff-ctr-output");
}

#[test]
fn decrypt_command_supports_retained_piff_cbc_movie_files() {
    assert_retained_file_fixture_cli_decrypts(&piff_cbc_fixture(), "cli-decrypt-piff-cbc-output");
}

#[test]
fn decrypt_command_supports_retained_piff_ctr_media_segments() {
    assert_retained_fragmented_fixture_cli_decrypts(
        &piff_ctr_segment_fixture(),
        "cli-decrypt-piff-ctr-segment-output",
    );
}

#[test]
fn decrypt_command_supports_retained_piff_cbc_media_segments() {
    assert_retained_fragmented_fixture_cli_decrypts(
        &piff_cbc_segment_fixture(),
        "cli-decrypt-piff-cbc-segment-output",
    );
}

#[test]
fn decrypt_command_supports_retained_oma_dcf_cbc_movie_files() {
    assert_retained_file_fixture_cli_decrypts(&oma_dcf_cbc_fixture(), "cli-decrypt-oma-cbc-output");
}

#[test]
fn decrypt_command_supports_retained_oma_dcf_ctr_grouped_atom_files() {
    assert_retained_file_fixture_cli_decrypts(
        &oma_dcf_ctr_grpi_fixture(),
        "cli-decrypt-oma-ctr-grpi-output",
    );
}

#[test]
fn decrypt_command_supports_retained_oma_dcf_cbc_grouped_atom_files() {
    assert_retained_file_fixture_cli_decrypts(
        &oma_dcf_cbc_grpi_fixture(),
        "cli-decrypt-oma-cbc-grpi-output",
    );
}

#[test]
fn decrypt_command_supports_retained_isma_iaec_movie_files() {
    assert_retained_file_fixture_cli_decrypts(&isma_iaec_fixture(), "cli-decrypt-iaec-output");
}

#[test]
fn decrypt_command_rejects_iaec_movie_sample_description_indices_beyond_the_first_entry() {
    assert_generated_topology_fixture_cli_rejects_first_sample_description_limit(
        build_iaec_sample_description_index_unsupported_movie_fixture(),
        "cli-decrypt-iaec-sample-description-index-input",
    );
}

#[test]
fn decrypt_command_supports_broader_iaec_movie_layouts() {
    assert_generated_topology_fixture_cli_decrypts(
        build_iaec_broader_movie_fixture(),
        "cli-decrypt-iaec-broader-input",
    );
}

#[test]
fn decrypt_command_supports_retained_marlin_ipmp_acbc_movie_files() {
    assert_retained_file_fixture_cli_decrypts(
        &marlin_ipmp_acbc_fixture(),
        "cli-decrypt-marlin-acbc-output",
    );
}

#[test]
fn decrypt_command_supports_broader_marlin_ipmp_acbc_movie_layouts() {
    assert_generated_topology_fixture_cli_decrypts(
        build_marlin_ipmp_acbc_broader_movie_fixture(),
        "cli-decrypt-marlin-acbc-broader-input",
    );
}

#[test]
fn decrypt_command_supports_marlin_ipmp_acbc_od_track_sample_description_indices() {
    assert_generated_topology_fixture_cli_decrypts(
        build_marlin_ipmp_acbc_sample_description_index_movie_fixture(),
        "cli-decrypt-marlin-acbc-stsc-input",
    );
}

#[test]
fn decrypt_command_supports_retained_marlin_ipmp_acgk_movie_files() {
    assert_retained_file_fixture_cli_decrypts(
        &marlin_ipmp_acgk_fixture(),
        "cli-decrypt-marlin-acgk-output",
    );
}

#[test]
fn decrypt_command_supports_broader_marlin_ipmp_acgk_movie_layouts() {
    assert_generated_topology_fixture_cli_decrypts(
        build_marlin_ipmp_acgk_broader_movie_fixture(),
        "cli-decrypt-marlin-acgk-broader-input",
    );
}

#[test]
fn decrypt_command_supports_marlin_ipmp_acgk_od_track_sample_description_indices() {
    assert_generated_topology_fixture_cli_decrypts(
        build_marlin_ipmp_acgk_sample_description_index_movie_fixture(),
        "cli-decrypt-marlin-acgk-stsc-input",
    );
}

#[test]
fn decrypt_command_supports_retained_common_encryption_multi_track_files() {
    assert_retained_file_fixture_cli_decrypts(
        &common_encryption_multi_track_fixture(),
        "cli-decrypt-cenc-multi-track-output",
    );
}

#[test]
fn decrypt_command_supports_multi_sample_entry_fragmented_tracks() {
    let fixture = build_multi_sample_entry_decrypt_fixture();
    let input_path = write_temp_file("cli-decrypt-multi-entry-input", &fixture.single_file);
    let output_path = write_temp_file("cli-decrypt-multi-entry-output", &[]);
    let mut args = vec!["decrypt".to_string()];
    for key in &fixture.all_keys {
        args.push("--key".to_string());
        args.push(key.to_spec());
    }
    args.push(input_path.to_string_lossy().into_owned());
    args.push(output_path.to_string_lossy().into_owned());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = cli::dispatch(&args, &mut stdout, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), "");

    let output = fs::read(&output_path).unwrap();
    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);
    assert_eq!(output, fixture.decrypted_single_file);
}

#[test]
fn decrypt_command_supports_zero_kid_multi_sample_entry_fragmented_tracks() {
    let fixture = build_zero_kid_multi_sample_entry_decrypt_fixture();
    let input_path = write_temp_file(
        "cli-decrypt-zero-kid-multi-entry-input",
        &fixture.single_file,
    );
    let output_path = write_temp_file("cli-decrypt-zero-kid-multi-entry-output", &[]);
    let mut args = vec!["decrypt".to_string()];
    for key in &fixture.ordered_track_id_keys {
        args.push("--key".to_string());
        args.push(key.to_spec());
    }
    args.push(input_path.to_string_lossy().into_owned());
    args.push(output_path.to_string_lossy().into_owned());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = cli::dispatch(&args, &mut stdout, &mut stderr);
    assert_eq!(exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), "");

    let output = fs::read(&output_path).unwrap();
    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);
    assert_eq!(output, fixture.decrypted_single_file);
}

common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cenc_single_video_media_segments,
    "cenc-single",
    "video",
    "cli-decrypt-cenc-single-video-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cenc_single_audio_media_segments,
    "cenc-single",
    "audio",
    "cli-decrypt-cenc-single-audio-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cenc_multi_video_media_segments,
    "cenc-multi",
    "video",
    "cli-decrypt-cenc-multi-video-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cenc_multi_audio_media_segments,
    "cenc-multi",
    "audio",
    "cli-decrypt-cenc-multi-audio-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cens_single_video_media_segments,
    "cens-single",
    "video",
    "cli-decrypt-cens-single-video-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cens_single_audio_media_segments,
    "cens-single",
    "audio",
    "cli-decrypt-cens-single-audio-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cens_multi_video_media_segments,
    "cens-multi",
    "video",
    "cli-decrypt-cens-multi-video-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cens_multi_audio_media_segments,
    "cens-multi",
    "audio",
    "cli-decrypt-cens-multi-audio-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cbc1_single_video_media_segments,
    "cbc1-single",
    "video",
    "cli-decrypt-cbc1-single-video-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cbc1_single_audio_media_segments,
    "cbc1-single",
    "audio",
    "cli-decrypt-cbc1-single-audio-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cbc1_multi_video_media_segments,
    "cbc1-multi",
    "video",
    "cli-decrypt-cbc1-multi-video-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cbc1_multi_audio_media_segments,
    "cbc1-multi",
    "audio",
    "cli-decrypt-cbc1-multi-audio-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cbcs_single_video_media_segments,
    "cbcs-single",
    "video",
    "cli-decrypt-cbcs-single-video-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cbcs_single_audio_media_segments,
    "cbcs-single",
    "audio",
    "cli-decrypt-cbcs-single-audio-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cbcs_multi_video_media_segments,
    "cbcs-multi",
    "video",
    "cli-decrypt-cbcs-multi-video-segment-output"
);
common_encryption_fragment_cli_case!(
    decrypt_command_supports_retained_cbcs_multi_audio_media_segments,
    "cbcs-multi",
    "audio",
    "cli-decrypt-cbcs-multi-audio-segment-output"
);
