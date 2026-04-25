use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::flac::{DfLa, FlacMetadataBlock};
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
fn flac_catalog_roundtrips() {
    assert_any_box_roundtrip(
        AudioSampleEntry {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"fLaC"),
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

    let block_data: Vec<u8> = (1..=34).collect();
    let mut dfla = DfLa::default();
    dfla.metadata_blocks = vec![FlacMetadataBlock {
        last_metadata_block_flag: true,
        block_type: 0,
        length: 34,
        block_data: block_data.clone(),
    }];
    assert_box_roundtrip(
        dfla,
        &[
            0x00, 0x00, 0x00, 0x00, //
            0x80, 0x00, 0x00, 0x22, //
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
            0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x22,
        ],
        "Version=0 Flags=0x000000 MetadataBlocks=[{LastMetadataBlockFlag=true BlockType=0 Length=34 BlockData=[0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8, 0x9, 0xa, 0xb, 0xc, 0xd, 0xe, 0xf, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x22]}]",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_flac_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"fLaC")),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"dfLa")),
        Some(&[0][..])
    );
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"fLaC"), 9));
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"dfLa"), 0));
    assert!(!registry.is_supported_version(FourCc::from_bytes(*b"dfLa"), 1));
    assert!(registry.is_registered(FourCc::from_bytes(*b"fLaC")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"dfLa")));
}

#[test]
fn dfla_rejects_block_length_mismatch_during_marshal() {
    let mut dfla = DfLa::default();
    dfla.metadata_blocks = vec![FlacMetadataBlock {
        last_metadata_block_flag: true,
        block_type: 0,
        length: 34,
        block_data: vec![0x01, 0x02, 0x03],
    }];

    let error = marshal(&mut Vec::new(), &dfla, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for MetadataBlocks: block length does not match BlockData length"
    );
}

#[test]
fn dfla_rejects_missing_final_metadata_flag_during_unmarshal() {
    let mut decoded = DfLa::default();
    let error = unmarshal(
        &mut Cursor::new(vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        8,
        &mut decoded,
        None,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for MetadataBlocks: final metadata block flag must be set"
    );
}
