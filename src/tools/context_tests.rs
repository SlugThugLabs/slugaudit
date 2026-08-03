use super::*;
use std::fs;

fn activated_project(relative: &str, content: &[u8]) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    fs::write(project.path().join(relative), content).expect("write fixture file");
    project
}

/// Exercises the real production entry point tools actually call, not a
/// weaker stand-in: `with_verified_read` keeps the transaction open for the
/// whole closure, so a real query issued from inside `f` proves verification
/// and the read share one atomic snapshot end to end.
#[test]
fn a_verified_connection_succeeds_when_nothing_changed_since_sync() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let synced = ensure_synced(&project.path().to_string_lossy()).expect("sync");

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
    let stale = ensure_synced(&path).expect("first sync");

    // Simulate a concurrent publish from another process: modify the file
    // and sync again independently of `stale`.
    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify file");
    let fresh = ensure_synced(&path).expect("second sync");
    assert_ne!(
        stale.revision_id, fresh.revision_id,
        "the revision must actually have moved"
    );

    let ran_closure = std::cell::Cell::new(false);
    let result = with_verified_read(&stale, |_tx| {
        ran_closure.set(true);
        Ok(())
    });
    assert!(
        result.is_err(),
        "a stale revision handle must never silently open against newer data"
    );
    assert!(
        !ran_closure.get(),
        "the closure must never run against a revision that failed verification"
    );
}

#[test]
fn a_database_copied_from_a_different_project_root_fails_closed() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let path = project.path().to_string_lossy().into_owned();
    ensure_synced(&path).expect("first sync establishes the project row");

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

    let result = ensure_synced(&path);
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
    let stale = ensure_synced(&path).expect("first sync");

    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify file");
    let fresh = ensure_synced(&path).expect("second sync");

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
    let synced = ensure_synced(&project.path().to_string_lossy()).expect("sync");

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
