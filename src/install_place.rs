//! Placing a staged binary over the installed one — the step `install` and
//! `update` share once each has a complete executable beside its target.
//! One implementation, so a failed install and a failed update leave the
//! same state behind: the old binary in place and no staged copy.

use crate::install::{PREVIOUS_BINARY_NAME, slugthug_home};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// A placement step that failed, naming the step and the path it hit.
#[derive(Debug, Error)]
#[error("could not {action} {}: {source}", path.display())]
pub struct PlaceError {
    pub action: &'static str,
    pub path: PathBuf,
    #[source]
    pub source: std::io::Error,
}

/// Moves the complete executable at `staged` to `target`: marks it
/// executable, retains the binary it displaces as `slugaudit.prev`, renames
/// it into place, then removes the pre-rename `slugaudit-mcp` leftovers.
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
    let previous = target.with_file_name(PREVIOUS_BINARY_NAME);
    crate::install::retain_previous_binary(target)
        .map_err(step("retain the previous binary at", &previous))?;
    std::fs::rename(staged, target).map_err(step("move the new binary to", target))
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
