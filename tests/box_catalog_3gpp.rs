use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::threegpp::{D263, Damr, Devc, Dqcp, Dsmv, Udta3gppString};
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

#[test]
fn damr_roundtrips_and_is_registered() {
    let payload = [0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x81, 0x03, 0x01];
    let src = Damr {
        vendor: 0,
        decoder_version: 2,
        mode_set: 0x0081,
        mode_change_period: 3,
        frames_per_sample: 1,
    };
    let expected = "Vendor=0 DecoderVersion=2 ModeSet=0x81 ModeChangePeriod=3 FramesPerSample=1";

    let mut encoded = Vec::new();
    let written = marshal(&mut encoded, &src, None).unwrap();
    assert_eq!(written, payload.len() as u64);
    assert_eq!(encoded, payload);

    let mut decoded = Damr::default();
    let mut reader = Cursor::new(payload.to_vec());
    let read = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap();
    assert_eq!(read, payload.len() as u64);
    assert_eq!(decoded, src);

    let registry = default_registry();
    assert!(registry.is_registered(FourCc::from_bytes(*b"damr")));
    let mut any_reader = Cursor::new(payload.to_vec());
    let (any_box, any_read) = unmarshal_any(
        &mut any_reader,
        payload.len() as u64,
        FourCc::from_bytes(*b"damr"),
        &registry,
        None,
    )
    .unwrap();
    assert_eq!(any_read, payload.len() as u64);
    assert_eq!(any_box.as_any().downcast_ref::<Damr>().unwrap(), &src);
    assert_eq!(stringify(&src, None).unwrap(), expected);
}

fn assert_voice_decoder_config_roundtrip<T>(
    box_type: FourCc,
    src: T,
    payload: &[u8],
    expected: &str,
) where
    T: Default
        + PartialEq
        + std::fmt::Debug
        + ImmutableBox
        + mp4forge::codec::MutableBox
        + mp4forge::codec::FieldValueRead
        + mp4forge::codec::FieldValueWrite
        + mp4forge::codec::CodecBox
        + 'static,
{
    let mut encoded = Vec::new();
    let written = marshal(&mut encoded, &src, None).unwrap();
    assert_eq!(written, payload.len() as u64);
    assert_eq!(encoded, payload);

    let mut decoded = T::default();
    let mut reader = Cursor::new(payload.to_vec());
    let read = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap();
    assert_eq!(read, payload.len() as u64);
    assert_eq!(decoded, src);

    let registry = default_registry();
    assert!(registry.is_registered(box_type));
    let mut any_reader = Cursor::new(payload.to_vec());
    let (any_box, any_read) = unmarshal_any(
        &mut any_reader,
        payload.len() as u64,
        box_type,
        &registry,
        None,
    )
    .unwrap();
    assert_eq!(any_read, payload.len() as u64);
    assert_eq!(any_box.as_any().downcast_ref::<T>().unwrap(), &src);
    assert_eq!(stringify(&src, None).unwrap(), expected);
}

#[test]
fn voice_decoder_config_boxes_roundtrip_and_are_registered() {
    assert_voice_decoder_config_roundtrip(
        FourCc::from_bytes(*b"dqcp"),
        Dqcp {
            vendor: 0,
            decoder_version: 1,
            frames_per_sample: 1,
        },
        &[0, 0, 0, 0, 1, 1],
        "Vendor=0 DecoderVersion=1 FramesPerSample=1",
    );
    assert_voice_decoder_config_roundtrip(
        FourCc::from_bytes(*b"devc"),
        Devc {
            vendor: 0,
            decoder_version: 2,
            frames_per_sample: 3,
        },
        &[0, 0, 0, 0, 2, 3],
        "Vendor=0 DecoderVersion=2 FramesPerSample=3",
    );
    assert_voice_decoder_config_roundtrip(
        FourCc::from_bytes(*b"dsmv"),
        Dsmv {
            vendor: 0,
            decoder_version: 4,
            frames_per_sample: 1,
        },
        &[0, 0, 0, 0, 4, 1],
        "Vendor=0 DecoderVersion=4 FramesPerSample=1",
    );
}

#[test]
fn d263_roundtrips_and_is_registered() {
    let payload = [0, 0, 0, 0, 1, 10, 0];
    let src = D263 {
        vendor: 0,
        decoder_version: 1,
        h263_level: 10,
        h263_profile: 0,
    };
    assert_voice_decoder_config_roundtrip(
        FourCc::from_bytes(*b"d263"),
        src,
        &payload,
        "Vendor=0 DecoderVersion=1 H263Level=10 H263Profile=0",
    );
}
