//! Database connection open and refusal contract: the read-write and
//! read-only entry points, plus `StoreError` and corrupt-database discard.
//! The inspection, permission, and pragma guards they call live in
//! [`super::connection_guards`].

use super::connection_guards::{
    BUSY_TIMEOUT, configure, create_with_private_permissions_if_missing, guard_against_attach,
    guard_read_only, open_flags, reject_insecure_permissions, reject_symlink, resolve_parent_dir,
};
use super::netfs::reject_network_filesystem;
use rusqlite::{Connection, OpenFlags};
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
    /// Another process kept the database locked for the whole bounded wait
    /// (the busy timeout plus the WAL-conversion retry budget) while this
    /// connection was being set up. Retrying the call is safe.
    #[error("the project database stayed locked by another process: {0}")]
    Busy(#[source] rusqlite::Error),
    /// Reading or initializing the schema failed; see
    /// [`MigrationError`](super::migrations::MigrationError).
    #[error(transparent)]
    Migration(#[from] super::migrations::MigrationError),
    /// The database path is a symlink; SlugAudit refuses to follow it.
    #[error("refusing to open a database path that is a symlink")]
    Symlink,
    /// The database file does not exist and the caller asked only to open an
    /// existing one (see [`open_existing_read_write`]).
    #[error("the project database does not exist")]
    Missing,
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
            | Self::Missing
            | Self::Busy(_)
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
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            std::path::PathBuf::from(name)
        };
        match std::fs::remove_file(candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Whether opening a read-write connection may create the database file.
/// Only `project_control on` and the corrupt-database rebuild (which recreates
/// the file inside an already-active project) may create it; every other path
/// must not resurrect a project that `project_control off` just deactivated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CreateMode {
    MayCreate,
    MustExist,
}

/// Opens a read-write connection, creating the database file if needed, and
/// brings its schema up to date. This is the only connection sync/store
/// repositories write through. Use it only where creating the file is the
/// intent (`project_control on`, rebuilding a discarded database); everything
/// else uses [`open_existing_read_write`].
///
/// # Errors
///
/// Returns an error if the file can't be opened/created, if pragmas can't
/// be applied, if the schema can't be migrated to the current version
/// (including when the database is from a newer, unsupported version), or
/// if the database path resides on a network filesystem (NFS/CIFS/SMB)
/// where SQLite WAL mode is unreliable.
pub fn open_read_write(path: &Path) -> Result<Connection, StoreError> {
    open_read_write_with(path, CreateMode::MayCreate)
}

/// [`open_read_write`] for a database that must already exist: it never
/// creates the file, so a project deactivated between a caller's activation
/// check and this open stays deactivated.
///
/// # Errors
///
/// Returns [`StoreError::Missing`] if the file does not exist (including when
/// it vanishes during the open), otherwise the errors of [`open_read_write`].
pub fn open_existing_read_write(path: &Path) -> Result<Connection, StoreError> {
    open_read_write_with(path, CreateMode::MustExist)
}

fn open_read_write_with(path: &Path, mode: CreateMode) -> Result<Connection, StoreError> {
    let resolved = resolve_parent_dir(path);
    let path = resolved.as_path();
    if mode == CreateMode::MustExist {
        ensure_present(path)?;
    }
    reject_symlink(path)?;
    reject_network_filesystem(path)?;
    let mut flags = open_flags(true);
    match mode {
        CreateMode::MayCreate => create_with_private_permissions_if_missing(path)?,
        CreateMode::MustExist => flags.remove(OpenFlags::SQLITE_OPEN_CREATE),
    }
    reject_insecure_permissions(path)?;
    let mut connection = match Connection::open_with_flags(path, flags) {
        Ok(connection) => connection,
        Err(error) => {
            if mode == CreateMode::MustExist {
                ensure_present(path)?;
            }
            return Err(StoreError::Open(error));
        }
    };
    configure(&connection)?;
    guard_against_attach(&connection);
    super::sql_functions::register(&connection).map_err(StoreError::Configure)?;
    super::migrations::ensure_current_schema(&mut connection)?;
    Ok(connection)
}

/// `Missing` when nothing exists at `path` (a dangling symlink counts as
/// present so the symlink refusal still applies); other `lstat` failures are
/// reported as themselves.
fn ensure_present(path: &Path) -> Result<(), StoreError> {
    match path.symlink_metadata() {
        Ok(_) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            Err(StoreError::Missing)
        }
        Err(error) => Err(StoreError::Inspect(error)),
    }
}

/// Opens a connection that cannot write no matter what SQL it executes.
/// This is the safety boundary for the `query` tool: correctness comes from
/// the connection itself, never from inspecting query text. Requires an
/// already-migrated database; it never creates or alters the schema. Its
/// authorizer denies ATTACH/DETACH and every PRAGMA except schema
/// introspection, at prepare time, before a pragma can take effect.
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
    guard_read_only(&connection);
    super::sql_functions::register(&connection).map_err(StoreError::Configure)?;
    Ok(connection)
}

#[cfg(test)]
#[path = "connection_guard_tests.rs"]
mod guard_tests;

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
