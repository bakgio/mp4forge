use std::io::Cursor;

use mp4forge::boxes::AnyTypeBox;
use mp4forge::boxes::iso14496_12::{Ftyp, Meta, Moov, Trak, Udta};
use mp4forge::boxes::metadata::{
    DATA_TYPE_STRING_UTF8, Data, Ilst, Key, Keys, NumberedMetadataItem,
};
use mp4forge::codec::{CodecBox, marshal};
use mp4forge::extract::{ExtractError, extract_box, extract_box_with_payload, extract_boxes};
use mp4forge::stringify::stringify;
use mp4forge::walk::BoxPath;
use mp4forge::{BoxInfo, FourCc};

mod support;

use support::fixture_path;

#[test]
fn extract_boxes_match_exact_wildcard_and_relative_paths() {
    let trak = encode_supported_box(&Trak, &[]);
    let meta = encode_supported_box(&Meta::default(), &[]);
    let udta = encode_supported_box(&Udta, &meta);
    let moov = encode_supported_box(&Moov, &[trak, udta].concat());

    let wildcard = extract_box(
        &mut Cursor::new(moov.clone()),
        None,
        BoxPath::from([fourcc("moov"), FourCc::ANY]),
    )
    .unwrap();
    assert_eq!(box_types(&wildcard), vec![fourcc("trak"), fourcc("udta")]);

    let exact = extract_boxes(
        &mut Cursor::new(moov.clone()),
        None,
        &[
            BoxPath::from([fourcc("moov")]),
            BoxPath::from([fourcc("moov"), fourcc("udta")]),
        ],
    )
    .unwrap();
    assert_eq!(box_types(&exact), vec![fourcc("moov"), fourcc("udta")]);

    let parent = extract_box(
        &mut Cursor::new(moov.clone()),
        None,
        BoxPath::from([fourcc("moov")]),
    )
    .unwrap()
    .pop()
    .unwrap();
    let relative = extract_box(
        &mut Cursor::new(moov),
        Some(&parent),
        BoxPath::from([fourcc("udta")]),
    )
    .unwrap();
    assert_eq!(box_types(&relative), vec![fourcc("udta")]);
}

#[test]
fn extract_box_with_payload_uses_walked_lookup_context() {
    let qt = fourcc("qt  ");
    let ftyp = Ftyp {
        major_brand: qt,
        minor_version: 0x0200,
        compatible_brands: vec![qt],
    };
    let mut keys = Keys::default();
    keys.entry_count = 1;
    keys.entries = vec![Key {
        key_size: 9,
        key_namespace: fourcc("mdta"),
        key_value: vec![b'x'],
    }];

    let mut numbered = NumberedMetadataItem::default();
    numbered.set_box_type(FourCc::from_u32(1));
    numbered.item_name = fourcc("data");
    numbered.data = Data {
        data_type: DATA_TYPE_STRING_UTF8,
        data_lang: 0,
        data: b"1.0.0".to_vec(),
    };

    let keys_box = encode_supported_box(&keys, &[]);
    let numbered_box = encode_supported_box(&numbered, &[]);
    let ilst_box = encode_supported_box(&Ilst, &numbered_box);
    let meta_box = encode_supported_box(&Meta::default(), &[keys_box, ilst_box].concat());
    let moov_box = encode_supported_box(&Moov, &meta_box);
    let file = [encode_supported_box(&ftyp, &[]), moov_box].concat();

    let extracted = extract_box_with_payload(
        &mut Cursor::new(file),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("meta"),
            fourcc("ilst"),
            FourCc::from_u32(1),
        ]),
    )
    .unwrap();

    assert_eq!(extracted.len(), 1);
    let extracted = &extracted[0];
    assert_eq!(extracted.info.box_type(), FourCc::from_u32(1));
    assert!(extracted.info.lookup_context().under_ilst());
    assert_eq!(
        extracted.info.lookup_context().metadata_keys_entry_count(),
        1
    );

    let numbered = extracted
        .payload
        .as_ref()
        .as_any()
        .downcast_ref::<NumberedMetadataItem>()
        .unwrap();
    assert_eq!(numbered.item_name, fourcc("data"));
    assert_eq!(numbered.data.data_type, DATA_TYPE_STRING_UTF8);
    assert_eq!(numbered.data.data, b"1.0.0");
}

#[test]
fn extract_box_rejects_empty_paths() {
    let error =
        extract_box(&mut Cursor::new(Vec::<u8>::new()), None, BoxPath::default()).unwrap_err();
    assert!(matches!(error, ExtractError::EmptyPath));
}

#[test]
fn extract_boxes_match_shared_fixture_reference_paths() {
    let sample = std::fs::read(fixture_path("sample.mp4")).unwrap();
    let ftyp = extract_box(
        &mut Cursor::new(sample.clone()),
        None,
        BoxPath::from([fourcc("ftyp")]),
    )
    .unwrap();
    assert_eq!(box_types(&ftyp), vec![fourcc("ftyp")]);
    assert_eq!(ftyp[0].size(), 32);

    let mdhd = extract_box(
        &mut Cursor::new(sample),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    )
    .unwrap();
    assert_eq!(box_types(&mdhd), vec![fourcc("mdhd"), fourcc("mdhd")]);
    assert_eq!(mdhd.iter().map(BoxInfo::size).sum::<u64>(), 64);

    let fragmented = std::fs::read(fixture_path("sample_fragmented.mp4")).unwrap();
    let trun = extract_box(
        &mut Cursor::new(fragmented),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("trun")]),
    )
    .unwrap();
    assert_eq!(trun.len(), 8);
    assert!(trun.iter().all(|info| info.box_type() == fourcc("trun")));
    assert_eq!(trun.iter().map(BoxInfo::size).sum::<u64>(), 452);
}

#[test]
fn extract_box_with_payload_normalizes_nested_quicktime_numbered_items() {
    let sample = std::fs::read(fixture_path("sample_qt.mp4")).unwrap();
    let extracted = extract_box_with_payload(
        &mut Cursor::new(sample),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("udta"),
            fourcc("meta"),
            fourcc("ilst"),
            FourCc::from_u32(1),
        ]),
    )
    .unwrap();

    assert_eq!(extracted.len(), 1);
    let numbered = extracted[0]
        .payload
        .as_ref()
        .as_any()
        .downcast_ref::<NumberedMetadataItem>()
        .unwrap();

    assert_eq!(
        stringify(numbered, None).unwrap(),
        "Version=0 Flags=0x000000 ItemName=\"data\" Data={DataType=UTF8 DataLang=0 Data=\"1.0.0\"}"
    );
}

fn box_types(boxes: &[BoxInfo]) -> Vec<FourCc> {
    boxes.iter().map(BoxInfo::box_type).collect()
}

fn fourcc(value: &str) -> FourCc {
    FourCc::try_from(value).unwrap()
}

fn encode_supported_box<B>(box_value: &B, children: &[u8]) -> Vec<u8>
where
    B: CodecBox,
{
    let mut payload = Vec::new();
    marshal(&mut payload, box_value, None).unwrap();
    payload.extend_from_slice(children);
    encode_raw_box(box_value.box_type(), &payload)
}

fn encode_raw_box(box_type: FourCc, payload: &[u8]) -> Vec<u8> {
    let info = BoxInfo::new(box_type, 8 + payload.len() as u64);
    let mut bytes = info.encode();
    bytes.extend_from_slice(payload);
    bytes
}
