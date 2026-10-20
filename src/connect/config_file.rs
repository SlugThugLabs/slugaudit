//! Low-level JSON config-file access for `connect` and `disconnect`: detect an
//! existing `slugaudit` entry, merge exactly one key into a file this tool can
//! parse, and remove it again. The registration policy above it (probe-or-
//! fallback dispatch and user-facing guidance) stays in
//! [`crate::connect_exec`].

use crate::cli::ConnectError;
use crate::connect_agents::AgentDef;
use crate::connect_guidance as guidance;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
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
    write_json_server(&path, section, agent.shape.entry(binary)?)?;
    if let Some(note) = note {
        crate::cli::out(format_args!("{note}\n"))?;
    }
    let action = format!("Wrote {section}.slugaudit to {}", path.display());
    guidance::print_lines(&guidance::success_lines(agent, &action))
}

static ATOMIC_WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The file an atomic write must replace. A config reached through a symlink
/// (a dotfiles setup) is written through to its target, so the link survives;
/// a dangling link is refused rather than replaced.
fn write_destination(path: &Path) -> Result<PathBuf, ConnectError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::canonicalize(path).map_err(ConnectError::Io)
        }
        Ok(_) => Ok(path.to_path_buf()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(error) => Err(ConnectError::Io(error)),
    }
}

/// Replaces `path` with `content` through a temp file in the same directory.
///
/// The temp is created owner-only (0600 on Unix) before any bytes are written,
/// so a client config holding other servers' credentials never becomes
/// group/world-readable. The destination inode is replaced atomically; a
/// failed write or rename removes the temp, and a cleanup failure is logged.
pub(crate) fn write_atomic(path: &Path, content: &[u8]) -> Result<(), ConnectError> {
    write_atomic_with(
        path,
        content,
        |file| file.sync_all(),
        |parent| fs::File::open(parent)?.sync_all(),
    )
}

/// The atomic write with its durability steps injected, so both failure arms
/// are testable without a production-only branch.
///
/// `sync_file` runs after the bytes are written and before the rename: it
/// observes any deferred write error while the destination is still untouched.
/// The rename is the commit point. `sync_directory` is durability for the
/// rename itself; its failure happens after the write is already visible, so
/// it is logged and the successful write is still reported as successful.
fn write_atomic_with(
    path: &Path,
    content: &[u8],
    sync_file: impl FnOnce(&fs::File) -> std::io::Result<()>,
    sync_directory: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<(), ConnectError> {
    let destination = write_destination(path)?;
    let parent = destination.parent().ok_or(ConnectError::HomeUnavailable)?;
    let count = ATOMIC_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".slugaudit-tmp-{}-{}", std::process::id(), count));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    if let Err(error) = file.write_all(content) {
        // Close before removing: an open handle would make removal fail on
        // some platforms.
        drop(file);
        remove_abandoned_temp(&temp);
        return Err(ConnectError::Io(error));
    }
    if let Err(error) = sync_file(&file) {
        drop(file);
        remove_abandoned_temp(&temp);
        return Err(ConnectError::Io(error));
    }
    drop(file);
    if let Err(error) = fs::rename(&temp, &destination) {
        remove_abandoned_temp(&temp);
        return Err(ConnectError::Io(error));
    }
    if let Err(error) = sync_directory(parent) {
        tracing::warn!(
            error = %error,
            "client config renamed but its directory was not synced; the file is on disk, but the rename may not survive a crash"
        );
    }
    Ok(())
}

/// Removes the temp left by a failed atomic write. The write error is already
/// on its way to the caller; a cleanup failure is logged once and never
/// replaces it.
fn remove_abandoned_temp(temp: &Path) {
    match fs::remove_file(temp) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            // Deliberate raw path: one-shot diagnostic naming the leftover
            // temp file beside the config it was meant to replace.
            tracing::warn!(error = %error, path = %temp.display(), "failed to remove temporary file after a failed config write");
        }
    }
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
        write_atomic(path, serde_json::to_string_pretty(&root)?.as_bytes())?;
    }
    Ok(removed)
}

#[cfg(test)]
#[path = "config_file_tests.rs"]
mod tests;
