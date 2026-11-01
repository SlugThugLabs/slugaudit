pub use super::gitignore::{GitignoreOutcome, ensure_gitignore};

#[cfg(test)]
pub(super) use super::gitignore::GITIGNORE_FILE;
#[cfg(test)]
use crate::util::MAX_FOREIGN_TEXT_BYTES;

use super::database_path::database_path;
use super::root::{ProjectRoot, RootError};
use std::path::{Path, PathBuf};
use thiserror::Error;

// Runtime customer-project state, not repository-development state.
// SlugAudit owns only <project-root>/.planning/slugaudit/; the rest of a
// user's .planning directory remains ordinary project data.
const PLANNING_DIR: &str = ".planning";
const ACTIVATION_DIR: &str = "slugaudit";

#[derive(Debug, Error)]
/// Failures from enabling or disabling a project: the activation directory
/// and `.gitignore` maintenance.
pub enum ActivationError {
    /// Resolving the project root failed; see [`RootError`].
    #[error(transparent)]
    Root(#[from] RootError),
    /// No ancestor of the requested path has a `.planning/slugaudit` directory.
    #[error("SlugAudit is not enabled for this project or any parent directory")]
    NotActive,
    /// `.planning` is a symlink, or `enable` met a symlinked
    /// `.planning/slugaudit`, so the activation path is refused.
    #[error("refusing a symlinked SlugAudit activation path")]
    SymlinkedActivationPath,
    /// Filesystem work on the activation directory failed.
    #[error("failed to update the activation directory: {0}")]
    Io(#[from] std::io::Error),
}

/// The SlugAudit-owned runtime directory under a resolved user project:
/// `<project-root>/.planning/slugaudit/`. The surrounding `.planning`
/// directory remains user project data.
#[must_use]
pub fn activation_dir(root: &ProjectRoot) -> PathBuf {
    root.as_path().join(PLANNING_DIR).join(ACTIVATION_DIR)
}

/// True when `path` is a symlink. A path that does not exist (or sits under a
/// non-directory) is not a symlink; any other `lstat` failure is returned, so
/// the symlink guard fails closed instead of being skipped.
fn is_symlink(path: &Path) -> std::io::Result<bool> {
    match path.symlink_metadata() {
        Ok(metadata) => Ok(metadata.file_type().is_symlink()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// The one "is this project on?" predicate: the project is on iff
/// `<activation>` is a real directory. A symlink there does not count, and
/// neither does anything inside the directory: `project.db` is disposable
/// scratch that the next tool call rebuilds. Any `lstat` failure other than
/// "absent" is returned rather than read as "off".
fn is_active_at(activation: &Path) -> std::io::Result<bool> {
    match activation.symlink_metadata() {
        Ok(metadata) => Ok(metadata.file_type().is_dir()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// Returns the activation directory under `canonical_root`, refusing a
/// symlinked `.planning` component. A symlinked `.planning/slugaudit` is
/// returned as is: callers decide, because it reads as "off" and `off`
/// removes only the link.
fn planning_checked_activation_dir(canonical_root: &Path) -> Result<PathBuf, ActivationError> {
    let planning = canonical_root.join(PLANNING_DIR);
    if is_symlink(&planning)? {
        return Err(ActivationError::SymlinkedActivationPath);
    }
    Ok(planning.join(ACTIVATION_DIR))
}

/// Finds the nearest ancestor of `start` (inclusive) that has a
/// `.planning/slugaudit` directory. `start` may be a file, in which
/// case its parent directory is the search origin.
///
/// # Errors
///
/// Returns an error if `start` cannot be resolved, if a `.planning`
/// component along the way is a symlink or cannot be inspected, or if no
/// ancestor has an activation directory.
pub fn find_project_root(start: &Path) -> Result<ProjectRoot, ActivationError> {
    let canonical = start
        .canonicalize()
        .map_err(|error| ActivationError::Root(RootError::Unresolvable(error)))?;
    let search_origin = if canonical.is_file() {
        canonical
            .parent()
            .map(Path::to_path_buf)
            .ok_or(ActivationError::NotActive)?
    } else {
        canonical
    };

    for ancestor in search_origin.ancestors() {
        let activation = planning_checked_activation_dir(ancestor)?;
        if is_active_at(&activation)? {
            return Ok(ProjectRoot::resolve(ancestor)?);
        }
    }
    Err(ActivationError::NotActive)
}

/// The standard prologue for every state-bearing entry point: resolve
/// `start` to an active project root and its database path in one step.
/// Keeping the find-root → database-path sequence here means the callers
/// that need both (sync's `ensure_current`, the `health` tool) can't
/// drift in ordering or error handling.
///
/// `project_control` deliberately does **not** use this: enabling a
/// project must work *before* the activation directory exists, so it
/// resolves the root with [`ProjectRoot::resolve`] and only then creates
/// the directory.
///
/// # Errors
///
/// Returns an error if `start` can't be resolved or no ancestor is an
/// active project (see [`find_project_root`]).
pub fn resolve_project(start: &Path) -> Result<(ProjectRoot, PathBuf), ActivationError> {
    let root = find_project_root(start)?;
    let database_path = database_path(&root);
    Ok((root, database_path))
}

/// Creates the `.planning/slugaudit` directory under `root`: turning
/// SlugAudit "on". The directory is the whole switch, and this is the only
/// code that creates it; the database inside is built by the caller
/// (`project_control on`) or, if missing, by the next tool call. Idempotent:
/// succeeds silently if the directory exists. Refuses to create through a
/// symlinked `.planning` or `.planning/slugaudit` component, for the same
/// reason `find_project_root` will not read through one.
///
/// # Errors
///
/// Returns an error if a `.planning`/`.planning/slugaudit` component is a
/// symlink, or if the directory can't be created.
pub fn enable(root: &ProjectRoot) -> Result<PathBuf, ActivationError> {
    let activation = planning_checked_activation_dir(root.as_path())?;
    if is_symlink(&activation)? {
        return Err(ActivationError::SymlinkedActivationPath);
    }
    std::fs::create_dir_all(&activation)?;
    Ok(activation)
}

/// What `disable` actually did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DisableReport {
    /// Paths that existed and were removed: the activation directory (or the
    /// symlink that stood in for it), then `.planning` when that left it empty.
    pub removed: Vec<PathBuf>,
}

/// Turns SlugAudit "off" for `root` by deleting the whole
/// `.planning/slugaudit` directory, `config.json` included: everything in it
/// is SlugAudit's scratch. `.planning` is removed afterwards only if it is
/// empty, because the rest of it is the user's.
///
/// `remove_dir_all` never follows a symlink inside the directory. If
/// `.planning/slugaudit` is itself a symlink it is not "on" and not ours to
/// empty, so only the link is removed and its target is left alone.
///
/// An absent directory is not an error and reports nothing removed.
///
/// # Errors
///
/// Returns an error if `.planning` is a symlink (refuses to remove through
/// it), or if any removal fails.
pub fn disable(root: &ProjectRoot) -> Result<DisableReport, ActivationError> {
    let activation = planning_checked_activation_dir(root.as_path())?;
    let mut report = DisableReport::default();
    let metadata = match activation.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(ActivationError::Io(error)),
    };
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(&activation)?;
    } else {
        std::fs::remove_file(&activation)?;
    }
    report.removed.push(activation);
    let planning = root.as_path().join(PLANNING_DIR);
    match std::fs::remove_dir(&planning) {
        Ok(()) => report.removed.push(planning),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
            ) => {}
        Err(error) => return Err(ActivationError::Io(error)),
    }
    Ok(report)
}

#[cfg(test)]
#[path = "activation_tests.rs"]
mod tests;
