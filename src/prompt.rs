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
}
