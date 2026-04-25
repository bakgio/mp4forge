use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::avs3::Av3c;
use mp4forge::boxes::iso14496_12::{SampleEntry, VisualSampleEntry};
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
fn avs3_catalog_roundtrips() {
    assert_any_box_roundtrip(
        VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"avs3"),
                data_reference_index: 0x1234,
            },
            pre_defined: 0x0101,
            pre_defined2: [0x01000001, 0x01000002, 0x01000003],
            width: 0x0102,
            height: 0x0103,
            horizresolution: 0x01000004,
            vertresolution: 0x01000005,
            reserved2: 0x01000006,
            frame_count: 0x0104,
            compressorname: [
                8, b'a', b'v', b's', b'3', b'.', b't', b'e', b's', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
            depth: 0x0105,
            pre_defined3: 1001,
        },
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x01, 0x01, 0x00, 0x00, 0x02, 0x01, 0x00, 0x00, 0x03, 0x01, 0x02, 0x01, 0x03,
            0x01, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x05, 0x01, 0x00, 0x00, 0x06, 0x01, 0x04,
            0x08, b'a', b'v', b's', b'3', b'.', b't', b'e', b's', 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x01, 0x05, 0x03, 0xe9,
        ],
        "DataReferenceIndex=4660 PreDefined=257 PreDefined2=[16777217, 16777218, 16777219] Width=258 Height=259 Horizresolution=16777220 Vertresolution=16777221 FrameCount=260 Compressorname=\"avs3.tes\" Depth=261 PreDefined3=1001",
    );

    assert_box_roundtrip(
        Av3c {
            configuration_version: 1,
            sequence_header_length: 4,
            sequence_header: vec![0x01, 0x02, 0x03, 0x04],
            library_dependency_idc: 2,
        },
        &[0x01, 0x00, 0x04, 0x01, 0x02, 0x03, 0x04, 0xfe],
        "ConfigurationVersion=1 SequenceHeaderLength=4 SequenceHeader=[0x1, 0x2, 0x3, 0x4] LibraryDependencyIDC=0x2",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_avs3_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"avs3")),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"av3c")),
        Some(&[][..])
    );
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"avs3"), 9));
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"av3c"), 9));
    assert!(registry.is_registered(FourCc::from_bytes(*b"avs3")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"av3c")));
}

#[test]
fn av3c_rejects_sequence_header_length_mismatch_during_marshal() {
    let av3c = Av3c {
        configuration_version: 1,
        sequence_header_length: 5,
        sequence_header: vec![0x01, 0x02, 0x03, 0x04],
        library_dependency_idc: 2,
    };

    let error = marshal(&mut Vec::new(), &av3c, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for SequenceHeader: length does not match SequenceHeaderLength"
    );
}

#[test]
fn av3c_rejects_reserved_bit_mismatches_during_unmarshal() {
    let mut decoded = Av3c::default();
    let error = unmarshal(
        &mut Cursor::new(vec![0x01, 0x00, 0x00, 0x02]),
        4,
        &mut decoded,
        None,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "constant mismatch for field Reserved: expected 63"
    );
}
