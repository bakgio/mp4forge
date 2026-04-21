use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::{AudioSampleEntry, SampleEntry};
use mp4forge::boxes::iso23001_5::PcmC;
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
fn pcm_catalog_roundtrips() {
    let mut pcmc = PcmC::default();
    pcmc.set_version(0);
    pcmc.format_flags = 1;
    pcmc.pcm_sample_size = 32;

    assert_box_roundtrip(
        pcmc,
        &[0x00, 0x00, 0x00, 0x00, 0x01, 0x20],
        "Version=0 Flags=0x000000 FormatFlags=0x1 PCMSampleSize=0x20",
    );

    assert_any_box_roundtrip(
        AudioSampleEntry {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"ipcm"),
                data_reference_index: 0x1234,
            },
            entry_version: 0x0123,
            channel_count: 0x2345,
            sample_size: 0x4567,
            pre_defined: 0x6789,
            sample_rate: 0x01234567,
            quicktime_data: Vec::new(),
        },
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x01, 0x23, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x23, 0x45, 0x45, 0x67, 0x67, 0x89, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67,
        ],
        "DataReferenceIndex=4660 EntryVersion=291 ChannelCount=9029 SampleSize=17767 PreDefined=26505 SampleRate=291.27110",
    );

    assert_any_box_roundtrip(
        AudioSampleEntry {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"fpcm"),
                data_reference_index: 0x1234,
            },
            entry_version: 1,
            channel_count: 0x2345,
            sample_size: 0x4567,
            pre_defined: 0x6789,
            sample_rate: 0x01234567,
            quicktime_data: vec![
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ],
        },
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x23, 0x45, 0x45, 0x67, 0x67, 0x89, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67,
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ],
        "DataReferenceIndex=4660 EntryVersion=1 ChannelCount=9029 SampleSize=17767 PreDefined=26505 SampleRate=291.27110 QuickTimeData=[0x0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_pcm_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"pcmC")),
        Some(&[0, 1][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"ipcm")),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"fpcm")),
        Some(&[][..])
    );
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"pcmC"), 0));
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"pcmC"), 1));
    assert!(!registry.is_supported_version(FourCc::from_bytes(*b"pcmC"), 2));
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"ipcm"), 9));
    assert!(registry.is_supported_version(FourCc::from_bytes(*b"fpcm"), 9));
    assert!(registry.is_registered(FourCc::from_bytes(*b"ipcm")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"fpcm")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"pcmC")));
}
