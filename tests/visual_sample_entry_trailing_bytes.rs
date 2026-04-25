use std::io::Cursor;

use mp4forge::boxes::iso14496_12::VisualSampleEntry;
use mp4forge::cli::dump::{DumpOptions, dump_reader};
use mp4forge::extract::extract_box_as_bytes;
use mp4forge::rewrite::rewrite_box_as_bytes;
use mp4forge::walk::BoxPath;

mod support;

use support::{
    build_visual_sample_entry_box_with_trailing_bytes, fourcc, visual_sample_entry_trailing_bytes,
};

#[test]
fn visual_sample_entry_extract_decodes_trailing_byte_layout() {
    let file = build_visual_sample_entry_box_with_trailing_bytes();

    let entries =
        extract_box_as_bytes::<VisualSampleEntry>(&file, BoxPath::from([fourcc("avc1")])).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].width, 640);
    assert_eq!(entries[0].height, 360);
}

#[test]
fn visual_sample_entry_dump_descends_through_children_without_eof() {
    let file = build_visual_sample_entry_box_with_trailing_bytes();
    let mut output = Vec::new();

    dump_reader(&mut Cursor::new(file), &DumpOptions::default(), &mut output).unwrap();

    let rendered = String::from_utf8(output).unwrap();
    assert!(rendered.contains("[avc1]"));
    assert!(rendered.contains("[pasp]"));
    assert!(!rendered.contains("unexpected EOF"));
}

#[test]
fn visual_sample_entry_rewrite_roundtrip_preserves_children_and_trailing_bytes() {
    let file = build_visual_sample_entry_box_with_trailing_bytes();

    let rewritten = rewrite_box_as_bytes::<VisualSampleEntry, _>(
        &file,
        BoxPath::from([fourcc("avc1")]),
        |_| {},
    )
    .unwrap();

    assert_eq!(rewritten, file);
    assert!(rewritten.ends_with(&visual_sample_entry_trailing_bytes()));
}
