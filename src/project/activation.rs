pub use super::gitignore::{GitignoreOutcome, ensure_gitignore};

#[cfg(test)]
pub(super) use super::gitignore::GITIGNORE_FILE;
#[cfg(test)]
use crate::util::MAX_FOREIGN_TEXT_BYTES;

use std::path::{Path, PathBuf};
use thiserror::Error;

const PLANNING_DIR: &str = ".planning";
const ACTIVATION_DIR: &str = "slugaudit";
const DATABASE_FILENAME: &str = "project.db";

#[derive(Debug, Error)]
/// Why a path could not be resolved to an enabled project root.
pub enum RootError {
    /// The path could not be resolved to a canonical path.
    #[error("failed to resolve project root: {0}")]
    Unresolvable(#[source] std::io::Error),
    /// The path resolved, but to something other than a directory.
    #[error("project root is not a directory")]
    NotADirectory,
}

/// A canonical, on-disk project root directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoot(PathBuf);

impl ProjectRoot {
    /// Resolves `path` to a canonical existing directory.
    ///
    /// # Errors
    ///
    /// Returns [`RootError`] if `path` cannot be canonicalized or is not a directory.
    pub fn resolve(path: &Path) -> Result<Self, RootError> {
        let canonical = path.canonicalize().map_err(RootError::Unresolvable)?;
        if !std::fs::metadata(&canonical)
            .map_err(RootError::Unresolvable)?
            .is_dir()
        {
            return Err(RootError::NotADirectory);
        }
        Ok(Self(canonical))
    }

    #[must_use]
    /// The resolved, canonical project root.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Returns the SlugAudit scratch index database path:
/// `<project-root>/.planning/slugaudit/project.db`.
#[must_use]
pub fn database_path(root: &ProjectRoot) -> PathBuf {
    activation_dir(root).join(DATABASE_FILENAME)
}

#[derive(Debug, Error)]
/// Failures from enabling or disabling a project or resolving its activation directory.
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
    /// Session findings setup or dead-session cleanup failed.
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

/// Returns `<project-root>/.planning/slugaudit/`.
#[must_use]
pub fn activation_dir(root: &ProjectRoot) -> PathBuf {
    activation_path(root.as_path())
}

fn activation_path(project_root: &Path) -> PathBuf {
    project_root.join(PLANNING_DIR).join(ACTIVATION_DIR)
}

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
/// `.planning/slugaudit` directory.
///
/// # Errors
///
/// Returns [`ActivationError`] if `start` cannot be canonicalized, no ancestor
/// has `.planning/slugaudit`, or inspecting an activation path fails.
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

/// Resolves `start` to an active [`ProjectRoot`] and its `project.db` path.
///
/// # Errors
///
/// Propagates any [`ActivationError`] from [`find_project_root`].
pub fn resolve_project(start: &Path) -> Result<(ProjectRoot, PathBuf), ActivationError> {
    let root = find_project_root(start)?;
    let db = database_path(&root);
    Ok((root, db))
}

// Owner decision (2026-10-31): the directory is the on/off switch, created only by `on`, so a global install does not index every repo; a repo opts in.
/// Creates `.planning/slugaudit` under `root` (`project_control on`).
///
/// # Errors
///
/// Returns [`ActivationError`] if `.planning` bootstrap (`slugplan::ensure`) or
/// creating `.planning/slugaudit` fails.
pub fn enable(root: &ProjectRoot) -> Result<PathBuf, ActivationError> {
    slugplan::ensure(root.as_path())?;
    let activation = activation_dir(root);
    std::fs::create_dir_all(&activation)?;
    Ok(activation)
}

/// Paths removed by [`disable`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DisableReport {
    /// `.planning/slugaudit` (and `.planning` when left empty).
    pub removed: Vec<PathBuf>,
}

/// Deletes `.planning/slugaudit` under `root` (`project_control off`), leaving
/// non-empty `.planning` user files untouched.
///
/// # Errors
///
/// Returns [`ActivationError`] if clearing session findings state, inspecting,
/// or removing `.planning/slugaudit` fails.
pub fn disable(root: &ProjectRoot) -> Result<DisableReport, ActivationError> {
    let activation = activation_dir(root);
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
