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
use crate::connect_agents::{ConfigShape, Strategy, find_agent};
use crate::connect_exec::{DisconnectOutcome, connect_agent, disconnect_agent};
use crate::util::TEST_ENV_LOCK;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::fs;

#[test]
fn json_agent_connect_and_disconnect_modifies_config() -> TestResult {
    with_no_cli_on_path(|| -> TestResult {
        let temp = tempfile::tempdir().check("tempdir")?;
        let home = temp.path();
        let agent = find_agent("cursor").check("cursor agent")?;
        let binary = Path::new("/bin/slugaudit");

        assert!(!agent.is_connected(Some(home)));

        connect_agent(&agent, binary, Some(home)).check("connect cursor")?;
        assert!(agent.is_connected(Some(home)));

        let cfg_path = home.join(".cursor/mcp.json");
        let content = fs::read_to_string(&cfg_path).check("read config")?;
        assert!(content.contains("slugaudit"));
        assert!(content.contains("/bin/slugaudit"));

        let outcome = disconnect_agent(&agent, Some(home)).check("disconnect cursor")?;
        assert_eq!(outcome, DisconnectOutcome::Disconnected);
        assert!(!agent.is_connected(Some(home)));
        let after = fs::read_to_string(&cfg_path).check("read config after")?;
        assert!(!after.contains("/bin/slugaudit"));
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

        assert!(!agent.is_connected(Some(home)));

        connect_agent(&agent, binary, Some(home)).check("connect zed")?;
        assert!(agent.is_connected(Some(home)));

        let cfg_path = home.join(".config/zed/settings.json");
        let content = fs::read_to_string(&cfg_path).check("read zed settings")?;
        assert!(content.contains("context_servers"));
        assert!(content.contains("slugaudit"));

        disconnect_agent(&agent, Some(home)).check("disconnect zed")?;
        assert!(!agent.is_connected(Some(home)));
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
        let detected = crate::connect_agents::detected_agents(Some(home));
        assert!(detected.agents.iter().any(|a| a.id == "cursor"));
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

        assert_eq!(failure.agent, "cursor");
        assert_eq!(failure.path, home.join(".cursor/mcp.json"));
        assert_eq!(failure.source.kind(), std::io::ErrorKind::NotADirectory);
        let detection = crate::connect_agents::detected_agents(Some(home));
        assert!(detection.failures.iter().any(|f| f.agent == "cursor"));
        assert!(!detection.agents.iter().any(|a| a.id == "cursor"));
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
            assert!(
                result.is_ok(),
                "fake claude CLI must register successfully: {result:?}"
            );
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
                    assert_eq!(cli, "claude");
                    assert!(
                        error.to_string().contains("printed no output"),
                        "a silent client must not be described as having output: {error}"
                    );
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
fn json_agent_refuses_to_overwrite_invalid_or_commented_json() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let home = temp.path();
    let agent = find_agent("cursor").check("cursor agent")?;
    let binary = Path::new("/bin/slugaudit");

    let cfg_path = home.join(".cursor/mcp.json");
    fs::create_dir_all(cfg_path.parent().check("parent")?).check("mkdir")?;
    let original = "{\n  // User comment\n  \"mcpServers\": {}\n}";
    fs::write(&cfg_path, original).check("write original")?;

    let result = connect_agent(&agent, binary, Some(home));
    assert!(
        matches!(result, Err(ConnectError::InvalidConfig { .. })),
        "must error on invalid json, got {result:?}"
    );
    let after = fs::read_to_string(&cfg_path).check("read after")?;
    assert_eq!(after, original, "original file must remain untouched");
    Ok(())
}

#[test]
fn connect_refuses_entries_it_cannot_write_safely() -> TestResult {
    // goose: YAML config with no machine-writable shape.
    // crush: `mcp add` is a crushrc bash builtin, so no shape is established.
    let cases: &[(&str, Option<&str>)] = &[
        ("goose", Some(".config/goose/config.yaml")),
        ("crush", Some(".config/crush/crushrc")),
    ];
    for (id, file) in cases {
        let temp = tempfile::tempdir().check("tempdir")?;
        let home = temp.path();
        let agent = find_agent(id).check("agent")?;
        let err = connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home))
            .check_err("connect must refuse")?;
        match err {
            ConnectError::UnsupportedAgent {
                agent: name,
                reason,
            } => {
                assert_eq!(name, agent.display_name);
                assert!(
                    reason.contains("no automated write is possible"),
                    "{id} must say the write cannot proceed: {reason}"
                );
                assert!(
                    reason.contains(&agent.remedy.replace("{binary}", "/bin/slugaudit")),
                    "{id} must print its remedy: {reason}"
                );
            }
            other => {
                return Err(format!(
                    "{id} must refuse automated registration, got {other:?}"
                ))
                .check("unexpected outcome");
            }
        }
        if let Some(file) = file {
            assert!(!home.join(file).exists(), "{id} must not write {file}");
        }
    }
    Ok(())
}

#[test]
fn goose_refusal_prints_the_paste_ready_yaml_block() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let home = temp.path();
    let goose = find_agent("goose").check("goose")?;

    // The YAML shape has no machine-writable section, so the generic JSON
    // merge can never be reached for goose.
    assert!(
        ConfigShape::GooseExtensions.write_section().is_none(),
        "the YAML shape must not be machine-written"
    );
    assert_eq!(
        ConfigShape::GooseExtensions.sections(),
        ["extensions"],
        "detection still reads the extensions key"
    );

    let err = connect_agent(&goose, Path::new("/bin/slugaudit"), Some(home))
        .check_err("connect must refuse")?;
    let ConnectError::UnsupportedAgent { reason, .. } = err else {
        return Err("expected an UnsupportedAgent refusal".to_owned()).check("unexpected outcome");
    };
    assert!(
        reason.contains(".config/goose/config.yaml"),
        "the refusal must name the target file: {reason}"
    );
    assert!(
        reason.contains(
            "extensions:\n  slugaudit:\n    type: stdio\n    cmd: /bin/slugaudit\n    args: []"
        ),
        "the refusal must carry the paste-ready block: {reason}"
    );
    assert!(
        !home.join(".config/goose/config.yaml").exists(),
        "goose's write path must be unreachable"
    );
    Ok(())
}

#[test]
fn connect_writes_the_config_file_for_interactive_only_clients() -> TestResult {
    with_no_cli_on_path(|| -> TestResult {
        let temp = tempfile::tempdir().check("tempdir")?;
        let home = temp.path();
        let agent = find_agent("omp").check("omp")?;
        assert_eq!(agent.strategy, Strategy::CliInteractiveOnly);

        connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home)).check("connect omp")?;
        let content = fs::read_to_string(home.join(".omp/agent/mcp.json")).check("omp config")?;
        assert!(content.contains("slugaudit"));
        assert!(content.contains("/bin/slugaudit"));
        assert!(agent.is_connected(Some(home)));

        disconnect_agent(&agent, Some(home)).check("disconnect omp")?;
        assert!(!agent.is_connected(Some(home)));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn config_file_clients_write_their_documented_filename_without_a_cli() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let home = temp.path();
    let warp = find_agent("warp").check("warp")?;
    assert_eq!(warp.cli, Some("oz"), "oz is detection-only for warp");

    connect_agent(&warp, Path::new("/bin/slugaudit"), Some(home)).check("connect warp")?;
    let content = fs::read_to_string(home.join(".warp/.mcp.json")).check("warp config")?;
    assert!(content.contains("mcpServers"));
    assert!(content.contains("/bin/slugaudit"));
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
    assert!(
        !home.join(".pi/agent/mcp.json").exists(),
        "a present, succeeding CLI must win over the file fallback"
    );
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
    // The transport spelling is per-client and inconsistent (`--transport` for
    // openhands/bob, `--type` for droid), and Bob's scope defaults to workspace.
    // The recorded argv also proves the attempt is add-only: slugaudit never runs
    // a client's `remove` as part of registering, because a failed remove can
    // deregister a working server.
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
        assert_eq!(&recorded, expected, "{id} emitted the wrong argv");
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
        assert!(
            config.is_file(),
            "the documented file path must be used when the CLI is absent"
        );
        disconnect_agent(&agent, Some(home)).check("disconnect pi")?;
        assert!(!agent.is_connected(Some(home)));
        Ok(())
    })?;

    // Remove the file the CLI-less path wrote, so the assertion below proves
    // the rejected `mcp add` wrote it again rather than merely leaving it.
    fs::remove_file(&config).check("remove config before the fallback")?;

    let bin_dir = fake_agent_cli("pi", 1)?;
    with_fake_clis_on_path(bin_dir.path(), || -> TestResult {
        connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home))
            .check("fall back after a rejected add")?;
        Ok(())
    })?;
    assert!(
        config.is_file(),
        "a CLI that rejects `mcp add` must still fall back to the config file"
    );
    Ok(())
}

/// A config whose top level is not an object has nowhere to hold an MCP
/// section; the refusal names the file, not an unrelated missing home.
#[test]
fn a_json_config_whose_top_level_is_not_an_object_is_refused_by_path() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let agent = find_agent("cursor").check("cursor agent")?;
    let cfg_path = temp.path().join(".cursor/mcp.json");
    fs::create_dir_all(cfg_path.parent().check("parent")?).check("mkdir")?;
    fs::write(&cfg_path, "[]").check("write array config")?;

    let result = connect_agent(&agent, Path::new("/bin/slugaudit"), Some(temp.path()));

    let Err(ConnectError::RootNotAnObject { path }) = result else {
        return Err(format!("expected RootNotAnObject, got {result:?}"))
            .check("unexpected outcome");
    };
    assert_eq!(path, cfg_path, "the refusal names the config file");
    assert_eq!(
        fs::read_to_string(&cfg_path).check("read")?,
        "[]",
        "file untouched"
    );
    Ok(())
}

/// A section key holding a non-object is refused rather than rewritten
/// unchanged under a "Wrote" message.
#[test]
fn a_json_config_whose_server_section_is_not_an_object_is_refused_by_section() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let agent = find_agent("cursor").check("cursor agent")?;
    let cfg_path = temp.path().join(".cursor/mcp.json");
    fs::create_dir_all(cfg_path.parent().check("parent")?).check("mkdir")?;
    fs::write(&cfg_path, r#"{"mcpServers": []}"#).check("write config")?;

    let result = connect_agent(&agent, Path::new("/bin/slugaudit"), Some(temp.path()));

    let Err(ConnectError::SectionNotAnObject { section }) = result else {
        return Err(format!("expected SectionNotAnObject, got {result:?}"))
            .check("unexpected outcome");
    };
    assert_eq!(section, "mcpServers");
    Ok(())
}

#[test]
fn an_unknown_agent_name_is_rejected_by_connect_and_disconnect() {
    let connect = run_connect(Some("no-such-agent"));
    let disconnect = run_disconnect(Some("no-such-agent"));

    for result in [connect, disconnect] {
        assert!(
            matches!(&result, Err(ConnectError::UnknownAgent(name)) if name == "no-such-agent"),
            "expected UnknownAgent, got {result:?}"
        );
    }
}

#[test]
fn disconnect_surfaces_a_remove_failure_from_the_agent_cli() -> TestResult {
    let bin_dir = fake_agent_cli("claude", 1)?;
    let agent = find_agent("claude").check("claude agent")?;

    let result = with_fake_clis_on_path(bin_dir.path(), || {
        Ok(crate::connect_cli::run_remove(&agent, "claude"))
    })?;

    let Err(ConnectError::RemoveFailed { cli, .. }) = result else {
        return Err(format!("expected RemoveFailed, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(cli, "claude");
    Ok(())
}

#[test]
fn an_agent_cli_that_cannot_be_executed_is_reported_as_a_command_failure() -> TestResult {
    let agent = find_agent("claude").check("claude agent")?;

    let result = with_no_cli_on_path(|| Ok(crate::connect_cli::run_remove(&agent, "claude")))?;

    let Err(ConnectError::AgentCommand { cli, .. }) = result else {
        return Err(format!("expected AgentCommand, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(cli, "claude");
    Ok(())
}

#[path = "connect_binary_tests.rs"]
mod binary_tests;

#[path = "connect_prompt_tests.rs"]
mod prompt_tests;
