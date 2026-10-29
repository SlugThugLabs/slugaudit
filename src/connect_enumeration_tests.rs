//! `AgentDef::is_connected` reads local config files only.

use super::*;

/// Enumeration is config-only: a client CLI on `PATH` that records any
/// invocation must never be run by `is_connected`, whatever the config says.
#[cfg(unix)]
#[test]
fn is_connected_never_spawns_a_client_cli() {
    use std::os::unix::fs::PermissionsExt as _;
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    let bin = tempfile::tempdir().expect("bin dir");
    let marker = bin.path().join("spawned");
    for agent in AGENTS {
        for name in agent.cli.into_iter().chain(agent.verify_cli) {
            let script = bin.path().join(name);
            std::fs::write(
                &script,
                format!(
                    "#!/bin/sh\necho {name} >> '{}'\necho slugaudit\n",
                    marker.display()
                ),
            )
            .expect("write fake cli");
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
    }
    let home = tempfile::tempdir().expect("home");

    let connected: Vec<&str> = temp_env::with_var("PATH", Some(bin.path().as_os_str()), || {
        AGENTS
            .iter()
            .filter(|agent| agent.is_connected(Some(home.path())))
            .map(|agent| agent.id)
            .collect()
    });

    assert!(connected.is_empty(), "no config exists: {connected:?}");
    assert!(
        !marker.exists(),
        "a client CLI was spawned during enumeration"
    );
}

/// A client with no known config file cannot be detected from config: it is
/// deterministically reported as not connected.
#[test]
fn a_client_without_a_config_path_is_not_detected_as_connected() {
    let home = tempfile::tempdir().expect("home");
    for id in ["agy", "amp", "q"] {
        let agent = find_agent(id).expect("agent");
        assert!(!agent.is_connected(Some(home.path())), "{id}");
    }
}
