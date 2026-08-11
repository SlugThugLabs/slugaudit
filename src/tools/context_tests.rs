use super::*;
use crate::tools::test_support::activated_project;
use std::fs;

#[test]
fn a_corrupt_derived_database_is_discarded_and_rebuilt_from_project_files() {
    let project = activated_project("lib.rs", b"pub fn rebuilt() {}\n");
    let path = project.path().to_string_lossy().into_owned();
    ensure_synced_no_progress(&path).expect("initial sync");

    let database = project.path().join(".planning/slugaudit/project.db");
    fs::write(&database, b"corrupt sqlite bytes").expect("corrupt derived database");
    fs::write(
        database.with_file_name("project.db-wal"),
        b"stale wal sidecar",
    )
    .expect("write stale wal sidecar");

    let rebuilt = ensure_synced_no_progress(&path).expect("corrupt cache is rebuilt");
    assert!(!rebuilt.revision_id.is_empty());
    assert!(rebuilt.database_path.exists(), "rebuilt database exists");
    let connection = crate::store::open_read_only(&rebuilt.database_path).expect("rebuilt db");
    let count: i64 = connection
        .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
        .expect("rebuilt file row");
    assert_eq!(count, 1);
}

/// Deterministic sync for the freshness tests: a local manager with NO
/// filesystem watcher, so every `ensure_current` runs a full publish and a
/// content change always moves the revision. The global watcher-backed
/// manager (`ensure_synced_no_progress`) delivers modify events on an async
/// watcher thread — under parallel test load the second sync in these
/// tests could return the same revision if the event hadn't landed yet,
/// turning a deterministic "stale handle must be rejected" assertion into a
/// flaky one.
fn synced_locally(path: &str) -> SyncedProject {
    crate::sync::SourceSyncManager::new()
        .ensure_current(path, &crate::progress::NoopProgressSink)
        .expect("local sync")
}

/// Exercises the real production entry point tools actually call, not a
/// weaker stand-in: `with_verified_read` keeps the transaction open for the
/// whole closure, so a real query issued from inside `f` proves verification
/// and the read share one atomic snapshot end to end.
#[test]
fn a_verified_connection_succeeds_when_nothing_changed_since_sync() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let synced = ensure_synced_no_progress(&project.path().to_string_lossy()).expect("sync");

    let count: i64 = with_verified_read(&synced, |tx| {
        tx.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    })
    .expect("still-fresh revision opens fine and the closure's query runs");
    assert_eq!(count, 1_i64);
}

#[test]
fn a_stale_synced_handle_fails_loudly_instead_of_returning_mismatched_data() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let path = project.path().to_string_lossy().into_owned();
    let stale = synced_locally(&path);

    // Simulate a concurrent publish from another process: modify the file
    // and sync again independently of `stale`.
    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify file");
    let fresh = synced_locally(&path);
    assert_ne!(
        stale.revision_id, fresh.revision_id,
        "the revision must actually have moved"
    );

    let ran_closure = std::cell::Cell::new(false);
    let result = with_verified_read(&stale, |_tx| {
        ran_closure.set(true);
        Ok(())
    });
    let error = result.expect_err("a stale revision handle must never open against newer data");
    assert!(
        !ran_closure.get(),
        "the closure must never run against a revision that failed verification"
    );
    // The message must actually be actionable, not a bare rusqlite/io
    // Display string with no guidance — this is the specific caller-facing
    // gap `internal_error` exists to close.
    let message = error.message.to_string();
    assert!(
        message.contains("retry the call"),
        "expected a retry hint in the error message, got: {message}"
    );
}

#[test]
fn a_database_copied_from_a_different_project_root_fails_closed() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let path = project.path().to_string_lossy().into_owned();
    ensure_synced_no_progress(&path).expect("first sync establishes the project row");

    // Simulate a database file copied in from a different project: rewrite
    // the stored root_path directly through a raw connection, bypassing
    // ensure_synced entirely.
    let database_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let raw = rusqlite::Connection::open(&database_path).expect("open raw connection");
    raw.execute(
        "UPDATE project SET root_path = ?1 WHERE id = 1",
        rusqlite::params!["/some/other/project/root"],
    )
    .expect("simulate a copied database from another project");
    drop(raw);

    let result = ensure_synced_no_progress(&path);
    assert!(
        result.is_err(),
        "a database whose stored root_path doesn't match this project's canonical root \
         must never be silently accepted"
    );
}

/// Same protection as the read side, proven end to end: `with_verified_write`
/// must refuse to run its closure (a real INSERT) against a stale handle, and
/// the row must never land.
#[test]
fn verified_read_write_has_the_same_protection() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let path = project.path().to_string_lossy().into_owned();
    let stale = synced_locally(&path);

    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify file");
    let fresh = synced_locally(&path);

    let result = with_verified_write(&stale, |tx| {
        tx.execute(
            "INSERT INTO findings (\
                path, source_hash, line_start, line_end, severity, category, title, \
                description, created_at_unix, evidence_revision, status\
             ) VALUES ('lib.rs', 'deadbeef', 1, 1, 'low', 'test', 'x', 'y', 0, 'r', 'current')",
            [],
        )
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    });
    assert!(result.is_err(), "a stale write handle must be rejected");

    // Confirm the rejected write really never landed, through a fresh,
    // correctly-revisioned handle rather than trusting the error alone.
    let count: i64 = with_verified_read(&fresh, |tx| {
        tx.query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    })
    .expect("fresh handle reads fine");
    assert_eq!(
        count, 0_i64,
        "the closure's INSERT must never have executed once verification failed"
    );
}

/// Proves `with_verified_write`'s closure genuinely executes and commits a
/// real write when the revision is current, so the failure-path assertions
/// above are meaningful contrasts rather than a tautology.
#[test]
fn verified_write_actually_commits_a_real_change_on_a_fresh_revision() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let synced = ensure_synced_no_progress(&project.path().to_string_lossy()).expect("sync");

    with_verified_write(&synced, |tx| {
        tx.execute(
            "INSERT INTO findings (\
                path, source_hash, line_start, line_end, severity, category, title, \
                description, created_at_unix, evidence_revision, status\
             ) VALUES ('lib.rs', 'deadbeef', 1, 1, 'low', 'test', 'x', 'y', 0, 'r', 'current')",
            [],
        )
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    })
    .expect("write against a current revision succeeds");

    let count: i64 = with_verified_read(&synced, |tx| {
        tx.query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    })
    .expect("read back the committed row");
    assert_eq!(count, 1_i64);
}

/// Two syncs against an unchanged project converge on the same revision —
/// the publish is deterministic.
#[test]
fn two_syncs_against_unchanged_project_converge_on_same_revision() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let path = project.path().to_string_lossy().into_owned();

    let first = ensure_synced_no_progress(&path).expect("first sync");
    let second = ensure_synced_no_progress(&path).expect("second sync");

    assert_eq!(
        first.revision_id, second.revision_id,
        "two full publishes against an unchanged project must converge on the same revision"
    );
}
