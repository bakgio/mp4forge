use std::env;
use std::fs::File;
use std::io;

use mp4forge::cli::divide::{DivideTrackRole, validate_divide_reader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_path = env::args().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: validate_divide_layout INPUT.mp4",
        )
    })?;

    let mut file = File::open(&input_path)?;
    let report = validate_divide_reader(&mut file)?;

    for track in &report.tracks {
        let role = match track.role {
            DivideTrackRole::Video => "video",
            DivideTrackRole::Audio => "audio",
        };
        let codec = track
            .original_format
            .or(track.sample_entry_type)
            .map(|value| value.to_string())
            .unwrap_or_else(|| format!("{:?}", track.codec_family));
        println!(
            "track {} role {} codec {} segments {}",
            track.track_id, role, codec, track.segment_count
        );
    }

    Ok(())
}
