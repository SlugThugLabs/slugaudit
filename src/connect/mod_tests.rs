//! Tests for connect, disconnect, and agent detection.
//!
//! Client CLIs are never the machine's: `AgentDef::is_connected`, `is_installed`
//! and the `CliDirect` route all resolve executables through `PATH`, so every
//! test below that can reach one runs under [`with_no_cli_on_path`] (an empty
//! `PATH`) or [`with_fake_clis_on_path`] (a directory holding only the fakes
//! that test wrote). Prepending to the inherited `PATH` is not enough: a real
//! `codex`/`claude` further along it would still be found and executed, and
//! could rewrite the developer's own client config. Tests driving
//! [`run_connect`] additionally pin both homes with [`with_config_home`].

use super::*;
use crate::util::TEST_ENV_LOCK;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::fs;

#[test]
fn json_agent_connect_and_disconnect_modifies_config() -> TestResult {
    with_no_cli_on_path(|| -> TestResult {
        let temp = tempfile::tempdir().check("tempdir")?;
        let home = temp.path();
        let agent = find_agent("cursor").check("cursor agent")?;
        let binary = Path::new("/bin/slugaudit");

        ensure(
            !agent.is_connected(Some(home)).check("probe config")?,
            "cursor must start disconnected in empty home",
        )?;

        connect_agent(&agent, binary, Some(home)).check("connect cursor")?;
        ensure(
            agent.is_connected(Some(home)).check("probe config")?,
            "cursor must report connected after connect_agent",
        )?;

        let cfg_path = home.join(".cursor/mcp.json");
        let content = fs::read_to_string(&cfg_path).check("read config")?;
        ensure(
            content.contains("slugaudit"),
            "written cursor config includes slugaudit key",
        )?;
        ensure(
            content.contains("/bin/slugaudit"),
            "written cursor config includes binary path",
        )?;

        let outcome = disconnect_agent(&agent, Some(home)).check("disconnect cursor")?;
        ensure_eq(
            &outcome,
            &DisconnectOutcome::Disconnected,
            "disconnecting cursor reports Disconnected",
        )?;
        ensure(
            !agent.is_connected(Some(home)).check("probe config")?,
            "cursor reports disconnected after removal",
        )?;
        let after = fs::read_to_string(&cfg_path).check("read config after")?;
        ensure(
            !after.contains("/bin/slugaudit"),
            "removed config no longer references binary",
        )?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn zed_agent_connect_and_disconnect_modifies_settings() -> TestResult {
    with_no_cli_on_path(|| -> TestResult {
        let temp = tempfile::tempdir().check("tempdir")?;
        let home = temp.path();
        let agent = find_agent("zed").check("zed agent")?;
        let binary = Path::new("/bin/slugaudit");

        ensure(
            !agent.is_connected(Some(home)).check("probe config")?,
            "zed starts disconnected in empty home",
        )?;

        connect_agent(&agent, binary, Some(home)).check("connect zed")?;
        ensure(
            agent.is_connected(Some(home)).check("probe config")?,
            "zed reports connected after connect_agent",
        )?;

        let cfg_path = home.join(".config/zed/settings.json");
        let content = fs::read_to_string(&cfg_path).check("read zed settings")?;
        ensure(
            content.contains("context_servers"),
            "zed config uses context_servers section",
        )?;
        ensure(
            content.contains("slugaudit"),
            "zed config contains slugaudit entry",
        )?;

        disconnect_agent(&agent, Some(home)).check("disconnect zed")?;
        ensure(
            !agent.is_connected(Some(home)).check("probe config")?,
            "zed reports disconnected after removal",
        )?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn detected_agents_filters_by_installed_presence() -> TestResult {
    with_no_cli_on_path(|| -> TestResult {
        let temp = tempfile::tempdir().check("tempdir")?;
        let home = temp.path();
        fs::create_dir_all(home.join(".cursor")).check("mkdir .cursor")?;
        let detected = detected_agents(Some(home));
        ensure(
            detected.agents.iter().any(|a| a.id == "cursor"),
            "cursor is detected when .cursor directory exists",
        )?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn a_config_path_that_cannot_be_examined_is_returned_not_read_as_absent() -> TestResult {
    with_no_cli_on_path(|| {
        let temp = tempfile::tempdir().check("tempdir")?;
        let home = temp.path();
        fs::write(home.join(".cursor"), b"a file where a directory belongs").check("plant file")?;
        let agent = find_agent("cursor").check("cursor agent")?;

        let failure = agent
            .is_installed(Some(home))
            .check_err("a path under a file cannot be examined")?;

        ensure_eq(&failure.agent, &"cursor", "probe failure names cursor")?;
        ensure_eq(
            &failure.path,
            &home.join(".cursor/mcp.json"),
            "probe failure reports exact cursor config path",
        )?;
        ensure_eq(
            &failure.source.kind(),
            &std::io::ErrorKind::NotADirectory,
            "probe failure keeps NotADirectory I/O kind",
        )?;
        let detection = detected_agents(Some(home));
        ensure(
            detection.failures.iter().any(|f| f.agent == "cursor"),
            "detected_agents records cursor in failures list",
        )?;
        ensure(
            !detection.agents.iter().any(|a| a.id == "cursor"),
            "failed probe does not list cursor as installed",
        )?;
        Ok(())
    })
}

fn fake_agent_cli(name: &str, exit_code: i32) -> TestResult<tempfile::TempDir> {
    let dir = tempfile::tempdir().check("bin dir")?;
    let script = dir.path().join(name);
    fs::write(&script, format!("#!/bin/sh\nexit {exit_code}\n")).check("write fake cli")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).check("metadata")?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).check("chmod fake cli")?;
    }
    Ok(dir)
}

/// Runs `f` with `PATH` pointing at exactly `bin_dir`: the inherited entries
/// are dropped, so a real client CLI installed on this machine cannot be found
/// by `which` or executed by `Command::new` while the closure runs.
fn with_fake_clis_on_path<T>(
    bin_dir: &std::path::Path,
    f: impl FnOnce() -> TestResult<T>,
) -> TestResult<T> {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_var("PATH", Some(bin_dir.as_os_str()), f)
}

/// Runs `f` with `PATH` pointing at an empty directory, so no client CLI can
/// be resolved, matched by `which`, or executed at all.
fn with_no_cli_on_path<T>(f: impl FnOnce() -> TestResult<T>) -> TestResult<T> {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let empty = tempfile::tempdir().check("empty bin dir")?;
    temp_env::with_var("PATH", Some(empty.path().as_os_str()), f)
}

/// Points both SlugAudit's own home and the client-config home at `home`, so a
/// test driving [`run_connect`] cannot read the developer's real
/// `~/.claude.json` or install into the real `~/.slugthug`.
fn with_config_home<T>(home: &std::path::Path, f: impl FnOnce() -> TestResult<T>) -> TestResult<T> {
    temp_env::with_vars(
        [
            ("SLUGTHUG_CONFIG_HOME", Some(home.as_os_str())),
            ("SLUGTHUG_HOME", Some(home.as_os_str())),
        ],
        f,
    )
}

#[test]
fn connect_registers_with_a_fake_agent_cli_successfully() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let bin_dir = fake_agent_cli("claude", 0)?;
    with_fake_clis_on_path(bin_dir.path(), || -> TestResult {
        with_config_home(temp.path(), || -> TestResult {
            let result = run_connect(Some("claude"));
            result.as_ref().check("claude registration succeeds")?;
            Ok(())
        })?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn connect_surfaces_an_add_failure_from_the_agent_cli() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let bin_dir = fake_agent_cli("claude", 1)?;
    with_fake_clis_on_path(bin_dir.path(), || -> TestResult {
        with_config_home(temp.path(), || -> TestResult {
            let result = run_connect(Some("claude"));
            match &result {
                Err(error @ ConnectError::AddFailedNoOutput { cli, .. }) => {
                    ensure_eq(&cli, &"claude", "add failure names claude executable")?;
                    ensure(
                        error.to_string().contains("printed no output"),
                        format!("a silent client must not be described as having output: {error}"),
                    )?;
                }
                other => {
                    return Err(format!("expected AddFailedNoOutput, got {other:?}"))
                        .check("unexpected outcome");
                }
            }
            Ok(())
        })?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn connect_writes_the_config_file_for_interactive_only_clients() -> TestResult {
    with_no_cli_on_path(|| -> TestResult {
        let temp = tempfile::tempdir().check("tempdir")?;
        let home = temp.path();
        let agent = find_agent("omp").check("omp")?;
        ensure_eq(
            &agent.strategy,
            &Strategy::CliInteractiveOnly,
            "omp uses CliInteractiveOnly strategy",
        )?;

        connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home)).check("connect omp")?;
        let content = fs::read_to_string(home.join(".omp/agent/mcp.json")).check("omp config")?;
        ensure(
            content.contains("slugaudit"),
            "omp config contains slugaudit entry",
        )?;
        ensure(
            content.contains("/bin/slugaudit"),
            "omp config contains binary path",
        )?;
        ensure(
            agent.is_connected(Some(home)).check("probe config")?,
            "omp reports connected after config write",
        )?;

        disconnect_agent(&agent, Some(home)).check("disconnect omp")?;
        ensure(
            !agent.is_connected(Some(home)).check("probe config")?,
            "omp reports disconnected after removal",
        )?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn config_file_clients_write_their_documented_filename_without_a_cli() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let home = temp.path();
    let warp = find_agent("warp").check("warp")?;
    ensure_eq(&warp.cli, &Some("oz"), "oz is detection-only for warp")?;

    connect_agent(&warp, Path::new("/bin/slugaudit"), Some(home)).check("connect warp")?;
    let content = fs::read_to_string(home.join(".warp/.mcp.json")).check("warp config")?;
    ensure(
        content.contains("mcpServers"),
        "warp config uses mcpServers section",
    )?;
    ensure(
        content.contains("/bin/slugaudit"),
        "warp config includes binary path",
    )?;
    Ok(())
}

#[test]
fn connect_uses_the_cli_when_a_scriptable_executable_exists() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let home = temp.path();
    let agent = find_agent("pi").check("pi")?;
    let bin_dir = fake_agent_cli("pi", 0)?;
    with_fake_clis_on_path(bin_dir.path(), || -> TestResult {
        connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home))
            .check("connect pi via cli")?;
        Ok(())
    })?;
    ensure(
        !home.join(".pi/agent/mcp.json").exists(),
        "a present, succeeding CLI must win over the file fallback",
    )?;
    Ok(())
}

fn write_recording_cli(script: &Path, log: &Path) -> TestResult {
    fs::write(
        script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> {}\nexit 0\n",
            log.display()
        ),
    )
    .check("write recording cli")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(script).check("metadata")?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(script, perms).check("chmod recording cli")?;
    }
    Ok(())
}

#[test]
fn connect_emits_the_transport_grammar_each_client_requires() -> TestResult {
    let cases: &[(&str, &str)] = &[
        (
            "openhands",
            "mcp\nadd\nslugaudit\n--transport\nstdio\n/bin/slugaudit\n",
        ),
        (
            "droid",
            "mcp\nadd\n--type\nstdio\nslugaudit\n/bin/slugaudit\n",
        ),
        (
            "bob",
            "mcp\nadd\n--transport\nstdio\n--scope\nglobal\nslugaudit\n/bin/slugaudit\n",
        ),
    ];
    for (id, expected) in cases {
        let temp = tempfile::tempdir().check("tempdir")?;
        let home = temp.path().join("home");
        fs::create_dir_all(&home).check("home")?;
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).check("bin dir")?;
        let log = temp.path().join("args.log");
        write_recording_cli(&bin_dir.join(id), &log)?;

        let agent = find_agent(id).check("agent")?;
        with_fake_clis_on_path(&bin_dir, || -> TestResult {
            connect_agent(&agent, Path::new("/bin/slugaudit"), Some(&home)).check("connect")?;
            Ok(())
        })?;
        let recorded = fs::read_to_string(&log).check("args log")?;
        ensure_eq(
            &&recorded,
            &expected,
            format!("{id} emitted the wrong argv"),
        )?;
    }
    Ok(())
}

#[test]
fn connect_falls_back_to_the_config_file_when_the_executable_is_absent_or_fails() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let home = temp.path();
    let agent = find_agent("pi").check("pi")?;
    let config = home.join(".pi/agent/mcp.json");

    with_no_cli_on_path(|| -> TestResult {
        connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home))
            .check("connect pi without cli")?;
        ensure(
            config.is_file(),
            "the documented file path must be used when the CLI is absent",
        )?;
        disconnect_agent(&agent, Some(home)).check("disconnect pi")?;
        ensure(
            !agent.is_connected(Some(home)).check("probe config")?,
            "pi reports disconnected after removing fallback config",
        )?;
        Ok(())
    })?;

    fs::remove_file(&config).check("remove config before the fallback")?;

    let bin_dir = fake_agent_cli("pi", 1)?;
    with_fake_clis_on_path(bin_dir.path(), || -> TestResult {
        connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home))
            .check("fall back after a rejected add")?;
        Ok(())
    })?;
    ensure(
        config.is_file(),
        "a CLI that rejects `mcp add` must still fall back to the config file",
    )?;
    Ok(())
}

#[test]
fn an_unknown_agent_name_is_rejected_by_connect_and_disconnect() -> TestResult {
    let connect = run_connect(Some("no-such-agent"));
    let disconnect = run_disconnect(Some("no-such-agent"));

    for result in [connect, disconnect] {
        ensure(
            matches!(&result, Err(ConnectError::UnknownAgent(name)) if name == "no-such-agent"),
            format!("expected UnknownAgent, got {result:?}"),
        )?;
    }
    Ok(())
}

#[test]
fn disconnect_surfaces_a_remove_failure_from_the_agent_cli() -> TestResult {
    let bin_dir = fake_agent_cli("claude", 1)?;
    let agent = find_agent("claude").check("claude agent")?;

    let result = with_fake_clis_on_path(bin_dir.path(), || {
        Ok(super::cli::run_remove(&agent, "claude"))
    })?;

    let Err(ConnectError::RemoveFailed { cli, .. }) = result else {
        return Err(format!("expected RemoveFailed, got {result:?}")).check("unexpected outcome");
    };
    ensure_eq(&cli, &"claude", "remove failure names claude executable")?;
    Ok(())
}

#[test]
fn an_agent_cli_that_cannot_be_executed_is_reported_as_a_command_failure() -> TestResult {
    let agent = find_agent("claude").check("claude agent")?;

    let result = with_no_cli_on_path(|| Ok(super::cli::run_remove(&agent, "claude")))?;

    let Err(ConnectError::AgentCommand { cli, .. }) = result else {
        return Err(format!("expected AgentCommand, got {result:?}")).check("unexpected outcome");
    };
    ensure_eq(&cli, &"claude", "command failure names claude executable")?;
    Ok(())
}

#[test]
fn verify_client_uses_verify_cli_and_skips_detection_only_clis() -> TestResult {
    let warp = find_agent("warp").check("warp")?;
    let cursor = find_agent("cursor").check("cursor")?;
    let claude = find_agent("claude").check("claude")?;
    let zed = find_agent("zed").check("zed")?;

    ensure_eq(
        &verify_client(&warp),
        &None,
        "warp's detection-only oz must not be suggested for mcp list",
    )?;
    ensure_eq(
        &verify_client(&cursor),
        &Some("agent"),
        "cursor must suggest its verify_cli",
    )?;
    ensure_eq(
        &verify_client(&claude),
        &Some("claude"),
        "CliDirect agents suggest their cli",
    )?;
    ensure_eq(&verify_client(&zed), &None, "zed has no CLI")?;
    Ok(())
}

#[path = "binary_tests.rs"]
mod binary_tests;

#[path = "prompt_tests.rs"]
mod prompt_tests;

#[path = "refusal_tests.rs"]
mod refusal_tests;
