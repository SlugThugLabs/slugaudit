//! Tests for the database-side content check: stored text is compared with
//! its own stored hash, and only rows that carry content are checked.

use super::*;
use rusqlite::{Connection, params};
use slugaudit_test_check::{Check, CheckErr, TestResult};

fn deadline() -> Deadline {
    Deadline::after(std::time::Duration::from_mins(1))
}

fn seeded_connection() -> TestResult<(tempfile::TempDir, Connection)> {
    let dir = tempfile::tempdir().check("db dir")?;
    let connection =
        crate::store::open_read_write(&dir.path().join("project.db")).check("open db")?;
    connection
        .execute(
            "INSERT INTO revisions \
                (revision_id, manifest_hash, parser_pack_version, created_at_unix, is_current) \
             VALUES ('rev-1', 'manifest', '1.0', 0, 1)",
            [],
        )
        .check("seed revision")?;
    Ok((dir, connection))
}

fn insert_file(connection: &Connection, path: &str, content: &str, hash: &str) -> TestResult {
    connection
        .execute(
            "INSERT INTO files (path, content, content_hash, hash_algorithm, \
             parser_availability, parse_outcome, extraction_completeness, last_revision_id) \
             VALUES (?1, ?2, ?3, 'blake3-v1', 'Available', 'Succeeded', 'Full', 1)",
            params![path, content, hash],
        )
        .check("insert file row")?;
    Ok(())
}

#[test]
fn matching_content_is_not_reported() -> TestResult {
    let (_dir, connection) = seeded_connection()?;
    let content = "fn a() {}\n";
    let hash = blake3::hash(content.as_bytes()).to_string();
    insert_file(&connection, "a.rs", content, &hash)?;

    let mismatched = content_mismatches(&connection, &deadline()).check("check")?;
    assert!(mismatched.is_empty(), "{mismatched:?}");
    Ok(())
}

#[test]
fn edited_content_with_an_untouched_hash_is_reported() -> TestResult {
    let (_dir, connection) = seeded_connection()?;
    let original = "fn a() {}\n";
    let hash = blake3::hash(original.as_bytes()).to_string();
    insert_file(&connection, "a.rs", original, &hash)?;
    connection
        .execute(
            "UPDATE files SET content = 'fn a() { changed(); }\n' WHERE path = 'a.rs'",
            [],
        )
        .check("tamper with the stored content")?;

    let mismatched: Vec<String> = content_mismatches(&connection, &deadline())
        .check("check")?
        .into_iter()
        .collect();
    assert_eq!(mismatched, ["a.rs"]);
    Ok(())
}

#[test]
fn rows_without_content_are_not_checked() -> TestResult {
    let (_dir, connection) = seeded_connection()?;
    connection
        .execute(
            "INSERT INTO files \
                (path, file_kind, skip_reason, content, content_hash, hash_algorithm, \
                 parser_availability, parse_outcome, extraction_completeness, last_revision_id) \
             VALUES ('skip.bin', 'skipped', 'oversize', NULL, 'stat-identity-hash', \
                     'stat-identity', 'Unavailable', 'NotAttempted', 'Unavailable', 1)",
            [],
        )
        .check("skipped row")?;

    let mismatched = content_mismatches(&connection, &deadline()).check("check")?;
    assert!(mismatched.is_empty(), "{mismatched:?}");
    Ok(())
}

#[test]
fn a_spent_deadline_fails_the_check_before_comparing() -> TestResult {
    let (_dir, connection) = seeded_connection()?;
    let content = "fn a() {}\n";
    let hash = blake3::hash(content.as_bytes()).to_string();
    insert_file(&connection, "a.rs", content, &hash)?;

    let error = content_mismatches(&connection, &Deadline::after(std::time::Duration::ZERO))
        .check_err("a spent budget must fail the check")?;
    assert!(matches!(error, PublishError::TimeBudgetExceeded { .. }));
    Ok(())
}
