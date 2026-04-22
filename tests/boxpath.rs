use std::io::Cursor;
use std::str::FromStr;

use mp4forge::boxes::iso14496_12::{Moov, Trak, Udta};
use mp4forge::codec::{CodecBox, marshal};
use mp4forge::extract::extract_box;
use mp4forge::walk::{BoxPath, ParseBoxPathError};
use mp4forge::{BoxInfo, FourCc};

#[test]
fn parse_box_path_accepts_slash_delimited_segments() {
    let path = BoxPath::from_str("moov/trak/tkhd").unwrap();

    assert_eq!(
        path.as_slice(),
        &[fourcc("moov"), fourcc("trak"), fourcc("tkhd")]
    );
}

#[test]
fn parse_box_path_supports_wildcard_segments_in_extract_matching() {
    let trak = encode_supported_box(&Trak, &[]);
    let udta = encode_supported_box(&Udta, &[]);
    let moov = encode_supported_box(&Moov, &[trak, udta].concat());

    let extracted = extract_box(
        &mut Cursor::new(moov),
        None,
        BoxPath::parse("moov/*").unwrap(),
    )
    .unwrap();

    assert_eq!(
        extracted.iter().map(BoxInfo::box_type).collect::<Vec<_>>(),
        vec![fourcc("trak"), fourcc("udta")]
    );
}

#[test]
fn parse_box_path_supports_root_marker() {
    let path = BoxPath::parse("<root>").unwrap();
    assert!(path.is_empty());
}

#[test]
fn parse_box_path_rejects_invalid_segment_lengths_with_context() {
    let error = BoxPath::parse("moov/trakk").unwrap_err();

    assert!(matches!(
        error,
        ParseBoxPathError::InvalidSegment {
            index: 1,
            ref segment,
            source
        } if segment == "trakk" && source.len() == 5
    ));
    assert_eq!(
        error.to_string(),
        "invalid box path segment 2 (\"trakk\"): fourcc values must be exactly 4 bytes, got 5"
    );
}

#[test]
fn parse_box_path_reports_empty_segments_and_misplaced_root_marker() {
    let empty_segment = BoxPath::parse("moov//trak").unwrap_err();
    assert!(matches!(
        empty_segment,
        ParseBoxPathError::EmptySegment { index: 1 }
    ));
    assert_eq!(
        empty_segment.to_string(),
        "box path segment 2 must not be empty"
    );

    let root_marker = BoxPath::parse("<root>/trak").unwrap_err();
    assert!(matches!(
        root_marker,
        ParseBoxPathError::RootMarkerMustAppearAlone
    ));
    assert_eq!(
        root_marker.to_string(),
        "box path root marker \"<root>\" must appear alone"
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
