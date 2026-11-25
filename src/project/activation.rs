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
    /// Filesystem work on the activation directory failed.
    #[error("failed to update the activation directory: {0}")]
    Io(#[from] std::io::Error),
    /// This process's session findings files could not be set up when it
    /// first touched the project: locking its own file or deleting a finished
    /// session's file failed.
    #[error("failed to set up the session findings files at '{path}': {source}")]
    FindingsSession {
        /// The file or directory the failed step was working on.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// This process's table of findings sessions was poisoned by a panic.
    #[error(transparent)]
    Lock(#[from] crate::util::LockPoisoned),
    /// Setting up `.planning/` and its README failed.
    #[error("failed to set up .planning: {0}")]
    Planning(#[from] slugplan::Error),
}

/// The SlugAudit-owned runtime directory under a resolved user project:
/// `<project-root>/.planning/slugaudit/`. The surrounding `.planning`
/// directory remains user project data.
#[must_use]
pub fn activation_dir(root: &ProjectRoot) -> PathBuf {
    activation_path(root.as_path())
}

fn activation_path(project_root: &Path) -> PathBuf {
    project_root.join(PLANNING_DIR).join(ACTIVATION_DIR)
}

/// The one "is this project on?" predicate: the project is on iff
/// `<activation>` is a directory. Nothing inside the directory counts:
/// `project.db` is disposable scratch that the next tool call rebuilds. A
/// path that does not exist, or sits under a non-directory, is "off"; any
/// other `stat` failure is returned rather than read as "off".
fn is_active_at(activation: &Path) -> std::io::Result<bool> {
    match activation.metadata() {
        Ok(metadata) => Ok(metadata.is_dir()),
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

/// Finds the nearest ancestor of `start` (inclusive) that has a
/// `.planning/slugaudit` directory. `start` may be a file, in which
/// case its parent directory is the search origin.
///
/// # Errors
///
/// Returns an error if `start` cannot be resolved, if an activation path
/// along the way cannot be inspected, or if no ancestor has an activation
/// directory.
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
        if is_active_at(&activation_path(ancestor))? {
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

// Owner decision (2026-10-31): the directory is the on/off switch, created only by `on`, so a global install does not index every repo; a repo opts in.
/// Creates the `.planning/slugaudit` directory under `root`: turning
/// SlugAudit "on". `.planning/` itself and its README come from
/// `slugplan::ensure`, the same in every slug tool; SlugAudit creates only
/// its own folder in it. The directory is the whole switch, and this is the
/// only code that creates it; the database inside is built by the caller
/// (`project_control on`) or, if missing, by the next tool call. Idempotent:
/// succeeds silently if the directories exist.
///
/// # Errors
///
/// Returns an error if `.planning/`, its README or the directory can't be
/// created.
pub fn enable(root: &ProjectRoot) -> Result<PathBuf, ActivationError> {
    slugplan::ensure(root.as_path())?;
    let activation = activation_dir(root);
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
/// `.planning/slugaudit` directory, whatever is in it (an old install's
/// `config.json` included): everything in it is SlugAudit's scratch. `.planning` is removed afterwards only if it is
/// empty, because the rest of it is the user's.
///
/// `remove_dir_all` never follows a symlink inside the directory. If
/// `.planning/slugaudit` is itself a symlink, only the link is removed and its
/// target is left alone.
///
/// An absent directory is not an error and reports nothing removed.
///
/// # Errors
///
/// Returns an error if any removal fails.
pub fn disable(root: &ProjectRoot) -> Result<DisableReport, ActivationError> {
    let activation = activation_dir(root);
    // Close this process's lock handle first: on Windows a locked file can't
    // be deleted. Another process's live session on Windows makes the removal
    // below fail, and that error is returned, not skipped.
    super::findings_session::end_process_session(root)?;
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
    // Clear again after removal so a concurrent `begin_process_session` cannot
    // leave a stale lock FD for the unlinked directory in `SESSIONS`.
    super::findings_session::end_process_session(root)?;
    report.removed.push(activation);
    let planning = root.as_path().join(PLANNING_DIR);
    match std::fs::remove_dir(&planning) {
        Ok(()) => report.removed.push(planning),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::DirectoryNotEmpty
                    | std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::NotADirectory
            ) => {}
        Err(error) => return Err(ActivationError::Io(error)),
    }
    Ok(report)
}

#[cfg(test)]
#[path = "activation_tests.rs"]
mod tests;
