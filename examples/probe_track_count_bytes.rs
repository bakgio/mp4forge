use std::env;
use std::error::Error;
use std::fs;

use mp4forge::probe::probe_bytes;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let Some(path) = env::args().nth(1) else {
        return Err("usage: cargo run --example probe_track_count_bytes -- <input.mp4>".into());
    };

    let input = fs::read(path)?;
    let info = probe_bytes(&input)?;
    println!("track num: {}", info.tracks.len());
    Ok(())
}
