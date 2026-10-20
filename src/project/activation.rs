// slugaudit-line-exception: approved-by=slugthug; reason=project activation is one cohesive root-resolution/marker/enable/disable contract; the .gitignore guard and its append path must stay beside the marker they protect, and splitting them would scatter the non-fatal-guard rule from the activation action it never undoes
use super::activation_lock::acquire_database_lock;
use super::database_path::database_path;
use super::root::{ProjectRoot, RootError};
use crate::ignore_rules::ACTIVATION_RELATIVE_DIR;
use crate::util::{CappedReadError, MAX_FOREIGN_TEXT_BYTES, read_file_capped_bytes};
use std::io::Write;
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
    #[error(transparent)]
    Root(#[from] RootError),
    #[error("SlugAudit is not enabled for this project or any parent directory")]
    NotActive,
    #[error("refusing a symlinked SlugAudit activation path")]
    SymlinkedActivationPath,
    #[error("failed to update the activation directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("database operation failed while disabling the project: {0}")]
    Database(#[source] crate::store::StoreError),
    #[error(
        "could not safely disable: another connection appears to be using the \
         database ({0}). Wait for other SlugAudit tool calls to finish and retry."
    )]
    DatabaseBusy(#[source] rusqlite::Error),
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

/// The ignore file at the project root. It belongs to the user: SlugAudit
/// only ever appends one rule to it and never rewrites or removes anything.
const GITIGNORE_FILE: &str = ".gitignore";

/// What [`ensure_gitignore`] did, and to which file.
///
/// There is deliberately no error case here. The activation directory is
/// what makes a project enabled; a `.gitignore` that cannot be read or
/// written is *reported* to the caller, never fatal to the enable, because a
/// convenience guard must not undo the primary action.
///
/// The file is foreign, user-controlled text, so it is read under the same
/// 1 MiB cap as other foreign text files (`MAX_FOREIGN_TEXT_BYTES`). A
/// `.gitignore` over that cap is reported as [`Self::NotWritten`] rather than
/// read, truncated, or appended to; the project stays enabled either way.
///
/// `disable` deliberately does not remove the rule: a `.gitignore` rule can
/// be hiding paths SlugAudit knows nothing about, and un-hiding them is not
/// SlugAudit's call.
#[derive(Debug, PartialEq, Eq)]
pub enum GitignoreOutcome {
    /// The rule was appended; the file was created when it was absent.
    Added(PathBuf),
    /// A rule that already ignores the directory was present; file untouched.
    AlreadyIgnored(PathBuf),
    /// The file was left untouched, and the project is enabled regardless.
    NotWritten { path: PathBuf, reason: String },
}

/// Appends the activation directory to the project-root `.gitignore` unless a
/// rule that already ignores it is present, so a customer's first
/// `git status` after `project_control {"action":"on"}` does not show the
/// index database and its sidecars as untracked files.
///
/// Every existing byte survives: the new contents are the old contents plus
/// the rule, a final line without a newline gets one first, and a file
/// written with CRLF keeps that convention instead of acquiring a mixed one.
/// The write is atomic — the complete contents go to a temp file in the same
/// directory (or beside a symlink's target, so a linked `.gitignore` stays a
/// link) which is then renamed over the path. A failed write therefore
/// leaves the file byte-identical, which is what
/// [`GitignoreOutcome::NotWritten`] promises; the trade-off is that a
/// concurrent writer's change made between the read and the rename is not
/// merged.
///
/// The file is read through [`read_file_capped_bytes`] with a 1 MiB cap
/// (`MAX_FOREIGN_TEXT_BYTES`), the same cap used for other foreign text
/// files. An over-cap file is reported as [`GitignoreOutcome::NotWritten`]
/// rather than read, and any other read failure is reported the same way:
/// this guard is non-fatal by design and never undoes the enable. A missing
/// file is not a failure — the atomic write below creates it.
///
/// Failure is reported through [`GitignoreOutcome::NotWritten`], never
/// propagated — see [`GitignoreOutcome`].
#[must_use]
pub fn ensure_gitignore(root: &ProjectRoot) -> GitignoreOutcome {
    let path = root.as_path().join(GITIGNORE_FILE);
    let existing = match read_file_capped_bytes(&path, MAX_FOREIGN_TEXT_BYTES) {
        Ok(contents) => contents,
        Err(CappedReadError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Vec::new()
        }
        Err(error) => {
            return GitignoreOutcome::NotWritten {
                path,
                reason: error.to_string(),
            };
        }
    };
    if ignores_activation_dir(&existing) {
        return GitignoreOutcome::AlreadyIgnored(path);
    }
    // Match the file's own line-ending convention rather than mixing one into
    // it, and give a newline-less final line its newline first.
    let newline = if existing.windows(2).any(|window| window == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let separator = if existing.is_empty() || existing.ends_with(b"\n") {
        ""
    } else {
        newline
    };
    let entry = format!("{ACTIVATION_RELATIVE_DIR}/");
    let addition = format!("{separator}{entry}{newline}");
    match write_gitignore_atomically(&path, &existing, addition.as_bytes()) {
        Ok(()) => GitignoreOutcome::Added(path),
        Err(error) => GitignoreOutcome::NotWritten {
            path,
            reason: error.to_string(),
        },
    }
}

/// Sequence making temp-file names unique within one process.
static GITIGNORE_TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Writes the existing bytes plus `addition` to a temp file beside the real
/// `.gitignore` and renames it into place.
///
/// The destination is the symlink target when `.gitignore` is a symlink, so
/// the user's link survives; a missing file is created at the original path.
/// Nothing is renamed until the whole file has been written, so a failure
/// before the rename leaves the original untouched. The temp file is
/// removed on failure; a cleanup failure is logged and never replaces the
/// write error that is already being reported.
fn write_gitignore_atomically(
    path: &Path,
    existing: &[u8],
    addition: &[u8],
) -> std::io::Result<()> {
    let destination = match std::fs::canonicalize(path) {
        Ok(resolved) => resolved,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(error),
    };
    let directory = destination.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the .gitignore path has no parent directory",
        )
    })?;
    let sequence = GITIGNORE_TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = directory.join(format!(
        ".gitignore.slugaudit.{}.{sequence}.tmp",
        std::process::id()
    ));

    // `create_new` fails rather than following a pre-existing path, so this
    // process can only ever clean up a temp file it created itself.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let written = file
        .write_all(existing)
        .and_then(|()| file.write_all(addition))
        .and_then(|()| match std::fs::metadata(&destination) {
            Ok(metadata) => file.set_permissions(metadata.permissions()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        });
    // Close before the rename: the close surfaces any deferred write error,
    // and a rename over an open handle is not portable.
    drop(file);
    let result = written.and_then(|()| std::fs::rename(&temp, &destination));
    if let Err(error) = result {
        remove_abandoned_temp(&temp);
        return Err(error);
    }
    Ok(())
}

/// Removes the temp file left by a failed atomic write. The primary error
/// is already on its way to the caller; a cleanup failure is logged once
/// and never replaces it.
fn remove_abandoned_temp(temp: &Path) {
    match std::fs::remove_file(temp) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!(
            target: "slugaudit::project",
            path_id = %crate::util::redacted_path(temp),
            %error,
            "failed to remove an abandoned .gitignore temp file"
        ),
    }
}

/// True when `contents` already holds a rule that ignores the activation
/// directory.
///
/// The rule recognised is exact-path, not glob: a line counts when, with
/// trailing whitespace removed (git ignores it unless escaped), one leading
/// `/` removed (at the project root it anchors to the same directory) and
/// trailing `/`s removed, it equals `.planning/slugaudit`. So
/// `.planning/slugaudit/`, `.planning/slugaudit`, `/.planning/slugaudit/` and
/// `/.planning/slugaudit` are all present. `#` comments and `!` negations are
/// not rules that ignore anything. Broader patterns (`.planning/`,
/// `**/slugaudit/`, a matching rule in a parent directory) are not
/// recognised; the duplicate rule that results is harmless, since gitignore
/// permits repeats and resolves them last-match-wins.
fn ignores_activation_dir(contents: &[u8]) -> bool {
    String::from_utf8_lossy(contents).lines().any(|line| {
        let rule = line.trim_end();
        if rule.starts_with('#') || rule.starts_with('!') {
            return false;
        }
        rule.strip_prefix('/').unwrap_or(rule).trim_end_matches('/') == ACTIVATION_RELATIVE_DIR
    })
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
