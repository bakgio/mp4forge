#![cfg(feature = "decrypt")]

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;

use mp4forge::boxes::iso14496_12::Trun;
use mp4forge::decrypt::{
    DecryptRewriteError, decrypt_common_encryption_file_bytes,
    decrypt_common_encryption_init_bytes, decrypt_common_encryption_media_segment_bytes,
};
use mp4forge::extract::{extract_box, extract_box_payload_bytes};
use mp4forge::probe::probe_detailed;
use mp4forge::rewrite::rewrite_box_as_bytes;
use mp4forge::walk::BoxPath;

use support::{
    RetainedDecryptFileFixture, build_decrypt_rewrite_fixture, fourcc, piff_cbc_fixture,
    piff_cbc_segment_fixture, piff_ctr_fixture, piff_ctr_segment_fixture,
};

#[test]
fn decrypt_common_encryption_init_bytes_clears_keyed_sample_entry_protection_state() {
    let fixture = build_decrypt_rewrite_fixture();

    let output =
        decrypt_common_encryption_init_bytes(&fixture.init_segment, &fixture.all_keys).unwrap();

    assert!(
        extract_box(
            &mut Cursor::new(output.clone()),
            None,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stsd"),
                fourcc("encv")
            ]),
        )
        .unwrap()
        .is_empty()
    );
    assert_eq!(
        extract_box(
            &mut Cursor::new(output.clone()),
            None,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stsd"),
                fourcc("avc1")
            ]),
        )
        .unwrap()
        .len(),
        2
    );
    assert!(
        extract_box(
            &mut Cursor::new(output),
            None,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stsd"),
                fourcc("avc1"),
                fourcc("sinf")
            ]),
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn decrypt_common_encryption_media_segment_bytes_decrypts_samples_and_removes_fragment_boxes() {
    let fixture = build_decrypt_rewrite_fixture();

    let output = decrypt_common_encryption_media_segment_bytes(
        &fixture.init_segment,
        &fixture.media_segment,
        &fixture.all_keys,
    )
    .unwrap();

    let mdat_payloads = extract_box_payload_bytes(
        &mut Cursor::new(output.clone()),
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

    for path in [
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("senc")]),
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("saiz")]),
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("saio")]),
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sgpd")]),
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sbgp")]),
    ] {
        assert!(
            extract_box(&mut Cursor::new(output.clone()), None, path)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn decrypt_common_encryption_media_segment_bytes_supports_piff_uuid_sample_encryption() {
    for fixture in [piff_ctr_segment_fixture(), piff_cbc_segment_fixture()] {
        let init_segment = fs::read(&fixture.fragments_info_path).unwrap();
        let encrypted_media_segment = fs::read(&fixture.encrypted_segment_path).unwrap();
        let clear_media_segment = fs::read(&fixture.clear_segment_path).unwrap();
        let output = decrypt_common_encryption_media_segment_bytes(
            &init_segment,
            &encrypted_media_segment,
            &fixture.keys,
        )
        .unwrap();

        assert_eq!(output, clear_media_segment);
        assert_eq!(
            extract_box(
                &mut Cursor::new(output),
                None,
                BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("uuid")]),
            )
            .unwrap()
            .len(),
            1
        );
    }
}

#[test]
fn decrypt_common_encryption_file_bytes_matches_split_outputs() {
    let fixture = build_decrypt_rewrite_fixture();

    let expected = [
        decrypt_common_encryption_init_bytes(&fixture.init_segment, &fixture.all_keys).unwrap(),
        decrypt_common_encryption_media_segment_bytes(
            &fixture.init_segment,
            &fixture.media_segment,
            &fixture.all_keys,
        )
        .unwrap(),
    ]
    .concat();
    let actual =
        decrypt_common_encryption_file_bytes(&fixture.single_file, &fixture.all_keys).unwrap();

    assert_eq!(actual, expected);

    let detailed = probe_detailed(&mut Cursor::new(actual)).unwrap();
    let tracks = detailed
        .tracks
        .into_iter()
        .map(|track| (track.summary.track_id, track))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(tracks.len(), 2);
    for track_id in [fixture.first_track_id, fixture.second_track_id] {
        let track = tracks.get(&track_id).unwrap();
        assert!(!track.summary.encrypted);
        assert_eq!(track.sample_entry_type, Some(fourcc("avc1")));
        assert!(track.original_format.is_none());
        assert!(track.protection_scheme.is_none());
    }
}

#[test]
fn decrypt_common_encryption_file_bytes_supports_piff_compatibility_tracks() {
    for fixture in [piff_ctr_fixture(), piff_cbc_fixture()] {
        assert_retained_piff_file_fixture_decrypts(&fixture);
    }
}

#[test]
fn decrypt_common_encryption_file_bytes_keeps_unkeyed_track_encrypted() {
    let fixture = build_decrypt_rewrite_fixture();

    let output =
        decrypt_common_encryption_file_bytes(&fixture.single_file, &fixture.first_track_only_keys)
            .unwrap();

    let detailed = probe_detailed(&mut Cursor::new(output.clone())).unwrap();
    let tracks = detailed
        .tracks
        .into_iter()
        .map(|track| (track.summary.track_id, track))
        .collect::<BTreeMap<_, _>>();

    let first = tracks.get(&fixture.first_track_id).unwrap();
    assert!(!first.summary.encrypted);
    assert_eq!(first.sample_entry_type, Some(fourcc("avc1")));
    assert!(first.protection_scheme.is_none());

    let second = tracks.get(&fixture.second_track_id).unwrap();
    assert!(second.summary.encrypted);
    assert_eq!(second.sample_entry_type, Some(fourcc("encv")));
    assert_eq!(second.original_format, Some(fourcc("avc1")));

    assert_eq!(
        extract_box(
            &mut Cursor::new(output),
            None,
            BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("senc")]),
        )
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn decrypt_common_encryption_media_segment_bytes_rejects_invalid_trun_offsets() {
    let fixture = build_decrypt_rewrite_fixture();
    let broken = rewrite_box_as_bytes::<Trun, _>(
        &fixture.media_segment,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("trun")]),
        |trun| {
            trun.data_offset = i32::MAX;
        },
    )
    .unwrap();

    let error = decrypt_common_encryption_media_segment_bytes(
        &fixture.init_segment,
        &broken,
        &fixture.all_keys,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        DecryptRewriteError::SampleDataRangeNotFound { .. }
            | DecryptRewriteError::InvalidLayout { .. }
    ));
}

fn assert_retained_piff_file_fixture_decrypts(fixture: &RetainedDecryptFileFixture) {
    let input = fs::read(&fixture.encrypted_path).unwrap();
    let expected = fs::read(&fixture.decrypted_path).unwrap();

    let output = decrypt_common_encryption_file_bytes(&input, &fixture.keys).unwrap();

    assert_eq!(output, expected);
    assert!(
        extract_box(
            &mut Cursor::new(output.clone()),
            None,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stsd"),
                fourcc("avc1"),
            ]),
        )
        .unwrap()
        .is_empty()
    );
    assert_eq!(
        extract_box(
            &mut Cursor::new(output.clone()),
            None,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stsd"),
                fourcc("encv"),
            ]),
        )
        .unwrap()
        .len(),
        1
    );
    assert_eq!(
        extract_box(
            &mut Cursor::new(output.clone()),
            None,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stsd"),
                fourcc("encv"),
                fourcc("sinf"),
            ]),
        )
        .unwrap()
        .len(),
        1
    );
    assert_eq!(
        extract_box(
            &mut Cursor::new(output),
            None,
            BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("uuid")]),
        )
        .unwrap()
        .len(),
        1
    );
}
