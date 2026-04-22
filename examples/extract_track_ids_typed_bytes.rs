use std::env;
use std::error::Error;
use std::fs;

use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::Tkhd;
use mp4forge::extract::extract_box_as_bytes;
use mp4forge::walk::BoxPath;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let Some(path) = env::args().nth(1) else {
        return Err(
            "usage: cargo run --example extract_track_ids_typed_bytes -- <input.mp4>".into(),
        );
    };

    let input = fs::read(path)?;
    let headers = extract_box_as_bytes::<Tkhd>(
        &input,
        BoxPath::from([
            FourCc::from_bytes(*b"moov"),
            FourCc::from_bytes(*b"trak"),
            FourCc::from_bytes(*b"tkhd"),
        ]),
    )?;

    for tkhd in headers {
        println!("track ID: {}", tkhd.track_id);
    }

    Ok(())
}
