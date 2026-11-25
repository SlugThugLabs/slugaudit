use super::*;
use crate::connect_agents::find_agent;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn an_atomic_write_for_a_path_without_a_parent_reports_the_home_unavailable() -> TestResult {
    let error = write_atomic(Path::new("/"), b"{}").check_err("root has no parent")?;

    ensure(
        matches!(error, ConnectError::HomeUnavailable),
        format!("expected HomeUnavailable, got {error:?}"),
    )?;
    Ok(())
}

#[test]
fn disconnecting_without_a_config_home_reports_nothing_to_do() -> TestResult {
    let goose = find_agent("goose").check("goose is registered")?;

    let outcome = disconnect_agent(&goose, None).check("a missing home is not a failure")?;

    ensure_eq(
        &outcome,
        &DisconnectOutcome::NothingToDo,
        "outcome == DisconnectOutcome::NothingToDo",
    )?;
    Ok(())
}

#[test]
fn disconnecting_an_entry_with_no_writable_section_or_cli_reports_nothing_to_do() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;

    for id in ["goose", "crush"] {
        let agent = find_agent(id).check(id)?;
        let outcome =
            disconnect_agent(&agent, Some(temp.path())).check("the no-op must not fail")?;

        ensure_eq(&outcome, &DisconnectOutcome::NothingToDo, id.to_string())?;
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn disconnecting_cli_direct_with_fallback_runs_cli_before_removing_json() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home");
    let config = home.join(".pi/agent/mcp.json");
    std::fs::create_dir_all(config.parent().check("parent")?).check("mkdir")?;
    std::fs::write(
        &config,
        r#"{"mcpServers":{"slugaudit":{"command":"/bin/s"}}}"#,
    )
    .check("write config")?;

    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).check("mkdir bin")?;
    let cli = bin_dir.join("pi");
    // The fake CLI succeeds only if `slugaudit` is still present in the JSON
    // config when `pi mcp remove slugaudit` runs.
    let script = format!(
        "#!/bin/sh\nread -r body < '{}'\ncase \"$body\" in\n  *'\"slugaudit\"'*) exit 0 ;;\n  *) exit 1 ;;\nesac\n",
        config.display()
    );
    std::fs::write(&cli, script).check("write fake pi")?;
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).check("chmod")?;

    let pi = find_agent("pi").check("pi is registered")?;
    let outcome = temp_env::with_var("PATH", Some(bin_dir.as_os_str()), || {
        disconnect_agent(&pi, Some(&home))
    })
    .check("cli remove must run before json removal")?;

    ensure_eq(
        &outcome,
        &DisconnectOutcome::Disconnected,
        "outcome == Disconnected",
    )?;
    let remaining = std::fs::read_to_string(&config).check("read config")?;
    ensure(
        !remaining.contains("slugaudit"),
        format!("json entry must also be removed: {remaining}"),
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn disconnecting_cli_direct_with_fallback_removes_json_when_cli_fails() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home");
    let config = home.join(".pi/agent/mcp.json");
    std::fs::create_dir_all(config.parent().check("parent")?).check("mkdir")?;
    std::fs::write(
        &config,
        r#"{"mcpServers":{"slugaudit":{"command":"/bin/s"}}}"#,
    )
    .check("write config")?;

    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).check("mkdir bin")?;
    let cli = bin_dir.join("pi");
    std::fs::write(&cli, "#!/bin/sh\necho 'remove failed' >&2\nexit 1\n").check("write fake pi")?;
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).check("chmod")?;

    let pi = find_agent("pi").check("pi is registered")?;
    let outcome = temp_env::with_var("PATH", Some(bin_dir.as_os_str()), || {
        disconnect_agent(&pi, Some(&home))
    })
    .check("fallback json removal must succeed despite cli failure")?;

    ensure_eq(
        &outcome,
        &DisconnectOutcome::Disconnected,
        "outcome == Disconnected",
    )?;
    let remaining = std::fs::read_to_string(&config).check("read config")?;
    ensure(
        !remaining.contains("slugaudit"),
        format!("json entry must be removed: {remaining}"),
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn disconnecting_cli_direct_with_fallback_returns_cli_error_when_json_absent() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).check("mkdir home")?;

    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).check("mkdir bin")?;
    let cli = bin_dir.join("pi");
    std::fs::write(&cli, "#!/bin/sh\necho 'remove failed' >&2\nexit 1\n").check("write fake pi")?;
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).check("chmod")?;

    let pi = find_agent("pi").check("pi is registered")?;
    let err = temp_env::with_var("PATH", Some(bin_dir.as_os_str()), || {
        disconnect_agent(&pi, Some(&home))
    })
    .check_err("cli error must surface when json had no entry")?;

    ensure(
        matches!(err, ConnectError::RemoveFailed { ref cli, .. } if cli == "pi"),
        format!("expected RemoveFailed for pi, got {err:?}"),
    )?;
    Ok(())
}
