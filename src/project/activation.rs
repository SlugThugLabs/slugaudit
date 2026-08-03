use super::root::{ProjectRoot, RootError};
use std::path::{Path, PathBuf};
use thiserror::Error;

const PLANNING_DIR: &str = ".planning";
const ACTIVATION_DIR: &str = "slugaudit";

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
/// SlugAudit "off," discarding its database (and every finding/evidence
/// row in it) along with the marker. Returns `false` if it was already
/// absent rather than treating that as an error.
///
/// # Errors
///
/// Returns an error if a `.planning`/`.planning/slugaudit` component is a
/// symlink (refuses to remove through one), or if removal fails.
pub fn disable(root: &ProjectRoot) -> Result<bool, ActivationError> {
    let activation = validated_activation_dir(root.as_path())?;
    if !activation.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&activation)?;
    Ok(true)
}

#[cfg(test)]
#[path = "activation_tests.rs"]
mod tests;
