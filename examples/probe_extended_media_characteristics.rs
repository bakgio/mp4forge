use std::env;
use std::fs::File;

use mp4forge::probe::probe_extended_media_characteristics;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(input_path) = env::args().nth(1) else {
        eprintln!("usage: cargo run --example probe_extended_media_characteristics -- <input.mp4>");
        std::process::exit(1);
    };

    let mut file = File::open(input_path)?;
    let summary = probe_extended_media_characteristics(&mut file)?;

    for track in &summary.tracks {
        println!(
            "track {} sample_entry_type={:?}",
            track.summary.summary.track_id, track.summary.sample_entry_type
        );
        if let Some(aperture) = track.visual_metadata.clean_aperture.as_ref() {
            println!(
                "  clean aperture: {}/{} x {}/{}",
                aperture.width_numerator,
                aperture.width_denominator,
                aperture.height_numerator,
                aperture.height_denominator
            );
        }
        if let Some(light) = track.visual_metadata.content_light_level.as_ref() {
            println!(
                "  content light level: max_cll={} max_fall={}",
                light.max_cll, light.max_fall
            );
        }
        if let Some(display) = track.visual_metadata.mastering_display.as_ref() {
            println!(
                "  mastering display: white_point=({}, {}) luminance={}..{}",
                display.white_point_chromaticity_x,
                display.white_point_chromaticity_y,
                display.luminance_min,
                display.luminance_max
            );
        }
    }

    Ok(())
}
