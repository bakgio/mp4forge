#[cfg(feature = "mux")]
#[path = "support/mux_example_support.rs"]
mod mux_example_support;

#[cfg(feature = "mux")]
use std::error::Error;

#[cfg(feature = "mux")]
use mp4forge::mux::{MuxRequest, MuxTrackSpec, mux_to_path};

#[cfg(feature = "mux")]
fn main() -> Result<(), Box<dyn Error>> {
    let audio_input = mux_example_support::write_test_flac_file("example-raw-audio", b"flac-frame");
    let video_input = mux_example_support::write_test_av1_ivf_file(
        "example-raw-video",
        640,
        360,
        &[0, 1],
        &[b"av01", b"tail"],
    );
    let output_path = mux_example_support::write_temp_file("example-raw-output", "mp4", &[]);

    let request = MuxRequest::new(vec![
        MuxTrackSpec::path(audio_input),
        MuxTrackSpec::path(video_input),
    ]);

    mux_to_path(&request, &output_path)?;
    println!("wrote {}", output_path.display());
    Ok(())
}

#[cfg(not(feature = "mux"))]
fn main() {
    eprintln!("Enable the `mux` feature to run this example.");
}
