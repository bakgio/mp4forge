use std::env;
use std::error::Error;
use std::fs::File;

use mp4forge::FourCc;
use mp4forge::extract::{extract_box_bytes, extract_box_payload_bytes};
use mp4forge::walk::BoxPath;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let Some(input_path) = env::args().nth(1) else {
        return Err("usage: cargo run --example extract_mdhd_bytes -- <input.mp4>".into());
    };

    let box_path = BoxPath::from([
        FourCc::from_bytes(*b"moov"),
        FourCc::from_bytes(*b"trak"),
        FourCc::from_bytes(*b"mdia"),
        FourCc::from_bytes(*b"mdhd"),
    ]);

    let mut file = File::open(input_path)?;
    let boxes = extract_box_bytes(&mut file, None, box_path.clone())?;
    let payloads = extract_box_payload_bytes(&mut file, None, box_path)?;

    for (index, (box_bytes, payload_bytes)) in boxes.iter().zip(payloads.iter()).enumerate() {
        println!(
            "match {index}: total_bytes={} payload_bytes={}",
            box_bytes.len(),
            payload_bytes.len()
        );
    }

    Ok(())
}
