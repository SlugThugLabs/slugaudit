// slugaudit-line-exception: approved-by=slugthug; reason=open/configure/permissions/symlink guards are a single atomic open contract; network-filesystem detection already split into netfs.rs
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;
use thiserror::Error;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Owner-only access for newly created database files. Existing files with
/// broader access are rejected rather than silently exposed.
#[cfg(unix)]
const PRIVATE_MODE: u32 = 0o600;

#[derive(Debug, Error)]
/// Database-boundary failures. Corruption is recoverable (callers discard and
/// rebuild); guard rejections — symlink, network filesystem, group-readable
/// mode — are deliberate refusals that name the offending path.
pub enum StoreError {
    #[error("failed to open database: {0}")]
    Open(#[source] rusqlite::Error),
    #[error("failed to configure database connection: {0}")]
    Configure(#[source] rusqlite::Error),
    #[error(transparent)]
    Migration(#[from] super::migrations::MigrationError),
    #[error("refusing to open a database path that is a symlink")]
    Symlink,
    #[error("failed to inspect the database path (lstat): {0}")]
    Inspect(#[source] std::io::Error),
    #[error(
        "SQLite did not enable WAL journal mode (granted '{granted}'); \
         every read-write connection requires WAL, so the database filesystem is unsupported"
    )]
    WalUnavailable { granted: String },
    #[error("failed to set database file permissions: {0}")]
    Permissions(#[source] std::io::Error),
    #[error(
        "refusing to open a database on a network filesystem (NFS/CIFS/SMB): \
         SQLite WAL mode is unreliable on network mounts and can produce locking \
         corruption, stale reads, or SQLITE_BUSY/SQLITE_IOERR errors. \
         Move the project to a local filesystem, or deactivate and re-enable \
         SlugAudit after relocating it."
    )]
    NetworkFilesystem,
    #[error(
        "could not verify whether the database is on a network filesystem \
         (inspection failed: {0}); refusing to open rather than risk SQLite \
         corruption on a network mount — move the project to a local \
         filesystem. On Linux the check reads /proc/self/mountinfo; on macOS \
         it runs /sbin/mount. This is not a permissions problem an individual \
         user can work around"
    )]
    NetworkFilesystemCheck(#[source] std::io::Error),
}

impl StoreError {
    #[must_use]
    /// True for SQLite errors that mean the database file is damaged beyond use —
    /// the signal to discard and rebuild rather than retry.
    pub fn is_corruption(&self) -> bool {
        match self {
            Self::Open(error) | Self::Configure(error) => super::is_rusqlite_corruption(error),
            Self::Migration(error) => error.is_corruption(),
            Self::Symlink
            | Self::Inspect(_)
            | Self::WalUnavailable { .. }
            | Self::Permissions(_)
            | Self::NetworkFilesystem
            | Self::NetworkFilesystemCheck(_) => false,
        }
    }
}

/// Discards a corrupt derived database and its SQLite journal sidecars.
/// Callers must recreate the schema and republish from project files.
pub fn discard_corrupt_database(path: &Path) -> Result<(), std::io::Error> {
    for suffix in ["", "-wal", "-shm"] {
        let candidate = if suffix.is_empty() {
            path.to_path_buf()
        } else {
            Path::new(&format!("{}{}", path.display(), suffix)).to_path_buf()
        };
        match std::fs::remove_file(candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Opens with `SQLITE_OPEN_NOFOLLOW` so a path that is (or becomes) a
/// symlink is rejected atomically by SQLite rather than by a separate
/// `lstat` that races with the open. The pre-check remains as a clearer
/// error for the common case where the path is already a symlink.
use super::netfs::reject_network_filesystem;

fn reject_symlink(path: &Path) -> Result<(), StoreError> {
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
fn resolve_parent_dir(path: &Path) -> std::path::PathBuf {
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

fn open_flags(read_write: bool) -> OpenFlags {
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
fn guard_against_attach(connection: &Connection) {
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
fn create_with_private_permissions_if_missing(path: &Path) -> Result<(), StoreError> {
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
fn create_with_private_permissions_if_missing(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

/// Refuse existing database files that are readable or writable by group or
/// other users. The index can contain source-derived evidence and must not be
/// opened under weaker permissions merely because it was created elsewhere.
#[cfg(unix)]
fn reject_insecure_permissions(path: &Path) -> Result<(), StoreError> {
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
fn reject_insecure_permissions(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

/// Opens a read-write connection, creating the database file if needed, and
/// brings its schema up to date. This is the only connection sync/store
/// repositories write through.
///
/// # Errors
///
/// Returns an error if the file can't be opened/created, if pragmas can't
/// be applied, if the schema can't be migrated to the current version
/// (including when the database is from a newer, unsupported version), or
/// if the database path resides on a network filesystem (NFS/CIFS/SMB)
/// where SQLite WAL mode is unreliable.
pub fn open_read_write(path: &Path) -> Result<Connection, StoreError> {
    let resolved = resolve_parent_dir(path);
    let path = resolved.as_path();
    reject_symlink(path)?;
    reject_network_filesystem(path)?;
    create_with_private_permissions_if_missing(path)?;
    reject_insecure_permissions(path)?;
    let mut connection =
        Connection::open_with_flags(path, open_flags(true)).map_err(StoreError::Open)?;
    configure(&connection)?;
    guard_against_attach(&connection);
    super::sql_functions::register(&connection).map_err(StoreError::Configure)?;
    super::migrations::ensure_current_schema(&mut connection)?;
    Ok(connection)
}

/// Opens a connection that cannot write no matter what SQL it executes.
/// This is the safety boundary for the `query` tool: correctness comes from
/// the connection itself, never from inspecting query text. Requires an
/// already-migrated database; it never creates or alters the schema.
///
/// # Errors
///
/// Returns an error if the file doesn't exist or can't be opened read-only,
/// if the busy timeout can't be configured, or if the database path resides
/// on a network filesystem (NFS/CIFS/SMB) where SQLite WAL mode is
/// unreliable.
pub fn open_read_only(path: &Path) -> Result<Connection, StoreError> {
    let resolved = resolve_parent_dir(path);
    let path = resolved.as_path();
    reject_symlink(path)?;
    reject_network_filesystem(path)?;
    reject_insecure_permissions(path)?;
    let connection =
        Connection::open_with_flags(path, open_flags(false)).map_err(StoreError::Open)?;
    // `sqlite3_busy_timeout` and `sqlite3_create_function_v2` fail only on
    // SQLITE_NOMEM or SQLITE_MISUSE from a NULL/invalid handle. The safe
    // rusqlite API above cannot hand either call an invalid handle, and
    // allocation failure is not a hermetic input, so these arms are
    // defensive: they keep an allocator or engine failure typed as
    // `Configure` rather than letting it become a panic.
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::Configure)?;
    guard_against_attach(&connection);
    super::sql_functions::register(&connection).map_err(StoreError::Configure)?;
    Ok(connection)
}

fn configure(connection: &Connection) -> Result<(), StoreError> {
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

/// Failure-path tests for the open/discard contract. They sit inline because
/// `connection_tests.rs` is at the 500-code-line test-file cap.
#[cfg(test)]
mod guard_tests {
    use super::*;
    use crate::store::test_capture::capture_warns;
    use std::io::ErrorKind;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn an_uninspectable_database_path_warning_redacts_the_path() {
        let path = Path::new("/tmp").join("a".repeat(300)).join("project.db");
        let (result, logs) = capture_warns(|| {
            tracing::warn!("lstat_control");
            reject_symlink(&path)
        });
        assert!(matches!(result, Err(StoreError::Inspect(_))) && logs.contains("lstat_control"));
        assert!(logs.contains("path_id="), "{logs}");
        assert!(!logs.contains(path.to_string_lossy().as_ref()), "{logs}");
    }

    #[test]
    fn a_canonicalize_fallback_warning_redacts_the_path() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("missing-parent").join("project.db");
        let (result, logs) = capture_warns(|| {
            tracing::warn!("canonicalize_control");
            open_read_only(&path)
        });
        assert!(matches!(&result, Err(StoreError::Open(_))), "{result:?}");
        assert!(logs.contains("canonicalize_control"), "{logs}");
        assert!(logs.contains("path_id="), "{logs}");
        assert!(!logs.contains(path.to_string_lossy().as_ref()), "{logs}");
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_at_the_database_path_fails_with_an_open_error() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        std::fs::create_dir(&path).expect("directory fixture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).expect("mode");
        let result = open_read_write(&path);
        assert!(matches!(&result, Err(StoreError::Open(_))), "{result:?}");
    }

    #[test]
    fn a_directory_at_the_database_path_fails_the_corrupt_database_discard() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        std::fs::create_dir(&path).expect("directory fixture");
        let error = discard_corrupt_database(&path).expect_err("directory unlink must fail");
        let kind = error.kind();
        assert!(kind == ErrorKind::IsADirectory || kind == ErrorKind::PermissionDenied);
        assert!(path.is_dir());
    }

    #[test]
    fn a_read_only_connection_carries_the_busy_timeout() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        open_read_write(&path).expect("create database");
        let ro = open_read_only(&path).expect("open read-only");
        let timeout: i64 = ro
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("read");
        assert_eq!(timeout, 5_000);
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
