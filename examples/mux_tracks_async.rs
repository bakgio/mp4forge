#[cfg(all(feature = "mux", feature = "async"))]
#[path = "support/mux_example_support.rs"]
mod mux_example_support;

#[cfg(all(feature = "mux", feature = "async"))]
use std::{error::Error, io};

#[cfg(all(feature = "mux", feature = "async"))]
use mp4forge::mux::{MuxMp4TrackSelector, MuxRequest, MuxTrackSpec, mux_to_path_async};

#[cfg(all(feature = "mux", feature = "async"))]
fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(8 * 1024 * 1024)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        tokio::spawn(run())
            .await
            .map_err(|error| io::Error::other(format!("async mux example task failed: {error}")))?
    })
}

#[cfg(all(feature = "mux", feature = "async"))]
async fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let audio_input = mux_example_support::build_audio_input_file_with_timing(
        "example-async-mux-audio",
        mux_example_support::fourcc("dash"),
        "mp4a",
        1_000,
        1_000,
        &[b"aud"],
    );
    let video_input = mux_example_support::build_video_input_file(
        "example-async-mux-video",
        mux_example_support::fourcc("isom"),
        "avc1",
        &[b"video"],
    );
    let output_path = mux_example_support::write_temp_file("example-async-mux-output", "mp4", &[]);

    let request = MuxRequest::new(vec![
        MuxTrackSpec::mp4(&audio_input, MuxMp4TrackSelector::Audio { occurrence: 1 }),
        MuxTrackSpec::mp4(&video_input, MuxMp4TrackSelector::Video),
    ]);

    mux_to_path_async(&request, &output_path).await?;
    println!("wrote {}", output_path.display());
    Ok(())
}

#[cfg(not(all(feature = "mux", feature = "async")))]
fn main() {
    eprintln!("Enable the `mux` and `async` features to run this example.");
}
