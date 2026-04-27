#![cfg(feature = "decrypt")]

mod support;

use std::fs;
use std::io::Cursor;

use mp4forge::decrypt::{
    DecryptError, DecryptOptions, DecryptProgress, DecryptProgressPhase, DecryptRewriteError,
    DecryptionKey, DecryptionKeyId, decrypt_bytes, decrypt_bytes_with_progress,
    decrypt_file_with_progress,
};
use mp4forge::extract::extract_box_payload_bytes;
use mp4forge::probe::probe_detailed;
use mp4forge::walk::BoxPath;

use support::{
    ProtectedMovieTopologyFixture, RetainedDecryptFileFixture, RetainedFragmentedDecryptFixture,
    build_decrypt_rewrite_fixture, build_iaec_broader_movie_fixture,
    build_marlin_ipmp_acbc_broader_movie_fixture, build_marlin_ipmp_acgk_broader_movie_fixture,
    build_multi_sample_entry_decrypt_fixture, build_oma_dcf_broader_movie_fixture,
    build_zero_kid_multi_sample_entry_decrypt_fixture, common_encryption_fragment_fixture,
    common_encryption_multi_track_fixture, common_encryption_single_key_fixture_keys, fourcc,
    isma_iaec_fixture, marlin_ipmp_acbc_fixture, marlin_ipmp_acgk_fixture, oma_dcf_cbc_fixture,
    oma_dcf_cbc_grpi_fixture, oma_dcf_ctr_fixture, oma_dcf_ctr_grpi_fixture, piff_cbc_fixture,
    piff_cbc_segment_fixture, piff_ctr_fixture, piff_ctr_segment_fixture, write_temp_file,
};

#[test]
fn decrypt_options_builder_accepts_repeated_keys_and_fragments_info() {
    let options = DecryptOptions::new()
        .with_key_spec("7:00112233445566778899aabbccddeeff")
        .unwrap()
        .with_key(DecryptionKey::kid([0xaa; 16], [0xbb; 16]))
        .with_fragments_info_bytes([1_u8, 2, 3, 4]);

    assert_eq!(options.keys().len(), 2);
    assert_eq!(options.keys()[0].id(), DecryptionKeyId::TrackId(7));
    assert_eq!(
        options.keys()[0].key_bytes(),
        [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ]
    );
    assert_eq!(options.keys()[1].id(), DecryptionKeyId::Kid([0xaa; 16]));
    assert_eq!(options.fragments_info_bytes(), Some(&[1_u8, 2, 3, 4][..]));
}

#[test]
fn decrypt_bytes_requires_fragments_info_for_standalone_media_segments() {
    let fixture = build_decrypt_rewrite_fixture();

    let error = decrypt_bytes(
        &fixture.media_segment,
        &options_with_keys(&fixture.all_keys),
    )
    .unwrap_err();

    assert!(matches!(error, DecryptError::MissingFragmentsInfo));
}

#[test]
fn decrypt_bytes_decrypts_standalone_media_segments_with_fragments_info() {
    let fixture = build_decrypt_rewrite_fixture();
    let options =
        options_with_keys(&fixture.all_keys).with_fragments_info_bytes(&fixture.init_segment);
    let mut progress = Vec::new();

    let output = decrypt_bytes_with_progress(&fixture.media_segment, &options, |snapshot| {
        progress.push(snapshot);
    })
    .unwrap();

    let mdat_payloads = extract_box_payload_bytes(
        &mut Cursor::new(output),
        None,
        BoxPath::from([fourcc("mdat")]),
    )
    .unwrap();
    assert_eq!(mdat_payloads.len(), 1);
    assert_eq!(
        mdat_payloads[0],
        [
            fixture.first_track_plaintext,
            fixture.second_track_plaintext
        ]
        .concat()
    );
    assert_eq!(
        phases(&progress),
        vec![
            DecryptProgressPhase::InspectStructure,
            DecryptProgressPhase::InspectStructure,
            DecryptProgressPhase::OpenFragmentsInfo,
            DecryptProgressPhase::OpenFragmentsInfo,
            DecryptProgressPhase::ProcessSamples,
            DecryptProgressPhase::ProcessSamples,
            DecryptProgressPhase::FinalizeOutput,
            DecryptProgressPhase::FinalizeOutput,
        ]
    );
}

#[test]
fn decrypt_bytes_keeps_partial_decrypt_behavior_for_missing_keys() {
    let fixture = build_decrypt_rewrite_fixture();

    let output = decrypt_bytes(
        &fixture.single_file,
        &options_with_keys(&fixture.first_track_only_keys),
    )
    .unwrap();

    let detailed = probe_detailed(&mut Cursor::new(output)).unwrap();
    let tracks = detailed
        .tracks
        .into_iter()
        .map(|track| (track.summary.track_id, track))
        .collect::<std::collections::BTreeMap<_, _>>();

    let first = tracks.get(&fixture.first_track_id).unwrap();
    assert!(!first.summary.encrypted);
    assert_eq!(first.sample_entry_type, Some(fourcc("avc1")));

    let second = tracks.get(&fixture.second_track_id).unwrap();
    assert!(second.summary.encrypted);
    assert_eq!(second.sample_entry_type, Some(fourcc("encv")));
    assert_eq!(second.original_format, Some(fourcc("avc1")));
}

#[test]
fn decrypt_file_with_progress_writes_clear_output() {
    let fixture = build_decrypt_rewrite_fixture();
    let input_path = write_temp_file("decrypt-api-input", &fixture.single_file);
    let output_path = write_temp_file("decrypt-api-output", &[]);
    let mut progress = Vec::new();

    decrypt_file_with_progress(
        &input_path,
        &output_path,
        &options_with_keys(&fixture.all_keys),
        |snapshot| progress.push(snapshot),
    )
    .unwrap();

    let output = fs::read(output_path).unwrap();
    let detailed = probe_detailed(&mut Cursor::new(output)).unwrap();
    let tracks = detailed
        .tracks
        .into_iter()
        .map(|track| (track.summary.track_id, track))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(tracks.len(), 2);
    for track_id in [fixture.first_track_id, fixture.second_track_id] {
        let track = tracks.get(&track_id).unwrap();
        assert!(!track.summary.encrypted);
        assert_eq!(track.sample_entry_type, Some(fourcc("avc1")));
        assert!(track.protection_scheme.is_none());
    }

    assert_eq!(
        phases(&progress),
        vec![
            DecryptProgressPhase::OpenInput,
            DecryptProgressPhase::OpenInput,
            DecryptProgressPhase::InspectStructure,
            DecryptProgressPhase::InspectStructure,
            DecryptProgressPhase::ProcessSamples,
            DecryptProgressPhase::ProcessSamples,
            DecryptProgressPhase::OpenOutput,
            DecryptProgressPhase::OpenOutput,
            DecryptProgressPhase::FinalizeOutput,
            DecryptProgressPhase::FinalizeOutput,
        ]
    );
}

fn assert_retained_file_fixture_decrypts_bytes(fixture: &RetainedDecryptFileFixture) {
    let input = fs::read(&fixture.encrypted_path).unwrap();
    let expected = fs::read(&fixture.decrypted_path).unwrap();
    let output = decrypt_bytes(&input, &options_with_keys(&fixture.keys)).unwrap();
    assert_eq!(output, expected);
}

fn assert_retained_file_fixture_decrypts_with_progress(
    fixture: &RetainedDecryptFileFixture,
    temp_prefix: &str,
) {
    let output_path = write_temp_file(temp_prefix, &[]);
    let expected = fs::read(&fixture.decrypted_path).unwrap();
    let mut progress = Vec::new();

    decrypt_file_with_progress(
        &fixture.encrypted_path,
        &output_path,
        &options_with_keys(&fixture.keys),
        |snapshot| progress.push(snapshot),
    )
    .unwrap();

    let output = fs::read(output_path).unwrap();
    assert_eq!(output, expected);
    assert_eq!(phases(&progress), expected_file_progress_phases());
}

fn assert_retained_fragmented_fixture_decrypts_bytes(fixture: &RetainedFragmentedDecryptFixture) {
    let segment = fs::read(&fixture.encrypted_segment_path).unwrap();
    let expected = fs::read(&fixture.clear_segment_path).unwrap();
    let fragments_info = fs::read(&fixture.fragments_info_path).unwrap();
    let options = options_with_keys(&fixture.keys).with_fragments_info_bytes(fragments_info);

    let output = decrypt_bytes(&segment, &options).unwrap();

    assert_eq!(output, expected);
}

fn assert_generated_topology_fixture_decrypts_bytes(fixture: ProtectedMovieTopologyFixture) {
    let output = decrypt_bytes(&fixture.encrypted, &options_with_keys(&fixture.keys)).unwrap();
    assert_eq!(output, fixture.decrypted);
}

fn assert_generated_topology_fixture_decrypts_with_progress(
    fixture: ProtectedMovieTopologyFixture,
    temp_prefix: &str,
) {
    let input_path = write_temp_file(temp_prefix, &fixture.encrypted);
    let output_path = write_temp_file(&format!("{temp_prefix}-output"), &[]);
    let mut progress = Vec::new();

    decrypt_file_with_progress(
        &input_path,
        &output_path,
        &options_with_keys(&fixture.keys),
        |snapshot| progress.push(snapshot),
    )
    .unwrap();

    let output = fs::read(&output_path).unwrap();
    assert_eq!(output, fixture.decrypted);
    assert_eq!(phases(&progress), expected_file_progress_phases());
}

macro_rules! common_encryption_fragment_bytes_case {
    ($test_name:ident, $directory:literal, $track:literal) => {
        #[test]
        fn $test_name() {
            let fixture = common_encryption_fragment_fixture($directory, $track);
            assert_retained_fragmented_fixture_decrypts_bytes(&fixture);
        }
    };
}

#[test]
fn decrypt_bytes_supports_retained_oma_dcf_ctr_movie_files() {
    assert_retained_file_fixture_decrypts_bytes(&oma_dcf_ctr_fixture());
}

#[test]
fn decrypt_bytes_supports_broader_oma_dcf_movie_layouts() {
    assert_generated_topology_fixture_decrypts_bytes(build_oma_dcf_broader_movie_fixture());
}

#[test]
fn decrypt_file_with_progress_supports_broader_oma_dcf_movie_layouts() {
    assert_generated_topology_fixture_decrypts_with_progress(
        build_oma_dcf_broader_movie_fixture(),
        "decrypt-api-oma-broader-input",
    );
}

#[test]
fn decrypt_bytes_supports_retained_piff_ctr_movie_files() {
    assert_retained_file_fixture_decrypts_bytes(&piff_ctr_fixture());
}

#[test]
fn decrypt_file_with_progress_supports_retained_piff_ctr_movie_files() {
    assert_retained_file_fixture_decrypts_with_progress(
        &piff_ctr_fixture(),
        "decrypt-api-piff-ctr-output",
    );
}

#[test]
fn decrypt_bytes_supports_retained_piff_cbc_movie_files() {
    assert_retained_file_fixture_decrypts_bytes(&piff_cbc_fixture());
}

#[test]
fn decrypt_file_with_progress_supports_retained_piff_cbc_movie_files() {
    assert_retained_file_fixture_decrypts_with_progress(
        &piff_cbc_fixture(),
        "decrypt-api-piff-cbc-output",
    );
}

#[test]
fn decrypt_bytes_supports_retained_piff_ctr_media_segments() {
    assert_retained_fragmented_fixture_decrypts_bytes(&piff_ctr_segment_fixture());
}

#[test]
fn decrypt_bytes_supports_retained_piff_cbc_media_segments() {
    assert_retained_fragmented_fixture_decrypts_bytes(&piff_cbc_segment_fixture());
}

#[test]
fn decrypt_file_with_progress_supports_retained_oma_dcf_ctr_movie_files() {
    assert_retained_file_fixture_decrypts_with_progress(
        &oma_dcf_ctr_fixture(),
        "decrypt-api-oma-ctr-output",
    );
}

#[test]
fn decrypt_bytes_supports_retained_oma_dcf_cbc_movie_files() {
    assert_retained_file_fixture_decrypts_bytes(&oma_dcf_cbc_fixture());
}

#[test]
fn decrypt_bytes_supports_retained_oma_dcf_ctr_grouped_atom_files() {
    assert_retained_file_fixture_decrypts_bytes(&oma_dcf_ctr_grpi_fixture());
}

#[test]
fn decrypt_file_with_progress_supports_retained_oma_dcf_ctr_grouped_atom_files() {
    assert_retained_file_fixture_decrypts_with_progress(
        &oma_dcf_ctr_grpi_fixture(),
        "decrypt-api-oma-ctr-grpi-output",
    );
}

#[test]
fn decrypt_bytes_supports_retained_oma_dcf_cbc_grouped_atom_files() {
    assert_retained_file_fixture_decrypts_bytes(&oma_dcf_cbc_grpi_fixture());
}

#[test]
fn decrypt_file_with_progress_supports_retained_oma_dcf_cbc_grouped_atom_files() {
    assert_retained_file_fixture_decrypts_with_progress(
        &oma_dcf_cbc_grpi_fixture(),
        "decrypt-api-oma-cbc-grpi-output",
    );
}

#[test]
fn decrypt_file_with_progress_supports_retained_oma_dcf_cbc_movie_files() {
    assert_retained_file_fixture_decrypts_with_progress(
        &oma_dcf_cbc_fixture(),
        "decrypt-api-oma-cbc-output",
    );
}

#[test]
fn decrypt_bytes_supports_retained_isma_iaec_movie_files() {
    assert_retained_file_fixture_decrypts_bytes(&isma_iaec_fixture());
}

#[test]
fn decrypt_bytes_supports_broader_iaec_movie_layouts() {
    assert_generated_topology_fixture_decrypts_bytes(build_iaec_broader_movie_fixture());
}

#[test]
fn decrypt_file_with_progress_supports_broader_iaec_movie_layouts() {
    assert_generated_topology_fixture_decrypts_with_progress(
        build_iaec_broader_movie_fixture(),
        "decrypt-api-iaec-broader-input",
    );
}

#[test]
fn decrypt_file_with_progress_supports_retained_isma_iaec_movie_files() {
    assert_retained_file_fixture_decrypts_with_progress(
        &isma_iaec_fixture(),
        "decrypt-api-iaec-output",
    );
}

#[test]
fn decrypt_bytes_supports_retained_marlin_ipmp_acbc_movie_files() {
    assert_retained_file_fixture_decrypts_bytes(&marlin_ipmp_acbc_fixture());
}

#[test]
fn decrypt_file_with_progress_supports_retained_marlin_ipmp_acbc_movie_files() {
    assert_retained_file_fixture_decrypts_with_progress(
        &marlin_ipmp_acbc_fixture(),
        "decrypt-api-marlin-acbc-output",
    );
}

#[test]
fn decrypt_bytes_supports_broader_marlin_ipmp_acbc_movie_layouts() {
    assert_generated_topology_fixture_decrypts_bytes(build_marlin_ipmp_acbc_broader_movie_fixture());
}

#[test]
fn decrypt_file_with_progress_supports_broader_marlin_ipmp_acbc_movie_layouts() {
    assert_generated_topology_fixture_decrypts_with_progress(
        build_marlin_ipmp_acbc_broader_movie_fixture(),
        "decrypt-api-marlin-acbc-broader-input",
    );
}

#[test]
fn decrypt_bytes_supports_retained_marlin_ipmp_acgk_movie_files() {
    assert_retained_file_fixture_decrypts_bytes(&marlin_ipmp_acgk_fixture());
}

#[test]
fn decrypt_file_with_progress_supports_retained_marlin_ipmp_acgk_movie_files() {
    assert_retained_file_fixture_decrypts_with_progress(
        &marlin_ipmp_acgk_fixture(),
        "decrypt-api-marlin-acgk-output",
    );
}

#[test]
fn decrypt_bytes_supports_broader_marlin_ipmp_acgk_movie_layouts() {
    assert_generated_topology_fixture_decrypts_bytes(build_marlin_ipmp_acgk_broader_movie_fixture());
}

#[test]
fn decrypt_file_with_progress_supports_broader_marlin_ipmp_acgk_movie_layouts() {
    assert_generated_topology_fixture_decrypts_with_progress(
        build_marlin_ipmp_acgk_broader_movie_fixture(),
        "decrypt-api-marlin-acgk-broader-input",
    );
}

#[test]
fn decrypt_bytes_supports_retained_common_encryption_multi_track_files() {
    assert_retained_file_fixture_decrypts_bytes(&common_encryption_multi_track_fixture());
}

#[test]
fn decrypt_file_with_progress_supports_retained_common_encryption_multi_track_files() {
    assert_retained_file_fixture_decrypts_with_progress(
        &common_encryption_multi_track_fixture(),
        "decrypt-api-cenc-multi-track-output",
    );
}

#[test]
fn decrypt_bytes_supports_multi_sample_entry_fragmented_tracks() {
    let fixture = build_multi_sample_entry_decrypt_fixture();
    let output =
        decrypt_bytes(&fixture.single_file, &options_with_keys(&fixture.all_keys)).unwrap();
    assert_eq!(output, fixture.decrypted_single_file);
}

#[test]
fn decrypt_file_with_progress_supports_multi_sample_entry_fragmented_tracks() {
    let fixture = build_multi_sample_entry_decrypt_fixture();
    let input_path = write_temp_file("decrypt-api-multi-entry-input", &fixture.single_file);
    let output_path = write_temp_file("decrypt-api-multi-entry-output", &[]);

    decrypt_file_with_progress(
        &input_path,
        &output_path,
        &options_with_keys(&fixture.all_keys),
        |_| {},
    )
    .unwrap();

    let output = fs::read(&output_path).unwrap();
    assert_eq!(output, fixture.decrypted_single_file);
}

#[test]
fn decrypt_bytes_supports_zero_kid_multi_sample_entry_fragmented_tracks() {
    let fixture = build_zero_kid_multi_sample_entry_decrypt_fixture();
    let output = decrypt_bytes(
        &fixture.single_file,
        &options_with_keys(&fixture.ordered_track_id_keys),
    )
    .unwrap();
    assert_eq!(output, fixture.decrypted_single_file);
}

#[test]
fn decrypt_bytes_rejects_ambiguous_track_id_keys_for_multi_sample_entry_tracks() {
    let fixture = build_multi_sample_entry_decrypt_fixture();
    let error = decrypt_bytes(
        &fixture.single_file,
        &options_with_keys(&fixture.ambiguous_track_id_keys),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        DecryptError::Rewrite(DecryptRewriteError::InvalidLayout { .. })
    ));
}

common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cenc_single_video_media_segments,
    "cenc-single",
    "video"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cenc_single_audio_media_segments,
    "cenc-single",
    "audio"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cenc_multi_video_media_segments,
    "cenc-multi",
    "video"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cenc_multi_audio_media_segments,
    "cenc-multi",
    "audio"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cens_single_video_media_segments,
    "cens-single",
    "video"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cens_single_audio_media_segments,
    "cens-single",
    "audio"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cens_multi_video_media_segments,
    "cens-multi",
    "video"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cens_multi_audio_media_segments,
    "cens-multi",
    "audio"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cbc1_single_video_media_segments,
    "cbc1-single",
    "video"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cbc1_single_audio_media_segments,
    "cbc1-single",
    "audio"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cbc1_multi_video_media_segments,
    "cbc1-multi",
    "video"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cbc1_multi_audio_media_segments,
    "cbc1-multi",
    "audio"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cbcs_single_video_media_segments,
    "cbcs-single",
    "video"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cbcs_single_audio_media_segments,
    "cbcs-single",
    "audio"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cbcs_multi_video_media_segments,
    "cbcs-multi",
    "video"
);
common_encryption_fragment_bytes_case!(
    decrypt_bytes_supports_retained_cbcs_multi_audio_media_segments,
    "cbcs-multi",
    "audio"
);

#[test]
fn decrypt_with_missing_audio_key_does_not_fully_decrypt_retained_common_encryption_multi_track_file()
 {
    let fixture = common_encryption_multi_track_fixture();
    let input = fs::read(&fixture.encrypted_path).unwrap();
    let expected = fs::read(&fixture.decrypted_path).unwrap();

    let output = decrypt_bytes(
        &input,
        &options_with_keys(&common_encryption_single_key_fixture_keys()),
    )
    .unwrap();

    assert_ne!(output, expected);
}

fn options_with_keys(keys: &[DecryptionKey]) -> DecryptOptions {
    let mut options = DecryptOptions::new();
    for key in keys {
        options.add_key(*key);
    }
    options
}

fn phases(progress: &[DecryptProgress]) -> Vec<DecryptProgressPhase> {
    progress.iter().map(|snapshot| snapshot.phase).collect()
}

fn expected_file_progress_phases() -> Vec<DecryptProgressPhase> {
    vec![
        DecryptProgressPhase::OpenInput,
        DecryptProgressPhase::OpenInput,
        DecryptProgressPhase::InspectStructure,
        DecryptProgressPhase::InspectStructure,
        DecryptProgressPhase::ProcessSamples,
        DecryptProgressPhase::ProcessSamples,
        DecryptProgressPhase::OpenOutput,
        DecryptProgressPhase::OpenOutput,
        DecryptProgressPhase::FinalizeOutput,
        DecryptProgressPhase::FinalizeOutput,
    ]
}
