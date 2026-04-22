#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;
use std::io::Cursor;

use mp4forge::boxes::iso14496_12::{Meta, Moof, Tfdt, Traf};
use mp4forge::extract::extract_box_as;
use mp4forge::rewrite::{
    RewriteError, rewrite_box_as, rewrite_box_as_bytes, rewrite_boxes_as_bytes,
};
use mp4forge::walk::BoxPath;

use support::{encode_raw_box, encode_supported_box, fixture_path, fourcc};

#[test]
fn rewrite_box_as_updates_matching_typed_payloads() {
    let input = build_rewrite_input_file();
    let mut reader = Cursor::new(input);
    let mut output = Cursor::new(Vec::new());

    let rewritten = rewrite_box_as::<_, _, Tfdt, _>(
        &mut reader,
        &mut output,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
        |tfdt| {
            tfdt.base_media_decode_time_v0 = 12_345;
        },
    )
    .unwrap();

    let tfdt = extract_box_as::<_, Tfdt>(
        &mut Cursor::new(output.into_inner()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
    )
    .unwrap();

    assert_eq!(rewritten, 1);
    assert_eq!(tfdt.len(), 1);
    assert_eq!(tfdt[0].base_media_decode_time_v0, 12_345);
}

#[test]
fn rewrite_box_as_bytes_updates_matching_typed_payloads() {
    let input = build_rewrite_input_file();
    let output = rewrite_box_as_bytes::<Tfdt, _>(
        &input,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
        |tfdt| {
            tfdt.base_media_decode_time_v0 = 12_345;
        },
    )
    .unwrap();

    let tfdt = extract_box_as::<_, Tfdt>(
        &mut Cursor::new(output),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
    )
    .unwrap();

    assert_eq!(tfdt.len(), 1);
    assert_eq!(tfdt[0].base_media_decode_time_v0, 12_345);
}

#[test]
fn rewrite_box_as_returns_zero_and_preserves_bytes_when_nothing_matches() {
    let input = fs::read(fixture_path("sample_fragmented.mp4")).unwrap();
    let mut reader = Cursor::new(input.clone());
    let mut output = Cursor::new(Vec::new());

    let rewritten = rewrite_box_as::<_, _, Tfdt, _>(
        &mut reader,
        &mut output,
        BoxPath::from([fourcc("zzzz")]),
        |_| {},
    )
    .unwrap();

    assert_eq!(rewritten, 0);
    assert_eq!(output.into_inner(), input);
}

#[test]
fn rewrite_boxes_as_bytes_preserves_bytes_when_nothing_matches() {
    let input = fs::read(fixture_path("sample_fragmented.mp4")).unwrap();

    let output =
        rewrite_boxes_as_bytes::<Tfdt, _>(&input, &[BoxPath::from([fourcc("zzzz")])], |_| {})
            .unwrap();

    assert_eq!(output, input);
}

#[test]
fn rewrite_box_as_reports_payload_type_context() {
    let input = build_rewrite_input_file();
    let mut reader = Cursor::new(input);
    let mut output = Cursor::new(Vec::new());

    let error = rewrite_box_as::<_, _, Meta, _>(
        &mut reader,
        &mut output,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
        |_| {},
    )
    .unwrap_err();

    assert!(matches!(
        error,
        RewriteError::UnexpectedPayloadType {
            ref path,
            box_type,
            offset,
            expected_type
        } if path.as_slice() == [fourcc("moof"), fourcc("traf"), fourcc("tfdt")]
            && box_type == fourcc("tfdt")
            && offset == 16
            && expected_type == std::any::type_name::<Meta>()
    ));
}

#[test]
fn rewrite_box_as_bytes_reports_payload_type_context() {
    let input = build_rewrite_input_file();

    let error = rewrite_box_as_bytes::<Meta, _>(
        &input,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
        |_| {},
    )
    .unwrap_err();

    assert!(matches!(
        error,
        RewriteError::UnexpectedPayloadType {
            ref path,
            box_type,
            offset,
            expected_type
        } if path.as_slice() == [fourcc("moof"), fourcc("traf"), fourcc("tfdt")]
            && box_type == fourcc("tfdt")
            && offset == 16
            && expected_type == std::any::type_name::<Meta>()
    ));
}

#[test]
fn rewrite_box_as_reports_payload_decode_context() {
    let mut bytes = encode_raw_box(fourcc("tfdt"), &[0x00, 0x00, 0x00, 0x00]);
    bytes.truncate(12);

    let mut reader = Cursor::new(bytes);
    let mut output = Cursor::new(Vec::new());

    let error = rewrite_box_as::<_, _, Tfdt, _>(
        &mut reader,
        &mut output,
        BoxPath::from([fourcc("tfdt")]),
        |_| {},
    )
    .unwrap_err();

    assert!(matches!(
        error,
        RewriteError::PayloadDecode {
            path,
            box_type,
            offset: 0,
            source: mp4forge::codec::CodecError::Io(ref io_error)
        } if path.as_slice() == [fourcc("tfdt")]
            && box_type == fourcc("tfdt")
            && io_error.kind() == std::io::ErrorKind::UnexpectedEof
    ));
}

#[test]
fn rewrite_box_as_rejects_empty_paths() {
    let mut reader = Cursor::new(Vec::<u8>::new());
    let mut output = Cursor::new(Vec::new());

    let error =
        rewrite_box_as::<_, _, Tfdt, _>(&mut reader, &mut output, BoxPath::default(), |_| {})
            .unwrap_err();

    assert!(matches!(error, RewriteError::EmptyPath));
}

fn build_rewrite_input_file() -> Vec<u8> {
    let mut tfdt = Tfdt::default();
    tfdt.base_media_decode_time_v0 = 9_000;

    let tfdt = encode_supported_box(&tfdt, &[]);
    let traf = encode_supported_box(&Traf, &tfdt);
    let moof = encode_supported_box(&Moof, &traf);
    let mdat = encode_raw_box(fourcc("mdat"), &[0, 1, 2, 3]);

    [moof, mdat].concat()
}
