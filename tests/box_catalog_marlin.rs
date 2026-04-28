use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::default_registry;
use mp4forge::boxes::marlin::{
    Gkey, Hmac, MARLIN_BRAND_MGSV, MARLIN_IPMPS_TYPE_MGSV, MARLIN_STYP_AUDIO, MarlinShortSchm,
    MarlinStyp, PROTECTION_SCHEME_TYPE_MARLIN_ACBC, PROTECTION_SCHEME_TYPE_MARLIN_ACGK, Satr,
};
use mp4forge::codec::{CodecBox, marshal, unmarshal, unmarshal_any};
use mp4forge::stringify::stringify;

fn assert_box_roundtrip<T>(src: T, payload: &[u8], expected: &str)
where
    T: CodecBox + Default + PartialEq + Debug + 'static,
{
    let mut encoded = Vec::new();
    let written = marshal(&mut encoded, &src, None).unwrap();
    assert_eq!(
        written,
        payload.len() as u64,
        "marshal length for {}",
        type_name::<T>()
    );
    assert_eq!(encoded, payload, "marshal bytes for {}", type_name::<T>());

    let mut decoded = T::default();
    let mut reader = Cursor::new(payload.to_vec());
    let read = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap();
    assert_eq!(
        read,
        payload.len() as u64,
        "unmarshal length for {}",
        type_name::<T>()
    );
    assert_eq!(decoded, src, "unmarshal value for {}", type_name::<T>());

    let registry = default_registry();
    let mut any_reader = Cursor::new(payload.to_vec());
    let (any_box, any_read) = unmarshal_any(
        &mut any_reader,
        payload.len() as u64,
        src.box_type(),
        &registry,
        None,
    )
    .unwrap();
    assert_eq!(
        any_read,
        payload.len() as u64,
        "registry unmarshal length for {}",
        type_name::<T>()
    );
    assert_eq!(any_box.as_any().downcast_ref::<T>().unwrap(), &src);

    assert_eq!(stringify(&src, None).unwrap(), expected);
}

#[test]
fn marlin_catalog_roundtrips_unique_atoms() {
    assert_box_roundtrip(Satr, &[], "");

    let hmac = Hmac {
        data: vec![0xde, 0xad, 0xbe, 0xef],
    };
    assert_box_roundtrip(
        hmac,
        &[0xde, 0xad, 0xbe, 0xef],
        "Data=[0xde, 0xad, 0xbe, 0xef]",
    );

    let gkey = Gkey {
        data: vec![0x10, 0x20, 0x30, 0x40],
    };
    assert_box_roundtrip(
        gkey,
        &[0x10, 0x20, 0x30, 0x40],
        "Data=[0x10, 0x20, 0x30, 0x40]",
    );
}

#[test]
fn marlin_helper_payloads_roundtrip() {
    assert_eq!(MARLIN_BRAND_MGSV, FourCc::from_bytes(*b"MGSV"));
    assert_eq!(MARLIN_IPMPS_TYPE_MGSV, 0xA551);

    let styp = MarlinStyp {
        value: MARLIN_STYP_AUDIO.into(),
    };
    let styp_payload = styp.encode_payload().unwrap();
    assert_eq!(MarlinStyp::parse_payload(&styp_payload).unwrap(), styp);

    let forced = MarlinStyp::parse_payload(b"video").unwrap();
    assert_eq!(forced.value, "vide");

    let track_key = MarlinShortSchm {
        scheme_type: PROTECTION_SCHEME_TYPE_MARLIN_ACBC,
        scheme_version: 0x0100,
    };
    let group_key = MarlinShortSchm {
        scheme_type: PROTECTION_SCHEME_TYPE_MARLIN_ACGK,
        scheme_version: 0x0100,
    };

    assert_eq!(
        MarlinShortSchm::parse_payload(&track_key.encode_payload()).unwrap(),
        track_key
    );
    assert!(track_key.uses_track_key());
    assert!(!track_key.uses_group_key());

    assert_eq!(
        MarlinShortSchm::parse_payload(&group_key.encode_payload()).unwrap(),
        group_key
    );
    assert!(!group_key.uses_track_key());
    assert!(group_key.uses_group_key());
}

#[test]
fn marlin_helpers_reject_invalid_payloads() {
    let error = MarlinShortSchm::parse_payload(&[0, 1, 2, 3, 4]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for Payload: expected a 6-byte Marlin short-form schm payload"
    );

    let error = MarlinStyp {
        value: "bad\0value".into(),
    }
    .encode_payload()
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for Value: string contains an embedded NUL"
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_marlin_types() {
    let registry = default_registry();

    for box_type in [
        FourCc::from_bytes(*b"satr"),
        FourCc::from_bytes(*b"hmac"),
        FourCc::from_bytes(*b"gkey"),
    ] {
        assert!(
            registry.is_registered(box_type),
            "missing registry entry for {box_type}"
        );
        assert!(
            registry.supported_versions(box_type) == Some(&[][..]),
            "unexpected supported-version table for {box_type}"
        );
    }
}
