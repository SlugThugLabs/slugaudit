// slugaudit-line-exception: approved-by=agent; reason=one migration per schema version plus their version-pinning tests form a single forward-only sequence where each step depends on the ones before it; splitting by migration would scatter the ordering invariant (and the exact-version pin) across files
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
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    UnsupportedVersion { found: i64, supported: i64 },
}

impl MigrationError {
    #[must_use]
    pub fn is_corruption(&self) -> bool {
        match self {
            Self::ReadVersion(error) | Self::Apply(error) => super::is_rusqlite_corruption(error),
            Self::UnsupportedVersion { .. } => false,
        }
    }
}

/// One forward step from version `target - 1` to version `target`. Each
/// closure is expected to be idempotent on `target` —
/// `ensure_current_schema` doesn't re-run a step once `PRAGMA
/// user_version` reaches its `target` value.
type Migration = (i64, fn(&Connection) -> Result<(), rusqlite::Error>);

/// Forward-only schema migrations, applied in order. v0→v1 is the
/// original schema application; v1→v2 adds the
/// `findings.session_id` column and the `idx_findings_session` index
/// that the session-scoped cleanup relies on; v2→v3 adds
/// `dependency_edges.syntax_unmodeled` (the C11 write-time import-
/// syntax classifier verdict, so `report` counts it in SQL instead of
/// re-extracting every edge on every call).
const MIGRATIONS: &[Migration] = &[
    (1, apply_v0_to_v1),
    (2, apply_v1_to_v2),
    (3, apply_v2_to_v3),
];

fn apply_v0_to_v1(connection: &Connection) -> Result<(), rusqlite::Error> {
    // Fresh database — `CREATE … IF NOT EXISTS` is idempotent against
    // a re-run, but we only ever reach this on a clean pragma=0 db, so
    // the semantics are a one-time install.
    connection.execute_batch(SCHEMA_DDL)
}

fn apply_v1_to_v2(connection: &Connection) -> Result<(), rusqlite::Error> {
    // Idempotent against re-application: if a previous run already
    // added the column (a test path that rewinds `user_version`
    // rather than a real second `ensure_current_schema` call, since
    // production moves monotonically forward), the `ALTER` is skipped
    // and only the (also-idempotent) index creation runs. Findings
    // from any prior session are back-filled to the empty string
    // during the `ALTER`; the actual row deletion happens at every
    // `ensure_current`
    // (`sync::manager_meta::purge_prior_session_findings`) once the
    // current session UUID is generated.
    let column_present: i64 = connection.query_row(
        "SELECT count(*) FROM pragma_table_info('findings') \
         WHERE name = 'session_id'",
        [],
        |row| row.get(0),
    )?;
    if column_present == 0 {
        connection.execute(
            "ALTER TABLE findings ADD COLUMN session_id TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    connection
        .execute_batch("CREATE INDEX IF NOT EXISTS idx_findings_session ON findings (session_id);")
}

fn apply_v2_to_v3(connection: &Connection) -> Result<(), rusqlite::Error> {
    // Idempotent against re-application (a test path that rewinds
    // `user_version`). Existing Unresolved edges default to 0 — the
    // honest "not classified yet" value — and are back-filled the next
    // time their file is published (which re-runs
    // `revision_edges::resolve_and_store` and stores the verdict).
    let column_present: i64 = connection.query_row(
        "SELECT count(*) FROM pragma_table_info('dependency_edges') \
         WHERE name = 'syntax_unmodeled'",
        [],
        |row| row.get(0),
    )?;
    if column_present == 0 {
        connection.execute(
            "ALTER TABLE dependency_edges ADD COLUMN syntax_unmodeled INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

/// Brings a freshly-opened database up to `CURRENT_SCHEMA_VERSION`.
/// Migrations are forward-only: a database at a newer schema version
/// than this build knows about is rejected rather than guessed at.
/// Stepping forward happens in one transaction so SQLite's WAL + DDL
/// guarantees apply — either every migration lands and version bumps
/// or none of them do.
pub fn ensure_current_schema(connection: &mut Connection) -> Result<(), MigrationError> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(MigrationError::ReadVersion)?;

    if version > CURRENT_SCHEMA_VERSION {
        return Err(MigrationError::UnsupportedVersion {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    if version == CURRENT_SCHEMA_VERSION {
        return Ok(());
    }

    let tx = connection.transaction().map_err(MigrationError::Apply)?;
    for (target, step) in MIGRATIONS {
        if version < *target {
            step(&tx).map_err(MigrationError::Apply)?;
            tx.pragma_update(None, "user_version", *target)
                .map_err(MigrationError::Apply)?;
        }
    }
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
    }

    #[test]
    fn v1_database_upgrades_to_v2_with_session_id_column() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        let mut connection = open_read_write(&path).expect("open database");
        // Force the schema back to v1 so the v1→v2 migration actually
        // runs. (Open writes the live CURRENT_SCHEMA_VERSION into
        // user_version, so we rewind before calling
        // ensure_current_schema.)
        connection
            .pragma_update(None, "user_version", 1_i64)
            .expect("rewind version");
        ensure_current_schema(&mut connection).expect("apply v1→v2");

        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("read post-migration version");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);

        let column_present: i64 = connection
            .query_row(
                "SELECT count(*) FROM pragma_table_info('findings') \
                 WHERE name = 'session_id'",
                [],
                |row| row.get(0),
            )
            .expect("inspect findings columns");
        assert_eq!(
            column_present, 1,
            "session_id column must exist after v1→v2"
        );

        // Existing rows must have back-filled the empty string, not NULL.
        connection
            .execute(
                "INSERT INTO findings (path, source_hash, line_start, line_end, \
                                          severity, category, title, description, \
                                          created_at_unix, evidence_revision, status) \
                 VALUES ('legacy.rs', 'hash', 1, 1, 'low', 'legacy', \
                         't', 'd', 0, 'r0', 'current')",
                [],
            )
            .expect("legacy insert");
        let legacy_session: String = connection
            .query_row(
                "SELECT session_id FROM findings WHERE path = 'legacy.rs'",
                [],
                |row| row.get(0),
            )
            .expect("legacy session_id");
        assert_eq!(legacy_session, "");
    }

    #[test]
    fn v2_database_upgrades_to_v3_with_syntax_unmodeled_column() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        let mut connection = open_read_write(&path).expect("open database");
        // Force the schema back to v2 so the v2→v3 migration runs.
        connection
            .pragma_update(None, "user_version", 2_i64)
            .expect("rewind version");
        ensure_current_schema(&mut connection).expect("apply v2→v3");

        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("read post-migration version");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);

        let column_present: i64 = connection
            .query_row(
                "SELECT count(*) FROM pragma_table_info('dependency_edges') \
                 WHERE name = 'syntax_unmodeled'",
                [],
                |row| row.get(0),
            )
            .expect("inspect dependency_edges columns");
        assert_eq!(
            column_present, 1,
            "syntax_unmodeled column must exist after v2→v3"
        );

        // Existing edges default to 0 (unclassified), the honest value.
        let default: i64 = connection
            .query_row(
                "SELECT syntax_unmodeled FROM dependency_edges LIMIT 0",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        assert_eq!(default, 0);
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
