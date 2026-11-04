//! Checked standard output for the gates. A gate's verdict lines are what CI
//! greps, so a failed write (a closed pipe, a full disk) is reported and
//! fails the run instead of panicking inside `println!`. Pulled into each gate
//! with `#[path]`, like the argument contract.

use std::fmt::Display;
use std::io::{self, Write};
use std::process::ExitCode;

/// Exit code for an output failure: the verdict could not be delivered, which
/// is distinct from a failed verdict (1).
const OUTPUT_EXIT: u8 = 2;

/// Writes `text` and a newline to the locked standard output.
pub(crate) fn line(text: impl Display) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{text}")?;
    stdout.flush()
}

/// Reports a failed [`line()`] on stderr, prefixed with the gate's name, and
/// returns the exit code the gate must end with.
pub(crate) fn failed(prefix: &str, error: &io::Error) -> ExitCode {
    eprintln!("{prefix}: cannot write to stdout: {error}");
    ExitCode::from(OUTPUT_EXIT)
}
