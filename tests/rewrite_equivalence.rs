mod support;

use std::fs;
use std::io::Cursor;

use mp4forge::cli::edit::{EditOptions, edit_reader};

use support::fixture_path;

#[test]
fn edit_reader_preserves_shared_fixture_bytes_without_mutations() {
    let fixtures = [
        "sample.mp4",
        "sample_fragmented.mp4",
        "sample_init.encv.mp4",
        "sample_init.enca.mp4",
        "sample_qt.mp4",
    ];

    for fixture in fixtures {
        let input = fs::read(fixture_path(fixture)).unwrap();
        let mut reader = Cursor::new(input.clone());
        let mut output = Cursor::new(Vec::new());

        edit_reader(&mut reader, &mut output, &EditOptions::default()).unwrap();

        assert_eq!(output.into_inner(), input, "rewrite drift for {fixture}");
    }
}
