#![cfg(not(feature = "async"))]

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use mp4forge::probe::probe;

#[test]
fn default_build_keeps_sync_probe_surface_available() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample.mp4");

    let bytes = fs::read(fixture).unwrap();
    let summary = probe(&mut Cursor::new(bytes)).unwrap();

    assert_eq!(summary.tracks.len(), 2);
}
