//! Unit tests for the interactive setup menu.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn next_choice(
    read: &mut impl FnMut() -> std::io::Result<Option<String>>,
    out: &mut impl std::io::Write,
) -> std::io::Result<MenuChoice> {
    next_choice_with_limit(read, out, Some(MAX_NON_INTERACTIVE_INVALID_ATTEMPTS))
}

#[test]
fn each_listed_number_selects_its_step_and_end_of_input_exits() -> TestResult {
    let cases = [
        (Some("1"), MenuChoice::Install),
        (Some(" 2\n"), MenuChoice::Connect),
        (Some("3"), MenuChoice::Disconnect),
        (Some("4"), MenuChoice::OtherClient),
        (Some("5"), MenuChoice::Serve),
        (Some("6"), MenuChoice::Exit),
        (None, MenuChoice::Exit),
    ];

    for (line, expected) in cases {
        ensure_eq(
            &menu_choice(line),
            &Some(expected),
            format!("menu line {line:?} selects expected step"),
        )?;
    }
    Ok(())
}

#[test]
fn an_invalid_answer_prints_one_error_line_and_reprompts_without_the_banner() -> TestResult {
    let mut answers = vec![
        Some("x".to_owned()),
        Some("9".to_owned()),
        Some("6".to_owned()),
    ]
    .into_iter();
    let mut out = Vec::new();

    let choice = next_choice(&mut || Ok(answers.next().flatten()), &mut out)
        .check("reads and writes succeed")?;

    ensure_eq(&choice, &MenuChoice::Exit, "eventual valid choice exits")?;
    let text = String::from_utf8(out).check("utf8 output")?;
    ensure_eq(
        &text.matches("Invalid choice;").count(),
        &2,
        "two invalid choices print two error lines",
    )?;
    ensure_eq(
        &text.matches("Choose an option [1-6]: ").count(),
        &2,
        "two invalid choices re-prompt twice",
    )?;
    ensure(
        !text.contains("Install"),
        format!("no banner redraw: {text}"),
    )?;
    ensure_eq(
        &text,
        &format!("{INVALID_CHOICE}{CHOICE_PROMPT}").repeat(2),
        "output matches exact repeated invalid-choice prompt",
    )?;
    // The banner is rendered only by `run_menu`'s outer loop, once per menu.
    ensure_eq(
        &render_menu(&Style::plain())
            .matches("Choose an option")
            .count(),
        &1,
        "plain menu renders prompt once",
    )?;
    Ok(())
}

#[test]
fn an_unlisted_or_non_numeric_menu_answer_is_invalid() -> TestResult {
    let answers = ["0", "7", "", "install", "-1"];

    for answer in answers {
        ensure_eq(
            &menu_choice(Some(answer)),
            &None,
            format!("unlisted menu answer {answer:?} is rejected"),
        )?;
    }
    Ok(())
}

#[test]
fn plain_style_renders_the_byte_exact_plain_menu() -> TestResult {
    let plain = render_menu(&Style::plain());
    ensure_eq(&plain, &MENU, "plain style matches unstyled menu constant")?;
    ensure(
        !plain.contains('\x1b'),
        "plain menu contains no ANSI escapes",
    )?;
    Ok(())
}

#[test]
fn colored_style_renders_ansi_escapes() -> TestResult {
    let colored = render_menu(&Style::colored());
    ensure(
        colored.contains('\x1b'),
        "colored menu includes ANSI escape sequences",
    )?;
    ensure(
        colored.contains("SlugAudit setup"),
        "colored menu includes header title",
    )?;
    Ok(())
}

#[test]
fn section_label_extracts_the_rule_label_and_nothing_else() -> TestResult {
    ensure_eq(
        &section_label("── Setup ──"),
        &Some("Setup"),
        "extracts Setup section label",
    )?;
    ensure_eq(
        &section_label("── Advanced ──"),
        &Some("Advanced"),
        "extracts Advanced section label",
    )?;
    ensure_eq(
        &section_label("── Exit ────"),
        &Some("Exit"),
        "extracts Exit section label",
    )?;
    ensure_eq(
        &section_label("not a rule"),
        &None,
        "non-rule line yields no section label",
    )?;
    Ok(())
}

#[test]
fn colored_menu_ends_on_the_prompt_without_a_trailing_newline() -> TestResult {
    let colored = render_menu(&Style::colored());
    ensure(!colored.ends_with('\n'), format!("{colored:?}"))?;
    ensure(
        colored.contains("Choose an option [1-6]: "),
        "colored menu retains prompt text",
    )?;
    ensure_eq(
        &colored.lines().count(),
        &MENU.lines().count(),
        "colored menu preserves exact line count",
    )?;
    Ok(())
}

#[test]
fn non_interactive_menu_bounds_consecutive_invalid_attempts() -> TestResult {
    let mut out = Vec::new();
    let error = next_choice_with_limit(
        &mut || Ok(Some("invalid\n".to_owned())),
        &mut out,
        Some(MAX_NON_INTERACTIVE_INVALID_ATTEMPTS),
    )
    .check_err("repeated invalid non-interactive choices must fail boundedly")?;
    ensure_eq(
        &error.kind(),
        &std::io::ErrorKind::InvalidInput,
        "returns InvalidInput after bound",
    )?;
    Ok(())
}
