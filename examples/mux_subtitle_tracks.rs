#[cfg(feature = "mux")]
#[path = "support/mux_example_support.rs"]
mod mux_example_support;

#[cfg(feature = "mux")]
use std::error::Error;

#[cfg(feature = "mux")]
use mp4forge::mux::{MuxMp4TrackSelector, MuxRequest, MuxTrackSpec, mux_to_path};

#[cfg(feature = "mux")]
fn main() -> Result<(), Box<dyn Error>> {
    let video_input = mux_example_support::build_video_input_file(
        "example-subtitle-video",
        mux_example_support::fourcc("isom"),
        "avc1",
        &[b"video"],
    );
    let audio_input = mux_example_support::build_audio_input_file_with_timing(
        "example-subtitle-audio",
        mux_example_support::fourcc("dash"),
        "mp4a",
        1_000,
        1_000,
        &[b"aud"],
    );
    let text_input = mux_example_support::build_text_input_file(
        "example-subtitle-text",
        mux_example_support::fourcc("mp42"),
    );
    let output_path = mux_example_support::write_temp_file("example-subtitle-output", "mp4", &[]);

    let request = MuxRequest::new(vec![
        MuxTrackSpec::mp4(&video_input, MuxMp4TrackSelector::Video),
        MuxTrackSpec::mp4(&audio_input, MuxMp4TrackSelector::Audio { occurrence: 1 }),
        MuxTrackSpec::mp4(&text_input, MuxMp4TrackSelector::Text { occurrence: 1 }),
        MuxTrackSpec::mp4(&text_input, MuxMp4TrackSelector::Text { occurrence: 2 }),
    ]);

    mux_to_path(&request, &output_path)?;
    println!("wrote {}", output_path.display());
    Ok(())
}

#[cfg(not(feature = "mux"))]
fn main() {
    eprintln!("Enable the `mux` feature to run this example.");
}
