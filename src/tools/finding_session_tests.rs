//! Proves the `findings` row is bound to the agent session that wrote
//! it, and that a fresh session wipes the prior session's notes.
//! Pulled out of `finding_tests.rs` to keep that file under the
//! 200-code-line soft cap while still exercising the cross-session
//! poisoning defense end to end.
use super::*;
use crate::tools::context::override_session_id_for_test;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use rusqlite::Connection;
use std::fs;
use std::sync::Mutex;
use uuid::Uuid;

/// Single global lock held by every test in this module. The
/// production `SESSION_ID` is a process-wide `Mutex<Option<Uuid>>`;
/// parallel-running tests that depend on a specific session
/// identity would race over that global. Taking this test-only
/// lock serializes every session-flipping test in this module and
/// in `manager_meta_tests.rs` so a sibling test's
/// `override_session_id_for_test` cannot slip between this test's
/// two session flips. Pure read-only assertions and writes that do
/// not override the session do not need the lock.
static SESSION_TEST_LOCK: Mutex<()> = Mutex::new(());

fn base_request(project: &tempfile::TempDir, file: &str) -> FindingRequest {
    FindingRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: file.to_owned(),
        line_start: 1,
        line_end: 1,
        severity: "medium".to_owned(),
        category: "correctness".to_owned(),
        title: "Reviewed conclusion".to_owned(),
        description: "A conclusion the AI actually checked.".to_owned(),
    }
}

/// Read-only verification helper that does NOT run the session-scoped
/// cleanup. `open_read_write` would re-enter `ensure_project_row` and
/// the purge we are testing against, so a sibling parallel test
/// overriding `SESSION_ID` mid-run could spuriously delete a row
/// we're trying to count. `Connection::open_with_flags`+`SQLITE_OPEN_READ_ONLY`
/// is the closest existing primitive and avoids the cleanup path
/// entirely — the schema is already at v2 (the test fixture's
/// `finding()` call above brought it there).
fn read_only_connection(db_path: &std::path::Path) -> Connection {
    use rusqlite::OpenFlags;
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let uri = format!(
        "file:{}?mode=ro",
        db_path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "%22")
    );
    Connection::open_with_flags(&uri, flags).expect("open read-only")
}

fn count_findings(db_path: &std::path::Path) -> i64 {
    read_only_connection(db_path)
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .expect("count findings")
}

/// Every newly inserted finding row must carry the current process's
/// session UUID. No row in the table may be un-tagged; the
/// `default ''` fallback is only ever used by a legacy DB whose
/// next `ensure_current` is the cleanup path.
#[test]
fn finding_carries_the_current_session_id() {
    let _session_guard = SESSION_TEST_LOCK
        .lock()
        .expect("session test lock poisoned");
    let project = activated_project("tagged.rs", b"pub fn t() {}\n");
    let response = finding(
        &Parameters(base_request(&project, "tagged.rs")),
        &crate::progress::NoopProgressSink,
    )
    .expect("finding succeeds");

    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let stamped: String = read_only_connection(&db_path)
        .query_row(
            "SELECT session_id FROM findings WHERE id = ?1",
            [response.0.id],
            |row| row.get(0),
        )
        .expect("read session_id");
    // Round-trip through `Uuid::parse_str` proves row shape is
    // canonical rather than incidental.
    let parsed = Uuid::parse_str(&stamped).expect("stamped session id parses as uuid");
    assert_eq!(
        parsed.to_string().len(),
        stamped.len(),
        "stamped session_id must round-trip through uuid::parse_str byte-for-byte"
    );
    assert_ne!(
        stamped, "",
        "migrated empty strings are never expected here"
    );
}

/// The cross-session poisoning defense's user-visible contract:
/// replacing the active session UUID (as a fresh process would) and
/// then triggering one more sync deletes every finding the prior
/// session wrote, while a fresh finding under the new session is
/// untouched. This is what stops a new AI agent from silently
/// inheriting a prior session's audit conclusions.
#[test]
fn a_new_session_drops_prior_session_findings_but_keeps_its_own() {
    let _session_guard = SESSION_TEST_LOCK
        .lock()
        .expect("session test lock poisoned");
    let project = activated_project("cross.rs", b"pub fn x() {}\n");

    let prior_session = Uuid::new_v4();
    override_session_id_for_test(prior_session);
    let prior_response = finding(
        &Parameters(base_request(&project, "cross.rs")),
        &crate::progress::NoopProgressSink,
    )
    .expect("first-session finding succeeds");

    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    assert_eq!(
        count_findings(&db_path),
        1,
        "after the first session, exactly one finding exists",
    );

    // Simulate a fresh process boot: a totally new UUID, then any
    // tool call that goes through `ensure_synced` triggers the purge.
    let next_session = Uuid::new_v4();
    assert_ne!(
        next_session, prior_session,
        "the two simulated sessions must be distinct",
    );
    override_session_id_for_test(next_session);
    // A full publish path naturally hits `ensure_project_row` and the
    // session-scoped cleanup; using `report()` here matches the
    // test pattern of "wipe via a no-side-effect read tool".
    let _ = crate::tools::report(
        &Parameters(crate::tools::ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
    )
    .expect("report under new session wipes and reports");

    assert_eq!(
        count_findings(&db_path),
        0,
        "the prior session's finding must be gone after a new-session sync",
    );

    let next_response = finding(
        &Parameters(base_request(&project, "cross.rs")),
        &crate::progress::NoopProgressSink,
    )
    .expect("second-session finding succeeds");

    assert_eq!(
        count_findings(&db_path),
        1,
        "the new session's own finding must remain",
    );
    let stamped_next: String = read_only_connection(&db_path)
        .query_row(
            "SELECT session_id FROM findings WHERE id = ?1",
            [next_response.0.id],
            |row| row.get(0),
        )
        .expect("read next session_id");
    assert_eq!(
        stamped_next,
        next_session.to_string(),
        "the surviving row must be tagged with the new session's id",
    );
    assert!(
        stamped_next != prior_session.to_string(),
        "the surviving row must NOT carry the prior session's id",
    );
    assert_ne!(
        prior_response.0.id, next_response.0.id,
        "the prior row was deleted; the next row is a fresh id"
    );
}

/// Walks the document-modification / re-sync path through the same
/// defense. The cleanup lives in `ensure_project_row`, which the
/// production hot path reaches via the manager's `ensure_current`;
/// using `ensure_synced_no_progress` (a `cfg(test)` shim around
/// `ensure_synced` for tests) takes the same code path the tool
/// handlers do, so a session change between two `ensure_synced`
/// calls must drop the first session's finding on the second call.
/// This pins that the modified-file / staleness-invalidating path
/// also runs the cleanup — not just a no-op read tool trigger.
#[test]
fn a_resync_after_a_session_change_drops_prior_session_findings() {
    let _session_guard = SESSION_TEST_LOCK
        .lock()
        .expect("session test lock poisoned");
    let project = activated_project("resync.rs", b"pub fn r() {}\n");

    let first_session = Uuid::new_v4();
    override_session_id_for_test(first_session);
    finding(
        &Parameters(base_request(&project, "resync.rs")),
        &crate::progress::NoopProgressSink,
    )
    .expect("first-session finding lands");

    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    assert_eq!(count_findings(&db_path), 1);

    fs::write(
        project.path().join("resync.rs"),
        b"pub fn r() { changed(); }\n",
    )
    .expect("modify file");

    let next_session = Uuid::new_v4();
    assert_ne!(next_session, first_session);
    override_session_id_for_test(next_session);

    // `ensure_synced_no_progress` is the production entry point
    // without the progress-sink argument, exercised here to drive the
    // manager's `ensure_current` → `ensure_project_row` →
    // `purge_prior_session_findings` chain under a fresh session.
    // A direct `sync::publish` would NOT exercise this path because
    // it operates on an already-cleaned connection that was opened
    // under a prior session.
    crate::tools::context::ensure_synced_no_progress(&project.path().to_string_lossy())
        .expect("resync under new session");

    assert_eq!(
        count_findings(&db_path),
        0,
        "ensure_synced during a new session must drop the prior session's findings"
    );
}
