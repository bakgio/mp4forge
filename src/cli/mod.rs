//! Reusable command-line routing and formatters.

use std::io::{self, Write};

pub mod divide;
pub mod dump;
pub mod edit;
pub mod extract;
pub mod probe;
pub mod pssh;
pub mod util;

/// Dispatches the top-level command-line arguments to the matching command handler.
pub fn dispatch<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    if args.is_empty() {
        let _ = write_usage(stderr);
        return 1;
    }

    match args[0].as_str() {
        "help" => {
            let _ = write_usage(stderr);
            0
        }
        "divide" => divide::run_with_output(&args[1..], stdout, stderr),
        "dump" => dump::run(&args[1..], stdout, stderr),
        "edit" => edit::run(&args[1..], stderr),
        "extract" => extract::run(&args[1..], stdout, stderr),
        "psshdump" => pssh::run(&args[1..], stdout, stderr),
        "probe" => probe::run(&args[1..], stdout, stderr),
        _ => {
            let _ = write_usage(stderr);
            1
        }
    }
}

/// Writes the top-level usage text for the current command router.
pub fn write_usage<W>(writer: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "USAGE: mp4forge COMMAND [ARGS]")?;
    writeln!(writer)?;
    writeln!(writer, "COMMAND:")?;
    writeln!(
        writer,
        "  divide       split a fragmented MP4 into track playlists"
    )?;
    writeln!(writer, "  dump         display the MP4 box tree")?;
    writeln!(writer, "  edit         rewrite selected boxes")?;
    writeln!(writer, "  extract      extract raw boxes by type or path")?;
    writeln!(writer, "  psshdump     summarize pssh boxes")?;
    writeln!(writer, "  probe        summarize an MP4 file")?;
    Ok(())
}
