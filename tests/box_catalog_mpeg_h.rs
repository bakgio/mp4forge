use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::{AudioSampleEntry, SampleEntry};
use mp4forge::boxes::mpeg_h::MhaC;
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
fn mpeg_h_catalog_roundtrips() {
    assert_any_box_roundtrip(
        AudioSampleEntry {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"mha1"),
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
        MhaC {
            config_version: 1,
            mpeg_h_3da_profile_level_indication: 12,
            reference_channel_layout: 6,
            mpeg_h_3da_config_length: 4,
            mpeg_h_3da_config: vec![0x01, 0x02, 0x03, 0x04],
        },
        &[0x01, 0x0c, 0x06, 0x00, 0x04, 0x01, 0x02, 0x03, 0x04],
        "ConfigVersion=1 MpegH3DAProfileLevelIndication=12 ReferenceChannelLayout=6 MpegH3DAConfigLength=4 MpegH3DAConfig=[0x1, 0x2, 0x3, 0x4]",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_mpeg_h_types() {
    let registry = default_registry();

    for box_type in ["mha1", "mha2", "mhm1", "mhm2", "mhaC"] {
        let fourcc = FourCc::from_bytes(box_type.as_bytes().try_into().unwrap());
        assert_eq!(registry.supported_versions(fourcc), Some(&[][..]));
        assert!(registry.is_supported_version(fourcc, 9));
        assert!(registry.is_registered(fourcc));
    }
}

#[test]
fn mhac_rejects_config_length_mismatch_during_marshal() {
    let mhac = MhaC {
        config_version: 1,
        mpeg_h_3da_profile_level_indication: 12,
        reference_channel_layout: 6,
        mpeg_h_3da_config_length: 5,
        mpeg_h_3da_config: vec![0x01, 0x02, 0x03, 0x04],
    };

    let error = marshal(&mut Vec::new(), &mhac, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for MpegH3DAConfig: length does not match MpegH3DAConfigLength"
    );
}
