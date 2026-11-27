//! Low-level JSON/JSONC config-file access for `connect` and `disconnect`:
//! detect an existing `slugaudit` entry, merge one key into a file this tool
//! can parse without destroying comments, and remove it again.

use super::ConnectError;
pub(super) use super::config_atomic::write_atomic;
#[cfg(test)]
pub(super) use super::config_atomic::write_atomic_with;
use super::guidance;
use super::jsonc::{has_jsonc_comments, is_jsonc_path, strip_jsonc_comments_and_trailing_commas};
use super::registry_types::AgentDef;
use std::fs;
use std::path::Path;

/// Reads a client config for detection: `Ok(None)` for an absent file, the
/// text for a readable one.
///
/// # Errors
///
/// A file that cannot be read, is not UTF-8, or exceeds the size cap is
/// returned with its cause, never read as "absent".
pub(super) fn readable_config(path: &Path) -> std::io::Result<Option<String>> {
    match crate::util::read_file_capped(path, crate::util::MAX_FOREIGN_TEXT_BYTES) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// True when the JSON/JSONC config at `path` carries a `slugaudit` entry under
/// any of `keys`; an absent file has none.
///
/// # Errors
///
/// An unreadable, oversized or malformed config is returned with
/// its cause (a malformed one as `InvalidData` wrapping the JSON error). The
/// write path bounds the same file again and fails explicitly.
pub fn has_slugaudit(path: &Path, keys: &[&str]) -> std::io::Result<bool> {
    let Some(contents) = readable_config(path)? else {
        return Ok(false);
    };
    let normalized;
    let json_text = if is_jsonc_path(path) {
        normalized = strip_jsonc_comments_and_trailing_commas(&contents);
        if normalized.trim().is_empty() {
            return Ok(false);
        }
        &normalized
    } else {
        &contents
    };
    let value: serde_json::Value = serde_json::from_str(json_text)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok(keys
        .iter()
        .any(|k| value.get(*k).and_then(|s| s.get("slugaudit")).is_some()))
}

/// Writes the `slugaudit` entry in the shape the client reads: exactly one key
/// merged into a file we can parse. Refuses when the path or shape is unknown,
/// and never rewrites a config it could not parse or whose comments would be
/// destroyed.
pub(super) fn write_agent_config(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
    note: Option<&str>,
) -> Result<(), ConnectError> {
    if !guidance::config_writable(agent) {
        return Err(guidance::refusal(agent, binary, home));
    }
    let home = home.ok_or(ConnectError::HomeUnavailable)?;
    let refuse = || guidance::refusal(agent, binary, Some(home));
    let path = agent.config.resolve(home).ok_or_else(refuse)?;
    let section = agent.shape.write_section().ok_or_else(refuse)?;
    write_json_server(&path, section, agent.shape.entry(binary)?)?;
    note.map(|note| crate::cli::out(format_args!("{note}\n")))
        .transpose()?;
    let action = format!("Wrote {section}.slugaudit to {}", path.display());
    guidance::print_lines(&guidance::success_lines(agent, &action))
}

fn parse_config_text(path: &Path, raw: &str) -> Result<serde_json::Value, ConnectError> {
    let normalized;
    let text = if is_jsonc_path(path) {
        normalized = strip_jsonc_comments_and_trailing_commas(raw);
        &normalized
    } else {
        raw
    };
    if text.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_str(text).map_err(|source| ConnectError::InvalidConfig {
        path: path.to_path_buf(),
        source,
    })
}

fn write_json_atomic(path: &Path, root: &serde_json::Value) -> Result<(), ConnectError> {
    let mut text = serde_json::to_string_pretty(root)?;
    text.push('\n');
    write_atomic(path, text.as_bytes())
}

pub(super) fn write_json_server(
    path: &Path,
    section: &str,
    val: serde_json::Value,
) -> Result<(), ConnectError> {
    path.parent().map(fs::create_dir_all).transpose()?;
    let raw = readable_config(path)?;
    let mut root = match raw.as_deref() {
        Some(text) => parse_config_text(path, text)?,
        None => serde_json::json!({}),
    };
    if let Some(raw_text) = raw.as_deref()
        && is_jsonc_path(path)
        && has_jsonc_comments(raw_text)
    {
        let snippet =
            serde_json::to_string_pretty(&serde_json::json!({ section: { "slugaudit": val } }))?;
        return Err(ConnectError::CommentedConfig {
            path: path.to_path_buf(),
            advice: format!(
                "paste the \"slugaudit\" entry under \"{section}\" in {} manually:\n{snippet}",
                path.display()
            ),
        });
    }
    let map = root
        .as_object_mut()
        .ok_or_else(|| ConnectError::RootNotAnObject {
            path: path.to_path_buf(),
        })?;
    let s = map.entry(section).or_insert_with(|| serde_json::json!({}));
    s.as_object_mut()
        .ok_or_else(|| ConnectError::SectionNotAnObject {
            section: section.to_owned(),
        })?
        .insert("slugaudit".into(), val);
    write_json_atomic(path, &root)
}

/// Removes the entry from `path` when the file exists; reports whether an
/// entry was actually removed. A file with no slugaudit entry is left
/// untouched — never rewritten merely because it exists — and a commented
/// `.jsonc` file is never overwritten.
pub(super) fn remove_json_slugaudit(path: &Path, keys: &[&str]) -> Result<bool, ConnectError> {
    let Some(raw) = readable_config(path)? else {
        return Ok(false);
    };
    let mut root = parse_config_text(path, &raw)?;
    let mut removed = false;
    for key in keys {
        if let Some(s) = root.get_mut(*key).and_then(|s| s.as_object_mut()) {
            removed |= s.remove("slugaudit").is_some();
        }
    }
    if removed {
        if is_jsonc_path(path) && has_jsonc_comments(&raw) {
            return Err(ConnectError::CommentedConfig {
                path: path.to_path_buf(),
                advice: format!(
                    "remove the \"slugaudit\" entry from {} manually",
                    path.display()
                ),
            });
        }
        write_json_atomic(path, &root)?;
    }
    Ok(removed)
}

#[cfg(test)]
#[path = "config_file_tests.rs"]
mod tests;
