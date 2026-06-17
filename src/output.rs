use crate::{EXIT_BROKEN_PIPE, EXIT_IO_ERROR};
use std::fmt::Display;
use std::io::{self, Write};

/// Resolve a write result into a clean process exit when the stream has failed.
///
/// Rust's `print!`/`writeln!` panic on `EPIPE`; routing writes through here lets
/// `locked-in . | head` terminate cleanly (exit `141`). This is the *only* place output
/// code may end the process — leaf renderers return `io::Result` and stay side-effect-free.
/// A broken pipe is an expected, clean end of output. Any *other* write error (e.g. a full
/// disk on a redirect) is surfaced loudly — a message plus a non-zero exit — rather than
/// leaving truncated output behind while reporting success.
pub fn guard(result: io::Result<()>) {
    let Err(e) = result else { return };
    if e.kind() == io::ErrorKind::BrokenPipe {
        std::process::exit(EXIT_BROKEN_PIPE);
    }
    // Best-effort diagnostic; the stream we'd report on may itself be the one that failed.
    let _ = writeln!(io::stderr(), "locked-in: error writing output: {e}");
    std::process::exit(EXIT_IO_ERROR);
}

pub fn out_line(text: impl Display) {
    let stdout = io::stdout();
    guard(writeln!(stdout.lock(), "{text}"));
}

pub fn err_line(text: impl Display) {
    let stderr = io::stderr();
    guard(writeln!(stderr.lock(), "{text}"));
}
