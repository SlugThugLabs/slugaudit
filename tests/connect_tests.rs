//! End-to-end test for the `connect` subcommand.
//!
//! Unlike the unit tests in `cli_tests.rs` (which only exercise argument
//! parsing), this spawns the real `slugaudit-mcp` binary as a subprocess,
//! runs `connect <agent>`, and inspects the agent's actual config file to
//! prove the MCP server registration was written correctly — right name,
//! right binary path, right transport.
//!
//! Requires the relevant agent CLI (`claude`, `grok`, `codex`) to be on
//! PATH. If none are present the whole module is skipped.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Agents this test knows how to verify. Each entry names the CLI tool,
/// the connect subcommand argument, and a closure that inspects the
/// agent's config file and returns true iff `slugaudit` is registered
/// with the expected binary path.
struct Agent {
    cli: &'static str,
    connect_arg: &'static str,
    /// Returns true if the agent's config contains a `slugaudit` stdio
    /// registration pointing at `binary`.
    is_registered: fn(binary: &Path) -> bool,
    /// Removes any existing `slugaudit` registration.
    remove: fn() -> std::io::Result<std::process::Output>,
}

impl Agent {
    fn claude() -> Self {
        Agent {
            cli: "claude",
            connect_arg: "claude",
            is_registered: |binary| {
                let Ok(raw) = fs::read_to_string(home().join(".claude.json")) else {
                    return false;
                };
                let Ok(json): Result<serde_json::Value, _> = serde_json::from_str(&raw) else {
                    return false;
                };
                let Some(server) = json.get("mcpServers").and_then(|s| s.get("slugaudit")) else {
                    return false;
                };
                server.get("type").and_then(|t| t.as_str()) == Some("stdio")
                    && server.get("command").and_then(|c| c.as_str())
                        == Some(&binary.display().to_string())
            },
            remove: || {
                Command::new("claude")
                    .args(["mcp", "remove", "slugaudit", "-s", "user"])
                    .output()
            },
        }
    }

    fn grok() -> Self {
        Agent {
            cli: "grok",
            connect_arg: "grok",
            is_registered: |binary| {
                let Ok(raw) = fs::read_to_string(home().join(".grok/config.toml")) else {
                    return false;
                };
                raw.contains("[mcp_servers.slugaudit]")
                    && raw.contains(&format!("command = \"{}\"", binary.display()))
            },
            remove: || {
                Command::new("grok")
                    .args(["mcp", "remove", "slugaudit", "--scope", "user"])
                    .output()
            },
        }
    }

    fn codex() -> Self {
        Agent {
            cli: "codex",
            connect_arg: "codex",
            is_registered: |binary| {
                let Ok(raw) = fs::read_to_string(home().join(".codex/config.toml")) else {
                    return false;
                };
                raw.contains("[mcp_servers.slugaudit]")
                    && raw.contains(&format!("command = \"{}\"", binary.display()))
            },
            remove: || {
                Command::new("codex")
                    .args(["mcp", "remove", "slugaudit"])
                    .output()
            },
        }
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("HOME must be set")
}

fn connect(binary: &Path, arg: &str) -> std::process::Output {
    Command::new(binary)
        .args(["connect", arg])
        .output()
        .expect("spawn connect")
}

/// Back up a config file if it exists so it can be restored after the
/// test. Returns the backup path (or None if the file didn't exist).
fn backup(path: &Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }
    let backup = path.with_extension(format!(
        "{}.connect-test-backup",
        path.extension()
            .map(|e| e.to_string_lossy())
            .unwrap_or_default()
    ));
    fs::copy(path, &backup).ok()?;
    Some(backup)
}

fn restore(path: &Path, backup: Option<&PathBuf>) {
    match backup {
        Some(b) => {
            let _ = fs::rename(b, path);
        }
        None => {
            let _ = fs::remove_file(path);
        }
    }
}

fn agents() -> Vec<Agent> {
    let mut out = Vec::new();
    if which::which("claude").is_ok() {
        out.push(Agent::claude());
    }
    if which::which("grok").is_ok() {
        out.push(Agent::grok());
    }
    if which::which("codex").is_ok() {
        out.push(Agent::codex());
    }
    out
}

#[test]
fn connect_writes_the_correct_registration_for_each_installed_agent() {
    let agents = agents();
    if agents.is_empty() {
        // No agent CLI installed in this environment — nothing to verify.
        // This is expected in CI and on machines without these tools.
        eprintln!("skipping connect integration test: none of claude/grok/codex on PATH");
        return;
    }

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_slugaudit-mcp"));
    assert!(
        binary.is_file(),
        "test binary not found: {}",
        binary.display()
    );

    for agent in agents {
        let config_path = match agent.connect_arg {
            "claude" => home().join(".claude.json"),
            "grok" => home().join(".grok/config.toml"),
            "codex" => home().join(".codex/config.toml"),
            other => panic!("unknown agent {other}"),
        };

        // Back up whatever was there before so we leave the user's config
        // untouched regardless of pass/fail.
        let backup = backup(&config_path);

        // Remove any stale registration so we're testing a clean add.
        let _ = (agent.remove)();

        let output = connect(&binary, agent.connect_arg);
        assert!(
            output.status.success(),
            "{} connect failed:\nstdout: {}\nstderr: {}",
            agent.cli,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        assert!(
            (agent.is_registered)(&binary),
            "{} connect ran, but {} does not contain a slugaudit stdio \
             registration pointing at {}",
            agent.cli,
            config_path.display(),
            binary.display(),
        );

        // Restore the user's original config.
        restore(&config_path, backup.as_ref());
    }
}
