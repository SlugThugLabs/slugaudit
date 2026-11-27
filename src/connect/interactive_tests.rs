//! Unit tests for interactive `disconnect`: connected config detection,
//! manual-only config reporting (`goose`, `crush`), and installed CLI-managed
//! agents without local config paths (`agy`, `amp`, verified `q`).

use super::*;
use crate::connect::find_agent;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::fs;

#[test]
fn interactive_disconnect_reports_none_when_no_connections_or_unverified_clis_exist() -> TestResult
{
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let empty_bin = tempfile::tempdir().check("empty bin")?;
    let home = tempfile::tempdir().check("temp home")?;
    let mut out = Vec::new();
    let mut err = Vec::new();

    temp_env::with_var("PATH", Some(empty_bin.path().as_os_str()), || {
        disconnect_interactive_with(Some(home.path()), &mut out, &mut err, &mut || Ok(None))
    })
    .check("empty interactive disconnect succeeds")?;

    let stdout = String::from_utf8(out).check("utf-8 stdout")?;
    ensure_eq(
        &stdout,
        &"No active SlugAudit connections found.\n",
        "empty disconnect reports no active connections",
    )?;
    ensure(err.is_empty(), "no stderr notes when probes succeed")?;
    Ok(())
}

#[test]
fn interactive_disconnect_removes_selected_json_connection() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let empty_bin = tempfile::tempdir().check("empty bin")?;
    let home = tempfile::tempdir().check("temp home")?;
    let cursor_cfg = home.path().join(".cursor/mcp.json");
    fs::create_dir_all(cursor_cfg.parent().check("parent")?).check("mkdir .cursor")?;
    fs::write(
        &cursor_cfg,
        r#"{"mcpServers":{"slugaudit":{"command":"/bin/slugaudit"},"other":{"command":"/bin/other"}}}"#,
    )
    .check("write cursor config")?;

    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut answers = vec![Some("1\n".to_owned())].into_iter();

    temp_env::with_var("PATH", Some(empty_bin.path().as_os_str()), || {
        disconnect_interactive_with(Some(home.path()), &mut out, &mut err, &mut || {
            Ok(answers.next().flatten())
        })
    })
    .check("interactive cursor disconnect succeeds")?;

    let stdout = String::from_utf8(out).check("utf-8 stdout")?;
    ensure(
        stdout.contains("1) Cursor IDE") && stdout.contains("Done."),
        format!("lists Cursor IDE and confirms removal: {stdout}"),
    )?;
    let cursor = find_agent("cursor").check("cursor")?;
    ensure(
        !cursor
            .is_connected(Some(home.path()))
            .check("probe cursor after disconnect")?,
        "cursor config no longer contains slugaudit",
    )?;
    Ok(())
}

#[test]
fn interactive_disconnect_reports_manual_removal_for_goose_and_crush_without_nothing_to_disconnect()
-> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let empty_bin = tempfile::tempdir().check("empty bin")?;
    let home = tempfile::tempdir().check("temp home")?;

    let goose_cfg = home.path().join(".config/goose/config.yaml");
    fs::create_dir_all(goose_cfg.parent().check("parent")?).check("mkdir goose")?;
    fs::write(
        &goose_cfg,
        "extensions:\n  slugaudit:\n    type: stdio\n    cmd: /bin/slugaudit\n",
    )
    .check("write goose config")?;

    let crush_cfg = home.path().join(".config/crush/crushrc");
    fs::create_dir_all(crush_cfg.parent().check("parent")?).check("mkdir crush")?;
    fs::write(&crush_cfg, "mcp add slugaudit --command /bin/slugaudit\n").check("write crushrc")?;

    let mut out = Vec::new();
    let mut err = Vec::new();

    temp_env::with_var("PATH", Some(empty_bin.path().as_os_str()), || {
        disconnect_interactive_with(Some(home.path()), &mut out, &mut err, &mut || Ok(None))
    })
    .check("manual-only disconnect reports instructions cleanly")?;

    let stdout = String::from_utf8(out).check("utf-8 stdout")?;
    ensure(
        stdout.contains("Goose CLI is configured manually")
            && stdout.contains(&goose_cfg.display().to_string()),
        format!("reports manual removal instructions for goose: {stdout}"),
    )?;
    ensure(
        stdout.contains("Crush is configured manually")
            && stdout.contains(&crush_cfg.display().to_string()),
        format!("reports manual removal instructions for crush: {stdout}"),
    )?;
    ensure(
        !stdout.contains("Nothing to disconnect.")
            && !stdout.contains("No active SlugAudit connections found.")
            && !stdout.contains("Choose an agent to disconnect"),
        format!("manual-only agents must not be offered as auto-disconnectable: {stdout}"),
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn interactive_disconnect_surfaces_installed_unverified_cli_agents_and_skips_debian_q() -> TestResult
{
    use std::os::unix::fs::PermissionsExt as _;
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let bin_dir = tempfile::tempdir().check("bin dir")?;
    let home = tempfile::tempdir().check("temp home")?;
    let argv_log = home.path().join("agy-argv.log");

    let agy_script = bin_dir.path().join("agy");
    fs::write(
        &agy_script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
            argv_log.display()
        ),
    )
    .check("write fake agy")?;
    fs::set_permissions(&agy_script, fs::Permissions::from_mode(0o755)).check("chmod agy")?;

    // Plant a Debian-style `python3-q-text-as-data` script at `q`: it must NOT
    // appear as Amazon Q Developer in the disconnect picker.
    let debian_q = bin_dir.path().join("q");
    fs::write(
        &debian_q,
        "#!/bin/sh\nprintf 'q allows performing SQL-like statements on tabular text data (harelba)\\n'\nexit 0\n",
    )
    .check("write fake debian q")?;
    fs::set_permissions(&debian_q, fs::Permissions::from_mode(0o755)).check("chmod debian q")?;

    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut answers = vec![Some("1\n".to_owned())].into_iter();

    temp_env::with_var("PATH", Some(bin_dir.path().as_os_str()), || {
        disconnect_interactive_with(Some(home.path()), &mut out, &mut err, &mut || {
            Ok(answers.next().flatten())
        })
    })
    .check("interactive agy disconnect succeeds")?;

    let stdout = String::from_utf8(out).check("utf-8 stdout")?;
    ensure(
        stdout.contains("1) Antigravity (agy) [CLI installed]") && stdout.contains("Done."),
        format!("lists installed CLI-managed agy candidate and disconnects: {stdout}"),
    )?;
    ensure(
        !stdout.contains("Amazon Q Developer"),
        format!("Debian text-query `q` must not appear as Amazon Q Developer: {stdout}"),
    )?;
    ensure_eq(
        &fs::read_to_string(&argv_log).check("read agy log")?,
        &"mcp remove slugaudit\n",
        "selecting agy runs `agy mcp remove slugaudit`",
    )?;
    Ok(())
}
