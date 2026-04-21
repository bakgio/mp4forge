use std::io::Cursor;

use mp4forge::boxes::iso14496_12::{Ftyp, Meta, Moov};
use mp4forge::boxes::metadata::{
    DATA_TYPE_STRING_UTF8, Data, Ilst, Key, Keys, NumberedMetadataItem,
};
use mp4forge::boxes::{AnyTypeBox, BoxLookupContext};
use mp4forge::codec::{CodecBox, marshal};
use mp4forge::walk::{BoxPath, WalkControl, walk_structure};
use mp4forge::{BoxInfo, FourCc};

#[test]
fn walk_structure_carries_quicktime_brand_and_keys_context() {
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

    let mut visited = Vec::new();
    walk_structure(&mut Cursor::new(file), |handle| {
        visited.push(handle.path().clone());

        match handle.info().box_type() {
            box_type if box_type == fourcc("ftyp") => {
                assert!(handle.info().lookup_context().is_quicktime_compatible());
                Ok(WalkControl::Continue)
            }
            box_type if box_type == fourcc("moov") => {
                assert!(handle.info().lookup_context().is_quicktime_compatible());
                Ok(WalkControl::Descend)
            }
            box_type if box_type == fourcc("meta") => {
                assert!(handle.info().lookup_context().is_quicktime_compatible());
                assert_eq!(
                    handle.descendant_lookup_context(),
                    BoxLookupContext::new().with_quicktime_compatible(true)
                );
                Ok(WalkControl::Descend)
            }
            box_type if box_type == fourcc("keys") => {
                let (payload, read) = handle.read_payload()?;
                assert_eq!(read, 17);
                let keys = payload.as_ref().as_any().downcast_ref::<Keys>().unwrap();
                assert_eq!(keys.entry_count, 1);
                assert_eq!(
                    handle.info().lookup_context().metadata_keys_entry_count(),
                    1
                );
                Ok(WalkControl::Continue)
            }
            box_type if box_type == fourcc("ilst") => {
                assert_eq!(
                    handle.info().lookup_context().metadata_keys_entry_count(),
                    1
                );
                assert!(handle.descendant_lookup_context().under_ilst());
                Ok(WalkControl::Descend)
            }
            box_type if box_type == FourCc::from_u32(1) => {
                assert!(handle.is_supported_type());
                assert_eq!(
                    handle.info().lookup_context().metadata_keys_entry_count(),
                    1
                );
                assert!(handle.info().lookup_context().under_ilst());

                let (payload, read) = handle.read_payload()?;
                assert_eq!(read, 21);
                let numbered = payload
                    .as_ref()
                    .as_any()
                    .downcast_ref::<NumberedMetadataItem>()
                    .unwrap();
                assert_eq!(numbered.item_name, fourcc("data"));
                assert_eq!(numbered.data.data_type, DATA_TYPE_STRING_UTF8);
                assert_eq!(numbered.data.data, b"1.0.0");
                Ok(WalkControl::Continue)
            }
            other => panic!("unexpected box {other}"),
        }
    })
    .unwrap();

    assert_eq!(
        visited,
        vec![
            BoxPath::from([fourcc("ftyp")]),
            BoxPath::from([fourcc("moov")]),
            BoxPath::from([fourcc("moov"), fourcc("meta")]),
            BoxPath::from([fourcc("moov"), fourcc("meta"), fourcc("keys")]),
            BoxPath::from([fourcc("moov"), fourcc("meta"), fourcc("ilst")]),
            BoxPath::from([
                fourcc("moov"),
                fourcc("meta"),
                fourcc("ilst"),
                FourCc::from_u32(1)
            ]),
        ]
    );
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
