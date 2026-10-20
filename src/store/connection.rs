//! Database connection open and refusal contract: the read-write and
//! read-only entry points, plus `StoreError` and corrupt-database discard.
//! The inspection, permission, and pragma guards they call live in
//! [`super::connection_guards`].

use super::connection_guards::{
    BUSY_TIMEOUT, configure, create_with_private_permissions_if_missing, guard_against_attach,
    open_flags, reject_insecure_permissions, reject_symlink, resolve_parent_dir,
};
use super::netfs::reject_network_filesystem;
use rusqlite::Connection;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
/// Database-boundary failures. Corruption is recoverable (callers discard and
/// rebuild); guard rejections — symlink, network filesystem, group-readable
/// mode — are deliberate refusals that name the offending path.
pub enum StoreError {
    /// Opening or creating the database file failed.
    #[error("failed to open database: {0}")]
    Open(#[source] rusqlite::Error),
    /// Applying connection pragmas or registering the SQL helper functions
    /// failed.
    #[error("failed to configure database connection: {0}")]
    Configure(#[source] rusqlite::Error),
    /// Reading or initializing the schema failed; see
    /// [`MigrationError`](super::migrations::MigrationError).
    #[error(transparent)]
    Migration(#[from] super::migrations::MigrationError),
    /// The database path is a symlink; SlugAudit refuses to follow it.
    #[error("refusing to open a database path that is a symlink")]
    Symlink,
    /// The database path could not be inspected with `lstat`.
    #[error("failed to inspect the database path (lstat): {0}")]
    Inspect(#[source] std::io::Error),
    /// SQLite did not enable WAL journal mode, which every read-write
    /// connection requires.
    #[error(
        "SQLite did not enable WAL journal mode (granted '{granted}'); \
         every read-write connection requires WAL, so the database filesystem is unsupported"
    )]
    WalUnavailable {
        /// The journal mode SQLite enabled instead of WAL.
        granted: String,
    },
    /// Applying private file permissions to the database failed.
    #[error("failed to set database file permissions: {0}")]
    Permissions(#[source] std::io::Error),
    /// The database path is on a network filesystem (NFS/CIFS/SMB), where
    /// SQLite WAL mode is unreliable.
    #[error(
        "refusing to open a database on a network filesystem (NFS/CIFS/SMB): \
         SQLite WAL mode is unreliable on network mounts and can produce locking \
         corruption, stale reads, or SQLITE_BUSY/SQLITE_IOERR errors. \
         Move the project to a local filesystem, or deactivate and re-enable \
         SlugAudit after relocating it."
    )]
    NetworkFilesystem,
    /// Whether the database is on a network filesystem could not be
    /// determined, so opening fails closed rather than risk SQLite
    /// corruption.
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
