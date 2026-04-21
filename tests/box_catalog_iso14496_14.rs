use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::default_registry;
use mp4forge::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    ES_DESCRIPTOR_TAG, EsDescriptor, Esds, SL_CONFIG_DESCRIPTOR_TAG,
};
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

#[test]
fn descriptor_catalog_roundtrips() {
    let mut esds = Esds::default();
    esds.set_version(0);
    esds.descriptors = vec![
        Descriptor {
            tag: ES_DESCRIPTOR_TAG,
            size: 0x1234567,
            es_descriptor: Some(EsDescriptor {
                es_id: 0x1234,
                stream_dependence_flag: true,
                ocr_stream_flag: true,
                stream_priority: 0x03,
                depends_on_es_id: 0x2345,
                ocr_es_id: 0x3456,
                ..EsDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: ES_DESCRIPTOR_TAG,
            size: 0x1234567,
            es_descriptor: Some(EsDescriptor {
                es_id: 0x1234,
                url_flag: true,
                stream_priority: 0x03,
                url_length: 11,
                url_string: b"http://hoge".to_vec(),
                ..EsDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_CONFIG_DESCRIPTOR_TAG,
            size: 0x1234567,
            decoder_config_descriptor: Some(DecoderConfigDescriptor {
                object_type_indication: 0x12,
                stream_type: 0x15,
                up_stream: true,
                reserved: false,
                buffer_size_db: 0x123456,
                max_bitrate: 0x12345678,
                avg_bitrate: 0x23456789,
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_SPECIFIC_INFO_TAG,
            size: 0x03,
            data: vec![0x11, 0x22, 0x33],
            ..Descriptor::default()
        },
        Descriptor {
            tag: SL_CONFIG_DESCRIPTOR_TAG,
            size: 0x05,
            data: vec![0x11, 0x22, 0x33, 0x44, 0x55],
            ..Descriptor::default()
        },
    ];

    assert_box_roundtrip(
        esds,
        &[
            0x00, 0x00, 0x00, 0x00, 0x03, 0x89, 0x8d, 0x8a, 0x67, 0x12, 0x34, 0xa3, 0x23, 0x45,
            0x34, 0x56, 0x03, 0x89, 0x8d, 0x8a, 0x67, 0x12, 0x34, 0x43, 0x0b, b'h', b't', b't',
            b'p', b':', b'/', b'/', b'h', b'o', b'g', b'e', 0x04, 0x89, 0x8d, 0x8a, 0x67, 0x12,
            0x56, 0x12, 0x34, 0x56, 0x12, 0x34, 0x56, 0x78, 0x23, 0x45, 0x67, 0x89, 0x05, 0x80,
            0x80, 0x80, 0x03, 0x11, 0x22, 0x33, 0x06, 0x80, 0x80, 0x80, 0x05, 0x11, 0x22, 0x33,
            0x44, 0x55,
        ],
        "Version=0 Flags=0x000000 Descriptors=[{Tag=ESDescr Size=19088743 ESID=4660 StreamDependenceFlag=true UrlFlag=false OcrStreamFlag=true StreamPriority=3 DependsOnESID=9029 OCRESID=13398}, {Tag=ESDescr Size=19088743 ESID=4660 StreamDependenceFlag=false UrlFlag=true OcrStreamFlag=false StreamPriority=3 URLLength=0xb URLString=\"http://hoge\"}, {Tag=DecoderConfigDescr Size=19088743 ObjectTypeIndication=0x12 StreamType=21 UpStream=true Reserved=false BufferSizeDB=1193046 MaxBitrate=305419896 AvgBitrate=591751049}, {Tag=DecSpecificInfo Size=3 Data=[0x11, 0x22, 0x33]}, {Tag=SLConfigDescr Size=5 Data=[0x11, 0x22, 0x33, 0x44, 0x55]}]",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_descriptor_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"esds")),
        Some(&[0][..])
    );
    assert!(registry.is_registered(FourCc::from_bytes(*b"esds")));
}

#[test]
fn esds_helpers_surface_decoder_config_and_specific_info() {
    let mut esds = Esds::default();
    esds.descriptors = vec![
        Descriptor {
            tag: DECODER_CONFIG_DESCRIPTOR_TAG,
            decoder_config_descriptor: Some(DecoderConfigDescriptor {
                object_type_indication: 0x40,
                ..DecoderConfigDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_SPECIFIC_INFO_TAG,
            size: 2,
            data: vec![0x10, 0x00],
            ..Descriptor::default()
        },
    ];

    assert_eq!(
        esds.decoder_config_descriptor()
            .map(|descriptor| descriptor.object_type_indication),
        Some(0x40)
    );
    assert_eq!(esds.decoder_specific_info(), Some(&[0x10, 0x00][..]));
    assert_eq!(
        esds.first_descriptor_with_tag(DECODER_SPECIFIC_INFO_TAG)
            .and_then(Descriptor::tag_name),
        Some("DecSpecificInfo")
    );
}

#[test]
fn esds_rejects_data_descriptor_size_mismatch_during_marshal() {
    let mut esds = Esds::default();
    esds.descriptors = vec![Descriptor {
        tag: DECODER_SPECIFIC_INFO_TAG,
        size: 4,
        data: vec![0x11, 0x22, 0x33],
        ..Descriptor::default()
    }];

    let error = marshal(&mut Vec::new(), &esds, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for Data: value length does not match Size"
    );
}
