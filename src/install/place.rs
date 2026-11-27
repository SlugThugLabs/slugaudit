//! Placing a staged binary over the installed one — the step `install` and
//! `update` share once each has a complete executable beside its target.
//! One implementation, so a failed install and a failed update leave the
//! same state behind: the old binary in place and no staged copy.

use super::slugthug_home;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

/// The name retained for the binary a replacement install or update displaces.
/// Both `slugaudit install` and `slugaudit update` publish it next to the
/// binary they replace, so a revert is a copy in the same directory.
pub const PREVIOUS_BINARY_NAME: &str = "slugaudit.prev";

static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Monotonic per-process sequence counter used to keep staging filenames unique.
pub(super) fn next_staging_seq() -> u64 {
    STAGING_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// A placement step that failed, naming the step and the path it hit.
#[derive(Debug, Error)]
#[error("could not {action} {}: {source}", path.display())]
pub struct PlaceError {
    /// Placement step that failed.
    pub action: &'static str,
    /// Path being operated on when the step failed.
    pub path: PathBuf,
    /// Underlying filesystem error.
    #[source]
    pub source: std::io::Error,
}

/// Copies `source` into a brand-new `temporary` file created with `O_EXCL`
/// (`create_new(true)`), preserving `source`'s permissions and syncing the
/// bytes before returning.
pub(super) fn stage_copy(source: &Path, temporary: &Path) -> std::io::Result<()> {
    let metadata = std::fs::metadata(source)?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the source path is neither a regular file nor a symlink to a regular file",
        ));
    }
    let mut src = std::fs::File::open(source)?;
    let mut dst = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)?;
    let written = (|| -> std::io::Result<()> {
        dst.set_permissions(metadata.permissions())?;
        std::io::copy(&mut src, &mut dst)?;
        dst.sync_all()
    })();
    drop(dst);
    if let Err(error) = written {
        remove_leftover(temporary, "could not remove staged binary");
        return Err(error);
    }
    Ok(())
}

/// Copies the binary at `target` to `slugaudit.prev` beside it, so the install
/// or update that follows can be undone. Returns `None` when there is nothing
/// to retain (a first install).
///
/// Called **before** the replacement rename, so a crash during replacement
/// leaves the current binary intact, and the retained copy is a complete file.
/// Staging uses `create_new(true)` so a pre-existing file or symlink is never
/// followed.
pub(super) fn retain_previous_binary(target: &Path) -> Result<Option<PathBuf>, std::io::Error> {
    if !target.try_exists()? {
        return Ok(None);
    }
    let Some(directory) = target.parent() else {
        return Ok(None);
    };
    let previous = directory.join(PREVIOUS_BINARY_NAME);
    let stamp = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_nanos(),
        Err(_) => 0,
    };
    let seq = next_staging_seq();
    let temporary = directory.join(format!(
        ".{PREVIOUS_BINARY_NAME}.{}.{stamp}.{seq}.tmp",
        std::process::id()
    ));
    stage_copy(target, &temporary)?;
    if let Err(error) = std::fs::rename(&temporary, &previous) {
        remove_leftover(&temporary, "could not remove staged previous binary");
        return Err(error);
    }
    Ok(Some(previous))
}

/// Moves the complete executable at `staged` to `target`: marks it
/// executable, retains the binary it displaces as `slugaudit.prev`, renames
/// it into place, then removes any leftover `slugaudit-mcp` files.
///
/// On failure the staged file is removed before the error returns — it is a
/// full copy of the binary, and nothing would ever reclaim it.
pub(crate) fn place_staged_binary(staged: &Path, target: &Path) -> Result<(), PlaceError> {
    let result = place(staged, target);
    if result.is_err() {
        remove_leftover(staged, "could not remove staged binary");
    }
    result?;
    if let Some(directory) = target.parent() {
        remove_leftover(
            &directory.join("slugaudit-mcp"),
            "could not remove stale binary",
        );
    }
    if let Some(home) = slugthug_home() {
        remove_leftover(
            &home.join("bin").join("slugaudit-mcp"),
            "could not remove stale binary",
        );
    }
    Ok(())
}

fn place(staged: &Path, target: &Path) -> Result<(), PlaceError> {
    let step = |action, path: &Path| {
        let path = path.to_path_buf();
        move |source| PlaceError {
            action,
            path,
            source,
        }
    };
    make_executable(staged).map_err(step("mark executable", staged))?;
    sync_staged_file(staged).map_err(step("sync the staged binary", staged))?;
    let previous = target.with_file_name(PREVIOUS_BINARY_NAME);
    retain_previous_binary(target).map_err(step("retain the previous binary at", &previous))?;
    std::fs::rename(staged, target).map_err(step("move the new binary to", target))?;
    if let Some(parent) = target.parent() {
        sync_directory_best_effort(parent);
    }
    Ok(())
}

/// Flushes staged binary data and metadata to disk before renaming it into
/// place, so a crash after the rename cannot expose a zero-length executable.
fn sync_staged_file(path: &Path) -> Result<(), std::io::Error> {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)?
        .sync_all()
}

/// Best-effort directory `fsync` after an atomic rename so the updated
/// directory entry is durable on filesystems that support it.
fn sync_directory_best_effort(dir: &Path) {
    match std::fs::File::open(dir).and_then(|handle| handle.sync_all()) {
        Ok(()) => {}
        Err(error) => {
            tracing::warn!(
                path = %dir.display(),
                error = %error,
                "could not sync install directory after rename"
            );
        }
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

/// Best-effort removal of a file this flow created or superseded. A missing
/// file is the goal state, not a failure.
///
/// The log names the real path: a one-shot cleanup diagnostic beside the
/// installed binary — naming the file the operator can see is the point.
pub(crate) fn remove_leftover(path: &Path, message: &str) {
    if let Err(error) = std::fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %path.display(), error = %error, "{message}");
    }
}
