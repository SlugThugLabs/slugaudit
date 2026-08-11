//! Tests for `reconcile`'s dirty-path reconciliation and barrier loop.
// slugaudit-line-exception: approved-by=agent; reason=one test per reconcile outcome (unchanged/modified/new/deleted/mixed/race/cap) sharing setup_project from sync/test_support and `use super::*` access to MAX_BARRIER_LOOPS + ReconcileError; splitting would fragment the scenario set from the constants it asserts against
use super::*;
use crate::sync::test_support::{setup_project, write};
use crate::watch::WatchState;
use std::fs;

#[test]
fn unchanged_dirty_paths_are_skipped() {
    let (project, _db_dir, mut connection, revision) = setup_project();

    // Both files are "dirty" but their content hasn't changed.
    let dirty = ["a.rs", "b.rs"].iter().map(|s| s.to_string()).collect();
    let deleted = HashSet::new();

    let report = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some(&revision),
    )
    .expect("reconcile");

    assert_eq!(report.reconciled, 0, "no files should have been re-indexed");
    assert_eq!(report.unchanged, 2, "both files should have been skipped");
    assert_eq!(report.deleted, 0);
}

#[test]
fn modified_dirty_paths_are_reindexed() {
    let (project, _db_dir, mut connection, revision) = setup_project();

    // Modify a.rs on disk so its hash no longer matches the stored one.
    write(project.path(), "a.rs", b"fn a_modified() {}");

    let dirty = ["a.rs"].iter().map(|s| s.to_string()).collect();
    let deleted = HashSet::new();

    let report = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some(&revision),
    )
    .expect("reconcile");

    assert_eq!(
        report.reconciled, 1,
        "the modified file should be re-indexed"
    );
    assert_eq!(report.unchanged, 0);
    assert_eq!(report.deleted, 0);
}

#[test]
fn new_dirty_paths_are_indexed() {
    let (project, _db_dir, mut connection, revision) = setup_project();

    // Add a new file that isn't in the database yet.
    write(project.path(), "c.rs", b"fn c() {}");

    let dirty = ["c.rs"].iter().map(|s| s.to_string()).collect();
    let deleted = HashSet::new();

    let report = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some(&revision),
    )
    .expect("reconcile");

    assert_eq!(report.reconciled, 1, "the new file should be indexed");
    assert_eq!(report.unchanged, 0);
    assert_eq!(report.deleted, 0);

    // Verify the file is now in the database.
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM files WHERE path = 'c.rs'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(count, 1, "the new file should be stored in the database");
}

#[test]
fn deleted_paths_are_removed() {
    let (project, _db_dir, mut connection, revision) = setup_project();

    let dirty = HashSet::new();
    let deleted = ["a.rs"].iter().map(|s| s.to_string()).collect();

    let report = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some(&revision),
    )
    .expect("reconcile");

    assert_eq!(report.reconciled, 0);
    assert_eq!(report.unchanged, 0);
    assert_eq!(report.deleted, 1, "one file should be deleted");

    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM files WHERE path = 'a.rs'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(
        count, 0,
        "the deleted file should be gone from the database"
    );
}

#[test]
fn mixed_dirty_and_deleted_paths_are_handled() {
    let (project, _db_dir, mut connection, revision) = setup_project();

    // Modify a.rs and delete b.rs.
    write(project.path(), "a.rs", b"fn a_modified() {}");

    let dirty = ["a.rs"].iter().map(|s| s.to_string()).collect();
    let deleted = ["b.rs"].iter().map(|s| s.to_string()).collect();

    let report = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some(&revision),
    )
    .expect("reconcile");

    assert_eq!(report.reconciled, 1, "a.rs should be re-indexed");
    assert_eq!(report.unchanged, 0);
    assert_eq!(report.deleted, 1, "b.rs should be deleted");
}

#[test]
fn missing_dirty_file_is_treated_as_deleted() {
    let (project, _db_dir, mut connection, revision) = setup_project();

    // Delete a.rs from disk, but mark it as dirty (simulating a race).
    fs::remove_file(project.path().join("a.rs")).expect("remove file");

    let dirty = ["a.rs"].iter().map(|s| s.to_string()).collect();
    let deleted = HashSet::new();

    let report = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some(&revision),
    )
    .expect("reconcile");

    assert_eq!(report.reconciled, 0);
    assert_eq!(report.unchanged, 0);
    assert_eq!(
        report.deleted, 1,
        "a missing dirty file should be treated as a deletion"
    );
}

#[test]
fn no_changes_produces_no_revision() {
    let (project, _db_dir, mut connection, revision) = setup_project();

    // Both files are dirty but unchanged.
    let dirty = ["a.rs", "b.rs"].iter().map(|s| s.to_string()).collect();
    let deleted = HashSet::new();

    let report = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some(&revision),
    )
    .expect("reconcile");

    assert_eq!(report.reconciled, 0);
    assert_eq!(report.unchanged, 2);
    assert_eq!(report.deleted, 0);
}

#[test]
fn sync_with_barrier_stops_when_no_events() {
    let state = WatchState::new();
    let mut call_count = 0;

    let result = sync_with_barrier(&state, |_dirty, _deleted| {
        call_count += 1;
        Ok(())
    });

    result.expect("barrier sync should succeed");
    assert_eq!(
        call_count, 0,
        "reconcile_fn should not be called when there are no events"
    );
}

#[test]
fn sync_with_barrier_calls_reconcile_for_dirty_events() {
    let state = WatchState::new();
    state.mark_dirty("src/lib.rs".to_owned());

    let mut call_count = 0;
    let mut received_paths: Option<HashSet<String>> = None;

    let result = sync_with_barrier(&state, |dirty, _deleted| {
        call_count += 1;
        received_paths = Some(dirty);
        Ok(())
    });

    result.expect("barrier sync should succeed");
    assert_eq!(call_count, 1, "reconcile_fn should be called exactly once");
    assert!(
        received_paths.as_ref().unwrap().contains("src/lib.rs"),
        "reconcile_fn should receive the dirty path"
    );
}

#[test]
fn sync_with_barrier_loops_when_new_events_arrive() {
    let state = WatchState::new();
    state.mark_dirty("a.rs".to_owned());

    let mut call_count = 0;

    let result = sync_with_barrier(&state, |_dirty, _deleted| {
        call_count += 1;
        // Simulate a new event arriving during reconciliation.
        if call_count == 1 {
            state.mark_dirty("b.rs".to_owned());
        }
        Ok(())
    });

    result.expect("barrier sync should succeed");
    assert_eq!(
        call_count, 2,
        "reconcile_fn should be called again when new events arrive during reconciliation"
    );
}

/// A producer racing reconciliation (e.g. an editor saving on every
/// keystroke or a fsmonitor firing hundreds of events per second) must
/// not loop forever. The barrier-sync cap kicks in,
/// `BarrierCapExceeded` is returned with the iteration count, and the
/// watcher is marked `Desynced` so the next call falls back to a full
/// verification.
#[test]
fn sync_with_barrier_cap_protects_against_runaway_event_producers() {
    let state = WatchState::new();
    // Seed at least one dirty path so the loop enters and `reconcile_fn`
    // gets called. We keep inserting new events on every invocation, so
    // the watcher sequence never stabilizes and the loop would otherwise
    // spin forever.
    state.mark_dirty("a.rs".to_owned());

    let mut call_count = 0_u32;
    let result = sync_with_barrier(&state, |_dirty, _deleted| {
        call_count += 1;
        // Always emit a brand-new dirty path so `current_sequence` keeps
        // advancing past the snapshot's `seq` — the loop never reaches
        // its early-bail condition.
        state.mark_dirty(format!("racing-{}.rs", call_count));
        Ok(())
    });

    let error = result.expect_err("the loop must hit the cap");
    match error {
        ReconcileError::BarrierCapExceeded { iterations } => {
            assert_eq!(iterations, crate::sync::reconcile::MAX_BARRIER_LOOPS);
        }
        other => panic!("expected BarrierCapExceeded, got {other:?}"),
    }

    // We should have stopped exactly at the cap, not overdrawn it.
    assert_eq!(
        call_count,
        crate::sync::reconcile::MAX_BARRIER_LOOPS,
        "reconcile_fn must be called exactly MAX_BARRIER_LOOPS times",
    );

    // And the watcher must be Desynced so the next call falls back to a
    // full verification instead of trying to drain the racing producer.
    assert_eq!(state.health(), crate::watch::WatcherHealth::Desynced);
}
