//! Open-time guards and connection configuration for the store.
//!
//! These are the private halves of the open contract in [`super::connection`]:
//! symlink and permission refusals, network-filesystem rejection is in
//! `netfs`, parent-directory resolution, open flags, the ATTACH authorizer,
//! and post-open pragma configuration. Kept apart so `connection.rs` stays
//! within the production line limit while preserving the single atomic open
//! ordering: inspect, create private, inspect permissions, open, configure.
//!
//! The guard order in `open_read_write`/`open_read_only` is load-bearing, so
//! the entry points stay in `connection.rs` and call these helpers in order.

use super::connection::StoreError;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;

pub(super) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Owner-only access for newly created database files. Existing files with
/// broader access are rejected rather than silently exposed.
#[cfg(unix)]
const PRIVATE_MODE: u32 = 0o600;

/// Opens with `SQLITE_OPEN_NOFOLLOW` so a path that is (or becomes) a
/// symlink is rejected atomically by SQLite rather than by a separate
/// `lstat` that races with the open. The pre-check remains as a clearer
/// error for the common case where the path is already a symlink.
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
                path_id = %crate::util::redacted_path(path),
                error_kind = ?error.kind(),
                "could not inspect the database path; refusing to open"
            );
            return Err(StoreError::Inspect(error));
        }
    };
    if is_symlink {
        // Logged at the rejection moment so an operator investigating
        // "why did this open fail" sees the exact path that was a
        // symlink, even when only the typed `StoreError::Symlink`
        // surfaces to the tool-call failure log. The `target` keeps
        // these warnings filterable as a single category.
        tracing::warn!(
            target: "slugaudit::store",
            path = %path.display(),
            reason = "symlink",
            "refusing to open a database path that resolves to a symlink; \
             SQLite opens follow targets, which would silently cross a \
             permission or backing-file boundary"
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
                        path_id = %crate::util::redacted_path(path),
                        error_kind = ?error.kind(),
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

/// Enforces the "this connection can only ever touch the one database it
/// was opened on" invariant at the engine level, independent of how the
/// caller shapes its SQL. `SQLITE_OPEN_READ_ONLY` blocks *writes* but not
/// *other reads*: `ATTACH DATABASE '<any file>'` opens a second database
/// inside the same connection, so a read-only connection can silently read
/// any SQLite file the user can. The `query` tool's subquery wrapping and
/// per-call connections currently hide this at the tool layer, but the
/// documented boundary is "correctness comes from the connection itself"
/// (ARCHITECTURE.md invariant #1) — this authorizer makes that claim true
/// at the layer it is made, so ATTACH stays blocked even if the tool
/// surface changes.
///
/// Installed on both open paths so no connection — read-only *or*
/// read-write — can ever reach a second database file.
pub(super) fn guard_against_attach(connection: &Connection) {
    use rusqlite::hooks::{AuthAction, Authorization};
    // rusqlite 0.37's `authorizer` stores the hook and returns `()`; the
    // callback fires during statement preparation, so a denied ATTACH
    // fails at prepare time and surfaces as an ordinary statement error.
    connection.authorizer(Some(|context: rusqlite::hooks::AuthContext<'_>| {
        match context.action {
            AuthAction::Attach { .. } | AuthAction::Detach { .. } => Authorization::Deny,
            // Everything else (SELECTs, joins, CTEs, window functions,
            // JSON functions, the server's own DML/PRAGMA) passes through
            // untouched.
            _ => Authorization::Allow,
        }
    }));
}

/// If `path` doesn't exist yet, creates it with owner-only permissions set
/// atomically at creation time (`O_CREAT | O_EXCL` plus the mode, in one
/// syscall) so there is never a window where the file exists with a wider,
/// umask-derived mode before being tightened after the fact. If another
/// process wins the race and creates the file first, `create_new` fails
/// with `AlreadyExists`; that's fine — the file already exists, so we fall
/// through and let `Connection::open_with_flags` open it as-is. We never
/// `chmod` a file this call didn't create: doing so on a path we didn't
/// just create atomically would be a TOCTOU/symlink race against whatever
/// actually created it (the `reject_symlink` pre-check plus
/// `SQLITE_OPEN_NOFOLLOW` already guard the open itself).
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
        Err(error) => Err(StoreError::Permissions(error)),
    }
}

#[cfg(not(unix))]
pub(super) fn create_with_private_permissions_if_missing(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

/// Refuse existing database files that are readable or writable by group or
/// other users. The index can contain source-derived evidence and must not be
/// opened under weaker permissions merely because it was created elsewhere.
#[cfg(unix)]
pub(super) fn reject_insecure_permissions(path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(StoreError::Permissions(error)),
    };
    let mode = metadata.permissions().mode();
    if mode & 0o077 != 0 {
        // Deliberately the raw path, not redacted_path(): a rejection that
        // cannot say which file it rejected leaves the operator stuck. The
        // hash convention is for correlation-heavy sync logs; one-shot
        // security rejections are diagnostics, and stderr stays local.
        tracing::warn!(
            target: "slugaudit::store",
            path = %path.display(),
            mode = format!("{mode:o}"),
            reason = "insecure_permissions",
            "refusing to open a database file with broader-than-owner permissions; \
             the index can contain source-derived evidence and must not be opened \
             under group- or world-readable/writable modes"
        );
        return Err(StoreError::Permissions(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("database mode {mode:o} is broader than owner-only 600"),
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn reject_insecure_permissions(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

/// Applies the pragmas every read-write connection requires: foreign keys
/// on, WAL journal mode granted, and the shared busy timeout.
pub(super) fn configure(connection: &Connection) -> Result<(), StoreError> {
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(StoreError::Configure)?;
    let granted: String = connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
        .map_err(StoreError::Configure)?;
    if !granted.eq_ignore_ascii_case("wal") {
        tracing::warn!(
            target: "slugaudit::store",
            granted = %granted,
            "SQLite did not grant WAL journal mode; refusing the connection"
        );
        return Err(StoreError::WalUnavailable { granted });
    }
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::Configure)?;
    Ok(())
}
