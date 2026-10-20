//! Low-level JSON config-file access for `connect` and `disconnect`: detect an
//! existing `slugaudit` entry, merge exactly one key into a file this tool can
//! parse, and remove it again. The registration policy above it (probe-or-
//! fallback dispatch and user-facing guidance) stays in
//! [`crate::connect_exec`].

use crate::cli::ConnectError;
use crate::connect_agents::AgentDef;
use crate::connect_guidance as guidance;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// True when the JSON config at `path` carries a `slugaudit` entry under any
/// of `sections`.
pub fn has_slugaudit(path: &Path, keys: &[&str]) -> bool {
    // Advisory probe: an oversized, unreadable or malformed config reads as
    // "not registered", logged with the cause by `read_config_or_warn` / below;
    // the write path bounds the same file again and fails explicitly.
    let Some(contents) = crate::probe::read_config_or_warn(path) else {
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
    write_json_server(&path, section, agent.shape.entry(binary))?;
    if let Some(note) = note {
        crate::cli::out(format_args!("{note}\n"))?;
    }
    let action = format!("Wrote {section}.slugaudit to {}", path.display());
    print_lines(&guidance::success_lines(agent, &action))
}

pub(crate) fn print_lines(lines: &[String]) -> Result<(), ConnectError> {
    for line in lines {
        crate::cli::out(format_args!("{line}\n"))?;
    }
    Ok(())
}

static ATOMIC_WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn write_atomic(path: &Path, content: &[u8]) -> Result<(), ConnectError> {
    let parent = path.parent().ok_or(ConnectError::HomeUnavailable)?;
    let count = ATOMIC_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".slugaudit-tmp-{}-{}", std::process::id(), count));
    fs::write(&temp, content)?;
    if let Err(err) = fs::rename(&temp, path) {
        if let Err(remove_err) = fs::remove_file(&temp) {
            // Deliberate raw path: one-shot diagnostic naming the leftover
            // temp file beside the config it was meant to replace.
            tracing::warn!(error = %remove_err, path = %temp.display(), "failed to remove temporary file after rename error");
        }
        return Err(ConnectError::Io(err));
    }
    Ok(())
}

fn load_json_or_empty(path: &Path) -> Result<serde_json::Value, ConnectError> {
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

fn write_json_server(
    path: &Path,
    section: &str,
    val: serde_json::Value,
) -> Result<(), ConnectError> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
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
    write_atomic(path, serde_json::to_string_pretty(&root)?.as_bytes())
}

/// Removes the entry from `path` when the file exists; reports whether there
/// was a file to touch.
pub(crate) fn remove_json_slugaudit(path: &Path, keys: &[&str]) -> Result<bool, ConnectError> {
    // A stat failure (e.g. an unreadable parent directory) fails the
    // disconnect instead of skipping the file and reporting success.
    if !path.try_exists()? {
        return Ok(false);
    }
    let mut root = load_json_or_empty(path)?;
    for key in keys {
        if let Some(s) = root.get_mut(*key).and_then(|s| s.as_object_mut()) {
            s.remove("slugaudit");
        }
    }
    write_atomic(path, serde_json::to_string_pretty(&root)?.as_bytes())?;
    Ok(true)
}
