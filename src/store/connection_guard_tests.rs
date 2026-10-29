use super::*;
use crate::store::connection_guards::enable_wal;
use crate::store::test_capture::capture_warns;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

#[test]
fn an_uninspectable_database_path_warning_names_the_path_and_the_os_error() {
    let path = Path::new("/tmp").join("a".repeat(300)).join("project.db");
    let (result, logs) = capture_warns(|| {
        tracing::warn!("lstat_control");
        reject_symlink(&path)
    });
    assert!(matches!(result, Err(StoreError::Inspect(_))) && logs.contains("lstat_control"));
    let Err(StoreError::Inspect(source)) = &result else {
        panic!("expected an inspect error, got {result:?}");
    };
    assert!(logs.contains(path.to_string_lossy().as_ref()), "{logs}");
    assert!(logs.contains(&source.to_string()), "{logs}");
    assert!(!logs.contains("path_id="), "{logs}");
}

#[test]
fn a_canonicalize_fallback_warning_names_the_path_and_the_os_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("missing-parent").join("project.db");
    let (result, logs) = capture_warns(|| {
        tracing::warn!("canonicalize_control");
        open_read_only(&path)
    });
    assert!(matches!(&result, Err(StoreError::Open(_))), "{result:?}");
    assert!(logs.contains("canonicalize_control"), "{logs}");
    let os_error = directory
        .path()
        .join("missing-parent")
        .canonicalize()
        .expect_err("the parent does not exist")
        .to_string();
    assert!(logs.contains(path.to_string_lossy().as_ref()), "{logs}");
    assert!(logs.contains(&os_error), "{logs}");
    assert!(!logs.contains("path_id="), "{logs}");
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

/// A rollback-mode database whose lock another connection holds: converting it
/// to WAL reports busy immediately (no busy handler involved), which is the
/// state two processes creating the same fresh project database hit.
fn locked_fresh_database() -> (tempfile::TempDir, std::path::PathBuf, Connection) {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    let holder = Connection::open(&path).expect("holder");
    holder
        .execute_batch("CREATE TABLE t(x); BEGIN EXCLUSIVE")
        .expect("hold the exclusive lock");
    (directory, path, holder)
}

#[test]
fn enabling_wal_waits_out_a_lock_another_process_releases() {
    let (_directory, path, holder) = locked_fresh_database();
    let contender = Connection::open(&path).expect("contender");
    contender
        .busy_timeout(Duration::ZERO)
        .expect("no busy wait");
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        holder.execute_batch("COMMIT").expect("release the lock");
    });

    let result = enable_wal(&contender, Duration::from_secs(5));
    releaser.join().expect("releaser thread");

    result.expect("WAL enabled after the lock was released");
    let mode: String = contender
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal_mode");
    assert_eq!(mode, "wal");
}

#[test]
fn enabling_wal_fails_with_busy_when_the_lock_outlasts_the_budget() {
    let (_directory, path, _holder) = locked_fresh_database();
    let contender = Connection::open(&path).expect("contender");
    contender
        .busy_timeout(Duration::ZERO)
        .expect("no busy wait");

    let error =
        enable_wal(&contender, Duration::from_millis(100)).expect_err("the lock is never released");

    assert!(matches!(error, StoreError::Busy(_)), "{error:?}");
}

#[test]
fn two_threads_opening_one_fresh_database_both_succeed() {
    for round in 0..20 {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let (path, barrier) = (path.clone(), std::sync::Arc::clone(&barrier));
                std::thread::spawn(move || {
                    barrier.wait();
                    open_read_write(&path).map(|_| ())
                })
            })
            .collect();
        for worker in workers {
            worker
                .join()
                .expect("open thread")
                .unwrap_or_else(|error| panic!("round {round}: {error}"));
        }
    }
}

#[test]
fn result_store_surface_read_only_connection_denies_every_create_temp_form() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    open_read_write(&path).expect("create database");
    let ro = open_read_only(&path).expect("open read-only");

    for statement in [
        "CREATE TEMP TABLE t (x)",
        "CREATE TEMP VIEW v AS SELECT 1",
        "CREATE TEMP TABLE IF NOT EXISTS u (x)",
    ] {
        let error = ro
            .execute_batch(statement)
            .expect_err("a temp object is refused");
        assert!(
            error.to_string().contains("not authorized"),
            "{statement}: {error}"
        );
    }
    let temp_objects: i64 = ro
        .query_row("SELECT count(*) FROM sqlite_temp_master", [], |row| {
            row.get(0)
        })
        .expect("count temp objects");
    assert_eq!(temp_objects, 0);
}

#[test]
fn result_store_surface_read_write_connection_still_creates_temp_tables() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    let rw = open_read_write(&path).expect("create database");

    rw.execute_batch("CREATE TEMP TABLE t (x)")
        .expect("the writer is not restricted");
}
