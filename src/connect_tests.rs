//! Tests for connect, disconnect, and agent detection.
//!
//! Client CLIs are never the machine's: `AgentDef::is_connected`, `is_installed`
//! and the `CliDirect` route all resolve executables through `PATH`, so every
//! test below that can reach one runs under [`with_no_cli_on_path`] (an empty
//! `PATH`) or [`with_fake_clis_on_path`] (a directory holding only the fakes
//! that test wrote). Prepending to the inherited `PATH` is not enough — a real
//! `codex`/`claude` further along it would still be found and executed, which
//! is how a test run once rewrote the owner's `~/.codex/config.toml`
//! (`.planning/DEVIATIONS.md`). Tests driving [`run_connect`] additionally pin
//! both homes with [`with_config_home`].

use super::*;
use crate::connect_agents::{ConfigShape, Strategy, find_agent};
use crate::connect_exec::{connect_agent, disconnect_agent};
use crate::util::TEST_ENV_LOCK;
use std::fs;
use std::path::PathBuf;

#[test]
fn prefer_slugthug_binary_uses_the_installed_path_when_present() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    let bin_dir = temp.path().join("slugaudit");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    fs::write(bin_dir.join("slugaudit"), b"#!fake").expect("fake binary");

    let current = PathBuf::from("/build/artifacts/slugaudit");
    temp_env::with_var("SLUGTHUG_HOME", Some(temp.path().as_os_str()), || {
        assert_eq!(
            prefer_slugthug_binary(&current),
            bin_dir.join("slugaudit"),
            "the stable installed path wins over the current build artifact"
        );
    });
}

#[test]
fn prefer_slugthug_binary_keeps_current_when_not_installed() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", None::<&std::ffi::OsStr>),
            ("HOME", None::<&std::ffi::OsStr>),
        ],
        || {
            let current = PathBuf::from("/build/artifacts/slugaudit");
            assert_eq!(prefer_slugthug_binary(&current), current);
        },
    );
}

#[test]
fn json_agent_connect_and_disconnect_modifies_config() {
    with_no_cli_on_path(|| {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        let agent = find_agent("cursor").expect("cursor agent");
        let binary = Path::new("/bin/slugaudit");

        assert!(!agent.is_connected(Some(home)));

        connect_agent(&agent, binary, Some(home)).expect("connect cursor");
        assert!(agent.is_connected(Some(home)));

        let cfg_path = home.join(".cursor/mcp.json");
        let content = fs::read_to_string(&cfg_path).expect("read config");
        assert!(content.contains("slugaudit"));
        assert!(content.contains("/bin/slugaudit"));

        disconnect_agent(&agent, Some(home)).expect("disconnect cursor");
        assert!(!agent.is_connected(Some(home)));
        let after = fs::read_to_string(&cfg_path).expect("read config after");
        assert!(!after.contains("/bin/slugaudit"));
    });
}

#[test]
fn zed_agent_connect_and_disconnect_modifies_settings() {
    with_no_cli_on_path(|| {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        let agent = find_agent("zed").expect("zed agent");
        let binary = Path::new("/bin/slugaudit");

        assert!(!agent.is_connected(Some(home)));

        connect_agent(&agent, binary, Some(home)).expect("connect zed");
        assert!(agent.is_connected(Some(home)));

        let cfg_path = home.join(".config/zed/settings.json");
        let content = fs::read_to_string(&cfg_path).expect("read zed settings");
        assert!(content.contains("context_servers"));
        assert!(content.contains("slugaudit"));

        disconnect_agent(&agent, Some(home)).expect("disconnect zed");
        assert!(!agent.is_connected(Some(home)));
    });
}

#[test]
fn detected_agents_filters_by_installed_presence() {
    with_no_cli_on_path(|| {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        fs::create_dir_all(home.join(".cursor")).expect("mkdir .cursor");
        let detected = crate::connect_agents::detected_agents(Some(home));
        assert!(detected.iter().any(|a| a.id == "cursor"));
    });
}

fn fake_agent_cli(name: &str, exit_code: i32) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("bin dir");
    let script = dir.path().join(name);
    fs::write(&script, format!("#!/bin/sh\nexit {exit_code}\n")).expect("write fake cli");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).expect("chmod fake cli");
    }
    dir
}

/// Runs `f` with `PATH` pointing at exactly `bin_dir`: the inherited entries
/// are dropped, so a real client CLI installed on this machine cannot be found
/// by `which` or executed by `Command::new` while the closure runs.
fn with_fake_clis_on_path<T>(bin_dir: &std::path::Path, f: impl FnOnce() -> T) -> T {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("PATH", Some(bin_dir.as_os_str()), f)
}

/// Runs `f` with `PATH` pointing at an empty directory, so no client CLI can
/// be resolved, matched by `which`, or executed at all.
fn with_no_cli_on_path<T>(f: impl FnOnce() -> T) -> T {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let empty = tempfile::tempdir().expect("empty bin dir");
    temp_env::with_var("PATH", Some(empty.path().as_os_str()), f)
}

/// Points both SlugAudit's own home and the client-config home at `home`, so a
/// test driving [`run_connect`] cannot read the developer's real
/// `~/.claude.json` or install into the real `~/.slugthug`.
fn with_config_home<T>(home: &std::path::Path, f: impl FnOnce() -> T) -> T {
    temp_env::with_vars(
        [
            ("SLUGTHUG_CONFIG_HOME", Some(home.as_os_str())),
            ("SLUGTHUG_HOME", Some(home.as_os_str())),
        ],
        f,
    )
}

#[test]
fn connect_registers_with_a_fake_agent_cli_successfully() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = fake_agent_cli("claude", 0);
    with_fake_clis_on_path(bin_dir.path(), || {
        with_config_home(temp.path(), || {
            let result = run_connect(Some("claude"));
            assert!(
                result.is_ok(),
                "fake claude CLI must register successfully: {result:?}"
            );
        });
    });
}

#[test]
fn connect_surfaces_an_add_failure_from_the_agent_cli() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = fake_agent_cli("claude", 1);
    with_fake_clis_on_path(bin_dir.path(), || {
        with_config_home(temp.path(), || {
            let result = run_connect(Some("claude"));
            match result {
                Err(ConnectError::AddFailed { cli, .. }) => {
                    assert_eq!(cli, "claude");
                }
                other => panic!("expected AddFailed, got {other:?}"),
            }
        });
    });
}

#[test]
fn json_agent_refuses_to_overwrite_invalid_or_commented_json() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path();
    let agent = find_agent("cursor").expect("cursor agent");
    let binary = Path::new("/bin/slugaudit");

    let cfg_path = home.join(".cursor/mcp.json");
    fs::create_dir_all(cfg_path.parent().expect("parent")).expect("mkdir");
    let original = "{\n  // User comment\n  \"mcpServers\": {}\n}";
    fs::write(&cfg_path, original).expect("write original");

    let result = connect_agent(&agent, binary, Some(home));
    assert!(
        matches!(result, Err(ConnectError::InvalidConfig { .. })),
        "must error on invalid json, got {result:?}"
    );
    let after = fs::read_to_string(&cfg_path).expect("read after");
    assert_eq!(after, original, "original file must remain untouched");
}

#[test]
fn connect_refuses_entries_it_cannot_write_safely() {
    // goose: YAML config with no machine-writable shape.
    // crush: `mcp add` is a crushrc bash builtin, so no shape is established.
    let cases: &[(&str, Option<&str>)] = &[
        ("goose", Some(".config/goose/config.yaml")),
        ("crush", Some(".config/crush/crushrc")),
    ];
    for (id, file) in cases {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        let agent = find_agent(id).expect("agent");
        let err = connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home))
            .expect_err("connect must refuse");
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
            other => panic!("{id} must refuse automated registration, got {other:?}"),
        }
        if let Some(file) = file {
            assert!(!home.join(file).exists(), "{id} must not write {file}");
        }
    }
}

#[test]
fn goose_refusal_prints_the_paste_ready_yaml_block() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path();
    let goose = find_agent("goose").expect("goose");

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
        .expect_err("connect must refuse");
    let ConnectError::UnsupportedAgent { reason, .. } = err else {
        panic!("expected an UnsupportedAgent refusal");
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
}

#[test]
fn connect_writes_the_config_file_for_interactive_only_clients() {
    with_no_cli_on_path(|| {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        let agent = find_agent("omp").expect("omp");
        assert_eq!(agent.strategy, Strategy::CliInteractiveOnly);

        connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home)).expect("connect omp");
        let content = fs::read_to_string(home.join(".omp/agent/mcp.json")).expect("omp config");
        assert!(content.contains("slugaudit"));
        assert!(content.contains("/bin/slugaudit"));
        assert!(agent.is_connected(Some(home)));

        disconnect_agent(&agent, Some(home)).expect("disconnect omp");
        assert!(!agent.is_connected(Some(home)));
    });
}

#[test]
fn config_file_clients_write_their_documented_filename_without_a_cli() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path();
    let warp = find_agent("warp").expect("warp");
    assert_eq!(warp.cli, Some("oz"), "oz is detection-only for warp");

    connect_agent(&warp, Path::new("/bin/slugaudit"), Some(home)).expect("connect warp");
    let content = fs::read_to_string(home.join(".warp/.mcp.json")).expect("warp config");
    assert!(content.contains("mcpServers"));
    assert!(content.contains("/bin/slugaudit"));
}

#[test]
fn connect_uses_the_cli_when_a_scriptable_executable_exists() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path();
    let agent = find_agent("pi").expect("pi");
    let bin_dir = fake_agent_cli("pi", 0);
    with_fake_clis_on_path(bin_dir.path(), || {
        connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home)).expect("connect pi via cli");
    });
    assert!(
        !home.join(".pi/agent/mcp.json").exists(),
        "a present, succeeding CLI must win over the file fallback"
    );
}

fn write_recording_cli(script: &Path, log: &Path) {
    fs::write(
        script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> {}\nexit 0\n",
            log.display()
        ),
    )
    .expect("write recording cli");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(script).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(script, perms).expect("chmod recording cli");
    }
}

#[test]
fn connect_emits_the_transport_grammar_each_client_requires() {
    // The probe wave found the transport spelling is per-client and
    // inconsistent (`--transport` for openhands/bob, `--type` for droid), and
    // that Bob's scope defaults to workspace. The recorded argv also proves the
    // attempt is add-only: slugaudit never runs a client's `remove` as part of
    // registering, because a failed remove can deregister a working server.
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
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("home");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        let log = temp.path().join("args.log");
        write_recording_cli(&bin_dir.join(id), &log);

        let agent = find_agent(id).expect("agent");
        with_fake_clis_on_path(&bin_dir, || {
            connect_agent(&agent, Path::new("/bin/slugaudit"), Some(&home)).expect("connect");
        });
        let recorded = fs::read_to_string(&log).expect("args log");
        assert_eq!(&recorded, expected, "{id} emitted the wrong argv");
    }
}

#[test]
fn connect_falls_back_to_the_config_file_when_the_executable_is_absent_or_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path();
    let agent = find_agent("pi").expect("pi");
    let config = home.join(".pi/agent/mcp.json");

    with_no_cli_on_path(|| {
        connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home))
            .expect("connect pi without cli");
        assert!(
            config.is_file(),
            "the documented file path must be used when the CLI is absent"
        );
        disconnect_agent(&agent, Some(home)).expect("disconnect pi");
        assert!(!agent.is_connected(Some(home)));
    });

    // Remove the file the CLI-less path wrote, so the assertion below proves
    // the rejected `mcp add` wrote it again rather than merely leaving it.
    fs::remove_file(&config).expect("remove config before the fallback");

    let bin_dir = fake_agent_cli("pi", 1);
    with_fake_clis_on_path(bin_dir.path(), || {
        connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home))
            .expect("fall back after a rejected add");
    });
    assert!(
        config.is_file(),
        "a CLI that rejects `mcp add` must still fall back to the config file"
    );
}

/// A config whose top level is not an object has nowhere to hold an MCP
/// section; the refusal names the file, not an unrelated missing home.
#[test]
fn a_json_config_whose_top_level_is_not_an_object_is_refused_by_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let agent = find_agent("cursor").expect("cursor agent");
    let cfg_path = temp.path().join(".cursor/mcp.json");
    fs::create_dir_all(cfg_path.parent().expect("parent")).expect("mkdir");
    fs::write(&cfg_path, "[]").expect("write array config");

    let result = connect_agent(&agent, Path::new("/bin/slugaudit"), Some(temp.path()));

    let Err(ConnectError::RootNotAnObject { path }) = result else {
        panic!("expected RootNotAnObject, got {result:?}");
    };
    assert_eq!(path, cfg_path, "the refusal names the config file");
    assert_eq!(
        fs::read_to_string(&cfg_path).expect("read"),
        "[]",
        "file untouched"
    );
}

/// A section key holding a non-object is refused rather than rewritten
/// unchanged under a "Wrote" message.
#[test]
fn a_json_config_whose_server_section_is_not_an_object_is_refused_by_section() {
    let temp = tempfile::tempdir().expect("tempdir");
    let agent = find_agent("cursor").expect("cursor agent");
    let cfg_path = temp.path().join(".cursor/mcp.json");
    fs::create_dir_all(cfg_path.parent().expect("parent")).expect("mkdir");
    fs::write(&cfg_path, r#"{"mcpServers": []}"#).expect("write config");

    let result = connect_agent(&agent, Path::new("/bin/slugaudit"), Some(temp.path()));

    let Err(ConnectError::SectionNotAnObject { section }) = result else {
        panic!("expected SectionNotAnObject, got {result:?}");
    };
    assert_eq!(section, "mcpServers");
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
fn a_menu_answer_is_valid_only_when_it_is_a_listed_number() {
    let valid = [("1", 0), (" 3\n", 2)];
    let invalid = ["0", "4", "", "two", "-1"];

    for (answer, index) in valid {
        assert_eq!(
            parse_choice(answer, 3).expect(answer),
            index,
            "answer {answer:?}"
        );
    }
    for answer in invalid {
        assert!(
            matches!(parse_choice(answer, 3), Err(ConnectError::InvalidChoice)),
            "answer {answer:?} must be rejected"
        );
    }
}

#[test]
fn disconnect_surfaces_a_remove_failure_from_the_agent_cli() {
    let bin_dir = fake_agent_cli("claude", 1);
    let agent = find_agent("claude").expect("claude agent");

    let result = with_fake_clis_on_path(bin_dir.path(), || {
        crate::connect_cli::run_remove(&agent, "claude")
    });

    let Err(ConnectError::RemoveFailed { cli, .. }) = result else {
        panic!("expected RemoveFailed, got {result:?}");
    };
    assert_eq!(cli, "claude");
}

#[test]
fn an_agent_cli_that_cannot_be_executed_is_reported_as_a_command_failure() {
    let agent = find_agent("claude").expect("claude agent");

    let result = with_no_cli_on_path(|| crate::connect_cli::run_remove(&agent, "claude"));

    let Err(ConnectError::AgentCommand { cli, .. }) = result else {
        panic!("expected AgentCommand, got {result:?}");
    };
    assert_eq!(cli, "claude");
}
