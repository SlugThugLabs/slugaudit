//! Unit tests for the interactive setup menu.

use super::*;
use crate::menu_render::{MENU, section_label};
use slugaudit_test_check::{Check, TestResult};
use std::path::Path;

#[test]
fn each_listed_number_selects_its_step_and_end_of_input_exits() {
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
        assert_eq!(menu_choice(line), Some(expected), "line {line:?}");
    }
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

    assert_eq!(choice, MenuChoice::Exit);
    let text = String::from_utf8(out).check("utf8 output")?;
    assert_eq!(text.matches("Invalid choice;").count(), 2, "{text}");
    assert_eq!(
        text.matches("Choose an option [1-6]: ").count(),
        2,
        "{text}"
    );
    assert!(!text.contains("Install"), "no banner redraw: {text}");
    assert_eq!(text, INVALID_CHOICE.repeat(2));
    // The banner is rendered only by `run_menu`'s outer loop, once per menu.
    assert_eq!(
        render_menu(&Style::plain())
            .matches("Choose an option")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn an_unlisted_or_non_numeric_menu_answer_is_invalid() {
    let answers = ["0", "7", "", "install", "-1"];

    for answer in answers {
        assert_eq!(menu_choice(Some(answer)), None, "{answer:?}");
    }
}

#[test]
fn plain_style_renders_the_byte_exact_plain_menu() {
    let plain = render_menu(&Style::plain());
    assert_eq!(plain, MENU);
    assert!(!plain.contains('\x1b'));
}

#[test]
fn colored_style_renders_ansi_escapes() {
    let colored = render_menu(&Style::colored());
    assert!(colored.contains('\x1b'));
    assert!(colored.contains("SlugAudit setup"));
}

#[test]
fn section_label_extracts_the_rule_label_and_nothing_else() {
    assert_eq!(section_label("── Setup ──"), Some("Setup"));
    assert_eq!(section_label("── Advanced ──"), Some("Advanced"));
    assert_eq!(section_label("── Exit ────"), Some("Exit"));
    assert_eq!(section_label("not a rule"), None);
}

#[test]
fn other_agent_instructions_name_the_server_and_command() -> TestResult {
    let text =
        other_agent_instructions(Path::new("/opt/slugaudit"), None).check("instructions render")?;
    assert!(text.contains("  Command:      /opt/slugaudit\n"), "{text}");
    assert!(text.contains("\"command\": \"/opt/slugaudit\""), "{text}");
    assert!(text.contains("\"args\": []"), "{text}");
    Ok(())
}

#[test]
fn other_agent_instructions_print_the_resolved_install_location() -> TestResult {
    let text = other_agent_instructions(
        Path::new("slugaudit"),
        Some(Path::new("/custom/home/slugaudit")),
    )
    .check("instructions render")?;
    assert!(
        text.ends_with("Stable command path after install: /custom/home/slugaudit/slugaudit\n"),
        "{text}"
    );
    assert!(!text.contains(".slugthug"), "{text}");
    Ok(())
}

#[test]
fn other_agent_instructions_snippet_is_valid_json_for_awkward_paths() -> TestResult {
    let binary = r#"C:\Program Files\Slug "Audit"\slugaudit.exe"#;
    let text = other_agent_instructions(Path::new(binary), None).check("instructions render")?;
    let start = text.find('{').check("snippet start")?;
    let end = text.rfind('}').check("snippet end")?;
    let value: serde_json::Value =
        serde_json::from_str(&text[start..=end]).check("snippet parses")?;
    assert_eq!(
        value["mcpServers"]["slugaudit"]["command"].as_str(),
        Some(binary)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn other_agent_instructions_refuse_a_non_utf8_binary_path() {
    use std::os::unix::ffi::OsStrExt as _;
    let binary = Path::new(std::ffi::OsStr::from_bytes(b"/opt/\xff/slugaudit"));
    let result = other_agent_instructions(binary, None);
    assert!(
        matches!(result, Err(crate::cli::ConnectError::BinaryPath(_))),
        "{result:?}"
    );
}

#[test]
fn colored_menu_ends_on_the_prompt_without_a_trailing_newline() {
    let colored = render_menu(&Style::colored());
    assert!(!colored.ends_with('\n'), "{colored:?}");
    assert!(colored.contains("Choose an option [1-6]: "));
    assert_eq!(colored.lines().count(), MENU.lines().count());
}
