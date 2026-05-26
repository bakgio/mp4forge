use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::dolby::Dmlp;
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
fn dolby_catalog_roundtrips() {
    assert_any_box_roundtrip(
        AudioSampleEntry {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"mlpa"),
                data_reference_index: 1,
            },
            entry_version: 0,
            channel_count: 2,
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
            0x00, 0x02, //
            0x00, 0x10, //
            0x00, 0x00, //
            0x00, 0x00, //
            0xbb, 0x80, 0x00, 0x00,
        ],
        "DataReferenceIndex=1 EntryVersion=0 ChannelCount=2 SampleSize=16 PreDefined=0 SampleRate=48000",
    );

    assert_box_roundtrip(
        Dmlp {
            format_info: 0x1234_5678,
            peak_data_rate: 0x2345,
        },
        &[
            0x12, 0x34, 0x56, 0x78, //
            0x23, 0x45, //
            0x00, 0x00, 0x00, 0x00,
        ],
        "FormatInfo=305419896 PeakDataRate=9029",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_dolby_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"mlpa")),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"dmlp")),
        Some(&[][..])
    );
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"mlpa"), 7));
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"dmlp"), 0));
    assert!(registry.is_registered(FourCc::from_bytes(*b"mlpa")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"dmlp")));
}

#[test]
fn dmlp_rejects_peak_data_rate_that_does_not_fit_in_15_bits() {
    let error = marshal(
        &mut Vec::new(),
        &Dmlp {
            format_info: 0,
            peak_data_rate: 0x8000,
        },
        None,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for PeakDataRate: value does not fit in 15 bits"
    );
}
