use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::SampleEntry;
use mp4forge::boxes::iso14496_30::{
    CueIDBox, CuePayloadBox, CueSettingsBox, CueSourceIDBox, CueTimeBox, VTTAdditionalTextBox,
    VTTCueBox, VTTEmptyCueBox, WVTTSampleEntry, WebVTTConfigurationBox, WebVTTSourceLabelBox,
};
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
fn webvtt_catalog_roundtrips() {
    assert_box_roundtrip(
        WebVTTConfigurationBox {
            config: String::from("WEBVTT\n"),
        },
        b"WEBVTT\n",
        "Config=\"WEBVTT.\"",
    );

    assert_box_roundtrip(
        WebVTTSourceLabelBox {
            source_label: String::from("Source"),
        },
        b"Source",
        "SourceLabel=\"Source\"",
    );

    assert_any_box_roundtrip(
        WVTTSampleEntry {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"wvtt"),
                data_reference_index: 0x1234,
            },
        },
        &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34],
        "DataReferenceIndex=4660",
    );

    assert_box_roundtrip(VTTCueBox, &[], "");
    assert_box_roundtrip(
        CueSourceIDBox { source_id: 0 },
        &[0x00, 0x00, 0x00, 0x00],
        "SourceId=0",
    );
    assert_box_roundtrip(
        CueTimeBox {
            cue_current_time: String::from("00:00:00.000"),
        },
        b"00:00:00.000",
        "CueCurrentTime=\"00:00:00.000\"",
    );
    assert_box_roundtrip(
        CueIDBox {
            cue_id: String::from("example_id"),
        },
        b"example_id",
        "CueId=\"example_id\"",
    );
    assert_box_roundtrip(
        CueSettingsBox {
            settings: String::from("line=0"),
        },
        b"line=0",
        "Settings=\"line=0\"",
    );
    assert_box_roundtrip(
        CuePayloadBox {
            cue_text: String::from("sample"),
        },
        b"sample",
        "CueText=\"sample\"",
    );
    assert_box_roundtrip(VTTEmptyCueBox, &[], "");
    assert_box_roundtrip(
        VTTAdditionalTextBox {
            cue_additional_text: String::from("test"),
        },
        b"test",
        "CueAdditionalText=\"test\"",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_webvtt_types() {
    let registry = default_registry();

    for box_type in [
        FourCc::from_bytes(*b"vttC"),
        FourCc::from_bytes(*b"vlab"),
        FourCc::from_bytes(*b"wvtt"),
        FourCc::from_bytes(*b"vttc"),
        FourCc::from_bytes(*b"vsid"),
        FourCc::from_bytes(*b"ctim"),
        FourCc::from_bytes(*b"iden"),
        FourCc::from_bytes(*b"sttg"),
        FourCc::from_bytes(*b"payl"),
        FourCc::from_bytes(*b"vtte"),
        FourCc::from_bytes(*b"vtta"),
    ] {
        assert_eq!(registry.supported_versions(box_type), Some(&[][..]));
        assert!(registry.is_supported_version(box_type, 0));
        assert!(registry.is_supported_version(box_type, 9));
        assert!(registry.is_registered(box_type));
    }
}
