//! The interactive prompt of `connect` and `disconnect`: which answers pick
//! an agent, and the typed errors for ended input and a closed stdout.

use super::super::interactive::{answer_from, parse_choice, write_prompt};
use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn a_menu_answer_is_valid_only_when_it_is_a_listed_number() -> TestResult {
    let valid = [("1", 0), (" 3\n", 2)];
    let invalid = ["0", "4", "", "two", "-1"];

    for (answer, index) in valid {
        ensure_eq(
            &parse_choice(answer, 3).check(answer)?,
            &index,
            format!("answer {answer:?}"),
        )?;
    }
    for answer in invalid {
        ensure(
            matches!(parse_choice(answer, 3), Err(ConnectError::InvalidChoice)),
            format!("answer {answer:?} must be rejected"),
        )?;
    }
    Ok(())
}

#[test]
fn end_of_input_at_a_prompt_is_the_dedicated_error() -> TestResult {
    let error = answer_from(None).check_err("end of input must not read as an answer")?;

    ensure(
        matches!(&error, ConnectError::EndOfInput),
        format!("expected EndOfInput, got {error:?}"),
    )?;
    ensure_eq(
        &error.to_string(),
        &"input ended before an answer was given",
        "EndOfInput renders dedicated message",
    )?;
    Ok(())
}

/// A writer that reports a closed pipe, standing in for a stdout whose reader
/// (e.g. `head`) has exited.
struct PipeClosingWriter;

impl std::io::Write for PipeClosingWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_prompt_written_to_a_closed_stdout_is_the_typed_io_error() -> TestResult {
    let mut writer = PipeClosingWriter;

    let error = write_prompt(&mut writer, format_args!("Choose an agent [1-2]: "))
        .check_err("a closed stdout must not be ignored")?;

    let ConnectError::Io(inner) = error else {
        return Err(format!("expected Io, got {error:?}")).check("unexpected outcome");
    };
    ensure_eq(
        &inner.kind(),
        &std::io::ErrorKind::BrokenPipe,
        "preserves BrokenPipe error kind",
    )?;
    Ok(())
}
