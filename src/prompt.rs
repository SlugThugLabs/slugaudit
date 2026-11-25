//! Bounded line input for the interactive setup prompts (`menu`, `connect`).

/// The longest answer an interactive prompt keeps. Every prompt takes a menu
/// number or `y`/`n`, so anything longer is already an invalid answer.
const MAX_PROMPT_LINE_BYTES: u64 = 256;

/// Maximum tail bytes drained past `MAX_PROMPT_LINE_BYTES` while looking for
/// the end of an over-long line so non-newline streams cannot stall forever.
const MAX_DRAIN_BYTES: u64 = 64 * 1024;

/// Reads one line of interactive input, keeping at most
/// `MAX_PROMPT_LINE_BYTES`; the rest of an over-long line is discarded so it
/// cannot be read as the next answer. `None` at end of input. Piped stdin is
/// accepted on purpose: `tests/menu_tests.rs` drives the menu that way.
///
/// # Errors
///
/// Returns the underlying I/O error if stdin cannot be read.
pub(crate) fn read_prompt_line() -> std::io::Result<Option<String>> {
    read_capped_line(&mut std::io::stdin().lock())
}

/// Whether a `[y/N]` answer accepts: `y` or `yes`, any case, surrounding
/// whitespace ignored. Anything else — including an empty answer — declines.
pub(crate) fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Asks a `[y/N]` question on a terminal and reports whether it was accepted.
/// Non-interactive (piped) stdin declines immediately, so an unattended run
/// never waits; a failure to ask or to read the answer is reported on stderr
/// and declines.
pub(crate) fn confirm(msg: &str) -> bool {
    use std::io::IsTerminal as _;
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        return false;
    }
    confirm_from(
        &mut std::io::stdout(),
        &mut stdin.lock(),
        &mut std::io::stderr(),
        msg,
    )
}

/// Asks `msg` on `output`, reads the answer from `input` and reports whether
/// it accepts. A failure to ask or read is written to `report` and declines.
/// Split from [`confirm`] so the terminal-only path is testable with injected
/// streams.
fn confirm_from(
    output: &mut impl std::io::Write,
    input: &mut impl std::io::BufRead,
    report: &mut impl std::io::Write,
    msg: &str,
) -> bool {
    match ask_yes_no(output, input, msg) {
        Ok(accepted) => accepted,
        Err(error) => {
            // The question failed, so the answer is "no"; a failure to
            // report that is logged, since `report` is the only channel
            // left and cannot itself be escalated from here.
            if let Err(write_error) = crate::cli::write_to(
                report,
                format_args!("could not read your answer: {error}\n"),
            ) {
                tracing::warn!(
                    error = %write_error,
                    "could not report a prompt read failure on stderr",
                );
            }
            false
        }
    }
}

/// Writes `msg`, reads one capped line, and reports whether it accepts. End
/// of input declines. Split from [`confirm`] so the failure paths are
/// testable.
fn ask_yes_no(
    output: &mut impl std::io::Write,
    input: &mut impl std::io::BufRead,
    msg: &str,
) -> std::io::Result<bool> {
    write!(output, "{msg}")?;
    output.flush()?;
    Ok(read_capped_line(input)?.is_some_and(|line| is_yes(&line)))
}

fn read_capped_line(input: &mut impl std::io::BufRead) -> std::io::Result<Option<String>> {
    use std::io::{BufRead as _, Read as _};
    let mut bytes = Vec::new();
    let read = (&mut *input)
        .take(MAX_PROMPT_LINE_BYTES)
        .read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.last() != Some(&b'\n') && (read as u64) == MAX_PROMPT_LINE_BYTES {
        let drained = (&mut *input).take(MAX_DRAIN_BYTES).skip_until(b'\n')?;
        if drained > 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("prompt input exceeded {MAX_PROMPT_LINE_BYTES} bytes"),
            ));
        }
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
