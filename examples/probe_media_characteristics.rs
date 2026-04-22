use std::env;
use std::fs::File;

use mp4forge::probe::probe_media_characteristics;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(input_path) = env::args().nth(1) else {
        eprintln!("usage: cargo run --example probe_media_characteristics -- <input.mp4>");
        std::process::exit(1);
    };

    let mut file = File::open(input_path)?;
    let summary = probe_media_characteristics(&mut file)?;

    for track in &summary.tracks {
        println!(
            "track {} codec_family={:?}",
            track.summary.summary.track_id, track.summary.codec_family
        );
        if let Some(declared) = track.media_characteristics.declared_bitrate.as_ref() {
            println!(
                "  declared bitrate: avg={} max={} buffer={}",
                declared.avg_bitrate, declared.max_bitrate, declared.buffer_size_db
            );
        }
        if let Some(color) = track.media_characteristics.color.as_ref() {
            println!("  color type: {}", color.colour_type);
        }
        if let Some(par) = track.media_characteristics.pixel_aspect_ratio.as_ref() {
            println!("  pixel aspect ratio: {}/{}", par.h_spacing, par.v_spacing);
        }
        if let Some(field_order) = track.media_characteristics.field_order.as_ref() {
            println!(
                "  field order: count={} ordering={} interlaced={}",
                field_order.field_count, field_order.field_ordering, field_order.interlaced
            );
        }
    }

    Ok(())
}
