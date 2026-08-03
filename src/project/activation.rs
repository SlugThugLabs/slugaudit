use super::root::{ProjectRoot, RootError};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

const PLANNING_DIR: &str = ".planning";
const ACTIVATION_DIR: &str = "slugaudit";

/// Top-level trash directory under the shared home (`~/.slugthug/trash/`).
/// Disabled projects are moved here rather than deleted outright, so an
/// accidental `disable` can be recovered by restoring the archived tree.
fn trash_root() -> Option<PathBuf> {
    std::env::var_os("SLUGTHUG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .map(|home| home.join(".slugthug").join("trash"))
}

#[derive(Debug, Error)]
pub enum ActivationError {
    #[error(transparent)]
    Root(#[from] RootError),
    #[error("SlugAudit is not enabled for this project or any parent directory")]
    NotActive,
    #[error("refusing a symlinked SlugAudit activation path")]
    SymlinkedActivationPath,
    #[error("failed to update the activation directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not archive {path} before deletion: {inner}")]
    Archive { path: String, inner: std::io::Error },
}

/// The `.planning/slugaudit` directory under a resolved project root.
#[must_use]
pub fn activation_dir(root: &ProjectRoot) -> PathBuf {
    root.as_path().join(PLANNING_DIR).join(ACTIVATION_DIR)
}

fn is_symlink(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
}

/// Returns the activation directory under `canonical_root`, refusing a
/// symlinked `.planning` or `.planning/slugaudit` component.
fn validated_activation_dir(canonical_root: &Path) -> Result<PathBuf, ActivationError> {
    let planning = canonical_root.join(PLANNING_DIR);
    let activation = planning.join(ACTIVATION_DIR);
    if is_symlink(&planning) || is_symlink(&activation) {
        return Err(ActivationError::SymlinkedActivationPath);
    }
    Ok(activation)
}

/// Finds the nearest ancestor of `start` (inclusive) whose activation
/// directory exists. `start` may be a file, in which case its parent
/// directory is the search origin.
///
/// # Errors
///
/// Returns an error if `start` cannot be resolved, if a `.planning` or
/// `.planning/slugaudit` component along the way is a symlink, or if no
/// ancestor has an activation directory at all.
pub fn find_project_root(start: &Path) -> Result<ProjectRoot, ActivationError> {
    let canonical = start
        .canonicalize()
        .map_err(|error| ActivationError::Root(RootError::Unresolvable(error.to_string())))?;
    let search_origin = if canonical.is_file() {
        canonical
            .parent()
            .map(Path::to_path_buf)
            .ok_or(ActivationError::NotActive)?
    } else {
        canonical
    };

    for ancestor in search_origin.ancestors() {
        let activation = validated_activation_dir(ancestor)?;
        if activation.is_dir() {
            return Ok(ProjectRoot::resolve(ancestor)?);
        }
    }
    Err(ActivationError::NotActive)
}

/// Creates `.planning/slugaudit` under `root` — the one action that turns
/// SlugAudit "on" for a project. Idempotent: succeeds silently if already
/// enabled. Refuses to create through a symlinked `.planning` or
/// `.planning/slugaudit` component, for the same reason `find_project_root`
/// refuses to read through one — the symlink-safety check is shared code,
/// not a separate rule that could drift out of sync with the read path.
///
/// # Errors
///
/// Returns an error if a `.planning`/`.planning/slugaudit` component is a
/// symlink, or if the directory can't be created.
pub fn enable(root: &ProjectRoot) -> Result<PathBuf, ActivationError> {
    let activation = validated_activation_dir(root.as_path())?;
    std::fs::create_dir_all(&activation)?;
    Ok(activation)
}

/// Removes `.planning/slugaudit` under `root` — the one action that turns
/// SlugAudit "off." Before deletion, the activation directory (database,
/// findings, evidence, everything) is archived under `~/.slugthug/trash/`
/// with a timestamped name, so an accidental disable can be recovered by
/// restoring the archived tree. If archiving fails, the disable is aborted
/// (fail closed: never delete without a backup).
///
/// Returns `false` if it was already absent rather than treating that as
/// an error.
///
/// # Errors
///
/// Returns an error if a `.planning`/`.planning/slugaudit` component is a
/// symlink (refuses to remove through one), if archiving fails, or if
/// removal fails.
pub fn disable(root: &ProjectRoot) -> Result<bool, ActivationError> {
    let activation = validated_activation_dir(root.as_path())?;
    if !activation.exists() {
        return Ok(false);
    }
    archive_before_delete(&activation, trash_root().as_deref())?;
    std::fs::remove_dir_all(&activation)?;
    Ok(true)
}

/// Copies `activation` into `<trash_root>/<timestamp>-<project>/` before it
/// is deleted. The copy must succeed before the caller proceeds to
/// `remove_dir_all` — destroying data without a backup is strictly worse
/// than leaving the activation dir untouched. When `trash_root` is `None`
/// (neither `SLUGTHUG_HOME` nor `HOME` is set), archiving is skipped and
/// the delete proceeds; this is the only case where data is deleted without
/// a backup, and it only happens in environments where SlugAudit cannot
/// be installed or connected anyway.
fn archive_before_delete(
    activation: &Path,
    trash_root: Option<&Path>,
) -> Result<(), ActivationError> {
    let Some(trash) = trash_root else {
        return Ok(());
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let project_name = activation
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    let archive = trash.join(format!("{timestamp}-{project_name}"));

    std::fs::create_dir_all(trash).map_err(|inner| ActivationError::Archive {
        path: trash.display().to_string(),
        inner,
    })?;
    copy_dir_recursive(activation, &archive).map_err(|inner| ActivationError::Archive {
        path: archive.display().to_string(),
        inner,
    })?;
    Ok(())
}

/// Recursively copies `src` into `dst`, mirroring the directory tree and
/// all file contents. `std::fs` has no `copy_dir_all`, so this walks the
/// tree itself. Used only for archiving a project before disable.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_child = entry.path();
        let dst_child = dst.join(entry.file_name());
        if src_child.is_dir() {
            copy_dir_recursive(&src_child, &dst_child)?;
        } else {
            std::fs::copy(&src_child, &dst_child)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "activation_tests.rs"]
mod tests;
