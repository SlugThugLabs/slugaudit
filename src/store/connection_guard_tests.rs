use super::*;
use crate::store::test_capture::capture_warns;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn an_uninspectable_database_path_warning_redacts_the_path() {
    let path = Path::new("/tmp").join("a".repeat(300)).join("project.db");
    let (result, logs) = capture_warns(|| {
        tracing::warn!("lstat_control");
        reject_symlink(&path)
    });
    assert!(matches!(result, Err(StoreError::Inspect(_))) && logs.contains("lstat_control"));
    assert!(logs.contains("path_id="), "{logs}");
    assert!(!logs.contains(path.to_string_lossy().as_ref()), "{logs}");
}

#[test]
fn a_canonicalize_fallback_warning_redacts_the_path() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("missing-parent").join("project.db");
    let (result, logs) = capture_warns(|| {
        tracing::warn!("canonicalize_control");
        open_read_only(&path)
    });
    assert!(matches!(&result, Err(StoreError::Open(_))), "{result:?}");
    assert!(logs.contains("canonicalize_control"), "{logs}");
    assert!(logs.contains("path_id="), "{logs}");
    assert!(!logs.contains(path.to_string_lossy().as_ref()), "{logs}");
}

#[cfg(unix)]
#[test]
fn a_directory_at_the_database_path_fails_with_an_open_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    std::fs::create_dir(&path).expect("directory fixture");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).expect("mode");
    let result = open_read_write(&path);
    assert!(matches!(&result, Err(StoreError::Open(_))), "{result:?}");
}

#[test]
fn a_directory_at_the_database_path_fails_the_corrupt_database_discard() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    std::fs::create_dir(&path).expect("directory fixture");
    let error = discard_corrupt_database(&path).expect_err("directory unlink must fail");
    let kind = error.kind();
    assert!(kind == ErrorKind::IsADirectory || kind == ErrorKind::PermissionDenied);
    assert!(path.is_dir());
}

#[test]
fn a_read_only_connection_carries_the_busy_timeout() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    open_read_write(&path).expect("create database");
    let ro = open_read_only(&path).expect("open read-only");
    // The read-only authorizer denies `PRAGMA busy_timeout`; the timeout
    // is a connection property independent of the authorizer, so the
    // test lifts the authorizer to read it.
    ro.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>);
    let timeout: i64 = ro
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .expect("read");
    assert_eq!(timeout, 5_000);
}

/// `soft_heap_limit` is applied while SQLite *prepares* the statement,
/// so only a prepare-time denial keeps it from taking effect. It is
/// process-wide, which is why a fresh connection reads it back.
#[test]
fn the_read_only_connection_denies_a_pragma_before_it_takes_effect() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    let rw = open_read_write(&path).expect("create database");
    let soft = |connection: &Connection| -> i64 {
        connection
            .query_row("PRAGMA soft_heap_limit", [], |row| row.get(0))
            .expect("read soft_heap_limit")
    };
    let before = soft(&rw);

    let denied = open_read_only(&path)
        .expect("open read-only")
        .prepare("PRAGMA soft_heap_limit = 123456789")
        .map(|_| ());

    assert_eq!(
        denied
            .as_ref()
            .err()
            .and_then(rusqlite::Error::sqlite_error_code),
        Some(rusqlite::ErrorCode::AuthorizationForStatementDenied),
        "{denied:?}"
    );
    assert_eq!(soft(&Connection::open_in_memory().expect("fresh")), before);
    assert_eq!(
        soft(&rw),
        before,
        "the read-write connection still runs pragmas"
    );
}

#[test]
fn the_read_only_connection_allows_only_schema_introspection_pragmas() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    open_read_write(&path).expect("create database");
    let ro = open_read_only(&path).expect("open read-only");
    let count = |sql: &str| ro.query_row(sql, [], |row| row.get::<_, i64>(0));

    for sql in [
        "SELECT count(*) FROM pragma_table_info('files')",
        "SELECT count(*) FROM pragma_TABLE_XINFO('files', 'main')",
        "SELECT count(*) FROM pragma_table_list",
        "SELECT count(*) FROM pragma_index_list('evidence')",
        "SELECT count(*) FROM pragma_foreign_key_list('evidence')",
    ] {
        assert!(count(sql).is_ok_and(|n| n > 0), "{sql}: {:?}", count(sql));
    }
    for sql in [
        "SELECT count(*) FROM pragma_user_version",
        "SELECT count(*) FROM pragma_optimize(0xfffe)",
        "SELECT count(*) FROM pragma_database_list",
    ] {
        assert_eq!(
            count(sql).err().and_then(|error| error.sqlite_error_code()),
            Some(rusqlite::ErrorCode::AuthorizationForStatementDenied),
            "{sql}"
        );
    }
}

#[test]
fn a_pragma_function_called_with_a_setter_argument_cannot_change_the_heap_limit() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    open_read_write(&path).expect("create database");
    let ro = open_read_only(&path).expect("open read-only");

    for sql in [
        "SELECT * FROM pragma_soft_heap_limit(1)",
        "SELECT * FROM pragma_hard_heap_limit(1)",
    ] {
        assert!(
            ro.query_row(sql, [], |row| row.get::<_, i64>(0)).is_err(),
            "{sql}"
        );
    }

    let rw = open_read_write(&path).expect("open read-write");
    for sql in ["PRAGMA soft_heap_limit", "PRAGMA hard_heap_limit"] {
        assert_eq!(
            rw.query_row(sql, [], |row| row.get::<_, i64>(0))
                .expect(sql),
            0,
            "{sql}"
        );
    }
}
