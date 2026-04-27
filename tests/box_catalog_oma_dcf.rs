use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::default_registry;
use mp4forge::boxes::oma_dcf::{
    Grpi, OHDR_ENCRYPTION_METHOD_AES_CTR, OHDR_PADDING_SCHEME_NONE, Odaf, Odda, Odhe, Odkm, Ohdr,
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
fn oma_dcf_catalog_roundtrips() {
    let mut odhe = Odhe::default();
    odhe.content_type = "video/mp4".into();
    assert_box_roundtrip(
        odhe,
        &[
            0x00, 0x00, 0x00, 0x00, 0x09, b'v', b'i', b'd', b'e', b'o', b'/', b'm', b'p', b'4',
        ],
        "Version=0 Flags=0x000000 ContentType=\"video/mp4\"",
    );

    let mut ohdr = Ohdr::default();
    ohdr.encryption_method = OHDR_ENCRYPTION_METHOD_AES_CTR;
    ohdr.padding_scheme = OHDR_PADDING_SCHEME_NONE;
    ohdr.plaintext_length = 0x0102_0304_0506_0708;
    ohdr.content_id = "cid-7".into();
    ohdr.rights_issuer_url = "https://issuer.example".into();
    ohdr.textual_headers = b"Header-One: a\0Header-Two: b".to_vec();
    assert_box_roundtrip(
        ohdr,
        &[
            0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
            0x00, 0x05, 0x00, 0x16, 0x00, 0x1b, b'c', b'i', b'd', b'-', b'7', b'h', b't', b't',
            b'p', b's', b':', b'/', b'/', b'i', b's', b's', b'u', b'e', b'r', b'.', b'e', b'x',
            b'a', b'm', b'p', b'l', b'e', b'H', b'e', b'a', b'd', b'e', b'r', b'-', b'O', b'n',
            b'e', b':', b' ', b'a', 0x00, b'H', b'e', b'a', b'd', b'e', b'r', b'-', b'T', b'w',
            b'o', b':', b' ', b'b',
        ],
        "Version=0 Flags=0x000000 EncryptionMethod=2 PaddingScheme=0 PlaintextLength=72623859790382856 ContentId=\"cid-7\" RightsIssuerUrl=\"https://issuer.example\" TextualHeaders=[0x48, 0x65, 0x61, 0x64, 0x65, 0x72, 0x2d, 0x4f, 0x6e, 0x65, 0x3a, 0x20, 0x61, 0x0, 0x48, 0x65, 0x61, 0x64, 0x65, 0x72, 0x2d, 0x54, 0x77, 0x6f, 0x3a, 0x20, 0x62]",
    );

    let mut odaf = Odaf::default();
    odaf.selective_encryption = true;
    odaf.key_indicator_length = 0;
    odaf.iv_length = 16;
    assert_box_roundtrip(
        odaf,
        &[0x00, 0x00, 0x00, 0x00, 0x80, 0x00, 0x10],
        "Version=0 Flags=0x000000 SelectiveEncryption=true KeyIndicatorLength=0 IvLength=16",
    );

    assert_box_roundtrip(
        Odkm::default(),
        &[0x00, 0x00, 0x00, 0x00],
        "Version=0 Flags=0x000000",
    );

    let mut odda = Odda::default();
    odda.encrypted_payload = vec![0xde, 0xad, 0xbe, 0xef];
    assert_box_roundtrip(
        odda,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0xde, 0xad,
            0xbe, 0xef,
        ],
        "Version=0 Flags=0x000000 EncryptedPayload=[0xde, 0xad, 0xbe, 0xef]",
    );

    let mut grpi = Grpi::default();
    grpi.key_encryption_method = 1;
    grpi.group_id = "group-a".into();
    grpi.group_key = vec![0x00, 0x11, 0x22, 0x33];
    assert_box_roundtrip(
        grpi,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x01, 0x00, 0x04, b'g', b'r', b'o', b'u', b'p',
            b'-', b'a', 0x00, 0x11, 0x22, 0x33,
        ],
        "Version=0 Flags=0x000000 KeyEncryptionMethod=1 GroupId=\"group-a\" GroupKey=[0x0, 0x11, 0x22, 0x33]",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_oma_dcf_types() {
    let registry = default_registry();

    for box_type in [
        FourCc::from_bytes(*b"odrm"),
        FourCc::from_bytes(*b"odkm"),
        FourCc::from_bytes(*b"odhe"),
        FourCc::from_bytes(*b"ohdr"),
        FourCc::from_bytes(*b"odaf"),
        FourCc::from_bytes(*b"odda"),
        FourCc::from_bytes(*b"grpi"),
    ] {
        assert!(
            registry.is_registered(box_type),
            "missing registry entry for {box_type}"
        );
    }

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"odhe")),
        Some(&[0][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"ohdr")),
        Some(&[0][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"odaf")),
        Some(&[0][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"odkm")),
        Some(&[0][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"odda")),
        Some(&[0][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"grpi")),
        Some(&[0][..])
    );
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"odhe"), 0));
    assert!(!registry.is_supported_version(FourCc::from_bytes(*b"odhe"), 1));
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"odrm"), 9));
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"odkm"), 0));
    assert!(!registry.is_supported_version(FourCc::from_bytes(*b"odkm"), 1));
}
