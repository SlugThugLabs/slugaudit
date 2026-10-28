//! Low-level JSON config-file access for `connect` and `disconnect`: detect an
//! existing `slugaudit` entry, merge exactly one key into a file this tool can
//! parse, and remove it again. The registration policy above it (probe-or-
//! fallback dispatch and user-facing guidance) stays in
//! [`crate::connect_exec`].

pub(crate) use super::config_atomic::write_atomic;
#[cfg(test)]
pub(crate) use super::config_atomic::write_atomic_with;
use crate::cli::ConnectError;
use crate::connect_agents::AgentDef;
use crate::connect_guidance as guidance;
use std::fs;
use std::path::Path;

/// Classifies `path` before anything opens it: `false` when there is no file,
/// `true` only for a regular file (following a symlink), and an error for
/// anything a read or write must not touch. `File::open` on a FIFO blocks
/// until a writer appears, so the type is judged from metadata first.
fn regular_config(path: &Path) -> std::io::Result<bool> {
    let target = match fs::symlink_metadata(path) {
        Ok(entry) if entry.file_type().is_symlink() => match fs::metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            result => result?,
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        result => result?,
    };
    if target.is_file() {
        return Ok(true);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "the client config path is not a regular file",
    ))
}

/// Reads a client config for detection: `None` for an absent file or one that
/// is not a regular file, with the latter logged once. A FIFO must never be
/// opened by a probe.
pub(crate) fn readable_config_or_warn(path: &Path) -> Option<String> {
    match regular_config(path) {
        Ok(true) => crate::probe::read_config_or_warn(path),
        Ok(false) => None,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "could not inspect client config; treating it as not registered",
            );
            None
        }
    }
}

/// True when the JSON config at `path` carries a `slugaudit` entry under any
/// of `sections`.
pub fn has_slugaudit(path: &Path, keys: &[&str]) -> bool {
    // Advisory probe: an oversized, unreadable or malformed config reads as
    // "not registered", logged with the cause by `readable_config_or_warn` /
    // below; the write path bounds the same file again and fails explicitly.
    let Some(contents) = readable_config_or_warn(path) else {
        return false;
    };
    match serde_json::from_str::<serde_json::Value>(&contents) {
        Ok(value) => keys
            .iter()
            .any(|k| value.get(*k).and_then(|s| s.get("slugaudit")).is_some()),
        Err(error) => {
            tracing::warn!(
                path = %crate::util::redacted_path(path),
                error = %error,
                "client config is not valid JSON; treating it as not registered",
            );
            false
        }
    }
}

/// Writes the `slugaudit` entry in the shape the client reads: exactly one key
/// merged into a file we can parse. Refuses when the path or shape is unknown,
/// and never rewrites a config it could not parse.
pub(crate) fn write_agent_config(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
    note: Option<&str>,
) -> Result<(), ConnectError> {
    if !guidance::config_writable(agent) {
        return Err(guidance::refusal(agent, binary, home));
    }
    let home = home.ok_or(ConnectError::HomeUnavailable)?;
    let path = agent
        .config
        .resolve(home)
        .ok_or_else(|| guidance::refusal(agent, binary, Some(home)))?;
    let section = agent
        .shape
        .write_section()
        .ok_or_else(|| guidance::refusal(agent, binary, Some(home)))?;
    write_json_server(&path, section, agent.shape.entry(binary)?)?;
    note.map(|note| crate::cli::out(format_args!("{note}\n")))
        .transpose()?;
    let action = format!("Wrote {section}.slugaudit to {}", path.display());
    guidance::print_lines(&guidance::success_lines(agent, &action))
}

fn load_json_or_empty(path: &Path) -> Result<serde_json::Value, ConnectError> {
    // A path that exists but is not a regular file (a FIFO, directory, or
    // device) is refused before the read, which could block or misread it.
    if !regular_config(path)? {
        return Ok(serde_json::json!({}));
    }
    match crate::util::read_file_capped(path, crate::util::MAX_FOREIGN_TEXT_BYTES) {
        Ok(c) if c.trim().is_empty() => Ok(serde_json::json!({})),
        Ok(c) => serde_json::from_str(&c).map_err(|source| ConnectError::InvalidConfig {
            path: path.to_path_buf(),
            source,
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(err) => Err(ConnectError::Io(err)),
    }
}

/// Pretty JSON ending in a newline, the shape editors and `cat` expect.
fn pretty_with_newline(root: &serde_json::Value) -> Result<String, ConnectError> {
    let mut text = serde_json::to_string_pretty(root)?;
    text.push('\n');
    Ok(text)
}

pub(super) fn write_json_server(
    path: &Path,
    section: &str,
    val: serde_json::Value,
) -> Result<(), ConnectError> {
    path.parent().map(fs::create_dir_all).transpose()?;
    let mut root = load_json_or_empty(path)?;
    let map = root
        .as_object_mut()
        .ok_or_else(|| ConnectError::RootNotAnObject {
            path: path.to_path_buf(),
        })?;
    let s = map.entry(section).or_insert_with(|| serde_json::json!({}));
    // An `if let` here meant a section key holding a non-object silently
    // skipped the insert, the file was rewritten unchanged, and the caller
    // still printed "Wrote {section}.slugaudit to {path}". Refusing is the
    // honest answer.
    s.as_object_mut()
        .ok_or_else(|| ConnectError::SectionNotAnObject {
            section: section.to_owned(),
        })?
        .insert("slugaudit".into(), val);
    write_atomic(path, pretty_with_newline(&root)?.as_bytes())
}

/// Removes the entry from `path` when the file exists; reports whether an
/// entry was actually removed. A file with no slugaudit entry is left
/// untouched — never rewritten merely because it exists.
pub(crate) fn remove_json_slugaudit(path: &Path, keys: &[&str]) -> Result<bool, ConnectError> {
    // A stat failure (e.g. an unreadable parent directory) fails the
    // disconnect instead of skipping the file and reporting success.
    if !path.try_exists()? {
        return Ok(false);
    }
    let mut root = load_json_or_empty(path)?;
    let mut removed = false;
    for key in keys {
        if let Some(s) = root.get_mut(*key).and_then(|s| s.as_object_mut()) {
            removed |= s.remove("slugaudit").is_some();
        }
    }
    if removed {
        write_atomic(path, pretty_with_newline(&root)?.as_bytes())?;
    }
    Ok(removed)
}

#[cfg(test)]
#[path = "config_file_tests.rs"]
mod tests;
