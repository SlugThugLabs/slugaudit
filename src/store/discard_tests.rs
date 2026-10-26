use super::*;
use rusqlite::Connection;

fn database_with_version(path: &Path, version: i64) {
    let connection = Connection::open(path).expect("create database");
    connection
        .pragma_update(None, "user_version", version)
        .expect("set user_version");
}

#[test]
fn a_database_at_a_foreign_schema_version_is_discarded() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    database_with_version(&path, CURRENT_SCHEMA_VERSION - 1);

    let outcome = discard_if_still_unusable(&path).expect("discard");

    assert_eq!(outcome, DiscardOutcome::Discarded);
    assert!(!path.exists());
}

#[test]
fn a_database_another_process_already_brought_current_is_not_discarded() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    database_with_version(&path, CURRENT_SCHEMA_VERSION);

    let outcome = discard_if_still_unusable(&path).expect("discard");

    assert_eq!(outcome, DiscardOutcome::AlreadyCurrent);
    assert!(path.exists());
}

#[test]
fn a_just_created_empty_database_is_not_discarded() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    database_with_version(&path, 0);

    let outcome = discard_if_still_unusable(&path).expect("discard");

    assert_eq!(outcome, DiscardOutcome::AlreadyCurrent);
    assert!(path.exists());
}

#[test]
fn a_file_that_is_not_a_database_is_discarded() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    std::fs::write(&path, vec![0x5a_u8; 4096]).expect("garbage fixture");

    let outcome = discard_if_still_unusable(&path).expect("discard");

    assert_eq!(outcome, DiscardOutcome::Discarded);
    assert!(!path.exists());
}

#[test]
fn a_database_another_process_already_removed_reports_discarded() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");

    let outcome = discard_if_still_unusable(&path).expect("discard");

    assert_eq!(outcome, DiscardOutcome::Discarded);
}

#[test]
fn the_version_check_under_the_discard_lock_reports_contention_when_held_elsewhere() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    database_with_version(&path, CURRENT_SCHEMA_VERSION - 1);
    let holder = Connection::open(&path).expect("holder");
    holder
        .execute_batch("BEGIN EXCLUSIVE")
        .expect("hold the write lock");
    // Shrinks the wait by making the discarding connection give up at once:
    // the production timeout is 5 s, which this test must not sit through.
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let mut contender = Connection::open_with_flags(&path, flags).expect("contender");
    contender
        .busy_timeout(std::time::Duration::ZERO)
        .expect("no wait");

    let error = locked_version(&mut contender).expect_err("lock must be refused");

    assert!(is_contention(&error), "{error:?}");
}
