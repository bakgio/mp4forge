use std::env;
use std::error::Error;
use std::fs::File;

use mp4forge::probe::probe;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let Some(path) = env::args().nth(1) else {
        return Err("usage: cargo run --example probe_track_count -- <input.mp4>".into());
    };

    let mut file = File::open(path)?;
    let info = probe(&mut file)?;
    println!("track num: {}", info.tracks.len());
    Ok(())
}
