use std::env;
use std::fs::File;

use mp4forge::cli::pssh::build_pssh_report;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_path = env::args()
        .nth(1)
        .expect("usage: cargo run --example pssh_report -- <input.mp4>");

    let mut file = File::open(input_path)?;
    let report = build_pssh_report(&mut file)?;

    for entry in report.entries {
        println!(
            "{} offset={} system_id={} kid_count={} data_size={}",
            entry.path, entry.offset, entry.system_id, entry.kid_count, entry.data_size
        );
    }

    Ok(())
}
