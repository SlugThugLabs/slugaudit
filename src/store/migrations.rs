use rusqlite::Connection;
use thiserror::Error;

const CURRENT_SCHEMA_VERSION: i64 = 1;
const SCHEMA_DDL: &str = include_str!("schema.sql");

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("failed to read schema version: {0}")]
    ReadVersion(#[source] rusqlite::Error),
    #[error("failed to apply schema: {0}")]
    Apply(#[source] rusqlite::Error),
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    UnsupportedVersion { found: i64, supported: i64 },
}

impl MigrationError {
    #[must_use]
    pub fn is_corruption(&self) -> bool {
        match self {
            Self::ReadVersion(error) | Self::Apply(error) => is_corruption(error),
            Self::UnsupportedVersion { .. } => false,
        }
    }
}

fn is_corruption(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            )
    )
}

/// Brings a freshly-opened database up to `CURRENT_SCHEMA_VERSION`.
/// Migrations are forward-only: a database at a newer schema version than
/// this build knows about is rejected rather than guessed at. Applying the
/// schema and recording the new version happen in one transaction — SQLite
/// supports transactional DDL, so a crash mid-migration can't leave tables
/// created but `user_version` still reporting the old version.
pub fn ensure_current_schema(connection: &mut Connection) -> Result<(), MigrationError> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(MigrationError::ReadVersion)?;

    if version == CURRENT_SCHEMA_VERSION {
        return Ok(());
    }
    if version > CURRENT_SCHEMA_VERSION {
        return Err(MigrationError::UnsupportedVersion {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }

    let tx = connection.transaction().map_err(MigrationError::Apply)?;
    tx.execute_batch(SCHEMA_DDL)
        .map_err(MigrationError::Apply)?;
    tx.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)
        .map_err(MigrationError::Apply)?;
    tx.commit().map_err(MigrationError::Apply)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::connection::{StoreError, open_read_write};

    #[test]
    fn applies_schema_to_a_fresh_database() {
        let directory = tempfile::tempdir().expect("temp dir");
        let connection = open_read_write(&directory.path().join("project.db")).expect("open");

        let table_count: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'files'",
                [],
                |row| row.get(0),
            )
            .expect("query sqlite_master");
        assert_eq!(table_count, 1);
    }

    #[test]
    fn applying_schema_twice_is_idempotent() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        open_read_write(&path).expect("first open");
        let second = open_read_write(&path).expect("second open");

        let version: i64 = second
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("read version");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn schema_has_no_risk_pattern_table() {
        let directory = tempfile::tempdir().expect("temp dir");
        let connection = open_read_write(&directory.path().join("project.db")).expect("open");

        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master \
                 WHERE type = 'table' AND name LIKE '%risk%'",
                [],
                |row| row.get(0),
            )
            .expect("query sqlite_master");
        assert_eq!(count, 0);
    }

    #[test]
    fn rejects_a_database_from_a_newer_schema_version() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        let connection = open_read_write(&path).expect("open database");
        connection
            .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION + 1)
            .expect("bump version");
        drop(connection);

        let reopened = open_read_write(&path);
        assert!(matches!(
            reopened,
            Err(StoreError::Migration(
                MigrationError::UnsupportedVersion { .. }
            ))
        ));
    }
}
