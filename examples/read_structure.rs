use std::env;
use std::error::Error;
use std::fs::File;
use std::io;

use mp4forge::stringify::stringify;
use mp4forge::walk::{WalkControl, WalkError, walk_structure};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let Some(path) = env::args().nth(1) else {
        return Err("usage: cargo run --example read_structure -- <input.mp4>".into());
    };

    let mut file = File::open(path)?;
    walk_structure(&mut file, |handle| {
        println!("depth {}", handle.path().len());
        println!("type {}", handle.info().box_type());
        println!("size {}", handle.info().size());

        if handle.is_supported_type() {
            let (payload, _) = handle.read_payload()?;
            let text = stringify(payload.as_ref(), None).map_err(walk_error)?;
            println!("payload {text}");
            Ok(WalkControl::Descend)
        } else {
            Ok(WalkControl::Continue)
        }
    })?;

    Ok(())
}

fn walk_error(error: impl ToString) -> WalkError {
    WalkError::from(io::Error::other(error.to_string()))
}
