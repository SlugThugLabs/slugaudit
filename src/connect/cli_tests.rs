//! Unit tests for `connect::cli`: command display quoting, spawn errors, and
//! client command timeouts.

use super::*;
use slugaudit_test_check::{Check, TestResult, ensure_eq};

#[cfg(unix)]
#[test]
fn an_add_that_outlives_its_deadline_is_a_timeout_error() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let dir = tempfile::tempdir().check("dir")?;
    let cli = dir.path().join("fake-client");
    std::fs::write(&cli, "#!/bin/sh\nexec /bin/sleep 30\n").check("write")?;
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).check("chmod")?;
    let cli = cli.display().to_string();

    let result = run_add_within(&cli, &["mcp".to_string()], Duration::from_millis(300));

    let Err(ConnectError::AgentTimeout { cli: named, .. }) = result else {
        return Err(format!("expected AgentTimeout, got {result:?}")).check("unexpected outcome");
    };
    ensure_eq(&named, &cli, "timeout error names timed-out executable")?;
    Ok(())
}

#[test]
fn an_add_that_cannot_be_spawned_is_reported_as_an_agent_command_failure() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let empty = tempfile::tempdir().check("empty bin dir")?;
    let args = ["mcp", "add", "slugaudit", "--", "/bin/slugaudit"].map(str::to_string);

    let result = temp_env::with_var("PATH", Some(empty.path().as_os_str()), || {
        run_add("claude", &args)
    });

    let Err(ConnectError::AgentCommand { cli, inner }) = result else {
        return Err(format!("expected AgentCommand, got {result:?}")).check("unexpected outcome");
    };
    ensure_eq(&cli, &"claude", "command failure names claude executable")?;
    ensure_eq(
        &inner.kind(),
        &std::io::ErrorKind::NotFound,
        format!("the missing executable must be the cause: {inner}"),
    )?;
    Ok(())
}

#[test]
fn a_displayed_command_quotes_arguments_a_shell_would_split() -> TestResult {
    let split = display_command(
        "claude",
        &["mcp", "add", "slugaudit", "--", "/bin/slug audit"].map(str::to_string),
    );

    ensure_eq(
        &split,
        &"claude mcp add slugaudit -- '/bin/slug audit'",
        "quotes argument containing whitespace",
    )?;

    let apostrophe = display_command("claude", &["it's".to_string()]);

    ensure_eq(
        &apostrophe,
        &"claude 'it'\\''s'",
        "escapes embedded single quote for POSIX shells",
    )?;
    Ok(())
}
