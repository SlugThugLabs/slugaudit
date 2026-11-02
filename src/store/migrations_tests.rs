use super::*;
use crate::store::connection::{StoreError, open_read_write};
use crate::store::test_join::join_checked;
use slugaudit_test_check::{Check, CheckErr, TestResult};

#[test]
fn applies_schema_to_a_fresh_database() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let connection = open_read_write(&directory.path().join("project.db")).check("open")?;

    let table_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'files'",
            [],
            |row| row.get(0),
        )
        .check("query sqlite_master")?;
    assert_eq!(table_count, 1);

    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .check("user_version")?;
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
    Ok(())
}

#[test]
fn applying_schema_twice_is_idempotent() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("project.db");
    open_read_write(&path).check("first open")?;
    let second = open_read_write(&path).check("second open")?;

    let version: i64 = second
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .check("read version")?;
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
    Ok(())
}

#[test]
fn schema_has_no_risk_pattern_table() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let connection = open_read_write(&directory.path().join("project.db")).check("open")?;

    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master \
             WHERE type = 'table' AND name LIKE '%risk%'",
            [],
            |row| row.get(0),
        )
        .check("query sqlite_master")?;
    assert_eq!(count, 0);
    Ok(())
}

#[test]
fn rejects_a_database_from_a_newer_schema_version() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("project.db");
    let connection = open_read_write(&path).check("open database")?;
    connection
        .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION + 1)
        .check("bump version")?;
    drop(connection);

    let reopened = open_read_write(&path);
    assert!(matches!(
        reopened,
        Err(StoreError::Migration(
            MigrationError::UnsupportedVersion { .. }
        ))
    ));
    assert!(
        reopened
            .check_err("a database from a newer version is refused")?
            .is_corruption()
    );
    Ok(())
}

#[test]
fn rejects_an_outdated_v1_database_and_marks_as_corruption_for_discard() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("project.db");
    let mut connection = open_read_write(&path).check("open database")?;
    connection
        .pragma_update(None, "user_version", 1_i64)
        .check("rewind version to v1")?;
    let err = ensure_current_schema(&mut connection).check_err("must reject v1")?;
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
    Ok(())
}

#[test]
fn rejects_an_outdated_v2_database_and_marks_as_corruption_for_discard() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("project.db");
    let mut connection = open_read_write(&path).check("open database")?;
    connection
        .pragma_update(None, "user_version", 2_i64)
        .check("rewind version to v2")?;
    let err = ensure_current_schema(&mut connection).check_err("must reject v2")?;
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
    Ok(())
}

#[test]
fn a_file_that_is_not_a_database_fails_reading_the_version_as_corruption() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("project.db");
    std::fs::write(&path, vec![0xA5_u8; 4096]).check("write garbage")?;
    let mut connection = Connection::open(&path).check("open handle")?;

    let err = ensure_current_schema(&mut connection).check_err("garbage must not migrate")?;

    assert!(
        matches!(err, MigrationError::ReadVersion(_)),
        "expected ReadVersion, got {err:?}"
    );
    assert!(err.is_corruption(), "garbage must be discarded and rebuilt");
    Ok(())
}

#[test]
fn a_read_only_fresh_database_fails_applying_the_schema_without_claiming_corruption() -> TestResult
{
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("project.db");
    drop(Connection::open(&path).check("create empty database")?);
    let mut connection =
        Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .check("open read-only")?;

    let err = ensure_current_schema(&mut connection).check_err("read-only cannot migrate")?;

    assert!(
        matches!(err, MigrationError::Apply(_)),
        "expected Apply, got {err:?}"
    );
    assert!(
        !err.is_corruption(),
        "a read-only handle is an operational failure, not a corrupt file to discard"
    );
    Ok(())
}

#[test]
fn ensure_current_schema_can_be_invoked_twice_without_error() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let mut connection =
        open_read_write(&directory.path().join("project.db")).check("open database")?;
    ensure_current_schema(&mut connection).check("second pass")?;
    ensure_current_schema(&mut connection).check("third pass")?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .check("user_version")?;
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
    Ok(())
}

/// Mirrors the version probe shape `ensure_current_schema` reads, so
/// a future pragma-repo bump can't silently regress back to
/// returning 0 (which would re-run every migration).
#[test]
fn pragma_user_version_round_trips_via_optional() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let connection = open_read_write(&directory.path().join("project.db")).check("open")?;
    connection
        .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)
        .check("set user_version")?;
    let version: Option<i64> = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .ok();
    assert_eq!(version, Some(CURRENT_SCHEMA_VERSION));
    Ok(())
}

/// A WAL database file plus a second connection holding its write lock, so
/// `ensure_current_schema` has to wait at its IMMEDIATE transaction.
fn wal_database_with_writer(path: &std::path::Path) -> TestResult<Connection> {
    let writer = Connection::open(path).check("writer")?;
    writer
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .check("wal")?;
    writer
        .execute_batch("BEGIN IMMEDIATE")
        .check("hold the write lock")?;
    Ok(writer)
}

#[test]
fn schema_init_waits_for_a_concurrent_creator_and_keeps_its_schema() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("project.db");
    let writer = wal_database_with_writer(&path)?;
    let mut connection = Connection::open(&path).check("second process")?;
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        writer
            .execute_batch(SCHEMA_DDL)
            .check("first creator's schema")?;
        writer
            .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)
            .check("first creator's version")?;
        writer.execute_batch("COMMIT").check("commit")
    });

    let result = ensure_current_schema(&mut connection);
    join_checked(releaser, "creator thread")??;

    result.check("second process accepts the first one's schema")?;
    Ok(())
}

#[test]
fn schema_init_does_not_overwrite_a_foreign_version_set_while_it_waited() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("project.db");
    let writer = wal_database_with_writer(&path)?;
    let mut connection = Connection::open(&path).check("second process")?;
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        writer
            .pragma_update(None, "user_version", 99)
            .check("foreign version")?;
        writer.execute_batch("COMMIT").check("commit")
    });

    let result = ensure_current_schema(&mut connection);
    join_checked(releaser, "creator thread")??;

    assert!(
        matches!(
            result,
            Err(MigrationError::UnsupportedVersion { found: 99, .. })
        ),
        "{result:?}"
    );
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .check("version")?;
    assert_eq!(version, 99);
    Ok(())
}
