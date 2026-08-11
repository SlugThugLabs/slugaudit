//! Tests for `connect`: per-agent scope arguments and the stable-binary
//! preference. Anything that would invoke a real agent CLI (`claude`,
//! `grok`, `codex`) is deliberately NOT exercised — on a machine where the
//! agent is installed, such a test would touch the user's real
//! registration.

use super::*;
use crate::util::TEST_ENV_LOCK;
use std::path::PathBuf;

#[test]
fn scope_args_match_the_documented_agent_scopes() {
    assert_eq!(scope_add_args(ConnectAgent::Claude), &["-s", "user"]);
    assert_eq!(scope_add_args(ConnectAgent::Grok), &["--scope", "user"]);
    assert!(scope_add_args(ConnectAgent::Codex).is_empty());
    assert_eq!(scope_remove_args(ConnectAgent::Claude), &["-s", "user"]);
    assert_eq!(scope_remove_args(ConnectAgent::Grok), &["--scope", "user"]);
    assert!(scope_remove_args(ConnectAgent::Codex).is_empty());
}

#[test]
fn prefer_slugthug_binary_uses_the_installed_path_when_present() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    std::fs::write(bin_dir.join("slugaudit-mcp"), b"#!fake").expect("fake binary");

    let current = PathBuf::from("/build/artifacts/slugaudit-mcp");
    temp_env::with_var("SLUGTHUG_HOME", Some(temp.path().as_os_str()), || {
        assert_eq!(
            prefer_slugthug_binary(&current),
            bin_dir.join("slugaudit-mcp"),
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
            let current = PathBuf::from("/build/artifacts/slugaudit-mcp");
            assert_eq!(prefer_slugthug_binary(&current), current);
        },
    );
}

/// Only meaningful on machines without the agent CLIs installed; skips
/// otherwise rather than risk touching a real agent registration. Proves
/// the missing-CLI error is a typed `ConnectError`, not a panic or a
/// silent no-op.
#[test]
fn connect_reports_a_missing_agent_cli_as_a_typed_error() {
    if ["claude", "grok", "codex"]
        .iter()
        .any(|cli| which::which(cli).is_ok())
    {
        return;
    }
    let result = run_connect(ConnectAgent::Claude);
    assert!(matches!(result, Err(ConnectError::AgentMissing { .. })));
}

/// Creates a fake agent CLI in a temp dir: a tiny inert shell script that
/// ignores its arguments and exits with `exit_code`. Prepending the dir to
/// `PATH` (under the env lock) lets the full `connect_one` flow run
/// against the fake instead of a real agent registration, which is never
/// touched.
fn fake_agent_cli(name: &str, exit_code: i32) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("bin dir");
    let script = dir.path().join(name);
    std::fs::write(&script, format!("#!/bin/sh\nexit {exit_code}\n")).expect("write fake cli");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod fake cli");
    }
    dir
}

/// Runs `f` with `bin_dir` prepended to `PATH`, holding the env lock so a
/// concurrent test can't observe the mutation window.
fn with_fake_cli_on_path<T>(bin_dir: &std::path::Path, f: impl FnOnce() -> T) -> T {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = std::env::split_paths(&old_path).collect::<Vec<_>>();
    paths.insert(0, bin_dir.to_path_buf());
    let new_path = std::env::join_paths(paths).expect("join paths");
    temp_env::with_var("PATH", Some(new_path.as_os_str()), f)
}

/// The full `connect_one` flow (remove existing registration, then add)
/// must succeed end-to-end against a fake agent CLI that exits 0.
#[test]
fn connect_registers_with_a_fake_agent_cli_successfully() {
    let bin_dir = fake_agent_cli("claude", 0);
    with_fake_cli_on_path(bin_dir.path(), || {
        let result = run_connect(ConnectAgent::Claude);
        assert!(
            result.is_ok(),
            "a fake agent CLI that exits 0 must register successfully: {result:?}"
        );
    });
}

/// A failed `mcp add` (fake CLI exits non-zero) must surface as the typed
/// `AddFailed` error with the agent's CLI named, not a panic.
#[test]
fn connect_surfaces_an_add_failure_from_the_agent_cli() {
    let bin_dir = fake_agent_cli("claude", 1);
    with_fake_cli_on_path(bin_dir.path(), || {
        let result = run_connect(ConnectAgent::Claude);
        match result {
            Err(ConnectError::AddFailed { cli, .. }) => {
                assert_eq!(cli, "claude");
            }
            other => panic!("expected AddFailed, got {other:?}"),
        }
    });
}
