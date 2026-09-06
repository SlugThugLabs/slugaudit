//! Tests for connect, disconnect, and agent detection.

use super::*;
use crate::connect_agents::find_agent;
use crate::connect_exec::{connect_agent, disconnect_agent};
use crate::util::TEST_ENV_LOCK;
use std::fs;
use std::path::PathBuf;

#[test]
fn find_agent_finds_standard_and_aliased_names() {
    assert_eq!(find_agent("agy").map(|a| a.id), Some("agy"));
    assert_eq!(find_agent("antigravity").map(|a| a.id), Some("agy"));
    assert_eq!(find_agent("claude").map(|a| a.id), Some("claude"));
    assert_eq!(find_agent("claude-code").map(|a| a.id), Some("claude"));
    assert_eq!(find_agent("claude_code").map(|a| a.id), Some("claude"));
    assert_eq!(find_agent("cursor").map(|a| a.id), Some("cursor"));
    assert_eq!(find_agent("zed").map(|a| a.id), Some("zed"));
    assert_eq!(find_agent("nonexistent_agent"), None);
}

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
