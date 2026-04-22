use std::env;
use std::fs::File;
use std::io;

use mp4forge::probe::probe_codec_detailed;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_path = env::args().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: probe_codec_details INPUT.mp4",
        )
    })?;

    let mut file = File::open(&input_path)?;
    let summary = probe_codec_detailed(&mut file)?;

    for track in &summary.tracks {
        println!(
            "track {} family {:?}",
            track.summary.summary.track_id, track.summary.codec_family
        );
        println!("  details: {:?}", track.codec_details);
    }

    Ok(())
}
