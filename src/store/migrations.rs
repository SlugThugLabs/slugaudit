//! Database schema verification and initialization.
use rusqlite::Connection;
use thiserror::Error;

/// The schema version this build writes to `PRAGMA user_version`. Bumping it
/// makes an older database resolve as `UnsupportedVersion`, which the sync
/// manager treats as corruption and rebuilds from project source — so a bump
/// needs no migration code, only a matching `schema.sql` change.
pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 11;
const SCHEMA_DDL: &str = include_str!("schema.sql");

#[derive(Debug, Error)]
/// Schema setup failures: applying `schema.sql`/migrations or reading
/// `user_version`.
pub enum MigrationError {
    /// Reading `PRAGMA user_version` failed.
    #[error("failed to read schema version: {0}")]
    ReadVersion(#[source] rusqlite::Error),
    /// Applying `schema.sql` or recording the schema version failed.
    #[error("failed to apply schema: {0}")]
    Apply(#[source] rusqlite::Error),
    // The message names the mismatch only: the sync path discards and rebuilds
    // on `is_corruption()`, but other callers of `open_*` surface this as-is.
    /// The database was written by a schema version this build cannot use,
    /// so it is treated as corrupt and rebuilt from project source.
    #[error(
        "database schema version {found} is incompatible with this build (supported: {supported})"
    )]
    UnsupportedVersion {
        /// The schema version recorded in the database.
        found: i64,
        /// The schema version this build writes.
        supported: i64,
    },
}

impl MigrationError {
    #[must_use]
    /// True when a migration failure means the file itself is corrupt (not
    /// merely an older schema).
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
#[path = "migrations_tests.rs"]
mod tests;
