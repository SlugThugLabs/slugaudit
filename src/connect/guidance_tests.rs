use super::*;
use crate::connect::find_agent;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

#[test]
fn a_rejected_add_points_at_the_client_output_instead_of_an_install() -> TestResult {
    let pi = find_agent("pi").check("pi is registered")?;
    let attempted = "pi mcp add slugaudit -- /bin/slugaudit";

    let lines = failure_lines(
        &pi,
        Path::new("/bin/slugaudit"),
        attempted,
        Some(attempted),
        "exit status: 1",
    );
    let text = lines.join("\n");

    ensure(text.contains("rejected the registration"), &text)?;
    ensure(text.contains("read the failure above"), &text)?;
    ensure(
        !text.contains("install `pi`"),
        format!("an installed CLI must not be described as missing: {text}"),
    )?;
    Ok(())
}

#[test]
fn only_an_absent_cli_is_advised_to_install() -> TestResult {
    let pi = find_agent("pi").check("pi is registered")?;

    let error = cli_absent(
        &pi,
        Path::new("/bin/slugaudit"),
        "pi",
        "pi mcp add slugaudit -- /bin/slugaudit",
    );
    let text = error.to_string();

    ensure(text.contains("`pi` was not found on PATH"), &text)?;
    ensure(text.contains("install `pi`"), &text)?;
    ensure(!text.contains("rejected the registration"), &text)?;
    Ok(())
}

#[test]
fn the_fallback_note_reads_as_one_sentence_for_each_reason() -> TestResult {
    let pi = find_agent("pi").check("pi is registered")?;
    let home = Path::new("/home/u");
    let written = target(&pi, Some(home));

    let absent = fallback_note(&pi, Some(home), "pi", "was not found on PATH");
    let rejected = fallback_note(
        &pi,
        Some(home),
        "pi",
        "rejected `mcp add` (`pi mcp add` exited with exit status: 1)",
    );

    ensure_eq(
        &absent,
        &format!("`pi` was not found on PATH, so slugaudit wrote {written} instead."),
        "fallback note formats missing-CLI reason as one sentence",
    )?;
    ensure_eq(
        &rejected,
        &format!(
            "`pi` rejected `mcp add` (`pi mcp add` exited with exit status: 1), \
             so slugaudit wrote {written} instead."
        ),
        "fallback note formats rejected-add reason as one sentence",
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
        "JSON snippet preserves paths with spaces, quotes, and backslashes",
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
        matches!(result, Err(ConnectError::BinaryPath(_))),
        format!("expected BinaryPath error, got {result:?}"),
    )?;
    Ok(())
}
