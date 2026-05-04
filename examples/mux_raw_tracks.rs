#[cfg(feature = "mux")]
#[path = "support/mux_example_support.rs"]
mod mux_example_support;

#[cfg(feature = "mux")]
use std::error::Error;

#[cfg(feature = "mux")]
use std::str::FromStr;

#[cfg(feature = "mux")]
use mp4forge::mux::{MuxRequest, MuxTrackSpec, mux_to_path};

#[cfg(feature = "mux")]
fn main() -> Result<(), Box<dyn Error>> {
    let audio_input =
        mux_example_support::write_temp_file("example-raw-audio", "alac", b"alac-payload");
    let video_input =
        mux_example_support::write_temp_file("example-raw-video", "av1", b"av1-payload");
    let output_path = mux_example_support::write_temp_file("example-raw-output", "mp4", &[]);

    let request = MuxRequest::new(vec![
        MuxTrackSpec::from_str(&format!(
            "alac:{}#sample_rate=48000,channel_count=2,sample_duration=1024",
            audio_input.display()
        ))?,
        MuxTrackSpec::from_str(&format!(
            "av1:{}#width=640,height=360,timescale=1000,sample_duration=1000",
            video_input.display()
        ))?,
    ]);

    mux_to_path(&request, &output_path)?;
    println!("wrote {}", output_path.display());
    Ok(())
}

#[cfg(not(feature = "mux"))]
fn main() {
    eprintln!("Enable the `mux` feature to run this example.");
}
