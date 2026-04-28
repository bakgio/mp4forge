use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::default_registry;
use mp4forge::boxes::isma_cryp::{Ikms, Isfm, Islt};
use mp4forge::codec::{CodecBox, MutableBox, marshal, unmarshal, unmarshal_any};

fn assert_box_roundtrip<T>(src: T, payload: &[u8])
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
}

#[test]
fn ikms_catalog_roundtrips_versions_zero_and_one() {
    let mut v0 = Ikms::default();
    v0.kms_uri = "https://kms.example/v0".into();
    v0.set_version(0);
    assert_box_roundtrip(
        v0,
        &[
            0x00, 0x00, 0x00, 0x00, b'h', b't', b't', b'p', b's', b':', b'/', b'/', b'k', b'm',
            b's', b'.', b'e', b'x', b'a', b'm', b'p', b'l', b'e', b'/', b'v', b'0', 0x00,
        ],
    );

    let mut v1 = Ikms::default();
    v1.kms_id = 0x6b6d_7331;
    v1.kms_version = 7;
    v1.kms_uri = "urn:keys:demo".into();
    v1.set_version(1);
    assert_box_roundtrip(
        v1,
        &[
            0x01, 0x00, 0x00, 0x00, 0x6b, 0x6d, 0x73, 0x31, 0x00, 0x00, 0x00, 0x07, b'u', b'r',
            b'n', b':', b'k', b'e', b'y', b's', b':', b'd', b'e', b'm', b'o', 0x00,
        ],
    );
}

#[test]
fn isfm_catalog_roundtrips() {
    let mut isfm = Isfm::default();
    isfm.selective_encryption = true;
    isfm.key_indicator_length = 4;
    isfm.iv_length = 8;
    isfm.set_version(0);
    isfm.set_flags(0x010203);

    assert_box_roundtrip(isfm, &[0x00, 0x01, 0x02, 0x03, 0x80, 0x04, 0x08]);
}

#[test]
fn islt_catalog_roundtrips() {
    assert_box_roundtrip(
        Islt {
            salt: [0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe],
        },
        &[0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe],
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_isma_boxes() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"iKMS")),
        Some(&[0, 1][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"iSFM")),
        Some(&[0][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"iSLT")),
        Some(&[][..])
    );
    assert!(registry.is_registered(FourCc::from_bytes(*b"iKMS")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"iSFM")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"iSLT")));
}

#[test]
fn ikms_rejects_embedded_null_bytes_during_marshal() {
    let mut ikms = Ikms::default();
    ikms.kms_uri = "bad\0uri".into();
    ikms.set_version(0);

    let error = marshal(&mut Vec::new(), &ikms, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for KmsUri: string value must not contain embedded null bytes"
    );
}

#[test]
fn isfm_rejects_unsupported_versions_during_unmarshal() {
    let mut isfm = Isfm::default();
    let payload = [0x01, 0x00, 0x00, 0x00, 0x80, 0x00, 0x08];

    let error = unmarshal(&mut Cursor::new(payload), 7, &mut isfm, None).unwrap_err();
    assert_eq!(error.to_string(), "unsupported box version 1 for type iSFM");
}

#[test]
fn islt_rejects_non_eight_byte_payloads() {
    let mut islt = Islt::default();
    let error = unmarshal(&mut Cursor::new(vec![0u8; 7]), 7, &mut islt, None).unwrap_err();
    assert_eq!(error.to_string(), "unexpected end of file");
}
