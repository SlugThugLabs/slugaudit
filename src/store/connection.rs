// slugaudit-line-exception: approved-by=split-judge; reason=single SQLite open contract for project.db and session findings-<id>.db: open flags, WAL retry, pragmas, SQL function registration, schema check, and sidecar cleanup
//! Database connection open contract for `project.db` and `findings-<id>.db`.
//!
//! Both files are disposable local scratch under `.planning/slugaudit/`. Read-write
//! connections enable WAL, foreign keys, and a bounded busy timeout; read-only
//! connections open with `SQLITE_OPEN_READ_ONLY` so `query` cannot mutate rows.

use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;
use thiserror::Error;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const WAL_RETRY_INTERVAL: Duration = Duration::from_millis(20);
const FINDINGS_DDL: &str = include_str!("findings_schema.sql");

#[derive(Debug, Error)]
/// Database-boundary failures. Corruption is recoverable (callers discard and
/// rebuild from source).
pub enum StoreError {
    /// Opening or creating the database file failed.
    #[error("failed to open database: {0}")]
    Open(#[source] rusqlite::Error),
    /// Applying connection pragmas or registering SQL helper functions failed.
    #[error("failed to configure database connection: {0}")]
    Configure(#[source] rusqlite::Error),
    /// Another process kept the database locked for the bounded busy wait.
    #[error("the project database stayed locked by another process: {0}")]
    Busy(#[source] rusqlite::Error),
    /// Reading or initializing the schema failed.
    #[error(transparent)]
    Migration(#[from] super::migrations::MigrationError),
    /// The database file does not exist and `MustExist` was requested.
    #[error("the project database does not exist")]
    Missing,
    /// The database path could not be inspected.
    #[error("failed to inspect the database path: {0}")]
    Inspect(#[source] std::io::Error),
    /// SQLite did not enable WAL journal mode, which read-write connections require.
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
    /// True when the SQLite failure indicates a damaged or incompatible file
    /// that sync should discard and rebuild from disk.
    pub fn is_corruption(&self) -> bool {
        match self {
            Self::Open(error) | Self::Configure(error) => super::is_rusqlite_corruption(error),
            Self::Migration(error) => error.is_corruption(),
            Self::Missing | Self::Busy(_) | Self::Inspect(_) | Self::WalUnavailable { .. } => false,
        }
    }
}

fn open_flags(read_write: bool) -> OpenFlags {
    let mut flags = OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if read_write {
        flags |= OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
    } else {
        flags |= OpenFlags::SQLITE_OPEN_READ_ONLY;
    }
    flags
}

fn configure(connection: &Connection) -> Result<(), StoreError> {
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::Configure)?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(StoreError::Configure)?;
    enable_wal(connection, BUSY_TIMEOUT)?;
    Ok(())
}

pub(super) fn enable_wal(connection: &Connection, budget: Duration) -> Result<(), StoreError> {
    let started = std::time::Instant::now();
    let granted: String = loop {
        match connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0)) {
            Ok(granted) => break granted,
            Err(error) if is_contention(&error) => {
                if started.elapsed() >= budget {
                    tracing::warn!(
                        target: "slugaudit::store",
                        budget_ms = budget.as_millis(),
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

fn is_contention(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

/// Discards a corrupt derived database and its SQLite journal sidecars.
///
/// # Errors
///
/// Propagates any non-`NotFound` I/O error from removing the database or sidecar files.
pub fn discard_corrupt_database(path: &Path) -> Result<(), std::io::Error> {
    remove_database_files(path)
}

/// Deletes a database file and its `-wal` / `-shm` sidecars; already-absent
/// files are ignored.
///
/// # Errors
///
/// Propagates any non-`NotFound` I/O error from removing the files.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CreateMode {
    MayCreate,
    MustExist,
}

/// Opens a read-write connection to `project.db`, creating it when absent and
/// initializing the current schema.
///
/// # Errors
///
/// Returns [`StoreError`] if opening, configuring PRAGMAs/functions, or migrating fails.
pub fn open_read_write(path: &Path) -> Result<Connection, StoreError> {
    open_read_write_with(path, CreateMode::MayCreate)
}

/// Opens an existing `project.db` read-write without creating a missing file.
///
/// # Errors
///
/// Returns [`StoreError::Missing`] when absent, or [`StoreError`] on open/schema failure.
pub fn open_existing_read_write(path: &Path) -> Result<Connection, StoreError> {
    open_read_write_with(path, CreateMode::MustExist)
}

fn open_read_write_with(path: &Path, mode: CreateMode) -> Result<Connection, StoreError> {
    if mode == CreateMode::MustExist {
        ensure_present(path)?;
    }
    let mut flags = open_flags(true);
    if mode == CreateMode::MustExist {
        flags.remove(OpenFlags::SQLITE_OPEN_CREATE);
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

fn ensure_present(path: &Path) -> Result<(), StoreError> {
    match path.metadata() {
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

/// Opens an existing database in read-only mode with SQL helpers registered.
///
/// # Errors
///
/// Returns [`StoreError`] if opening the file or configuring the connection fails.
pub fn open_existing(path: &Path) -> Result<Connection, StoreError> {
    let connection =
        Connection::open_with_flags(path, open_flags(false)).map_err(StoreError::Open)?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::Configure)?;
    super::sql_functions::register(&connection).map_err(StoreError::Configure)?;
    Ok(connection)
}

/// Opens the session findings database (`findings-<id>.db`) read-write,
/// creating the file and schema if needed.
///
/// # Errors
///
/// Returns [`StoreError`] if opening, configuring, or creating the findings schema fails.
pub fn open_findings(path: &Path) -> Result<Connection, StoreError> {
    let connection =
        Connection::open_with_flags(path, open_flags(true)).map_err(StoreError::Open)?;
    configure(&connection)?;
    connection
        .execute_batch(FINDINGS_DDL)
        .map_err(StoreError::Configure)?;
    Ok(connection)
}

/// Opens the session findings database read-only, returning `Ok(None)` when no
/// finding has been recorded in this session yet.
///
/// # Errors
///
/// Returns [`StoreError`] on I/O inspect errors or read-only open/setup failures.
pub fn open_findings_existing(path: &Path) -> Result<Option<Connection>, StoreError> {
    match ensure_present(path) {
        Ok(()) => open_existing(path).map(Some),
        Err(StoreError::Missing) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
