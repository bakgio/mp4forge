use std::env;
use std::fs::File;
use std::io;

use mp4forge::cli::dump::{DumpOptions, build_field_structured_report};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_path = env::args().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: dump_structured_fields INPUT.mp4",
        )
    })?;

    let mut file = File::open(&input_path)?;
    let report = build_field_structured_report(&mut file, &DumpOptions::default())?;

    for root in &report.boxes {
        println!("{} {}", root.path, root.payload_fields.len());
    }

    Ok(())
}
