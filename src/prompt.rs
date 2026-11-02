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
    if bytes.last() != Some(&b'\n') {
        input.skip_until(b'\n')?;
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use slugaudit_test_check::{Check, CheckErr, TestResult};

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
    fn a_prompt_answer_is_read_one_line_at_a_time_and_none_at_end_of_input() -> TestResult {
        let mut input = std::io::Cursor::new(b"2\ny\n".to_vec());

        let first = read_capped_line(&mut input).check("first line")?;
        let second = read_capped_line(&mut input).check("second line")?;
        let end = read_capped_line(&mut input).check("end of input")?;

        assert_eq!(first.as_deref(), Some("2\n"));
        assert_eq!(second.as_deref(), Some("y\n"));
        assert_eq!(end, None);
        Ok(())
    }

    #[test]
    fn an_over_long_prompt_line_is_capped_and_its_rest_is_not_read_as_the_next_answer() -> TestResult
    {
        let long = "9".repeat(10_000);
        let mut input = std::io::Cursor::new(format!("{long}\n3\n").into_bytes());

        let capped = read_capped_line(&mut input).check("capped line")?;
        let next = read_capped_line(&mut input).check("next line")?;

        assert_eq!(capped.map(|line| line.len()), Some(256));
        assert_eq!(next.as_deref(), Some("3\n"));
        Ok(())
    }

    #[test]
    fn a_prompt_line_cut_inside_a_multibyte_character_does_not_fail_the_read() -> TestResult {
        let mut input = std::io::Cursor::new(format!("{}é\n", "a".repeat(255)).into_bytes());

        let line = read_capped_line(&mut input).check("lossy, not an error")?;

        assert!(line.is_some_and(|text| text.starts_with(&"a".repeat(255))));
        Ok(())
    }

    struct FailingReader;

    impl std::io::Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("stdin closed under us"))
        }
    }

    #[test]
    fn a_failed_answer_read_is_an_error_not_a_no() -> TestResult {
        let mut output = Vec::new();
        let mut input = std::io::BufReader::new(FailingReader);

        let error = ask_yes_no(&mut output, &mut input, "Add to PATH? ")
            .check_err("a read failure must surface")?;

        assert!(error.to_string().contains("stdin closed under us"));
        assert_eq!(output, b"Add to PATH? ", "the question was still asked");
        Ok(())
    }

    #[test]
    fn a_yes_answer_is_accepted_and_anything_else_declines() -> TestResult {
        let mut output = Vec::new();

        let yes =
            ask_yes_no(&mut output, &mut std::io::Cursor::new(b"y\n"), "q ").check("reads")?;
        let no = ask_yes_no(&mut output, &mut std::io::Cursor::new(b"n\n"), "q ").check("reads")?;

        assert!(yes);
        assert!(!no);
        Ok(())
    }

    #[test]
    fn an_over_long_answer_is_bounded_and_end_of_input_declines() -> TestResult {
        let mut output = Vec::new();
        let long = format!("{}\n", "y".repeat(10_000));

        let capped = ask_yes_no(
            &mut output,
            &mut std::io::Cursor::new(long.into_bytes()),
            "q ",
        )
        .check("reads")?;
        let end =
            ask_yes_no(&mut output, &mut std::io::Cursor::new(Vec::new()), "q ").check("reads")?;

        assert!(!capped, "256 y's are not 'y'");
        assert!(!end, "end of input declines");
        Ok(())
    }

    #[test]
    fn confirm_from_accepts_a_yes_and_asks_the_question() {
        let mut output = Vec::new();
        let mut report = Vec::new();

        let accepted = confirm_from(
            &mut output,
            &mut std::io::Cursor::new(b"yes\n"),
            &mut report,
            "Add? ",
        );

        assert!(accepted);
        assert_eq!(output, b"Add? ");
        assert!(report.is_empty());
    }

    #[test]
    fn confirm_from_declines_and_reports_a_read_failure() {
        let mut output = Vec::new();
        let mut report = Vec::new();
        let mut input = std::io::BufReader::new(FailingReader);

        let accepted = confirm_from(&mut output, &mut input, &mut report, "Add? ");

        assert!(!accepted);
        assert_eq!(
            String::from_utf8_lossy(&report),
            "could not read your answer: stdin closed under us\n"
        );
    }
}
