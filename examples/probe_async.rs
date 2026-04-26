#[cfg(feature = "async")]
use std::env;
#[cfg(feature = "async")]
use std::error::Error;
#[cfg(feature = "async")]
use std::io;

#[cfg(feature = "async")]
use mp4forge::probe::probe_async;
#[cfg(feature = "async")]
use tokio::fs::File;

#[cfg(feature = "async")]
type ExampleError = Box<dyn Error + Send + Sync>;

#[cfg(feature = "async")]
#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(feature = "async")]
async fn run() -> Result<(), ExampleError> {
    let input_paths = env::args().skip(1).collect::<Vec<_>>();
    if input_paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: probe_async INPUT.mp4 [MORE.mp4 ...]",
        )
        .into());
    }

    let mut handles = Vec::new();
    for input_path in input_paths {
        handles.push(tokio::spawn(async move { probe_file(input_path).await }));
    }

    for handle in handles {
        handle
            .await
            .map_err(|error| io::Error::other(format!("probe task failed: {error}")))??;
    }

    Ok(())
}

#[cfg(not(feature = "async"))]
fn main() {
    eprintln!(
        "enable the async feature: cargo run --example probe_async --features async -- INPUT.mp4 [MORE.mp4 ...]"
    );
    std::process::exit(1);
}

#[cfg(feature = "async")]
async fn probe_file(input_path: String) -> Result<(), ExampleError> {
    let mut file = File::open(&input_path).await?;
    let summary = probe_async(&mut file).await?;

    println!("file: {input_path}");
    println!("  fast start: {}", summary.fast_start);
    println!("  track num: {}", summary.tracks.len());
    for track in &summary.tracks {
        println!(
            "  track {} codec {:?} encrypted {}",
            track.track_id, track.codec, track.encrypted
        );
    }

    Ok(())
}
