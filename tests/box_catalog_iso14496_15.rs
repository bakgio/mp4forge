use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::{SampleEntry, VisualSampleEntry};
use mp4forge::boxes::iso14496_15::VVCDecoderConfiguration;
use mp4forge::boxes::{AnyTypeBox, default_registry};
use mp4forge::codec::{CodecBox, MutableBox, marshal, unmarshal, unmarshal_any};
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
fn iso14496_15_catalog_roundtrips() {
    assert_any_box_roundtrip(
        VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"vvc1"),
                data_reference_index: 1,
            },
            width: 640,
            height: 360,
            horizresolution: 72 << 16,
            vertresolution: 72 << 16,
            frame_count: 1,
            depth: 0x0018,
            pre_defined3: -1,
            ..VisualSampleEntry::default()
        },
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x01, //
            0x00, 0x00, //
            0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, //
            0x02, 0x80, //
            0x01, 0x68, //
            0x00, 0x48, 0x00, 0x00, //
            0x00, 0x48, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, //
            0x00, 0x01, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x18, //
            0xff, 0xff,
        ],
        "DataReferenceIndex=1 PreDefined=0 PreDefined2=[0, 0, 0] Width=640 Height=360 Horizresolution=4718592 Vertresolution=4718592 FrameCount=1 Compressorname=\"\" Depth=24 PreDefined3=-1",
    );
    assert_any_box_roundtrip(
        VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"vvi1"),
                data_reference_index: 1,
            },
            width: 640,
            height: 360,
            horizresolution: 72 << 16,
            vertresolution: 72 << 16,
            frame_count: 1,
            depth: 0x0018,
            pre_defined3: -1,
            ..VisualSampleEntry::default()
        },
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x01, //
            0x00, 0x00, //
            0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, //
            0x02, 0x80, //
            0x01, 0x68, //
            0x00, 0x48, 0x00, 0x00, //
            0x00, 0x48, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, //
            0x00, 0x01, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x18, //
            0xff, 0xff,
        ],
        "DataReferenceIndex=1 PreDefined=0 PreDefined2=[0, 0, 0] Width=640 Height=360 Horizresolution=4718592 Vertresolution=4718592 FrameCount=1 Compressorname=\"\" Depth=24 PreDefined3=-1",
    );
    assert_box_roundtrip(
        {
            let mut vvcc = VVCDecoderConfiguration::default();
            vvcc.set_version(0);
            vvcc.decoder_configuration_record = vec![0x01, 0x23, 0x45, 0x67, 0x89];
            vvcc
        },
        &[0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89],
        "Version=0 Flags=0x000000 DecoderConfigurationRecord=[0x1, 0x23, 0x45, 0x67, 0x89]",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_iso14496_15_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"vvc1")),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"vvi1")),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"vvcC")),
        Some(&[0][..])
    );
    assert!(registry.is_registered(FourCc::from_bytes(*b"vvc1")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"vvi1")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"vvcC")));
}
