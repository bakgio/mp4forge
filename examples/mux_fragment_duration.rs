#[cfg(feature = "mux")]
#[path = "support/mux_example_support.rs"]
mod mux_example_support;

#[cfg(feature = "mux")]
use std::error::Error;

#[cfg(feature = "mux")]
use mp4forge::mux::{
    MuxDurationMode, MuxMp4TrackSelector, MuxOutputLayout, MuxRequest, MuxTrackSpec, mux_to_path,
};

#[cfg(feature = "mux")]
fn main() -> Result<(), Box<dyn Error>> {
    let audio_input = mux_example_support::build_audio_input_file(
        "example-fragment-audio",
        mux_example_support::fourcc("dash"),
        "mp4a",
        &[b"one", b"two", b"three"],
    );
    let output_path = mux_example_support::write_temp_file("example-fragment-output", "mp4", &[]);

    let request = MuxRequest::new(vec![MuxTrackSpec::mp4(
        &audio_input,
        MuxMp4TrackSelector::Audio { occurrence: 1 },
    )])
    .with_output_layout(MuxOutputLayout::Fragmented)
    .with_duration_mode(MuxDurationMode::Fragment { seconds: 0.05 });

    mux_to_path(&request, &output_path)?;
    println!("wrote {}", output_path.display());
    Ok(())
}

#[cfg(not(feature = "mux"))]
fn main() {
    eprintln!("Enable the `mux` feature to run this example.");
}
