//! Atomic, mode-preserving replacement of a client config file.

use super::ConnectError;
use std::fs;
use std::path::{Path, PathBuf};

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

/// Replaces `path` with `content` through a temp file in the same directory,
/// preserving permissions and dotfile symlinks.
pub(super) fn write_atomic(path: &Path, content: &[u8]) -> Result<(), ConnectError> {
    write_atomic_with(
        path,
        content,
        fs::File::sync_all,
        crate::util::sync_directory,
    )
}

/// Atomic config replacement with durability hooks injected so failure paths
/// stay deterministically testable.
pub(super) fn write_atomic_with(
    path: &Path,
    content: &[u8],
    sync_file: impl FnOnce(&fs::File) -> std::io::Result<()>,
    sync_directory: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<(), ConnectError> {
    let destination = write_destination(path)?;
    if destination.parent().is_none() {
        return Err(ConnectError::HomeUnavailable);
    }
    crate::util::write_atomic_chunks_with(&destination, &[content], sync_file, sync_directory)
        .map_err(ConnectError::Io)
}

#[cfg(test)]
#[path = "config_atomic_tests.rs"]
mod tests;
