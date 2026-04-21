use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::etsi_ts_102_366::Dac3;
use mp4forge::boxes::iso14496_12::{AudioSampleEntry, SampleEntry};
use mp4forge::boxes::{AnyTypeBox, default_registry};
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

fn assert_any_box_roundtrip<T>(src: T, payload: &[u8], expected: &str)
where
    T: CodecBox + AnyTypeBox + Default + PartialEq + Debug + 'static,
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
    decoded.set_box_type(src.box_type());
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
fn etsi_ts_102_366_catalog_roundtrips() {
    assert_any_box_roundtrip(
        AudioSampleEntry {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"ac-3"),
                data_reference_index: 1,
            },
            entry_version: 0,
            channel_count: 6,
            sample_size: 16,
            pre_defined: 0,
            sample_rate: 48_000 << 16,
            quicktime_data: Vec::new(),
        },
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x01, //
            0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x06, //
            0x00, 0x10, //
            0x00, 0x00, //
            0x00, 0x00, //
            0xbb, 0x80, 0x00, 0x00,
        ],
        "DataReferenceIndex=1 EntryVersion=0 ChannelCount=6 SampleSize=16 PreDefined=0 SampleRate=48000",
    );

    assert_box_roundtrip(
        Dac3 {
            fscod: 0,
            bsid: 8,
            bsmod: 0,
            acmod: 7,
            lfe_on: 1,
            bit_rate_code: 0x07,
        },
        &[0x10, 0x3c, 0xe0],
        "Fscod=0x0 Bsid=0x8 Bsmod=0x0 Acmod=0x7 LfeOn=0x1 BitRateCode=0x7",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_etsi_ts_102_366_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"ac-3")),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"dac3")),
        Some(&[][..])
    );
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"ac-3"), 9));
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"dac3"), 9));
    assert!(registry.is_registered(FourCc::from_bytes(*b"ac-3")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"dac3")));
}

#[test]
fn dac3_rejects_non_zero_reserved_bits_when_decoding() {
    let mut decoded = Dac3::default();
    let error = unmarshal(
        &mut Cursor::new(vec![0x10, 0x3c, 0xe1]),
        3,
        &mut decoded,
        None,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "constant mismatch for field Reserved: expected 0"
    );
}

#[test]
fn dac3_rejects_out_of_range_packed_field_values_when_encoding() {
    let dac3 = Dac3 {
        bsid: 0x20,
        ..Dac3::default()
    };

    let error = marshal(&mut Vec::new(), &dac3, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "numeric value does not fit field Bsid with width 5"
    );
}
