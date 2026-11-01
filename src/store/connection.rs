//! Database connection open contract: one writer path and one read-mode path.
//!
//! The database is runtime scratch: callers delete and rebuild it on any
//! doubt, so there is no trust boundary around the file, no authored SQL
//! authorizer, and no permission policing. Safety stays: a symlink at the
//! database path is refused (opening through it would write to some other
//! file), a new file is created owner-only, and the writer requires WAL.

use super::connection_guards::{
    BUSY_TIMEOUT, configure, create_with_private_permissions_if_missing, open_flags,
    reject_symlink, resolve_parent_dir,
};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
/// Database-boundary failures. Corruption is recoverable (callers discard and
/// rebuild); a symlink at the database path is a deliberate refusal.
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
    /// Creating the database file failed.
    #[error("failed to create the database file: {0}")]
    Create(#[source] std::io::Error),
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
            | Self::Create(_)
            | Self::WalUnavailable { .. } => false,
        }
    }
}

/// Discards a corrupt derived database and its SQLite journal sidecars.
/// Callers must recreate the schema and republish from project files.
pub fn discard_corrupt_database(path: &Path) -> Result<(), std::io::Error> {
    remove_database_files(path)
}

/// Deletes a database file and its `-wal` / `-shm` sidecars; files that are
/// already absent are not an error.
pub fn remove_database_files(path: &Path) -> Result<(), std::io::Error> {
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
/// be applied, or if the schema can't be migrated to the current version
/// (including when the database is from a newer, unsupported version).
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
    let mut flags = open_flags(true);
    match mode {
        CreateMode::MayCreate => create_with_private_permissions_if_missing(path)?,
        CreateMode::MustExist => flags.remove(OpenFlags::SQLITE_OPEN_CREATE),
    }
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
    super::sql_functions::register(&connection).map_err(StoreError::Configure)?;
    super::migrations::ensure_current_schema(&mut connection)?;
    Ok(connection)
}

/// `Missing` when nothing exists at `path` (a dangling symlink counts as
/// present so the symlink refusal still applies); other `lstat` failures are
/// reported as themselves.
pub(super) fn ensure_present(path: &Path) -> Result<(), StoreError> {
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

/// Opens an existing database for reading. The connection uses SQLite's
/// read-only flag so a read path cannot mutate the index or run schema
/// migrations; the flag is a connection mode, not a trust boundary — the
/// index is disposable and the caller owns the machine.
///
/// # Errors
///
/// Returns an error if the file doesn't exist or can't be opened read-only,
/// or if the busy timeout can't be configured.
pub fn open_existing(path: &Path) -> Result<Connection, StoreError> {
    let resolved = resolve_parent_dir(path);
    let path = resolved.as_path();
    reject_symlink(path)?;
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
    super::sql_functions::register(&connection).map_err(StoreError::Configure)?;
    Ok(connection)
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
