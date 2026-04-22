use std::env;
use std::fs::File;
use std::io;

use mp4forge::probe::{ProbeOptions, probe_codec_detailed_with_options};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_path = env::args().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: probe_lightweight INPUT.mp4",
        )
    })?;

    let mut file = File::open(&input_path)?;
    let summary = probe_codec_detailed_with_options(&mut file, ProbeOptions::lightweight())?;

    println!("fast start: {}", summary.fast_start);
    println!("track num: {}", summary.tracks.len());
    for track in &summary.tracks {
        println!(
            "track {} family {:?} expanded samples {}",
            track.summary.summary.track_id,
            track.summary.codec_family,
            track.summary.summary.samples.len()
        );
    }

    Ok(())
}
