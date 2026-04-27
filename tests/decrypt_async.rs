#![cfg(all(feature = "decrypt", feature = "async"))]

mod support;

use std::fs;
use std::io::Cursor;

use mp4forge::decrypt::{
    DecryptOptions, DecryptProgress, DecryptProgressPhase, DecryptionKey, decrypt_file_async,
    decrypt_file_with_progress, decrypt_file_with_progress_async,
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
    common_encryption_multi_track_fixture, fourcc, isma_iaec_fixture, marlin_ipmp_acbc_fixture,
    marlin_ipmp_acgk_fixture, oma_dcf_cbc_fixture, oma_dcf_cbc_grpi_fixture, oma_dcf_ctr_fixture,
    oma_dcf_ctr_grpi_fixture, piff_cbc_fixture, piff_cbc_segment_fixture, piff_ctr_fixture,
    piff_ctr_segment_fixture, write_temp_file,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_with_progress_matches_sync_output() {
    let fixture = build_decrypt_rewrite_fixture();
    let input_path = write_temp_file("decrypt-async-parity-input", &fixture.single_file);
    let sync_output_path = write_temp_file("decrypt-async-parity-sync-output", &[]);
    let async_output_path = write_temp_file("decrypt-async-parity-async-output", &[]);

    let options = options_with_keys(&fixture.all_keys);
    let mut sync_progress = Vec::new();
    decrypt_file_with_progress(&input_path, &sync_output_path, &options, |snapshot| {
        sync_progress.push(snapshot);
    })
    .unwrap();

    let mut async_progress = Vec::new();
    decrypt_file_with_progress_async(&input_path, &async_output_path, &options, |snapshot| {
        async_progress.push(snapshot);
    })
    .await
    .unwrap();

    assert_eq!(
        fs::read(sync_output_path).unwrap(),
        fs::read(async_output_path).unwrap()
    );
    assert_eq!(phases(&async_progress), phases(&sync_progress));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_helpers_can_run_on_tokio_worker_threads() {
    let fixture = build_decrypt_rewrite_fixture();
    let input_path = write_temp_file("decrypt-async-worker-input", &fixture.single_file);
    let output_path = write_temp_file("decrypt-async-worker-output", &[]);
    let options = options_with_keys(&fixture.all_keys);

    let output = tokio::spawn(async move {
        decrypt_file_async(&input_path, &output_path, &options)
            .await
            .unwrap();
        tokio::fs::read(output_path).await.unwrap()
    })
    .await
    .unwrap();

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
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn async_decrypt_independent_file_tasks_can_run_concurrently_on_tokio_worker_threads() {
    let fixture = build_decrypt_rewrite_fixture();

    let full_input = write_temp_file("decrypt-async-concurrent-full-input", &fixture.single_file);
    let full_output = write_temp_file("decrypt-async-concurrent-full-output", &[]);
    let partial_input = write_temp_file(
        "decrypt-async-concurrent-partial-input",
        &fixture.single_file,
    );
    let partial_output = write_temp_file("decrypt-async-concurrent-partial-output", &[]);
    let media_input = write_temp_file(
        "decrypt-async-concurrent-media-input",
        &fixture.media_segment,
    );
    let media_output = write_temp_file("decrypt-async-concurrent-media-output", &[]);

    let full_options = options_with_keys(&fixture.all_keys);
    let partial_options = options_with_keys(&fixture.first_track_only_keys);
    let media_options =
        options_with_keys(&fixture.all_keys).with_fragments_info_bytes(&fixture.init_segment);

    let full_handle = tokio::spawn(async move {
        decrypt_file_async(&full_input, &full_output, &full_options)
            .await
            .unwrap();
        tokio::fs::read(full_output).await.unwrap()
    });
    let partial_handle = tokio::spawn(async move {
        decrypt_file_async(&partial_input, &partial_output, &partial_options)
            .await
            .unwrap();
        tokio::fs::read(partial_output).await.unwrap()
    });
    let media_handle = tokio::spawn(async move {
        decrypt_file_async(&media_input, &media_output, &media_options)
            .await
            .unwrap();
        tokio::fs::read(media_output).await.unwrap()
    });

    let full_output = full_handle.await.unwrap();
    let partial_output = partial_handle.await.unwrap();
    let media_output = media_handle.await.unwrap();

    let full_tracks = probe_detailed(&mut Cursor::new(full_output))
        .unwrap()
        .tracks
        .into_iter()
        .map(|track| (track.summary.track_id, track))
        .collect::<std::collections::BTreeMap<_, _>>();
    for track_id in [fixture.first_track_id, fixture.second_track_id] {
        let track = full_tracks.get(&track_id).unwrap();
        assert!(!track.summary.encrypted);
        assert_eq!(track.sample_entry_type, Some(fourcc("avc1")));
    }

    let partial_tracks = probe_detailed(&mut Cursor::new(partial_output))
        .unwrap()
        .tracks
        .into_iter()
        .map(|track| (track.summary.track_id, track))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert!(
        !partial_tracks
            .get(&fixture.first_track_id)
            .unwrap()
            .summary
            .encrypted
    );
    assert!(
        partial_tracks
            .get(&fixture.second_track_id)
            .unwrap()
            .summary
            .encrypted
    );

    let mdat_payloads = extract_box_payload_bytes(
        &mut Cursor::new(media_output),
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
}

async fn assert_retained_file_fixture_decrypts_async(
    fixture: &RetainedDecryptFileFixture,
    temp_prefix: &str,
) {
    let output_path = write_temp_file(temp_prefix, &[]);
    let expected = fs::read(&fixture.decrypted_path).unwrap();

    decrypt_file_async(
        &fixture.encrypted_path,
        &output_path,
        &options_with_keys(&fixture.keys),
    )
    .await
    .unwrap();

    let output = fs::read(output_path).unwrap();
    assert_eq!(output, expected);
}

async fn assert_retained_fragmented_fixture_decrypts_async(
    fixture: &RetainedFragmentedDecryptFixture,
    temp_prefix: &str,
) {
    let output_path = write_temp_file(temp_prefix, &[]);
    let expected = fs::read(&fixture.clear_segment_path).unwrap();
    let fragments_info = fs::read(&fixture.fragments_info_path).unwrap();
    let options = options_with_keys(&fixture.keys).with_fragments_info_bytes(fragments_info);

    decrypt_file_async(&fixture.encrypted_segment_path, &output_path, &options)
        .await
        .unwrap();

    let output = fs::read(output_path).unwrap();
    assert_eq!(output, expected);
}

async fn assert_generated_topology_fixture_decrypts_async(
    fixture: ProtectedMovieTopologyFixture,
    temp_prefix: &str,
) {
    let input_path = write_temp_file(temp_prefix, &fixture.encrypted);
    let output_path = write_temp_file(&format!("{temp_prefix}-output"), &[]);

    decrypt_file_async(&input_path, &output_path, &options_with_keys(&fixture.keys))
        .await
        .unwrap();

    let output = fs::read(output_path).unwrap();
    assert_eq!(output, fixture.decrypted);
}

macro_rules! common_encryption_fragment_async_case {
    ($test_name:ident, $directory:literal, $track:literal, $prefix:literal) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $test_name() {
            let fixture = common_encryption_fragment_fixture($directory, $track);
            assert_retained_fragmented_fixture_decrypts_async(&fixture, $prefix).await;
        }
    };
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_retained_oma_dcf_ctr_movie_files() {
    assert_retained_file_fixture_decrypts_async(
        &oma_dcf_ctr_fixture(),
        "decrypt-async-oma-ctr-output",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_broader_oma_dcf_movie_layouts() {
    assert_generated_topology_fixture_decrypts_async(
        build_oma_dcf_broader_movie_fixture(),
        "decrypt-async-oma-broader-input",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_retained_piff_ctr_movie_files() {
    assert_retained_file_fixture_decrypts_async(
        &piff_ctr_fixture(),
        "decrypt-async-piff-ctr-output",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_retained_piff_cbc_movie_files() {
    assert_retained_file_fixture_decrypts_async(
        &piff_cbc_fixture(),
        "decrypt-async-piff-cbc-output",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_retained_piff_ctr_media_segments() {
    assert_retained_fragmented_fixture_decrypts_async(
        &piff_ctr_segment_fixture(),
        "decrypt-async-piff-ctr-segment-output",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_retained_piff_cbc_media_segments() {
    assert_retained_fragmented_fixture_decrypts_async(
        &piff_cbc_segment_fixture(),
        "decrypt-async-piff-cbc-segment-output",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_retained_oma_dcf_cbc_movie_files() {
    assert_retained_file_fixture_decrypts_async(
        &oma_dcf_cbc_fixture(),
        "decrypt-async-oma-cbc-output",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_retained_oma_dcf_ctr_grouped_atom_files() {
    assert_retained_file_fixture_decrypts_async(
        &oma_dcf_ctr_grpi_fixture(),
        "decrypt-async-oma-ctr-grpi-output",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_retained_oma_dcf_cbc_grouped_atom_files() {
    assert_retained_file_fixture_decrypts_async(
        &oma_dcf_cbc_grpi_fixture(),
        "decrypt-async-oma-cbc-grpi-output",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_retained_isma_iaec_movie_files() {
    assert_retained_file_fixture_decrypts_async(&isma_iaec_fixture(), "decrypt-async-iaec-output")
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_broader_iaec_movie_layouts() {
    assert_generated_topology_fixture_decrypts_async(
        build_iaec_broader_movie_fixture(),
        "decrypt-async-iaec-broader-input",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_retained_marlin_ipmp_acbc_movie_files() {
    assert_retained_file_fixture_decrypts_async(
        &marlin_ipmp_acbc_fixture(),
        "decrypt-async-marlin-acbc-output",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_broader_marlin_ipmp_acbc_movie_layouts() {
    assert_generated_topology_fixture_decrypts_async(
        build_marlin_ipmp_acbc_broader_movie_fixture(),
        "decrypt-async-marlin-acbc-broader-input",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_retained_marlin_ipmp_acgk_movie_files() {
    assert_retained_file_fixture_decrypts_async(
        &marlin_ipmp_acgk_fixture(),
        "decrypt-async-marlin-acgk-output",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_broader_marlin_ipmp_acgk_movie_layouts() {
    assert_generated_topology_fixture_decrypts_async(
        build_marlin_ipmp_acgk_broader_movie_fixture(),
        "decrypt-async-marlin-acgk-broader-input",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_retained_common_encryption_multi_track_files() {
    assert_retained_file_fixture_decrypts_async(
        &common_encryption_multi_track_fixture(),
        "decrypt-async-cenc-multi-track-output",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_multi_sample_entry_fragmented_tracks() {
    let fixture = build_multi_sample_entry_decrypt_fixture();
    let input_path = write_temp_file("decrypt-async-multi-entry-input", &fixture.single_file);
    let output_path = write_temp_file("decrypt-async-multi-entry-output", &[]);

    decrypt_file_async(
        &input_path,
        &output_path,
        &options_with_keys(&fixture.all_keys),
    )
    .await
    .unwrap();

    let output = fs::read(&output_path).unwrap();
    assert_eq!(output, fixture.decrypted_single_file);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_decrypt_file_supports_zero_kid_multi_sample_entry_fragmented_tracks() {
    let fixture = build_zero_kid_multi_sample_entry_decrypt_fixture();
    let input_path = write_temp_file(
        "decrypt-async-zero-kid-multi-entry-input",
        &fixture.single_file,
    );
    let output_path = write_temp_file("decrypt-async-zero-kid-multi-entry-output", &[]);

    decrypt_file_async(
        &input_path,
        &output_path,
        &options_with_keys(&fixture.ordered_track_id_keys),
    )
    .await
    .unwrap();

    let output = fs::read(&output_path).unwrap();
    assert_eq!(output, fixture.decrypted_single_file);
}

common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cenc_single_video_media_segments,
    "cenc-single",
    "video",
    "decrypt-async-cenc-single-video-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cenc_single_audio_media_segments,
    "cenc-single",
    "audio",
    "decrypt-async-cenc-single-audio-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cenc_multi_video_media_segments,
    "cenc-multi",
    "video",
    "decrypt-async-cenc-multi-video-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cenc_multi_audio_media_segments,
    "cenc-multi",
    "audio",
    "decrypt-async-cenc-multi-audio-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cens_single_video_media_segments,
    "cens-single",
    "video",
    "decrypt-async-cens-single-video-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cens_single_audio_media_segments,
    "cens-single",
    "audio",
    "decrypt-async-cens-single-audio-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cens_multi_video_media_segments,
    "cens-multi",
    "video",
    "decrypt-async-cens-multi-video-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cens_multi_audio_media_segments,
    "cens-multi",
    "audio",
    "decrypt-async-cens-multi-audio-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cbc1_single_video_media_segments,
    "cbc1-single",
    "video",
    "decrypt-async-cbc1-single-video-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cbc1_single_audio_media_segments,
    "cbc1-single",
    "audio",
    "decrypt-async-cbc1-single-audio-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cbc1_multi_video_media_segments,
    "cbc1-multi",
    "video",
    "decrypt-async-cbc1-multi-video-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cbc1_multi_audio_media_segments,
    "cbc1-multi",
    "audio",
    "decrypt-async-cbc1-multi-audio-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cbcs_single_video_media_segments,
    "cbcs-single",
    "video",
    "decrypt-async-cbcs-single-video-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cbcs_single_audio_media_segments,
    "cbcs-single",
    "audio",
    "decrypt-async-cbcs-single-audio-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cbcs_multi_video_media_segments,
    "cbcs-multi",
    "video",
    "decrypt-async-cbcs-multi-video-segment-output"
);
common_encryption_fragment_async_case!(
    async_decrypt_file_supports_retained_cbcs_multi_audio_media_segments,
    "cbcs-multi",
    "audio",
    "decrypt-async-cbcs-multi-audio-segment-output"
);

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
