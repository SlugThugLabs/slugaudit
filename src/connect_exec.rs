//! Low-level command execution and JSON config file modification for agent connect.
#![allow(clippy::print_stdout)]

use crate::cli::ConnectError;
use crate::connect_agents::{AgentDef, Dialect};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn json_contains_slugaudit(path: &Path) -> bool {
    has_slugaudit(path, &["mcpServers", "mcp"])
}

pub fn zed_contains_slugaudit(path: &Path) -> bool {
    has_slugaudit(path, &["context_servers"])
}

fn has_slugaudit(path: &Path, keys: &[&str]) -> bool {
    let Ok(c) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&c) else {
        return false;
    };
    keys.iter()
        .any(|k| v.get(*k).and_then(|s| s.get("slugaudit")).is_some())
}

pub fn cli_check_connected(cli: &str) -> bool {
    let Ok(out) = Command::new(cli).args(["mcp", "list"]).output() else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).contains("slugaudit")
}

pub fn connect_agent(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
) -> Result<(), ConnectError> {
    let home = home.ok_or(ConnectError::HomeUnavailable);
    match agent.dialect {
        Dialect::JsonFile => {
            let rel = agent.config_rel.ok_or(ConnectError::HomeUnavailable)?;
            let path = home?.join(rel);
            write_json_server(
                &path,
                "mcpServers",
                serde_json::json!({
                    "command": binary.to_string_lossy(),
                    "args": []
                }),
            )
        }
        Dialect::Zed => {
            let path = home?.join(".config/zed/settings.json");
            write_json_server(
                &path,
                "context_servers",
                serde_json::json!({
                    "command": { "path": binary.to_string_lossy(), "args": [] }
                }),
            )
        }
        _ => {
            let cli = agent.cli.ok_or_else(|| ConnectError::AgentMissing {
                agent: agent.display_name.to_string(),
                cli: agent.id.to_string(),
            })?;
            if which::which(cli).is_err() {
                return Err(ConnectError::AgentMissing {
                    agent: agent.display_name.to_string(),
                    cli: cli.to_string(),
                });
            }
            let _ = run_remove_cmd(agent, cli);
            run_add_cmd(agent, cli, binary)
        }
    }
}

fn write_json_server(
    path: &Path,
    section: &str,
    val: serde_json::Value,
) -> Result<(), ConnectError> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    let mut root = load_json_or_empty(path);
    let map = root.as_object_mut().ok_or(ConnectError::HomeUnavailable)?;
    let s = map.entry(section).or_insert_with(|| serde_json::json!({}));
    if let Some(s_map) = s.as_object_mut() {
        s_map.insert("slugaudit".into(), val);
    }
    fs::write(path, serde_json::to_string_pretty(&root)?)?;
    Ok(())
}

pub fn disconnect_agent(agent: &AgentDef, home: Option<&Path>) -> Result<(), ConnectError> {
    let Some(home) = home else { return Ok(()) };
    match agent.dialect {
        Dialect::JsonFile => {
            if let Some(rel) = agent.config_rel {
                remove_json_slugaudit(&home.join(rel), &["mcpServers", "mcp"])?;
            }
            Ok(())
        }
        Dialect::Zed => remove_json_slugaudit(
            &home.join(".config/zed/settings.json"),
            &["context_servers"],
        ),
        _ => {
            if let Some(cli) = agent.cli
                && which::which(cli).is_ok()
            {
                return run_remove_cmd(agent, cli);
            }
            Ok(())
        }
    }
}

fn remove_json_slugaudit(path: &Path, keys: &[&str]) -> Result<(), ConnectError> {
    if path.exists() {
        let mut root = load_json_or_empty(path);
        for key in keys {
            if let Some(s) = root.get_mut(*key).and_then(|s| s.as_object_mut()) {
                s.remove("slugaudit");
            }
        }
        fs::write(path, serde_json::to_string_pretty(&root)?)?;
    }
    Ok(())
}

fn load_json_or_empty(path: &Path) -> serde_json::Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .filter(|v: &serde_json::Value| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn run_remove_cmd(agent: &AgentDef, cli: &str) -> Result<(), ConnectError> {
    let args: &[&str] = match agent.dialect {
        Dialect::Claude => &["mcp", "remove", "-s", "user", "slugaudit"],
        Dialect::Q => &["mcp", "remove", "--name", "slugaudit"],
        Dialect::DashSeparator if agent.id == "bob" => {
            &["mcp", "remove", "slugaudit", "--scope", "global"]
        }
        Dialect::DashSeparator if agent.id == "grok" => {
            &["mcp", "remove", "slugaudit", "--scope", "user"]
        }
        _ => &["mcp", "remove", "slugaudit"],
    };
    let _ = Command::new(cli)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Ok(())
}

fn run_add_cmd(agent: &AgentDef, cli: &str, binary: &Path) -> Result<(), ConnectError> {
    let mut cmd = Command::new(cli);
    cmd.arg("mcp").arg("add");
    match agent.dialect {
        Dialect::Agy => {
            cmd.arg("slugaudit").arg(binary);
        }
        Dialect::Claude => {
            cmd.args(["-s", "user", "slugaudit", "--"]).arg(binary);
        }
        Dialect::Hermes | Dialect::Crush => {
            cmd.args(["slugaudit", "--command"]).arg(binary);
        }
        Dialect::Q => {
            cmd.args(["--name", "slugaudit", "--command"]).arg(binary);
        }
        Dialect::DashSeparator => {
            cmd.arg("slugaudit");
            if agent.id == "bob" {
                cmd.args(["--scope", "global"]);
            } else if agent.id == "grok" {
                cmd.args(["--scope", "user"]);
            }
            cmd.arg("--").arg(binary);
        }
        _ => {
            cmd.arg("slugaudit").arg(binary);
        }
    }
    let output = cmd.output().map_err(|inner| ConnectError::AgentCommand {
        cli: cli.to_string(),
        inner,
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            eprintln!("{stderr}");
        }
        return Err(ConnectError::AddFailed {
            cli: cli.to_string(),
            status: output.status.to_string(),
        });
    }
    Ok(())
}
