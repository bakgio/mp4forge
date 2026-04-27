use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::default_registry;
use mp4forge::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    DescriptorCommand, DescriptorUpdateCommand, ES_DESCRIPTOR_TAG, EsDescriptor, EsIdIncDescriptor,
    EsIdRefDescriptor, Esds, IPMP_DESCRIPTOR_UPDATE_COMMAND_TAG, InitialObjectDescriptor, Iods,
    IpmpDescriptor, IpmpDescriptorPointer, OBJECT_DESCRIPTOR_UPDATE_COMMAND_TAG,
    SL_CONFIG_DESCRIPTOR_TAG, UnknownDescriptorCommand, encode_descriptor_commands,
    parse_descriptor_commands,
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
fn iods_catalog_roundtrips() {
    let mut iods = Iods::default();
    iods.set_version(0);
    iods.descriptor = Some(
        Descriptor::from_initial_object_descriptor(InitialObjectDescriptor {
            object_descriptor_id: 18,
            include_inline_profile_level_flag: true,
            od_profile_level_indication: 0x11,
            scene_profile_level_indication: 0x22,
            audio_profile_level_indication: 0x33,
            visual_profile_level_indication: 0x44,
            graphics_profile_level_indication: 0x55,
            sub_descriptors: vec![
                Descriptor::from_es_id_inc_descriptor(EsIdIncDescriptor { track_id: 2 }),
                Descriptor::from_es_id_ref_descriptor(EsIdRefDescriptor { ref_index: 3 }),
                Descriptor::from_ipmp_descriptor_pointer(IpmpDescriptorPointer {
                    descriptor_id: 1,
                    ..IpmpDescriptorPointer::default()
                }),
                Descriptor::from_ipmp_descriptor(IpmpDescriptor {
                    descriptor_id: 1,
                    ipmps_type: 0xa551,
                    data: vec![0xaa, 0xbb],
                    ..IpmpDescriptor::default()
                }),
            ],
            ..InitialObjectDescriptor::default()
        })
        .unwrap(),
    );

    assert_box_roundtrip(
        iods,
        &[
            0x00, 0x00, 0x00, 0x00, 0x10, 0x80, 0x80, 0x80, 0x27, 0x04, 0x9f, 0x11, 0x22, 0x33,
            0x44, 0x55, 0x0e, 0x80, 0x80, 0x80, 0x04, 0x00, 0x00, 0x00, 0x02, 0x0f, 0x80, 0x80,
            0x80, 0x02, 0x00, 0x03, 0x0a, 0x80, 0x80, 0x80, 0x01, 0x01, 0x0b, 0x80, 0x80, 0x80,
            0x05, 0x01, 0xa5, 0x51, 0xaa, 0xbb,
        ],
        "Version=0 Flags=0x000000 Descriptor={Tag=MP4InitialObjectDescr Size=39 ObjectDescriptorID=18 UrlFlag=false IncludeInlineProfileLevelFlag=true ODProfileLevelIndication=0x11 SceneProfileLevelIndication=0x22 AudioProfileLevelIndication=0x33 VisualProfileLevelIndication=0x44 GraphicsProfileLevelIndication=0x55 SubDescriptors=[{Tag=ES_ID_Inc Size=4 TrackID=2}, {Tag=ES_ID_Ref Size=2 RefIndex=3}, {Tag=IPMPDescrPointer Size=1 DescriptorID=0x1}, {Tag=IPMPDescr Size=5 DescriptorID=0x1 IPMPSType=0xa551 Data=[0xaa, 0xbb]}]}",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_descriptor_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"esds")),
        Some(&[0][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"iods")),
        Some(&[0][..])
    );
    assert!(registry.is_registered(FourCc::from_bytes(*b"esds")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"iods")));
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
fn iods_helpers_surface_initial_object_descriptor() {
    let mut iods = Iods::default();
    iods.descriptor = Some(
        Descriptor::from_initial_object_descriptor(InitialObjectDescriptor {
            object_descriptor_id: 7,
            sub_descriptors: vec![Descriptor::from_es_id_inc_descriptor(EsIdIncDescriptor {
                track_id: 33,
            })],
            ..InitialObjectDescriptor::default()
        })
        .unwrap(),
    );

    let initial = iods.initial_object_descriptor().unwrap();
    assert_eq!(initial.object_descriptor_id, 7);
    assert_eq!(
        initial.sub_descriptors[0]
            .es_id_inc_descriptor()
            .unwrap()
            .track_id,
        33
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

#[test]
fn descriptor_command_helpers_roundtrip_known_update_streams() {
    let commands = vec![
        DescriptorCommand::DescriptorUpdate(DescriptorUpdateCommand::object_descriptor_update(
            vec![
                Descriptor::from_object_descriptor(
                    mp4forge::boxes::iso14496_14::ObjectDescriptor {
                        object_descriptor_id: 0x12,
                        sub_descriptors: vec![
                            Descriptor::from_es_id_ref_descriptor(EsIdRefDescriptor {
                                ref_index: 1,
                            }),
                            Descriptor::from_ipmp_descriptor_pointer(IpmpDescriptorPointer {
                                descriptor_id: 7,
                                ..IpmpDescriptorPointer::default()
                            }),
                        ],
                        ..mp4forge::boxes::iso14496_14::ObjectDescriptor::default()
                    },
                )
                .unwrap(),
            ],
        )),
        DescriptorCommand::DescriptorUpdate(DescriptorUpdateCommand::ipmp_descriptor_update(vec![
            Descriptor::from_ipmp_descriptor(IpmpDescriptor {
                descriptor_id: 7,
                ipmps_type: 0xa551,
                data: vec![0xaa, 0xbb, 0xcc],
                ..IpmpDescriptor::default()
            }),
        ])),
    ];

    let encoded = encode_descriptor_commands(&commands).unwrap();
    let decoded = parse_descriptor_commands(&encoded).unwrap();

    assert_eq!(decoded, commands);
    assert_eq!(decoded[0].tag(), OBJECT_DESCRIPTOR_UPDATE_COMMAND_TAG);
    assert_eq!(decoded[0].tag_name(), Some("ObjectDescriptorUpdate"));
    assert_eq!(
        decoded[0].descriptor_update().unwrap().descriptors[0]
            .object_descriptor()
            .unwrap()
            .sub_descriptors[0]
            .es_id_ref_descriptor()
            .unwrap()
            .ref_index,
        1
    );
    assert_eq!(decoded[1].tag(), IPMP_DESCRIPTOR_UPDATE_COMMAND_TAG);
    assert_eq!(decoded[1].tag_name(), Some("IPMPDescriptorUpdate"));
    assert_eq!(
        decoded[1].descriptor_update().unwrap().descriptors[0]
            .ipmp_descriptor()
            .unwrap()
            .ipmps_type,
        0xa551
    );
}

#[test]
fn descriptor_command_helpers_preserve_unknown_commands_as_raw_payloads() {
    let commands = vec![
        DescriptorCommand::Unknown(UnknownDescriptorCommand {
            tag: 0x08,
            data: vec![0x11, 0x22, 0x33, 0x44],
        }),
        DescriptorCommand::DescriptorUpdate(DescriptorUpdateCommand::ipmp_descriptor_update(vec![
            Descriptor::from_ipmp_descriptor(IpmpDescriptor {
                descriptor_id: 1,
                ipmps_type: 0xa551,
                data: vec![0x77],
                ..IpmpDescriptor::default()
            }),
        ])),
    ];

    let encoded = encode_descriptor_commands(&commands).unwrap();
    let decoded = parse_descriptor_commands(&encoded).unwrap();

    assert_eq!(decoded, commands);
    assert_eq!(decoded[0].tag(), 0x08);
    assert!(decoded[0].tag_name().is_none());
    assert_eq!(
        decoded[0].unknown().unwrap().data,
        vec![0x11, 0x22, 0x33, 0x44]
    );
}
