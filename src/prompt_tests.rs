use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn only_y_or_yes_in_any_case_accepts_a_confirmation() -> TestResult {
    let accepted = ["y", "Y", "yes", " YES\n"];
    let declined = ["", "n", "no", "yep", "y es"];

    for answer in accepted {
        ensure(is_yes(answer), format!("{answer:?} must accept"))?;
    }
    for answer in declined {
        ensure(!is_yes(answer), format!("{answer:?} must decline"))?;
    }
    Ok(())
}

#[test]
fn a_prompt_answer_is_read_one_line_at_a_time_and_none_at_end_of_input() -> TestResult {
    let mut input = std::io::Cursor::new(b"2\ny\n".to_vec());

    let first = read_capped_line(&mut input).check("first line")?;
    let second = read_capped_line(&mut input).check("second line")?;
    let end = read_capped_line(&mut input).check("end of input")?;

    ensure_eq(
        &first.as_deref(),
        &Some("2\n"),
        "first.as_deref() == Some(\"2\\n\")",
    )?;
    ensure_eq(
        &second.as_deref(),
        &Some("y\n"),
        "second.as_deref() == Some(\"y\\n\")",
    )?;
    ensure_eq(&end, &None, "end == None")?;
    Ok(())
}

#[test]
fn an_over_long_prompt_line_is_capped_and_its_rest_is_not_read_as_the_next_answer() -> TestResult {
    let long = "9".repeat(10_000);
    let mut input = std::io::Cursor::new(format!("{long}\n3\n").into_bytes());

    let capped = read_capped_line(&mut input).check("capped line")?;
    let next = read_capped_line(&mut input).check("next line")?;

    ensure_eq(
        &capped.map(|line| line.len()),
        &Some(256),
        "capped.map(|line| line.len()) == Some(256)",
    )?;
    ensure_eq(
        &next.as_deref(),
        &Some("3\n"),
        "next.as_deref() == Some(\"3\\n\")",
    )?;
    Ok(())
}

#[test]
fn a_prompt_line_cut_inside_a_multibyte_character_does_not_fail_the_read() -> TestResult {
    let mut input = std::io::Cursor::new(format!("{}é\n", "a".repeat(255)).into_bytes());

    let line = read_capped_line(&mut input).check("lossy, not an error")?;

    ensure(
        line.is_some_and(|text| text.starts_with(&"a".repeat(255))),
        "line.is_some_and(|text| text.starts_with(&\"a\".repeat(255)))",
    )?;
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

    ensure(
        error.to_string().contains("stdin closed under us"),
        "error.to_string().contains(\"stdin closed under us\")",
    )?;
    ensure_eq(&output, &b"Add to PATH? ", "the question was still asked")?;
    Ok(())
}

#[test]
fn a_yes_answer_is_accepted_and_anything_else_declines() -> TestResult {
    let mut output = Vec::new();

    let yes = ask_yes_no(&mut output, &mut std::io::Cursor::new(b"y\n"), "q ").check("reads")?;
    let no = ask_yes_no(&mut output, &mut std::io::Cursor::new(b"n\n"), "q ").check("reads")?;

    ensure(yes, "yes")?;
    ensure(!no, "!no")?;
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

    ensure(!capped, "256 y's are not 'y'")?;
    ensure(!end, "end of input declines")?;
    Ok(())
}

#[test]
fn confirm_from_accepts_a_yes_and_asks_the_question() -> TestResult {
    let mut output = Vec::new();
    let mut report = Vec::new();

    let accepted = confirm_from(
        &mut output,
        &mut std::io::Cursor::new(b"yes\n"),
        &mut report,
        "Add? ",
    );

    ensure(accepted, "accepted")?;
    ensure_eq(&output, &b"Add? ", "output == b\"Add? \"")?;
    ensure(report.is_empty(), "report.is_empty()")?;
    Ok(())
}

#[test]
fn confirm_from_declines_and_reports_a_read_failure() -> TestResult {
    let mut output = Vec::new();
    let mut report = Vec::new();
    let mut input = std::io::BufReader::new(FailingReader);

    let accepted = confirm_from(&mut output, &mut input, &mut report, "Add? ");

    ensure(!accepted, "!accepted")?;
    ensure_eq(
        &String::from_utf8_lossy(&report),
        &"could not read your answer: stdin closed under us\n",
        "String::from_utf8_lossy(&report) == \"could not read your answer: stdin closed under us\\n\"",
    )?;
    Ok(())
}
