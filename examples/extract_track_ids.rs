use std::env;
use std::error::Error;
use std::fs::File;

use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::Tkhd;
use mp4forge::extract::extract_box_with_payload;
use mp4forge::walk::BoxPath;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let Some(path) = env::args().nth(1) else {
        return Err("usage: cargo run --example extract_track_ids -- <input.mp4>".into());
    };

    let mut file = File::open(path)?;
    let boxes = extract_box_with_payload(
        &mut file,
        None,
        BoxPath::from([
            FourCc::from_bytes(*b"moov"),
            FourCc::from_bytes(*b"trak"),
            FourCc::from_bytes(*b"tkhd"),
        ]),
    )?;

    for entry in boxes {
        let Some(tkhd) = entry.payload.as_any().downcast_ref::<Tkhd>() else {
            return Err("expected tkhd payload".into());
        };
        println!("track ID: {}", tkhd.track_id);
    }

    Ok(())
}
