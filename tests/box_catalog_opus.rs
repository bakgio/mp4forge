use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::{AudioSampleEntry, SampleEntry};
use mp4forge::boxes::opus::DOps;
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
fn opus_catalog_roundtrips() {
    assert_any_box_roundtrip(
        AudioSampleEntry {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"Opus"),
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
        DOps {
            version: 0,
            output_channel_count: 2,
            pre_skip: 312,
            input_sample_rate: 48_000,
            output_gain: 0,
            channel_mapping_family: 2,
            stream_count: 1,
            coupled_count: 1,
            channel_mapping: vec![1, 2],
        },
        &[
            0x00, //
            0x02, //
            0x01, 0x38, //
            0x00, 0x00, 0xbb, 0x80, //
            0x00, 0x00, //
            0x02, //
            0x01, //
            0x01, //
            0x01, 0x02,
        ],
        "Version=0 OutputChannelCount=0x2 PreSkip=312 InputSampleRate=48000 OutputGain=0 ChannelMappingFamily=0x2 StreamCount=0x1 CoupledCount=0x1 ChannelMapping=[0x1, 0x2]",
    );
}

#[test]
fn dops_omits_channel_mapping_fields_when_family_is_zero() {
    assert_box_roundtrip(
        DOps {
            version: 0,
            output_channel_count: 2,
            pre_skip: 312,
            input_sample_rate: 48_000,
            output_gain: -15,
            channel_mapping_family: 0,
            stream_count: 0,
            coupled_count: 0,
            channel_mapping: Vec::new(),
        },
        &[
            0x00, //
            0x02, //
            0x01, 0x38, //
            0x00, 0x00, 0xbb, 0x80, //
            0xff, 0xf1, //
            0x00,
        ],
        "Version=0 OutputChannelCount=0x2 PreSkip=312 InputSampleRate=48000 OutputGain=-15 ChannelMappingFamily=0x0",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_opus_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"Opus")),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"dOps")),
        Some(&[][..])
    );
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"Opus"), 9));
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"dOps"), 9));
    assert!(registry.is_registered(FourCc::from_bytes(*b"Opus")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"dOps")));
}

#[test]
fn dops_rejects_channel_mapping_length_mismatch_when_family_is_non_zero() {
    let dops = DOps {
        version: 0,
        output_channel_count: 2,
        pre_skip: 312,
        input_sample_rate: 48_000,
        output_gain: 0,
        channel_mapping_family: 1,
        stream_count: 1,
        coupled_count: 1,
        channel_mapping: vec![1],
    };

    let error = marshal(&mut Vec::new(), &dops, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid element count for field ChannelMapping: expected 2, got 1"
    );
}
