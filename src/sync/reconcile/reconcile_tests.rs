//! Tests for `reconcile`'s dirty-path reconciliation and barrier loop.
use super::*;
use crate::model::ResourceLimits;
use crate::sync::test_support::{setup_project, write};
use crate::util::Deadline;
use crate::watch::WatchState;
use std::collections::HashSet;
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
        received_paths
            .as_ref()
            .expect("paths received")
            .contains("src/lib.rs"),
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
/// The reconcile publish is CAS-guarded: when the expected baseline doesn't
/// match the stored current revision, the whole reconcile must fail closed
/// with a revision (StaleBaseline) error rather than clobber a newer
/// revision with older disk state — the reconcile-path mirror of the
/// publish-side race guarantee.
#[test]
fn reconcile_with_a_stale_expected_revision_fails_closed() {
    let (project, _db_dir, mut connection, _revision) = setup_project();

    // The dirty path genuinely changed on disk, but the expected baseline
    // is stale — the commit must be rejected, not forced through.
    write(project.path(), "a.rs", b"fn a_changed() {}");
    let dirty = ["a.rs"].iter().map(|s| s.to_string()).collect();
    let deleted = HashSet::new();

    let error = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some("rev-stale"),
    )
    .expect_err("a mismatched expected revision must be rejected");

    assert!(
        matches!(error, ReconcileError::Revision(_)),
        "expected a revision (CAS) error, got {error:?}"
    );
}

/// A dirty path over the per-file import ceiling must be skipped with a
/// reported count — never fatal to the rest of the reconcile — while other
/// dirty paths are still re-indexed, mirroring the full-publish path's
/// per-file skip behavior.
#[test]
fn oversized_dirty_paths_are_skipped_while_other_files_reconcile() {
    let (project, _db_dir, mut connection, revision) = setup_project();
    write(project.path(), "big.rs", &vec![b'x'; 100_000]);
    write(project.path(), "small.rs", b"fn small() {}");

    let dirty = ["big.rs", "small.rs"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let deleted = HashSet::new();
    let options = ReconcileOptions {
        limits: ResourceLimits {
            max_file_bytes: 50_000,
            ..ResourceLimits::default()
        },
        deadline: Deadline::after(std::time::Duration::from_secs(60)),
        rules: None,
    };

    let report = reconcile_dirty_paths_with_deadline(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some(&revision),
        &options,
        &HashSet::new(),
    )
    .expect("an oversized dirty path must be skipped, not fatal");

    assert_eq!(report.reconciled, 1, "small.rs is still re-indexed");
    assert_eq!(report.unchanged, 0);
    assert_eq!(report.deleted, 0);
    assert_eq!(report.skipped, 1, "big.rs is reported as skipped");
}

/// The watcher-driven and sweep-driven reconcile both funnel through this
/// pipeline, so they must resolve imports against the discovered on-disk
/// set exactly like a full publish. Without it, editing an importer whose
/// target was discovered but skipped (oversize) rewrites the edge as
/// `Unresolved` — an auditing agent reads that as dead code.
#[test]
fn reconcile_resolves_imports_against_the_discovered_set_not_indexed_rows_alone() {
    let (project, _db_dir, mut connection, revision) = setup_project();
    write(project.path(), "pkg/__init__.py", b"");
    write(project.path(), "pkg/main.py", b"from .helper import f\n");
    // Larger than `max_file_bytes` below, so this path is discovered but
    // never gets a `files` row.
    write(project.path(), "pkg/helper.py", &[b'#'; 200]);

    let dirty = ["pkg/main.py", "pkg/helper.py"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let deleted = HashSet::new();
    let options = ReconcileOptions {
        limits: ResourceLimits {
            max_file_bytes: 50,
            ..ResourceLimits::default()
        },
        deadline: Deadline::after(std::time::Duration::from_secs(60)),
        rules: None,
    };
    let discovered: HashSet<String> = ["pkg/__init__.py", "pkg/main.py", "pkg/helper.py"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let report = reconcile_dirty_paths_with_deadline(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some(&revision),
        &options,
        &discovered,
    )
    .expect("reconcile");

    assert_eq!(report.skipped, 1, "helper.py is skipped as oversize");
    let kind: String = connection
        .query_row(
            "SELECT e.resolution_kind FROM dependency_edges e \
             JOIN files f ON f.id = e.from_file_id WHERE f.path = 'pkg/main.py'",
            [],
            |row| row.get(0),
        )
        .expect("main.py's edge");
    assert_eq!(
        kind, "Resolved",
        "a reconcile must not turn an export of a discovered-but-skipped file into Unresolved"
    );
}

/// `mkdir` inside a watched project raises `Create(Folder)`, which the
/// watcher marks dirty like any file. A directory has no content to index,
/// so it must not fail the reconcile that carries the real edits with it.
#[test]
fn a_directory_in_the_dirty_set_does_not_fail_the_reconcile() {
    let (project, _db_dir, mut connection, revision) = setup_project();
    fs::create_dir(project.path().join("newdir")).expect("create dir");
    write(project.path(), "a.rs", b"fn a_after_mkdir() {}");
    let dirty = ["newdir", "a.rs"].iter().map(|s| s.to_string()).collect();

    let report = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        HashSet::new(),
        Some(&revision),
    )
    .expect("a dirty directory must not fail the reconcile");

    assert_eq!(report.reconciled, 1, "the edited file is still re-indexed");
}

/// Discovery never follows symlinks, so the incremental path must not
/// either: a symlink the watcher reports would otherwise index its target —
/// possibly a file outside the project — that a full publish skips.
#[cfg(unix)]
#[test]
fn a_symlink_in_the_dirty_set_is_never_dereferenced() {
    let (project, _db_dir, mut connection, revision) = setup_project();
    let outside = tempfile::tempdir().expect("outside dir");
    fs::write(outside.path().join("secret.rs"), b"fn outside() {}").expect("write target");
    std::os::unix::fs::symlink(
        outside.path().join("secret.rs"),
        project.path().join("link.rs"),
    )
    .expect("create symlink");
    let dirty = ["link.rs"].iter().map(|s| s.to_string()).collect();

    let report = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        HashSet::new(),
        Some(&revision),
    )
    .expect("reconcile");

    assert_eq!(report.reconciled, 0, "a symlink target must not be indexed");
    let indexed: i64 = connection
        .query_row(
            "SELECT count(*) FROM files WHERE path = 'link.rs'",
            [],
            |row| row.get(0),
        )
        .expect("count link rows");
    assert_eq!(indexed, 0, "no row may exist for the symlink path");
}

/// A path that was an indexed file and is now a directory no longer holds
/// indexable content, so its row must go — exactly as if it were deleted.
#[test]
fn a_file_replaced_by_a_directory_loses_its_row() {
    let (project, _db_dir, mut connection, revision) = setup_project();
    fs::remove_file(project.path().join("a.rs")).expect("remove file");
    fs::create_dir(project.path().join("a.rs")).expect("create dir in its place");
    let dirty = ["a.rs"].iter().map(|s| s.to_string()).collect();

    let report = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        HashSet::new(),
        Some(&revision),
    )
    .expect("reconcile");

    assert_eq!(report.deleted, 1, "the stale file row must be removed");
}
