use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;
use thiserror::Error;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("failed to open database: {0}")]
    Open(#[source] rusqlite::Error),
    #[error("failed to configure database connection: {0}")]
    Configure(#[source] rusqlite::Error),
    #[error(transparent)]
    Migration(#[from] super::migrations::MigrationError),
}

/// Opens a read-write connection, creating the database file if needed, and
/// brings its schema up to date. This is the only connection sync/store
/// repositories write through.
///
/// # Errors
///
/// Returns an error if the file can't be opened/created, if pragmas can't
/// be applied, or if the schema can't be migrated to the current version
/// (including when the database is from a newer, unsupported version).
pub fn open_read_write(path: &Path) -> Result<Connection, StoreError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(StoreError::Open)?;
    configure(&connection)?;
    super::migrations::ensure_current_schema(&connection)?;
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
/// or if the busy timeout can't be configured.
pub fn open_read_only(path: &Path) -> Result<Connection, StoreError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(StoreError::Open)?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::Configure)?;
    Ok(connection)
}

fn configure(connection: &Connection) -> Result<(), StoreError> {
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(StoreError::Configure)?;
    connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |_row| Ok(()))
        .map_err(StoreError::Configure)?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::Configure)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_keys_are_enabled_on_a_read_write_connection() {
        let directory = tempfile::tempdir().expect("temp dir");
        let connection = open_read_write(&directory.path().join("project.db")).expect("open");
        let enabled: i64 = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("read pragma");
        assert_eq!(enabled, 1);
    }

    #[test]
    fn a_read_only_connection_cannot_write() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        open_read_write(&path).expect("create database");

        let read_only = open_read_only(&path).expect("open read-only");
        let result = read_only.execute("DELETE FROM findings", []);
        assert!(result.is_err());
    }

    #[test]
    fn read_only_open_fails_against_a_missing_database() {
        let directory = tempfile::tempdir().expect("temp dir");
        let result = open_read_only(&directory.path().join("missing.db"));
        assert!(matches!(result, Err(StoreError::Open(_))));
    }
}
