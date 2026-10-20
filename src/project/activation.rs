pub use super::gitignore::{GitignoreOutcome, ensure_gitignore};

#[cfg(test)]
pub(super) use super::gitignore::GITIGNORE_FILE;
#[cfg(test)]
use crate::util::MAX_FOREIGN_TEXT_BYTES;

use super::activation_lock::acquire_database_lock;
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
/// Failures from enabling or disabling a project: the activation marker, its
/// directory mode, and `.gitignore` maintenance.
pub enum ActivationError {
    /// Resolving the project root failed; see [`RootError`].
    #[error(transparent)]
    Root(#[from] RootError),
    /// No ancestor of the requested path carries an activation marker.
    #[error("SlugAudit is not enabled for this project or any parent directory")]
    NotActive,
    /// A `.planning` or `.planning/slugaudit` component is a symlink, so the
    /// activation path is refused.
    #[error("refusing a symlinked SlugAudit activation path")]
    SymlinkedActivationPath,
    /// Filesystem work on the activation directory failed.
    #[error("failed to update the activation directory: {0}")]
    Io(#[from] std::io::Error),
    /// A database operation failed while disabling the project.
    #[error("database operation failed while disabling the project: {0}")]
    Database(#[source] crate::store::StoreError),
    /// Another connection appears to be using the database, so disabling
    /// could not safely acquire it.
    #[error(
        "could not safely disable: another connection appears to be using the \
         database ({0}). Wait for other SlugAudit tool calls to finish and retry."
    )]
    DatabaseBusy(#[source] rusqlite::Error),
    /// Another connection still holds the database open, so its write-ahead
    /// log could not be checkpointed.
    #[error(
        "could not safely disable: another connection still holds the database open, \
         so its write-ahead log could not be checkpointed. Wait for other SlugAudit \
         tool calls to finish and retry."
    )]
    CheckpointIncomplete,
    /// The database opened but a lock/checkpoint statement failed for a
    /// reason other than contention (for example `SQLITE_IOERR`); the text
    /// carries the underlying SQLite error.
    #[error("database operation failed while disabling the project: {0}")]
    DatabaseOperation(#[source] rusqlite::Error),
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

/// True when `path` exists and is a directory. A missing path is `false`; any
/// other stat failure is returned rather than read as "absent".
fn is_existing_dir(path: &Path) -> std::io::Result<bool> {
    match std::fs::metadata(path) {
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

/// Returns the activation directory under `canonical_root`, refusing a
/// symlinked `.planning` or `.planning/slugaudit` component.
fn validated_activation_dir(canonical_root: &Path) -> Result<PathBuf, ActivationError> {
    let planning = canonical_root.join(PLANNING_DIR);
    let activation = planning.join(ACTIVATION_DIR);
    if is_symlink(&planning)? || is_symlink(&activation)? {
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
/// `.planning/slugaudit` component along the way is a symlink or cannot be
/// inspected, or if no ancestor has an activation directory at all.
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
        let activation = validated_activation_dir(ancestor)?;
        if is_existing_dir(&activation)? {
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
/// project must work *before* the activation marker exists, so it
/// resolves the root with [`ProjectRoot::resolve`] and only then creates
/// the marker.
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
    create_private_dir(&activation)?;
    Ok(activation)
}

/// Owner-only access for the activation directory, which holds the index
/// database and its sidecars — all of them containing source text.
#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;

/// Creates the activation directory with owner-only access.
///
/// `create_dir_all` alone honours the process umask, which is how a customer
/// ends up with a world-readable copy of their own source. This is the one
/// directory inside a user's project that SlugAudit owns, so it is created
/// `0700` — and tightened when an earlier run (or a permissive umask) left it
/// broader, matching the intent of the database file's own permission check.
#[cfg(unix)]
fn create_private_dir(path: &Path) -> Result<(), ActivationError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(path)?;
    let current = std::fs::metadata(path)?.permissions().mode() & 0o777;
    if current != PRIVATE_DIR_MODE {
        // Deliberate raw path: a one-shot security-repair diagnostic — the
        // operator must see *which* directory was world-readable to fix the
        // umask or mount that caused it. Correlation logs use `redacted_path`.
        tracing::warn!(
            target: "slugaudit::project",
            path = %path.display(),
            mode = format!("{current:o}"),
            reason = "tightening_activation_dir",
            "activation directory was not owner-only; tightening it"
        );
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(PRIVATE_DIR_MODE))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> Result<(), ActivationError> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

/// Removes `.planning/slugaudit` under `root` — the one action that turns
/// SlugAudit "off." The activation directory is disposable derived state;
/// it is deleted after acquiring an exclusive database lock so no archive or
/// stale SQLite copy is created.
///
/// Returns `false` if it was already absent rather than treating that as
/// an error.
///
/// # Errors
///
/// Returns an error if a `.planning`/`.planning/slugaudit` component is a
/// symlink (refuses to remove through one), if the database cannot be locked,
/// or if removal fails.
pub fn disable(root: &ProjectRoot) -> Result<bool, ActivationError> {
    let activation = validated_activation_dir(root.as_path())?;
    if !activation.try_exists()? {
        return Ok(false);
    }
    let database_lock = acquire_database_lock(root)?;
    std::fs::remove_dir_all(&activation)?;
    drop(database_lock);
    Ok(true)
}

#[cfg(test)]
#[path = "activation_tests.rs"]
mod tests;
