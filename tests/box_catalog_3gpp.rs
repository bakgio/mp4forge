use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::threegpp::Udta3gppString;
use mp4forge::boxes::{AnyTypeBox, default_registry};
use mp4forge::codec::{CodecError, ImmutableBox, marshal, unmarshal, unmarshal_any};
use mp4forge::stringify::stringify;

const TITL: FourCc = FourCc::from_bytes(*b"titl");
const DSCP: FourCc = FourCc::from_bytes(*b"dscp");
const CPRT: FourCc = FourCc::from_bytes(*b"cprt");
const PERF: FourCc = FourCc::from_bytes(*b"perf");
const AUTH: FourCc = FourCc::from_bytes(*b"auth");
const GNRE: FourCc = FourCc::from_bytes(*b"gnre");

fn sample_string(box_type: FourCc, data: &[u8]) -> Udta3gppString {
    let mut src = Udta3gppString::default();
    src.set_box_type(box_type);
    src.language = [0x05, 0x0e, 0x07];
    src.data = data.to_vec();
    src
}

fn assert_roundtrip(src: Udta3gppString, payload: &[u8], expected: &str) {
    let mut encoded = Vec::new();
    let written = marshal(&mut encoded, &src, None).unwrap();
    assert_eq!(written, payload.len() as u64);
    assert_eq!(encoded, payload);

    let mut decoded = Udta3gppString::default();
    decoded.set_box_type(src.box_type());
    let mut reader = Cursor::new(payload.to_vec());
    let read = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap();
    assert_eq!(read, payload.len() as u64);
    assert_eq!(decoded, src);

    assert_eq!(stringify(&src, None).unwrap(), expected);
}

#[test]
fn threegpp_catalog_roundtrips() {
    let payload = [0x00, 0x00, 0x00, 0x00, 0x15, 0xc7, 0x53, 0x49, 0x4e, 0x47];
    let expected = "Version=0 Flags=0x000000 Language=\"eng\" Data=\"SING\"";

    for box_type in [TITL, DSCP, CPRT, PERF, AUTH, GNRE] {
        assert_roundtrip(sample_string(box_type, b"SING"), &payload, expected);
    }

    let escaped_payload = [0x00, 0x00, 0x00, 0x00, 0x15, 0xc7, 0x00, 0x66, 0x6f, 0x6f];
    assert_roundtrip(
        sample_string(DSCP, &[0x00, b'f', b'o', b'o']),
        &escaped_payload,
        "Version=0 Flags=0x000000 Language=\"eng\" Data=\".foo\"",
    );
}

#[test]
fn built_in_registry_only_registers_flat_safe_threegpp_types() {
    let registry = default_registry();
    let payload = [0x00, 0x00, 0x00, 0x00, 0x15, 0xc7, 0x53, 0x49, 0x4e, 0x47];

    for box_type in [TITL, DSCP, PERF, AUTH] {
        assert!(registry.is_registered(box_type));
        assert_eq!(registry.supported_versions(box_type), Some(&[0_u8][..]));
        assert!(registry.is_supported_version(box_type, 0));
        assert!(!registry.is_supported_version(box_type, 1));

        let src = sample_string(box_type, b"SING");
        let mut reader = Cursor::new(payload.to_vec());
        let (decoded, read) =
            unmarshal_any(&mut reader, payload.len() as u64, box_type, &registry, None).unwrap();
        assert_eq!(read, payload.len() as u64);
        assert_eq!(
            decoded.as_any().downcast_ref::<Udta3gppString>().unwrap(),
            &src
        );
    }

    for box_type in [CPRT, GNRE] {
        assert!(!registry.is_registered(box_type));
        assert_eq!(registry.supported_versions(box_type), None);

        let mut reader = Cursor::new(Vec::<u8>::new());
        match unmarshal_any(&mut reader, 0, box_type, &registry, None) {
            Err(CodecError::UnknownBoxType { box_type: actual }) => assert_eq!(actual, box_type),
            Ok(_) => panic!("unexpected success for overlapping threegpp type {box_type}"),
            Err(other) => {
                panic!("unexpected error for overlapping threegpp type {box_type}: {other}")
            }
        }
    }
}
