//! Tests for connect, disconnect, and agent detection.

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
}

#[test]
fn zed_agent_connect_and_disconnect_modifies_settings() {
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
}

#[test]
fn detected_agents_filters_by_installed_presence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path();
    fs::create_dir_all(home.join(".cursor")).expect("mkdir .cursor");
    let detected = crate::connect_agents::detected_agents(Some(home));
    assert!(detected.iter().any(|a| a.id == "cursor"));
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

fn with_fake_cli_on_path<T>(bin_dir: &std::path::Path, f: impl FnOnce() -> T) -> T {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = std::env::split_paths(&old_path).collect::<Vec<_>>();
    paths.insert(0, bin_dir.to_path_buf());
    let new_path = std::env::join_paths(paths).expect("join paths");
    temp_env::with_var("PATH", Some(new_path.as_os_str()), f)
}

#[test]
fn connect_registers_with_a_fake_agent_cli_successfully() {
    let bin_dir = fake_agent_cli("claude", 0);
    with_fake_cli_on_path(bin_dir.path(), || {
        let result = run_connect(Some("claude"));
        assert!(
            result.is_ok(),
            "fake claude CLI must register successfully: {result:?}"
        );
    });
}

#[test]
fn connect_surfaces_an_add_failure_from_the_agent_cli() {
    let bin_dir = fake_agent_cli("claude", 1);
    with_fake_cli_on_path(bin_dir.path(), || {
        let result = run_connect(Some("claude"));
        match result {
            Err(ConnectError::AddFailed { cli, .. }) => {
                assert_eq!(cli, "claude");
            }
            other => panic!("expected AddFailed, got {other:?}"),
        }
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
    // trae: project-scoped, plus a UI toggle.
    // pearai: the mechanism could not be established at all.
    let cases: &[(&str, Option<&str>)] = &[
        ("goose", Some(".config/goose/config.yaml")),
        ("crush", Some(".config/crush/crushrc")),
        ("trae", Some(".trae/mcp.json")),
        ("pearai", None),
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
    with_fake_cli_on_path(bin_dir.path(), || {
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
        with_fake_cli_on_path(&bin_dir, || {
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

    connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home)).expect("connect pi without cli");
    assert!(
        home.join(".pi/agent/mcp.json").is_file(),
        "the documented file path must be used when the CLI is absent"
    );
    disconnect_agent(&agent, Some(home)).expect("disconnect pi");
    assert!(!agent.is_connected(Some(home)));

    let bin_dir = fake_agent_cli("pi", 1);
    with_fake_cli_on_path(bin_dir.path(), || {
        connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home))
            .expect("fall back after a rejected add");
    });
    assert!(
        home.join(".pi/agent/mcp.json").is_file(),
        "a CLI that rejects `mcp add` must still fall back to the config file"
    );
}
