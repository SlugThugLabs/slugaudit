//! Database schema verification and initialization.
use rusqlite::Connection;
use thiserror::Error;

const CURRENT_SCHEMA_VERSION: i64 = 3;
const SCHEMA_DDL: &str = include_str!("schema.sql");

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("failed to read schema version: {0}")]
    ReadVersion(#[source] rusqlite::Error),
    #[error("failed to apply schema: {0}")]
    Apply(#[source] rusqlite::Error),
    #[error(
        "database schema version {found} is incompatible with this build (supported: {supported}); \
         database will be discarded and rebuilt"
    )]
    UnsupportedVersion { found: i64, supported: i64 },
}

impl MigrationError {
    #[must_use]
    pub fn is_corruption(&self) -> bool {
        match self {
            Self::ReadVersion(error) | Self::Apply(error) => super::is_rusqlite_corruption(error),
            Self::UnsupportedVersion { .. } => true,
        }
    }
}

/// Brings a freshly-opened database up to `CURRENT_SCHEMA_VERSION`.
/// If the database version is not 0 (fresh) and does not match `CURRENT_SCHEMA_VERSION`,
/// it is rejected with `UnsupportedVersion`. Because `UnsupportedVersion` marks
/// `is_corruption() == true`, the sync manager discards the outdated database and
/// rebuilds a fresh one from scratch from project source code.
pub(super) fn ensure_current_schema(connection: &mut Connection) -> Result<(), MigrationError> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(MigrationError::ReadVersion)?;

    if version == CURRENT_SCHEMA_VERSION {
        return Ok(());
    }

    if version == 0 {
        let tx = connection.transaction().map_err(MigrationError::Apply)?;
        tx.execute_batch(SCHEMA_DDL)
            .map_err(MigrationError::Apply)?;
        tx.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)
            .map_err(MigrationError::Apply)?;
        tx.commit().map_err(MigrationError::Apply)?;
        return Ok(());
    }

    Err(MigrationError::UnsupportedVersion {
        found: version,
        supported: CURRENT_SCHEMA_VERSION,
    })
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

        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
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
        assert!(reopened.unwrap_err().is_corruption());
    }

    #[test]
    fn rejects_an_outdated_v1_database_and_marks_as_corruption_for_discard() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        let mut connection = open_read_write(&path).expect("open database");
        connection
            .pragma_update(None, "user_version", 1_i64)
            .expect("rewind version to v1");
        let err = ensure_current_schema(&mut connection).expect_err("must reject v1");
        assert!(matches!(
            err,
            MigrationError::UnsupportedVersion {
                found: 1,
                supported: CURRENT_SCHEMA_VERSION
            }
        ));
        assert!(
            err.is_corruption(),
            "must flag as corruption for auto-rebuild"
        );
    }

    #[test]
    fn rejects_an_outdated_v2_database_and_marks_as_corruption_for_discard() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        let mut connection = open_read_write(&path).expect("open database");
        connection
            .pragma_update(None, "user_version", 2_i64)
            .expect("rewind version to v2");
        let err = ensure_current_schema(&mut connection).expect_err("must reject v2");
        assert!(matches!(
            err,
            MigrationError::UnsupportedVersion {
                found: 2,
                supported: CURRENT_SCHEMA_VERSION
            }
        ));
        assert!(
            err.is_corruption(),
            "must flag as corruption for auto-rebuild"
        );
    }

    #[test]
    fn ensure_current_schema_can_be_invoked_twice_without_error() {
        let directory = tempfile::tempdir().expect("temp dir");
        let mut connection =
            open_read_write(&directory.path().join("project.db")).expect("open database");
        ensure_current_schema(&mut connection).expect("second pass");
        ensure_current_schema(&mut connection).expect("third pass");
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    /// Mirrors the version probe shape `ensure_current_schema` reads, so
    /// a future pragma-repo bump can't silently regress back to
    /// returning 0 (which would re-run every migration).
    #[test]
    fn pragma_user_version_round_trips_via_optional() {
        let directory = tempfile::tempdir().expect("temp dir");
        let connection = open_read_write(&directory.path().join("project.db")).expect("open");
        connection
            .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)
            .expect("set user_version");
        let version: Option<i64> = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .ok();
        assert_eq!(version, Some(CURRENT_SCHEMA_VERSION));
    }
}
