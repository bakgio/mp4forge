#![cfg(feature = "decrypt")]

use mp4forge::FourCc;
use mp4forge::decrypt::{
    BROADER_MP4_DECRYPTION_FAMILIES, DecryptProgress, DecryptProgressPhase, DecryptionFormatFamily,
    DecryptionKey, DecryptionKeyId, FULL_MP4_DECRYPTION_FAMILIES,
    NATIVE_COMMON_ENCRYPTION_SCHEME_TYPES, ParseDecryptionKeyError,
};

#[test]
fn decrypt_feature_exposes_the_planned_support_matrix() {
    assert_eq!(
        NATIVE_COMMON_ENCRYPTION_SCHEME_TYPES,
        [
            FourCc::from_bytes(*b"cenc"),
            FourCc::from_bytes(*b"cens"),
            FourCc::from_bytes(*b"cbc1"),
            FourCc::from_bytes(*b"cbcs"),
        ]
    );

    assert_eq!(
        FULL_MP4_DECRYPTION_FAMILIES[0],
        DecryptionFormatFamily::CommonEncryption
    );
    assert_eq!(
        BROADER_MP4_DECRYPTION_FAMILIES,
        [
            DecryptionFormatFamily::OmaDcf,
            DecryptionFormatFamily::MarlinIpmp,
            DecryptionFormatFamily::PiffCompatibility,
            DecryptionFormatFamily::StandardProtected,
        ]
    );
}

#[test]
fn decrypt_feature_parses_track_and_kid_key_specs() {
    let track = DecryptionKey::from_spec("7:00112233445566778899aabbccddeeff").unwrap();
    assert_eq!(track.id(), DecryptionKeyId::TrackId(7));
    assert_eq!(
        track.key_bytes(),
        [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ]
    );
    assert_eq!(track.to_spec(), "7:00112233445566778899aabbccddeeff");

    let kid = DecryptionKey::from_spec(
        "00112233445566778899aabbccddeeff:ffeeddccbbaa99887766554433221100",
    )
    .unwrap();
    assert_eq!(
        kid.id(),
        DecryptionKeyId::Kid([
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ])
    );
    assert_eq!(
        kid.key_bytes(),
        [
            0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22,
            0x11, 0x00,
        ]
    );
    assert_eq!(
        kid.to_spec(),
        "00112233445566778899aabbccddeeff:ffeeddccbbaa99887766554433221100"
    );
}

#[test]
fn decrypt_feature_reports_key_parse_errors_clearly() {
    assert_eq!(
        DecryptionKey::from_spec("missing-separator").unwrap_err(),
        ParseDecryptionKeyError::InvalidSpec {
            input: "missing-separator".to_owned(),
            reason: "expected <id>:<key>",
        }
    );

    assert_eq!(
        DecryptionKey::from_spec("abc:00112233445566778899aabbccddeeff").unwrap_err(),
        ParseDecryptionKeyError::InvalidTrackId {
            input: "abc".to_owned(),
        }
    );

    assert_eq!(
        DecryptionKey::from_spec("1:001122").unwrap_err(),
        ParseDecryptionKeyError::InvalidHexLength {
            field: "content key",
            actual: 6,
        }
    );
}

#[test]
fn decrypt_feature_progress_type_is_stable() {
    let progress = DecryptProgress::new(DecryptProgressPhase::ProcessSamples, 3, Some(8));

    assert_eq!(progress.phase, DecryptProgressPhase::ProcessSamples);
    assert_eq!(progress.completed, 3);
    assert_eq!(progress.total, Some(8));
}
