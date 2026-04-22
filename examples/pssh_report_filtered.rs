use std::env;
use std::fs::File;

use mp4forge::cli::pssh::{PsshReportFilter, build_pssh_report_with_filters};
use mp4forge::walk::BoxPath;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_path = env::args()
        .nth(1)
        .expect("usage: cargo run --example pssh_report_filtered -- <input.mp4>");

    let mut file = File::open(input_path)?;
    let report = build_pssh_report_with_filters(
        &mut file,
        &PsshReportFilter {
            paths: vec![BoxPath::parse("moov")?],
            system_ids: vec![[
                0x10, 0x77, 0xef, 0xec, 0xc0, 0xb2, 0x4d, 0x02, 0xac, 0xe3, 0x3c, 0x1e, 0x52, 0xe2,
                0xfb, 0x4b,
            ]],
            kids: Vec::new(),
        },
    )?;

    for entry in report.entries {
        println!(
            "{} system_id={} kid_count={} data_size={}",
            entry.path, entry.system_id, entry.kid_count, entry.data_size
        );
    }

    Ok(())
}
