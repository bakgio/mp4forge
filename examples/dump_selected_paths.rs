use std::env;
use std::fs::File;

use mp4forge::cli::dump::{DumpOptions, build_field_structured_report_paths};
use mp4forge::walk::BoxPath;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_path = env::args()
        .nth(1)
        .expect("usage: cargo run --example dump_selected_paths -- <input.mp4>");

    let mut file = File::open(input_path)?;
    let paths = [BoxPath::parse("moov/trak")?];
    let report = build_field_structured_report_paths(&mut file, &DumpOptions::default(), &paths)?;

    for entry in report.boxes {
        println!("{} children={}", entry.path, entry.children.len());
    }

    Ok(())
}
