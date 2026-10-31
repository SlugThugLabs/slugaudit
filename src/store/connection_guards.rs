//! Open-time configuration for the store: symlink refusal, private file
//! creation, pragma configuration and WAL enablement.
//!
//! The database is runtime scratch — deleted and rebuilt on any doubt — so
//! there is no authorizer, permission model, or filesystem-type refusal
//! around it. What remains is safety: never follow a symlink at the database
//! path (that would write through a link to some other file), never create
//! the file world-readable, and require the WAL journal the writer depends
//! on.

use super::connection::StoreError;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;

pub(super) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const WAL_RETRY_INTERVAL: Duration = Duration::from_millis(20);
/// Owner-only access for newly created database files. Existing files are
/// used as they are: the database is disposable, so its mode is not policed.
#[cfg(unix)]
const PRIVATE_MODE: u32 = 0o600;

/// Opens with `SQLITE_OPEN_NOFOLLOW` so a path that is (or becomes) a
/// symlink is rejected atomically by SQLite rather than by a separate
/// `lstat` that races with the open. The pre-check remains as a clearer
/// error for the common case. A symlink at `project.db` is not the index —
/// writing through it would overwrite whatever the user pointed it at.
pub(super) fn reject_symlink(path: &Path) -> Result<(), StoreError> {
    // An absent path is simply not a symlink (the first open creates it);
    // any other lstat failure fails the open instead of skipping the guard.
    let is_symlink = match path.symlink_metadata() {
        Ok(metadata) => metadata.file_type().is_symlink(),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            false
        }
        Err(error) => {
            tracing::warn!(
                target: "slugaudit::store",
                path = %path.display(),
                error = %error,
                "could not inspect the database path; refusing to open"
            );
            return Err(StoreError::Inspect(error));
        }
    };
    if is_symlink {
        tracing::warn!(
            target: "slugaudit::store",
            path = %path.display(),
            reason = "symlink",
            "refusing to open a database path that resolves to a symlink; \
             opening it would write through the link to another file"
        );
        return Err(StoreError::Symlink);
    }
    Ok(())
}

/// Resolves symlinks in the *parent directory* only, so `SQLITE_OPEN_NOFOLLOW`
/// (which rejects a symlink in any path component, `SQLITE_CANTOPEN_SYMLINK`)
/// keeps guarding the database file itself without refusing a project under a
/// symlinked ancestor — on macOS `/var` and `/tmp` are links into `/private`
/// (verified on macos-latest CI, 2026-10-16). Falls back to the path as given
/// when the parent cannot be resolved (logged with the cause); the open then
/// fails on its own terms.
pub(super) fn resolve_parent_dir(path: &Path) -> std::path::PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
            match parent.canonicalize() {
                Ok(real) => real.join(name),
                Err(error) => {
                    tracing::warn!(
                        target: "slugaudit::store",
                        path = %path.display(),
                        error = %error,
                        "database parent directory could not be canonicalized; using the path as given"
                    );
                    path.to_path_buf()
                }
            }
        }
        _ => path.to_path_buf(),
    }
}

pub(super) fn open_flags(read_write: bool) -> OpenFlags {
    let mut flags = OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    if read_write {
        flags |= OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
    } else {
        flags |= OpenFlags::SQLITE_OPEN_READ_ONLY;
    }
    flags
}

/// If `path` doesn't exist yet, creates it with owner-only permissions set
/// atomically at creation time (`O_CREAT | O_EXCL` plus the mode, in one
/// syscall) so there is never a window where the file exists with a wider,
/// umask-derived mode before being tightened after the fact. If another
/// process wins the race and creates the file first, `create_new` fails
/// with `AlreadyExists`; that's fine — the file already exists, so we fall
/// through and let `Connection::open_with_flags` open it as-is. We never
/// `chmod` a file this call didn't create.
#[cfg(unix)]
pub(super) fn create_with_private_permissions_if_missing(path: &Path) -> Result<(), StoreError> {
    use std::io::ErrorKind;
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PRIVATE_MODE)
        .open(path)
    {
        Ok(_file) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(StoreError::Create(error)),
    }
}

#[cfg(not(unix))]
pub(super) fn create_with_private_permissions_if_missing(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

/// Applies the pragmas every read-write connection requires: the shared busy
/// timeout first (so every later statement that needs a lock waits for it),
/// then foreign keys on and WAL journal mode granted.
pub(super) fn configure(connection: &Connection) -> Result<(), StoreError> {
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::Configure)?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(StoreError::Configure)?;
    enable_wal(connection, BUSY_TIMEOUT)?;
    Ok(())
}

/// Switches the database to WAL, retrying for up to `budget` while another
/// process holds it. Converting a fresh database to WAL needs an exclusive
/// lock and SQLite reports `SQLITE_BUSY` for that statement immediately, even
/// with a busy handler installed, so two processes creating the same project
/// database at once would otherwise fail one of them. Once WAL is set it
/// persists in the file, so the retry only matters for the first open.
pub(super) fn enable_wal(connection: &Connection, budget: Duration) -> Result<(), StoreError> {
    let deadline = std::time::Instant::now() + budget;
    let granted: String = loop {
        match connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0)) {
            Ok(granted) => break granted,
            Err(error) if is_contention(&error) => {
                if std::time::Instant::now() >= deadline {
                    tracing::warn!(
                        target: "slugaudit::store",
                        budget_ms = u64::try_from(budget.as_millis()).unwrap_or(u64::MAX),
                        "database stayed locked by another process while enabling WAL"
                    );
                    return Err(StoreError::Busy(error));
                }
                std::thread::sleep(WAL_RETRY_INTERVAL);
            }
            Err(error) => return Err(StoreError::Configure(error)),
        }
    };
    if !granted.eq_ignore_ascii_case("wal") {
        tracing::warn!(
            target: "slugaudit::store",
            granted = %granted,
            "SQLite did not grant WAL journal mode; refusing the connection"
        );
        return Err(StoreError::WalUnavailable { granted });
    }
    Ok(())
}

/// True for SQLite's busy/locked codes: another connection is in the way.
pub(super) fn is_contention(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}
