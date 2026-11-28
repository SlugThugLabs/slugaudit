//! Config-format inspection for `connect` probing: JSON/JSONC section checks,
//! `toml::Table` MCP-server lookup for `.toml` files, and `#`-comment-aware
//! line scanning for YAML and shell-rc files.

use super::config_file::{has_slugaudit, readable_config};
use super::registry_types::ConfigShape;
use std::path::Path;

/// Returns true when the local client config at `path` already registers
/// `slugaudit` under `shape` (or in a `.toml` `mcp_servers`/`mcp` table).
///
/// # Errors
///
/// Propagates unreadable, oversized, non-UTF-8, or malformed JSON/TOML config
/// errors so callers never treat an unexamined file as disconnected.
pub(super) fn config_has_slugaudit_entry(path: &Path, shape: ConfigShape) -> std::io::Result<bool> {
    let is_toml = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));
    if !is_toml && !matches!(shape, ConfigShape::GooseExtensions | ConfigShape::Unknown) {
        return has_slugaudit(path, shape.sections());
    }
    let Some(content) = readable_config(path)? else {
        return Ok(false);
    };
    if is_toml {
        toml_has_slugaudit(&content)
    } else {
        Ok(line_oriented_has_slugaudit(&content))
    }
}

/// True when a TOML config defines a `"slugaudit"` key inside an MCP server
/// table (`mcp_servers` or `mcp`), ignoring unrelated tables such as
/// `[projects."/home/user/slugaudit"]`.
fn toml_has_slugaudit(text: &str) -> std::io::Result<bool> {
    let table: toml::Table = toml::from_str(text)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok(["mcp_servers", "mcp"].iter().any(|section| {
        table
            .get(*section)
            .and_then(toml::Value::as_table)
            .is_some_and(|servers| servers.contains_key("slugaudit"))
    }))
}

/// Strips `#` comments from one YAML or shell-rc line while preserving `#`
/// characters inside single- or double-quoted strings. In YAML and POSIX shell
/// `#` begins a comment at line start or after whitespace.
fn strip_hash_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut after_ws_or_bol = true;
    for (idx, ch) in line.char_indices() {
        if in_single {
            if ch == '\'' {
                in_single = false;
                after_ws_or_bol = false;
            }
            continue;
        }
        if in_double {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
                after_ws_or_bol = false;
            }
            continue;
        }
        match ch {
            '\'' => {
                in_single = true;
                after_ws_or_bol = false;
            }
            '"' => {
                in_double = true;
                after_ws_or_bol = false;
            }
            '#' if after_ws_or_bol => {
                return match line.get(..idx) {
                    Some(prefix) => prefix,
                    None => line,
                };
            }
            other => {
                after_ws_or_bol = other.is_whitespace();
            }
        }
    }
    line
}

/// True when any non-comment portion of a YAML or rc config mentions `slugaudit`.
fn line_oriented_has_slugaudit(text: &str) -> bool {
    text.lines()
        .map(strip_hash_comment)
        .any(|code| code.contains("slugaudit"))
}
