//! Unit tests for `connect::exec`: connect/disconnect CLI dispatch, JSON
//! fallback sequencing, and `/usr/bin/q` collision avoidance.

use super::super::config_file::write_atomic;
use super::*;
use crate::connect::find_agent;
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
        "goose without config home reports NothingToDo",
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

        ensure_eq(
            &outcome,
            &DisconnectOutcome::NothingToDo,
            format!("{id} with no writable section reports NothingToDo"),
        )?;
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn non_amazon_q_binary_on_path_is_not_invoked_as_amazon_q_during_connect_or_disconnect()
-> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let bin_dir = temp.path().join("bin");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&bin_dir).check("mkdir bin")?;
    std::fs::create_dir_all(&home).check("mkdir home")?;
    let action_log = temp.path().join("q-actions.log");

    // Fake Debian `python3-q-text-as-data`: prints its help text on `mcp --help`
    // and records to `action_log` if `mcp add` or `mcp remove` is ever invoked.
    let q_script = bin_dir.join("q");
    let script = format!(
        "#!/bin/sh\nif [ \"$*\" = \"mcp --help\" ]; then\n  printf 'q allows performing SQL-like statements on tabular text data (https://harelba.github.io/q/)\\n'\n  exit 0\nfi\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        action_log.display()
    );
    std::fs::write(&q_script, script).check("write fake debian q")?;
    std::fs::set_permissions(&q_script, std::fs::Permissions::from_mode(0o755))
        .check("chmod fake debian q")?;

    let q_agent = find_agent("q").check("q is registered")?;
    temp_env::with_var("PATH", Some(bin_dir.as_os_str()), || -> TestResult {
        let connect_err = connect_agent(&q_agent, Path::new("/bin/slugaudit"), Some(&home))
            .check_err("unrelated q binary must not be treated as Amazon Q CLI")?;
        ensure(
            matches!(connect_err, ConnectError::UnsupportedAgent { .. }),
            format!("expected UnsupportedAgent when Amazon Q is absent, got {connect_err:?}"),
        )?;

        let disconnect = disconnect_agent(&q_agent, Some(&home))
            .check("disconnect skips unrelated q binary cleanly")?;
        ensure_eq(
            &disconnect,
            &DisconnectOutcome::NothingToDo,
            "unrelated q binary reports NothingToDo on disconnect",
        )?;
        Ok(())
    })?;

    ensure(
        !action_log.exists(),
        "unrelated /usr/bin/q binary must never be invoked with `mcp add` or `mcp remove`",
    )?;
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
        "pi reports Disconnected",
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
        "fallback JSON removal reports Disconnected",
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
