//! Atomic, mode-preserving replacement of a client config file.

use crate::cli::ConnectError;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
    let sync_file = |file: &fs::File| file.sync_all();
    let sync_directory = |parent: &Path| fs::File::open(parent)?.sync_all();
    write_atomic_with(path, content, sync_file, sync_directory)
}

/// The atomic write with its durability steps injected, so both failure arms
/// are testable without a production-only branch.
///
/// `sync_file` runs after the bytes are written and before the rename: it
/// observes any deferred write error while the destination is still untouched.
/// The rename is the commit point. `sync_directory` is durability for the
/// rename itself; its failure happens after the write is already visible, so
/// it is logged and the successful write is still reported as successful.
pub(crate) fn write_atomic_with(
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
    // A replaced config keeps the mode the user gave it; only a newly created
    // file stays at the owner-only 0600 the temp was opened with.
    if let Err(error) = preserve_mode(&file, &destination) {
        drop(file);
        remove_abandoned_temp(&temp);
        return Err(ConnectError::Io(error));
    }
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

/// Copies the permissions of an existing `destination` onto `file`; an absent
/// destination leaves the new file's mode alone.
fn preserve_mode(file: &fs::File, destination: &Path) -> std::io::Result<()> {
    match fs::metadata(destination) {
        Ok(metadata) => file.set_permissions(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
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

#[cfg(test)]
#[path = "config_atomic_tests.rs"]
mod tests;
