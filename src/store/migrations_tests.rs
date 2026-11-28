use super::*;
use crate::store::connection::{StoreError, open_read_write};
use crate::util::join_checked;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn applies_schema_v14_with_content_as_final_files_column() -> TestResult {
    ensure_eq(
        &CURRENT_SCHEMA_VERSION,
        &14,
        "CURRENT_SCHEMA_VERSION is bumped to 14",
    )?;
    let directory = tempfile::tempdir().check("temp dir")?;
    let connection = open_read_write(&directory.path().join("project.db")).check("open")?;

    let table_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'files'",
            [],
            |row| row.get(0),
        )
        .check("query sqlite_master")?;
    ensure_eq(&table_count, &1, "files table exists after migration")?;

    let last_col: String = connection
        .query_row(
            "SELECT name FROM pragma_table_info('files') ORDER BY cid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .check("read last column of files")?;
    ensure_eq(
        &last_col,
        &"content",
        "content is the final column of files so metadata scans avoid overflow pages",
    )?;

    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .check("user_version")?;
    ensure_eq(
        &version,
        &CURRENT_SCHEMA_VERSION,
        "PRAGMA user_version matches CURRENT_SCHEMA_VERSION",
    )?;
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
    ensure_eq(
        &version,
        &CURRENT_SCHEMA_VERSION,
        "reopening preserves CURRENT_SCHEMA_VERSION",
    )?;
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
    ensure_eq(&count, &0, "schema defines no risk_pattern tables")?;
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
    ensure(
        matches!(
            reopened,
            Err(StoreError::Migration(
                MigrationError::UnsupportedVersion { .. }
            ))
        ),
        "database with future schema version fails with UnsupportedVersion",
    )?;
    ensure(
        reopened
            .check_err("a database from a newer version is refused")?
            .is_corruption(),
        "future schema version marks database for discard and rebuild",
    )?;
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
    ensure(
        matches!(
            err,
            MigrationError::UnsupportedVersion {
                found: 1,
                supported: CURRENT_SCHEMA_VERSION
            }
        ),
        "v1 database fails with UnsupportedVersion naming found=1",
    )?;
    ensure(
        err.is_corruption(),
        "must flag as corruption for auto-rebuild",
    )?;
    Ok(())
}

#[test]
fn rejects_an_outdated_v12_database_and_marks_as_corruption_for_discard() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("project.db");
    let mut connection = open_read_write(&path).check("open database")?;
    connection
        .pragma_update(None, "user_version", 12_i64)
        .check("rewind version to v12")?;
    let err = ensure_current_schema(&mut connection).check_err("must reject v12")?;
    ensure(
        matches!(
            err,
            MigrationError::UnsupportedVersion {
                found: 12,
                supported: CURRENT_SCHEMA_VERSION
            }
        ),
        "v12 database fails with UnsupportedVersion naming found=12",
    )?;
    ensure(
        err.is_corruption(),
        "must flag as corruption for auto-rebuild",
    )?;
    Ok(())
}

#[test]
fn a_file_that_is_not_a_database_fails_reading_the_version_as_corruption() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("project.db");
    std::fs::write(&path, vec![0xA5_u8; 4096]).check("write garbage")?;
    let mut connection = Connection::open(&path).check("open handle")?;

    let err = ensure_current_schema(&mut connection).check_err("garbage must not migrate")?;

    ensure(
        matches!(err, MigrationError::ReadVersion(_)),
        format!("expected ReadVersion, got {err:?}"),
    )?;
    ensure(err.is_corruption(), "garbage must be discarded and rebuilt")?;
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

    ensure(
        matches!(err, MigrationError::Apply(_)),
        format!("expected Apply, got {err:?}"),
    )?;
    ensure(
        !err.is_corruption(),
        "a read-only handle is an operational failure, not a corrupt file to discard",
    )?;
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
    ensure_eq(
        &version,
        &CURRENT_SCHEMA_VERSION,
        "repeated ensure_current_schema keeps CURRENT_SCHEMA_VERSION",
    )?;
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
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .check("read user_version")?;
    ensure_eq(
        &version,
        &CURRENT_SCHEMA_VERSION,
        "user_version pragma round-trips accurately",
    )?;
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

    ensure(
        matches!(
            result,
            Err(MigrationError::UnsupportedVersion { found: 99, .. })
        ),
        format!("{result:?}"),
    )?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .check("version")?;
    ensure_eq(
        &version,
        &99,
        "foreign user_version is left untouched when rejected",
    )?;
    Ok(())
}
