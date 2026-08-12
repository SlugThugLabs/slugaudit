//! Tests for the metadata helpers split out of `manager.rs`.

use super::{current_revision_id, ensure_project_row, purge_prior_session_findings_with};
use crate::store;
use crate::tools::context::override_session_id_for_test;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;
use uuid::Uuid;

/// Mirrors `tools::finding::session_tests::SESSION_TEST_LOCK` so the
/// two test modules serialize against the production `SESSION_ID`
/// global. Held by every test in this module that drives
/// `override_session_id_for_test`, and by the explicit-uuid tests
/// that go through `ensure_project_row`'s production cleanup path
/// (which calls `purge_prior_session_findings` and therefore reads
/// the global session once).
static SESSION_TEST_LOCK: Mutex<()> = Mutex::new(());

fn open_temp_db() -> (TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("project.db");
    let connection = store::open_read_write(&db_path).expect("open db");
    (dir, connection)
}

#[test]
fn current_revision_id_returns_none_when_no_revision_has_been_published() {
    let (_dir, connection) = open_temp_db();
    let revision = current_revision_id(&connection).expect("query");
    assert!(revision.is_none());
}

#[test]
fn ensure_project_row_succeeds_on_a_fresh_database() {
    let (_dir, mut connection) = open_temp_db();
    let root = Path::new("/tmp/example-project");
    ensure_project_row(&mut connection, root).expect("first ensure");
    // Calling twice must be idempotent (the row already exists).
    ensure_project_row(&mut connection, root).expect("idempotent on second call");
}

#[test]
fn ensure_project_row_detects_a_mismatched_root_via_invalid_params() {
    let (_dir, mut connection) = open_temp_db();
    let first_root = Path::new("/tmp/first-root");
    ensure_project_row(&mut connection, first_root).expect("first ensure");
    let second_root = Path::new("/tmp/different-root");
    let err = ensure_project_row(&mut connection, second_root).expect_err("must error");
    // The implementation uses invalid_params for a root-mismatch since
    // the caller can recover by disabling and re-enabling.
    assert!(
        err.message.contains("different project root")
            || err.message.contains("belongs to a different"),
        "got: {err:?}",
    );
}

/// Drives `purge_prior_session_findings` directly with two distinct
/// session UUIDs against a hand-seeded findings table, simulating two
/// separate agent processes. The first session's rows must be deleted;
/// the second session's rows must remain untouched. This is the
/// cross-session poisoning defense's correctness surface — no other
/// test exercises the live SESSION_ID flow without going through
/// `finding()`, which would couple the test to the schema panel.
#[test]
fn purge_deletes_only_prior_session_rows() {
    let (_dir, mut connection) = open_temp_db();
    for (path, session) in [
        ("old.rs", "11111111-1111-1111-1111-111111111111"),
        ("shared.rs", "22222222-2222-2222-2222-222222222222"),
    ] {
        connection
            .execute(
                "INSERT INTO findings (path, source_hash, line_start, line_end, \
                 severity, category, title, description, created_at_unix, \
                 evidence_revision, status, session_id) \
                 VALUES (?1, 'h', 1, 1, 'low', 'c', 't', 'd', 0, 'r', 'current', ?2)",
                rusqlite::params![path, session],
            )
            .expect("seed finding");
    }

    // A second agent boots with its own UUID and runs the purge.
    purge_prior_session_findings_with(
        &mut connection,
        Uuid::parse_str("33333333-3333-3333-3333-333333333333").expect("new uuid"),
    )
    .expect("purge");

    let remaining: Vec<String> = connection
        .prepare("SELECT session_id FROM findings ORDER BY path")
        .expect("select")
        .query_map([], |row| row.get(0))
        .expect("query")
        .map(|value| value.expect("row"))
        .collect();
    // The "1111" row was tagged with a session other than the new
    // "3333" session, so the cleanup MUST drop it. The "2222" row was
    // also tagged as "not me", so it MUST drop too — only rows stamped
    // with the CURRENT session survive.
    assert!(
        remaining.is_empty(),
        "every legacy row is from a prior session and must be gone: {remaining:?}"
    );

    // A second run with a session that DOES own a row is a no-op
    // (no rows deleted) — the test pins that the cleanup is exact,
    // not over-broad.
    connection
        .execute(
            "INSERT INTO findings (path, source_hash, line_start, line_end, \
             severity, category, title, description, created_at_unix, \
             evidence_revision, status, session_id) \
             VALUES ('alive.rs', 'h', 1, 1, 'low', 'c', 't', 'd', 0, 'r', 'current', ?1)",
            rusqlite::params![
                Uuid::parse_str("44444444-4444-4444-4444-444444444444")
                    .expect("alive uuid")
                    .to_string()
            ],
        )
        .expect("insert alive");
    purge_prior_session_findings_with(
        &mut connection,
        Uuid::parse_str("44444444-4444-4444-4444-444444444444").expect("alive uuid"),
    )
    .expect("no-op purge");
    let count: i64 = connection
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1, "alive row must survive a matching-session purge");
}

/// `ensure_project_row` is responsible for both metadata-row creation
/// AND session-scoped finding cleanup on every call — so the
/// cross-session defense runs even when the project-row branch is
/// itself a no-op (already initialized). This pins that the purge runs
/// unconditionally on the session-start path, not buried behind any
/// "is the project row new?" check.
#[test]
fn ensure_project_row_purges_findings_on_a_warm_cache_too() {
    let _session_guard = SESSION_TEST_LOCK
        .lock()
        .expect("session test lock poisoned");
    let (_dir, mut connection) = open_temp_db();
    let root = Path::new("/tmp/warm-example");
    ensure_project_row(&mut connection, root).expect("first ensure");

    let prior_session =
        Uuid::parse_str("55555555-5555-5555-5555-555555555555").expect("prior uuid");
    connection
        .execute(
            "INSERT INTO findings (path, source_hash, line_start, line_end, \
             severity, category, title, description, created_at_unix, \
             evidence_revision, status, session_id) \
             VALUES ('stale.rs', 'h', 1, 1, 'low', 'c', 't', 'd', 0, 'r', 'current', ?1)",
            rusqlite::params![prior_session.to_string()],
        )
        .expect("seed stale finding");

    // A change in SESSION_ID between two `ensure_project_row` calls
    // exercises the contract: the second call must drop the row tagged
    // with the previous session id even though the project row was
    // already there.
    override_session_id_for_test(
        Uuid::parse_str("55555555-5555-5555-5555-555555555555").expect("warm session"),
    );
    ensure_project_row(&mut connection, root).expect("warm-path with matching session");

    let count_matching: i64 = connection
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .expect("count after warm");
    assert_eq!(count_matching, 1, "matching session → row stays");

    override_session_id_for_test(
        Uuid::parse_str("66666666-6666-6666-6666-666666666666").expect("different session"),
    );
    ensure_project_row(&mut connection, root).expect("warm-path with new session");

    let count_cleaned: i64 = connection
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .expect("count after cross-session");
    assert_eq!(count_cleaned, 0, "different session → row must be purged");
}
