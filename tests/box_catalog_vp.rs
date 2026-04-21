use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::{SampleEntry, VisualSampleEntry};
use mp4forge::boxes::vp::VpCodecConfiguration;
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

fn visual_sample_entry(box_type: FourCc, compressorname: [u8; 32]) -> VisualSampleEntry {
    VisualSampleEntry {
        sample_entry: SampleEntry {
            box_type,
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
        compressorname,
        depth: 0x0105,
        pre_defined3: 1001,
    }
}

#[test]
fn vp_catalog_roundtrips() {
    assert_any_box_roundtrip(
        visual_sample_entry(
            FourCc::from_bytes(*b"vp08"),
            [
                8, b'v', b'p', b'8', b'.', b't', b'e', b's', b't', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        ),
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x01, 0x01, 0x00, 0x00, 0x02, 0x01, 0x00, 0x00, 0x03, 0x01, 0x02, 0x01, 0x03,
            0x01, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x05, 0x01, 0x00, 0x00, 0x06, 0x01, 0x04,
            0x08, b'v', b'p', b'8', b'.', b't', b'e', b's', b't', 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x01, 0x05, 0x03, 0xe9,
        ],
        "DataReferenceIndex=4660 PreDefined=257 PreDefined2=[16777217, 16777218, 16777219] Width=258 Height=259 Horizresolution=16777220 Vertresolution=16777221 FrameCount=260 Compressorname=\"vp8.test\" Depth=261 PreDefined3=1001",
    );

    assert_any_box_roundtrip(
        visual_sample_entry(
            FourCc::from_bytes(*b"vp09"),
            [
                8, b'v', b'p', b'9', b'.', b't', b'e', b's', b't', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        ),
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x01, 0x01, 0x00, 0x00, 0x02, 0x01, 0x00, 0x00, 0x03, 0x01, 0x02, 0x01, 0x03,
            0x01, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x05, 0x01, 0x00, 0x00, 0x06, 0x01, 0x04,
            0x08, b'v', b'p', b'9', b'.', b't', b'e', b's', b't', 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x01, 0x05, 0x03, 0xe9,
        ],
        "DataReferenceIndex=4660 PreDefined=257 PreDefined2=[16777217, 16777218, 16777219] Width=258 Height=259 Horizresolution=16777220 Vertresolution=16777221 FrameCount=260 Compressorname=\"vp9.test\" Depth=261 PreDefined3=1001",
    );

    let mut vpcc = VpCodecConfiguration::default();
    vpcc.set_version(1);
    vpcc.profile = 1;
    vpcc.level = 50;
    vpcc.bit_depth = 10;
    vpcc.chroma_subsampling = 3;
    vpcc.video_full_range_flag = 1;
    vpcc.colour_primaries = 0;
    vpcc.transfer_characteristics = 1;
    vpcc.matrix_coefficients = 10;
    vpcc.codec_initialization_data_size = 3;
    vpcc.codec_initialization_data = vec![5, 4, 3];

    assert_box_roundtrip(
        vpcc,
        &[
            0x01, 0x00, 0x00, 0x00, 0x01, 0x32, 0xa7, 0x00, 0x01, 0x0a, 0x00, 0x03, 0x05, 0x04,
            0x03,
        ],
        "Version=1 Flags=0x000000 Profile=0x1 Level=0x32 BitDepth=0xa ChromaSubsampling=0x3 VideoFullRangeFlag=0x1 ColourPrimaries=0x0 TransferCharacteristics=0x1 MatrixCoefficients=0xa CodecInitializationDataSize=3 CodecInitializationData=[0x5, 0x4, 0x3]",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_vp_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"vp08")),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"vp09")),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"vpcC")),
        Some(&[][..])
    );
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"vp08"), 9));
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"vp09"), 9));
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"vpcC"), 9));
    assert!(registry.is_registered(FourCc::from_bytes(*b"vp08")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"vp09")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"vpcC")));
}

#[test]
fn vpcc_rejects_codec_initialization_data_length_mismatch_during_marshal() {
    let mut vpcc = VpCodecConfiguration::default();
    vpcc.set_version(1);
    vpcc.codec_initialization_data_size = 4;
    vpcc.codec_initialization_data = vec![0x05, 0x04, 0x03];

    let error = marshal(&mut Vec::new(), &vpcc, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid element count for field CodecInitializationData: expected 4, got 3"
    );
}
