#[cfg(feature = "decrypt")]
use std::env;
#[cfg(feature = "decrypt")]
use std::error::Error;
#[cfg(feature = "decrypt")]
use std::fs;
#[cfg(feature = "decrypt")]
use std::io;

#[cfg(feature = "decrypt")]
use mp4forge::decrypt::{DecryptOptions, decrypt_file_with_progress};

#[cfg(feature = "decrypt")]
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(feature = "decrypt")]
fn run() -> Result<(), Box<dyn Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 3 {
        return Err(
            "usage: cargo run --example decrypt_file --features decrypt -- <input.mp4> <output.mp4> <key-id:key> [more-keys...] [--fragments-info <init-or-movie.mp4>]"
                .into(),
        );
    }

    let input_path = args[0].clone();
    let output_path = args[1].clone();
    let mut options = DecryptOptions::new();
    let mut cursor = 2usize;
    while cursor < args.len() {
        match args[cursor].as_str() {
            "--fragments-info" => {
                let fragments_info_path = args.get(cursor + 1).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "missing path after --fragments-info",
                    )
                })?;
                options = options.with_fragments_info_bytes(fs::read(fragments_info_path)?);
                cursor += 2;
            }
            key_spec => {
                options = options.with_key_spec(key_spec)?;
                cursor += 1;
            }
        }
    }

    decrypt_file_with_progress(
        &input_path,
        &output_path,
        &options,
        |progress| match progress.total {
            Some(total) => eprintln!("{:?}: {}/{}", progress.phase, progress.completed, total),
            None => eprintln!("{:?}", progress.phase),
        },
    )?;

    println!("wrote clear output to {output_path}");
    Ok(())
}

#[cfg(not(feature = "decrypt"))]
fn main() {
    eprintln!(
        "enable the decrypt feature: cargo run --example decrypt_file --features decrypt -- <input.mp4> <output.mp4> <key-id:key> [more-keys...] [--fragments-info <init-or-movie.mp4>]"
    );
    std::process::exit(1);
}
