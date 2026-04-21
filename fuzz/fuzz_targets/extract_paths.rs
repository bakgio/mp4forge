#![no_main]

mod support;

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use mp4forge::boxes::iso14496_12::{Ftyp, Meta, Moov, Trak, Udta};
use mp4forge::codec::{CodecBox, marshal};
use mp4forge::extract::{extract_box, extract_box_with_payload, extract_boxes};
use mp4forge::walk::BoxPath;
use mp4forge::{BoxInfo, FourCc};

use support::FuzzInput;

const PATH_PARTS: [FourCc; 8] = [
    FourCc::from_bytes(*b"ftyp"),
    FourCc::from_bytes(*b"moov"),
    FourCc::from_bytes(*b"trak"),
    FourCc::from_bytes(*b"meta"),
    FourCc::from_bytes(*b"udta"),
    FourCc::from_bytes(*b"zzzz"),
    FourCc::from_bytes(*b"free"),
    FourCc::ANY,
];

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    let file = sample_file();

    let mut paths = Vec::new();
    for _ in 0..input.take_usize(8) {
        paths.push(take_known_path(&mut input));
    }

    let _ = extract_boxes(&mut Cursor::new(file.clone()), None, &paths);

    let _ = extract_box(
        &mut Cursor::new(file.clone()),
        None,
        take_known_path(&mut input),
    );

    let _ = extract_box_with_payload(
        &mut Cursor::new(file.clone()),
        None,
        take_known_path(&mut input),
    );

    let parent = extract_box(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([FourCc::from_bytes(*b"moov")]),
    )
    .ok()
    .and_then(|mut boxes| boxes.pop());
    if let Some(parent) = parent {
        let _ = extract_boxes(
            &mut Cursor::new(file),
            Some(&parent),
            &[take_known_path(&mut input)],
        );
    }
});

fn take_known_path(input: &mut FuzzInput<'_>) -> BoxPath {
    let depth = input.take_usize(4);
    let mut path = Vec::with_capacity(depth);
    for _ in 0..depth {
        path.push(input.choose_fourcc(&PATH_PARTS));
    }
    BoxPath::from(path)
}

fn sample_file() -> Vec<u8> {
    let ftyp = Ftyp {
        major_brand: FourCc::from_bytes(*b"isom"),
        minor_version: 0x200,
        compatible_brands: vec![FourCc::from_bytes(*b"isom")],
    };
    let unknown = encode_raw_box(FourCc::from_bytes(*b"zzzz"), &[0xde, 0xad, 0xbe, 0xef]);
    let trak = encode_supported_box(&Trak, &[]);
    let meta = encode_supported_box(&Meta::default(), &[]);
    let udta = encode_supported_box(&Udta, &unknown);
    let moov = encode_supported_box(&Moov, &[trak, meta, udta].concat());
    [encode_supported_box(&ftyp, &[]), moov].concat()
}

fn encode_supported_box<B>(box_value: &B, children: &[u8]) -> Vec<u8>
where
    B: CodecBox,
{
    let mut payload = Vec::new();
    let _ = marshal(&mut payload, box_value, None);
    payload.extend_from_slice(children);
    encode_raw_box(box_value.box_type(), &payload)
}

fn encode_raw_box(box_type: FourCc, payload: &[u8]) -> Vec<u8> {
    let info = BoxInfo::new(box_type, 8 + payload.len() as u64);
    let mut bytes = info.encode();
    bytes.extend_from_slice(payload);
    bytes
}
