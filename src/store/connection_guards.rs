// slugaudit-line-exception: approved-by=slugthug; reason=the open-time guards (symlink, permission, ATTACH and read-only authorizers, WAL) are the private half of one open contract and share its error type
//! Open-time guards and connection configuration for the store.
//!
//! These are the private halves of the open contract in [`super::connection`]:
//! symlink and permission refusals, network-filesystem rejection is in
//! `netfs`, parent-directory resolution, open flags, the ATTACH and PRAGMA
//! authorizers, and post-open pragma configuration. Kept apart so `connection.rs` stays
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
const WAL_RETRY_INTERVAL: Duration = Duration::from_millis(20);
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
                path = %path.display(),
                error = %error,
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
/// Installed on the read-write path; the read-only path installs
/// [`guard_read_only`], which denies the same ATTACH/DETACH and adds PRAGMA,
/// so no connection — read-only *or* read-write — can ever reach a second
/// database file.
pub(super) fn guard_against_attach(connection: &Connection) {
    // rusqlite 0.37's `authorizer` stores the hook and returns `()`; the
    // callback fires during statement preparation, so a denied ATTACH
    // fails at prepare time and surfaces as an ordinary statement error.
    // Everything else (SELECTs, joins, CTEs, window functions, JSON
    // functions, the server's own DML/PRAGMA) passes through untouched.
    connection.authorizer(Some(|context: rusqlite::hooks::AuthContext<'_>| {
        authorize(&context.action, false)
    }));
}

/// The read-only connection's authorizer: ATTACH/DETACH denied as in
/// [`guard_against_attach`], every CREATE TEMP TABLE/INDEX/TRIGGER/VIEW denied,
/// and every PRAGMA denied except the
/// schema-introspection names in [`INTROSPECTION_PRAGMAS`].
///
/// A PRAGMA is not inert until it runs: SQLite applies `hard_heap_limit`,
/// `soft_heap_limit`, `temp_store_directory` and others while *preparing*
/// the statement, so a statement the `query` tool goes on to refuse has
/// already changed process-wide state. The authorizer callback is invoked
/// from `sqlite3Pragma` before any of that code runs (verified against the
/// bundled SQLite 2026-10-25: a denied `PRAGMA hard_heap_limit=1` leaves the
/// limit at 0), so the denial is what makes the refusal side-effect free.
pub(super) fn guard_read_only(connection: &Connection) {
    connection.authorizer(Some(|context: rusqlite::hooks::AuthContext<'_>| {
        authorize(&context.action, true)
    }));
}

/// Pragmas the read-only connection authorizes, so the `pragma_<name>`
/// table-valued functions keep working: SQLite's pragma virtual table runs
/// them by preparing `PRAGMA <name>='<arg>'` on the same connection, and the
/// authorizer sees that internal statement exactly like one the caller
/// typed. Each name is a pure read of the schema catalogue whose only
/// argument is a table or index name to look up: `table_info`/`table_xinfo`
/// (columns), `table_list` (tables), `index_list`/`index_info`/`index_xinfo`
/// (indexes) and `foreign_key_list`. None of them sets a value, writes,
/// changes a limit or memory setting, or touches the filesystem.
///
/// Deliberately excluded: every pragma that is a setter when given a value
/// (`user_version`, `journal_mode`, the heap limits, ...), `optimize` (runs
/// ANALYZE), the whole-database checks (`integrity_check`, `quick_check`,
/// `foreign_key_check`), and catalogue reads outside the schema
/// (`database_list` names the database file path).
pub(super) const INTROSPECTION_PRAGMAS: [&str; 7] = [
    "table_info",
    "table_xinfo",
    "table_list",
    "index_list",
    "index_info",
    "index_xinfo",
    "foreign_key_list",
];

/// The shared decision. The pragma value is not inspected: SQLite hands the
/// authorizer the same `(name, value)` for `PRAGMA table_info(files)`,
/// `PRAGMA table_info = files` and `pragma_table_info('files')`, so the
/// allowlist is by name only, which is safe because for these names the
/// value is only ever a name to look up. The name arrives as typed, so it is
/// compared case-insensitively, as SQLite resolves it.
fn authorize(
    action: &rusqlite::hooks::AuthAction<'_>,
    read_only: bool,
) -> rusqlite::hooks::Authorization {
    use rusqlite::hooks::{AuthAction, Authorization};
    match action {
        AuthAction::Attach { .. } | AuthAction::Detach { .. } => Authorization::Deny,
        // A read-only connection can still create TEMP objects (they live in
        // the connection's own temp database), so the whole family is denied.
        AuthAction::CreateTempTable { .. }
        | AuthAction::CreateTempIndex { .. }
        | AuthAction::CreateTempTrigger { .. }
        | AuthAction::CreateTempView { .. }
            if read_only =>
        {
            Authorization::Deny
        }
        AuthAction::Pragma { pragma_name, .. }
            if read_only
                && !INTROSPECTION_PRAGMAS
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(pragma_name)) =>
        {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }
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
        // The real path: a rejection that cannot say which file it rejected
        // leaves the operator stuck, and stderr stays local.
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
