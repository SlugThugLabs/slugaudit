//! Bounded line input for the interactive setup prompts (`menu`, `connect`).

/// The longest answer an interactive prompt keeps. Every prompt takes a menu
/// number or `y`/`n`, so anything longer is already an invalid answer.
const MAX_PROMPT_LINE_BYTES: u64 = 256;

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
    #[cfg(test)]
    {
        let _ = msg;
        false
    }
    #[cfg(not(test))]
    {
        use std::io::IsTerminal as _;
        if !std::io::stdin().is_terminal() {
            return false;
        }
        match ask_yes_no(&mut std::io::stdout(), &mut std::io::stdin().lock(), msg) {
            Ok(accepted) => accepted,
            Err(error) => {
                eprintln!("could not read your answer: {error}");
                false
            }
        }
    }
}

/// Writes `msg`, reads one line, and reports whether it accepts. Split from
/// [`confirm`] so the failure paths are testable.
fn ask_yes_no(
    output: &mut impl std::io::Write,
    input: &mut impl std::io::BufRead,
    msg: &str,
) -> std::io::Result<bool> {
    write!(output, "{msg}")?;
    output.flush()?;
    let mut line = String::new();
    input.read_line(&mut line)?;
    Ok(is_yes(&line))
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
    if bytes.last() != Some(&b'\n') {
        input.skip_until(b'\n')?;
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_y_or_yes_in_any_case_accepts_a_confirmation() {
        let accepted = ["y", "Y", "yes", " YES\n"];
        let declined = ["", "n", "no", "yep", "y es"];

        for answer in accepted {
            assert!(is_yes(answer), "{answer:?} must accept");
        }
        for answer in declined {
            assert!(!is_yes(answer), "{answer:?} must decline");
        }
    }

    #[test]
    fn a_prompt_answer_is_read_one_line_at_a_time_and_none_at_end_of_input() {
        let mut input = std::io::Cursor::new(b"2\ny\n".to_vec());

        let first = read_capped_line(&mut input).expect("first line");
        let second = read_capped_line(&mut input).expect("second line");
        let end = read_capped_line(&mut input).expect("end of input");

        assert_eq!(first.as_deref(), Some("2\n"));
        assert_eq!(second.as_deref(), Some("y\n"));
        assert_eq!(end, None);
    }

    #[test]
    fn an_over_long_prompt_line_is_capped_and_its_rest_is_not_read_as_the_next_answer() {
        let long = "9".repeat(10_000);
        let mut input = std::io::Cursor::new(format!("{long}\n3\n").into_bytes());

        let capped = read_capped_line(&mut input).expect("capped line");
        let next = read_capped_line(&mut input).expect("next line");

        assert_eq!(capped.map(|line| line.len()), Some(256));
        assert_eq!(next.as_deref(), Some("3\n"));
    }

    #[test]
    fn a_prompt_line_cut_inside_a_multibyte_character_does_not_fail_the_read() {
        let mut input = std::io::Cursor::new(format!("{}é\n", "a".repeat(255)).into_bytes());

        let line = read_capped_line(&mut input).expect("lossy, not an error");

        assert!(line.is_some_and(|text| text.starts_with(&"a".repeat(255))));
    }

    struct FailingReader;

    impl std::io::Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("stdin closed under us"))
        }
    }

    #[test]
    fn a_failed_answer_read_is_an_error_not_a_no() {
        let mut output = Vec::new();
        let mut input = std::io::BufReader::new(FailingReader);

        let error = ask_yes_no(&mut output, &mut input, "Add to PATH? ")
            .expect_err("a read failure must surface");

        assert!(error.to_string().contains("stdin closed under us"));
        assert_eq!(output, b"Add to PATH? ", "the question was still asked");
    }

    #[test]
    fn a_yes_answer_is_accepted_and_anything_else_declines() {
        let mut output = Vec::new();

        let yes = ask_yes_no(&mut output, &mut std::io::Cursor::new(b"y\n"), "q ").expect("reads");
        let no = ask_yes_no(&mut output, &mut std::io::Cursor::new(b"n\n"), "q ").expect("reads");

        assert!(yes);
        assert!(!no);
    }
}
