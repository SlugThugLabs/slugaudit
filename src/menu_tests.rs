//! Unit tests for the interactive setup menu.

use super::*;
use crate::menu_render::{MENU, section_label};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::path::Path;

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
            format!("line {line:?}"),
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

    ensure_eq(&choice, &MenuChoice::Exit, "choice == MenuChoice::Exit")?;
    let text = String::from_utf8(out).check("utf8 output")?;
    ensure_eq(&text.matches("Invalid choice;").count(), &2, &text)?;
    ensure_eq(&text.matches("Choose an option [1-6]: ").count(), &2, &text)?;
    ensure(
        !text.contains("Install"),
        format!("no banner redraw: {text}"),
    )?;
    ensure_eq(
        &text,
        &INVALID_CHOICE.repeat(2),
        "text == INVALID_CHOICE.repeat(2)",
    )?;
    // The banner is rendered only by `run_menu`'s outer loop, once per menu.
    ensure_eq(
        &render_menu(&Style::plain())
            .matches("Choose an option")
            .count(),
        &1,
        "render_menu(&Style::plain()) .matches(\"Choose an option\") .count() == 1",
    )?;
    Ok(())
}

#[test]
fn an_unlisted_or_non_numeric_menu_answer_is_invalid() -> TestResult {
    let answers = ["0", "7", "", "install", "-1"];

    for answer in answers {
        ensure_eq(&menu_choice(Some(answer)), &None, format!("{answer:?}"))?;
    }
    Ok(())
}

#[test]
fn plain_style_renders_the_byte_exact_plain_menu() -> TestResult {
    let plain = render_menu(&Style::plain());
    ensure_eq(&plain, &MENU, "plain == MENU")?;
    ensure(!plain.contains('\x1b'), "!plain.contains('\\x1b')")?;
    Ok(())
}

#[test]
fn colored_style_renders_ansi_escapes() -> TestResult {
    let colored = render_menu(&Style::colored());
    ensure(colored.contains('\x1b'), "colored.contains('\\x1b')")?;
    ensure(
        colored.contains("SlugAudit setup"),
        "colored.contains(\"SlugAudit setup\")",
    )?;
    Ok(())
}

#[test]
fn section_label_extracts_the_rule_label_and_nothing_else() -> TestResult {
    ensure_eq(
        &section_label("── Setup ──"),
        &Some("Setup"),
        "section_label(\"── Setup ──\") == Some(\"Setup\")",
    )?;
    ensure_eq(
        &section_label("── Advanced ──"),
        &Some("Advanced"),
        "section_label(\"── Advanced ──\") == Some(\"Advanced\")",
    )?;
    ensure_eq(
        &section_label("── Exit ────"),
        &Some("Exit"),
        "section_label(\"── Exit ────\") == Some(\"Exit\")",
    )?;
    ensure_eq(
        &section_label("not a rule"),
        &None,
        "section_label(\"not a rule\") == None",
    )?;
    Ok(())
}

#[test]
fn other_agent_instructions_name_the_server_and_command() -> TestResult {
    let text =
        other_agent_instructions(Path::new("/opt/slugaudit"), None).check("instructions render")?;
    ensure(text.contains("  Command:      /opt/slugaudit\n"), &text)?;
    ensure(text.contains("\"command\": \"/opt/slugaudit\""), &text)?;
    ensure(text.contains("\"args\": []"), &text)?;
    Ok(())
}

#[test]
fn other_agent_instructions_print_the_resolved_install_location() -> TestResult {
    let text = other_agent_instructions(
        Path::new("slugaudit"),
        Some(Path::new("/custom/home/slugaudit")),
    )
    .check("instructions render")?;
    ensure(
        text.ends_with("Stable command path after install: /custom/home/slugaudit/slugaudit\n"),
        &text,
    )?;
    ensure(!text.contains(".slugthug"), &text)?;
    Ok(())
}

#[test]
fn other_agent_instructions_snippet_is_valid_json_for_awkward_paths() -> TestResult {
    let binary = r#"C:\Program Files\Slug "Audit"\slugaudit.exe"#;
    let text = other_agent_instructions(Path::new(binary), None).check("instructions render")?;
    let start = text.find('{').check("snippet start")?;
    let end = text.rfind('}').check("snippet end")?;
    let value: serde_json::Value = serde_json::from_str(
        text.get(start..=end)
            .check("snippet range is in the text")?,
    )
    .check("snippet parses")?;
    ensure_eq(
        &value
            .pointer("/mcpServers/slugaudit/command")
            .and_then(serde_json::Value::as_str),
        &Some(binary),
        "value[\"mcpServers\"][\"slugaudit\"][\"command\"].as_str() == Some(binary)",
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn other_agent_instructions_refuse_a_non_utf8_binary_path() -> TestResult {
    use std::os::unix::ffi::OsStrExt as _;
    let binary = Path::new(std::ffi::OsStr::from_bytes(b"/opt/\xff/slugaudit"));
    let result = other_agent_instructions(binary, None);
    ensure(
        matches!(result, Err(crate::cli::ConnectError::BinaryPath(_))),
        format!("{result:?}"),
    )?;
    Ok(())
}

#[test]
fn colored_menu_ends_on_the_prompt_without_a_trailing_newline() -> TestResult {
    let colored = render_menu(&Style::colored());
    ensure(!colored.ends_with('\n'), format!("{colored:?}"))?;
    ensure(
        colored.contains("Choose an option [1-6]: "),
        "colored.contains(\"Choose an option [1-6]: \")",
    )?;
    ensure_eq(
        &colored.lines().count(),
        &MENU.lines().count(),
        "colored.lines().count() == MENU.lines().count()",
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
