//! Reusable command-line routing and formatters.

use std::fmt;
use std::io::{self, Write};
#[cfg(any(feature = "mux", test))]
use std::path::Path;

#[cfg(feature = "decrypt")]
pub mod decrypt;
pub mod divide;
pub mod dump;
pub mod edit;
pub mod extract;
#[cfg(feature = "mux")]
pub mod inspect;
#[cfg(feature = "mux")]
pub mod mux;
pub mod probe;
pub mod pssh;
pub mod util;

pub(crate) fn write_error_line<W, E>(
    writer: &mut W,
    error: &E,
    diagnostics: Option<(&'static str, &'static str)>,
) -> io::Result<()>
where
    W: Write,
    E: fmt::Display + ?Sized,
{
    match diagnostics {
        Some((stage, category)) => {
            writeln!(writer, "Error [stage={stage} category={category}]: {error}")
        }
        None => writeln!(writer, "Error: {error}"),
    }
}

pub(crate) fn write_warning_lines<W>(writer: &mut W, warnings: &[String]) -> io::Result<()>
where
    W: Write,
{
    for warning in warnings {
        writeln!(writer, "Warning: {warning}")?;
    }
    Ok(())
}

#[cfg(any(feature = "mux", test))]
pub(crate) fn format_post_run_diagnostics_unavailable<E>(
    subject: &str,
    path: &Path,
    error: &E,
) -> String
where
    E: fmt::Display + ?Sized,
{
    format!("{subject} unavailable for {}: {error}", path.display())
}

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
        #[cfg(feature = "decrypt")]
        "decrypt" => decrypt::run(&args[1..], stderr),
        "divide" => divide::run_with_output(&args[1..], stdout, stderr),
        "dump" => dump::run(&args[1..], stdout, stderr),
        "edit" => edit::run(&args[1..], stderr),
        "extract" => extract::run(&args[1..], stdout, stderr),
        #[cfg(feature = "mux")]
        "inspect" => inspect::run(&args[1..], stdout, stderr),
        #[cfg(feature = "mux")]
        "mux" => mux::run(&args[1..], stderr),
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
    #[cfg(feature = "decrypt")]
    writeln!(
        writer,
        "  decrypt      decrypt protected MP4-family content"
    )?;
    writeln!(writer, "  dump         display the MP4 box tree")?;
    writeln!(writer, "  edit         rewrite selected boxes")?;
    writeln!(writer, "  extract      extract raw boxes by type or path")?;
    #[cfg(feature = "mux")]
    writeln!(
        writer,
        "  inspect      inspect one direct-ingest input without writing an MP4"
    )?;
    #[cfg(feature = "mux")]
    writeln!(
        writer,
        "  mux          merge one video track plus audio tracks into one MP4"
    )?;
    writeln!(writer, "  psshdump     summarize pssh boxes")?;
    writeln!(writer, "  probe        summarize an MP4 file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use super::format_post_run_diagnostics_unavailable;

    #[test]
    fn post_run_diagnostics_unavailable_formatter_is_stable() {
        let error = io::Error::other("permission denied");
        let message = format_post_run_diagnostics_unavailable(
            "fragmented output diagnostics",
            Path::new("out.mp4"),
            &error,
        );

        assert_eq!(
            message,
            "fragmented output diagnostics unavailable for out.mp4: permission denied"
        );
    }
}
