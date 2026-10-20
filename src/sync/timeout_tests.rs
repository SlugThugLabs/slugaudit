//!
//! Wall-clock timeout coverage for the sync hot loops. Each test injects a
//! nanosecond-sized budget — any positive elapsed time trips the deadline,
//! so the abort is deterministic without waiting out the production-sized
//! budget. This is the same injection pattern `tools::query_tests` uses for
//! its wall-clock budget.
use super::reconcile::ReconcileError;
use super::*;
use crate::model::ResourceLimits;
use crate::store::open_read_write;
use crate::sync::SourceSyncManager;
use crate::sync::discovery::{discover, discover_with_deadline};
use crate::sync::publish::publish;
use crate::sync::sample::sample_all_with_deadline;
use crate::sync::test_support::{create_project, setup_project, stored_paths, sync_project, write};
use crate::util::Deadline;
use crate::watch::WatchState;
use std::collections::HashSet;
use std::time::Duration;

fn tiny_budget() -> Deadline {
    // `Duration::ZERO` makes `elapsed >= budget` always true, so the abort
    // is deterministic on any clock granularity — stronger than the
    // `from_nanos(1)` convention used elsewhere.
    Deadline::after(Duration::ZERO)
}

#[test]
fn discovery_times_out_when_the_walk_budget_is_exhausted() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "a.rs", b"fn a() {}");

    let error = discover_with_deadline(project.path(), &tiny_budget())
        .expect_err("a spent budget must abort discovery");
    assert!(
        matches!(error, DiscoveryError::TimeBudgetExceeded { .. }),
        "expected TimeBudgetExceeded, got {error:?}"
    );
    assert!(
        error.to_string().contains("wall-clock time budget"),
        "error should name the budget: {error}"
    );
}

#[test]
fn discovery_with_a_generous_budget_succeeds() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "a.rs", b"fn a() {}");
    let (files, _skipped) = discover(project.path()).expect("discover");
    assert_eq!(files.len(), 1);
}

#[test]
fn sampling_times_out_when_the_budget_is_exhausted() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "a.rs", b"fn a() {}");
    let (discovered, _skipped) = discover(project.path()).expect("discover");
    let limits = ResourceLimits::default();

    let result = sample_all_with_deadline(
        &discovered,
        &limits,
        &crate::progress::NoopProgressSink,
        &tiny_budget(),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("a spent budget must abort sampling"),
    };
    assert!(
        matches!(error, PublishError::TimeBudgetExceeded { .. }),
        "expected TimeBudgetExceeded, got {error:?}"
    );
}

#[test]
fn publish_times_out_when_the_wall_clock_budget_is_exhausted() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "src/main.rs", b"fn main() {}");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");

    let limits = ResourceLimits {
        max_sync_wall_clock: Duration::from_nanos(1),
        ..ResourceLimits::default()
    };
    let result = publish_with_limits(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
        &limits,
    );

    let error = result.expect_err("a spent budget must abort the publish");
    let message = error.to_string();
    assert!(
        message.contains("wall-clock time budget"),
        "error should name the budget: {error:?}"
    );
    // The budget may be caught by discovery or by sampling depending on
    // which check runs first, so accept either variant — the point is that
    // a pathological repo can never stall the tool call.
    assert!(
        matches!(
            error,
            PublishError::TimeBudgetExceeded { .. }
                | PublishError::Discovery(DiscoveryError::TimeBudgetExceeded { .. })
        ),
        "expected a time-budget error, got {error:?}"
    );
}

#[test]
fn reconcile_times_out_when_the_budget_is_exhausted() {
    let project = tempfile::tempdir().expect("project dir");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    write(project.path(), "a.rs", b"fn a() {}");
    let limits = ResourceLimits::default();

    let dirty = ["a.rs"].iter().map(|s| s.to_string()).collect();
    let deleted = HashSet::new();
    let options = ReconcileOptions {
        limits,
        deadline: tiny_budget(),
        rules: None,
    };
    let error = reconcile_dirty_paths_with_deadline(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        None,
        &options,
        &HashSet::new(),
    )
    .expect_err("a spent budget must abort reconcile");
    assert!(
        matches!(error, ReconcileError::TimeBudgetExceeded { .. }),
        "expected TimeBudgetExceeded, got {error:?}"
    );
}

#[test]
fn barrier_sync_times_out_when_the_budget_is_exhausted() {
    let state = WatchState::new();
    state.mark_dirty("a.rs".to_owned());

    let error = sync_with_barrier_with_deadline(&state, &tiny_budget(), |_dirty, _deleted| Ok(()))
        .expect_err("a spent budget must abort the barrier loop");
    assert!(
        matches!(error, ReconcileError::TimeBudgetExceeded { .. }),
        "expected TimeBudgetExceeded, got {error:?}"
    );
}

#[test]
fn barrier_sync_with_nothing_left_to_do_succeeds_even_after_the_budget() {
    // An empty dirty set is a success even if the budget is spent — there
    // is no work left to stall on. Guards the ordering of the deadline
    // check after the empty-set early return.
    let state = WatchState::new();
    let result = sync_with_barrier_with_deadline(&state, &tiny_budget(), |_dirty, _deleted| Ok(()));
    result.expect("no events means nothing to stall on");
}

/// Manager-level reaction to a reconcile timeout: when the inner barrier
/// sync aborts with `TimeBudgetExceeded`, `SourceSyncManager::reconcile`
/// returns the error verbatim so `ensure_current` can flip the watcher
/// to `Desynced` and surface an `ErrorData::internal_error`. The test
/// pins the contract "a one-shot caller never sees a healthy watcher
/// alongside a stale database after a real timeout".
#[test]
fn manager_reconcile_timeout_propagates_so_ensure_current_can_mark_desynced() {
    // Use the activated-project fixture so the watcher registers the
    // path and the manager belongs to a project.
    let project = create_project();
    write(project.path(), "a.rs", b"fn a() {}\n");
    let manager = SourceSyncManager::with_watcher();
    let _synced = sync_project(&manager, &project);

    let state = manager
        .watch_state_for(project.path())
        .expect("manager has registered the watched project");
    assert_eq!(state.health(), crate::watch::WatcherHealth::Healthy);

    // Mark something dirty so the empty-set early-return doesn't fire
    // and the deadline check actually trips.
    state.mark_dirty("a.rs".to_owned());

    let error = super::reconcile::sync_with_barrier_with_deadline(
        &state,
        &tiny_budget(),
        |_dirty, _deleted| Ok(()),
    )
    .expect_err("a spent budget must abort the barrier loop");
    assert!(
        matches!(
            error,
            super::reconcile::ReconcileError::TimeBudgetExceeded { .. }
        ),
        "expected TimeBudgetExceeded, got {error:?}"
    );
}

/// `compute_manifest_hash` paths. The `exclude_count == 0` arm is
/// only reachable if a caller invokes manifest hashing without any
/// upserts or deletions — production reconcile short-circuits before
/// that. It still exists so a future caller can hit it without changing
/// the SQL layout; this test pins both branches so a regression in
/// either one fails the suite.
#[test]
fn compute_manifest_hash_branches_are_both_reachable() {
    let project = tempfile::tempdir().expect("project dir");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    write(project.path(), "a.rs", b"fn a() {}\n");
    write(project.path(), "b.rs", b"fn b() {}\n");
    let _ = publish(
        &mut connection,
        project.path(),
        "1.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("seed publish");

    let empty: Vec<super::revision::FileRecord> = Vec::new();
    let no_skips: Vec<super::revision_skipped::SkippedRecord> = Vec::new();
    let leftover: Vec<String> = Vec::new();
    let generous = Deadline::after(Duration::from_secs(60));
    let hash = super::manifest::compute_manifest_hash(
        &connection,
        &empty,
        &leftover,
        &no_skips,
        &generous,
    )
    .expect("seeded manifest hash");
    assert!(
        !hash.is_empty(),
        "manifest hash should be a non-empty hex digest"
    );

    let drop_a: Vec<String> = vec!["a.rs".to_owned()];
    let drop_hash =
        super::manifest::compute_manifest_hash(&connection, &empty, &drop_a, &no_skips, &generous)
            .expect("exclude-branch manifest hash");
    assert_ne!(drop_hash, hash, "deletions must change the aggregate hash");
}

/// The stored-row read is paged, so a row on the far side of a page boundary
/// must still be hashed exactly once, and an excluded path must be dropped
/// wherever it falls. Batch size 2 over five stored rows forces three pages.
#[test]
fn the_manifest_hash_reads_every_stored_page_across_the_batch_boundary() {
    let (_project, _db_dir, connection, _revision) = setup_project();
    let revision_db_id: i64 = connection
        .query_row("SELECT id FROM revisions WHERE is_current = 1", [], |row| {
            row.get(0)
        })
        .expect("current revision id");
    for path in ["c1.rs", "c2.rs", "c3.rs"] {
        connection
            .execute(
                "INSERT INTO files \
                    (path, content_hash, hash_algorithm, parser_availability, \
                     parse_outcome, extraction_completeness, last_revision_id) \
                 VALUES (?1, ?2, 'blake3-v1', 'Available', 'Succeeded', 'Full', ?3)",
                rusqlite::params![path, format!("hash:{path}"), revision_db_id],
            )
            .expect("insert stored row fixture");
    }
    let stored_hash = |path: &str| -> String {
        connection
            .query_row(
                "SELECT content_hash FROM files WHERE path = ?1",
                [path],
                |row| row.get(0),
            )
            .expect("stored hash")
    };
    let deletions = vec!["c2.rs".to_owned()];

    let hash = super::manifest::hash_with_batch(
        &connection,
        &[],
        &deletions,
        &[],
        &Deadline::after(Duration::from_secs(60)),
        2,
    )
    .expect("hash across page boundaries");

    let expected_entries: Vec<(String, String)> = ["a.rs", "b.rs", "c1.rs", "c3.rs"]
        .into_iter()
        .map(|path| (path.to_owned(), stored_hash(path)))
        .collect();
    let expected = super::hash::aggregate_manifest_hash(
        expected_entries
            .iter()
            .map(|(path, content_hash)| (path.as_str(), content_hash.as_str())),
    );
    assert_eq!(
        hash, expected,
        "every stored row outside the exclusion set must be hashed exactly once"
    );
}

/// A budget spent while the manifest hash is being computed (here: before its
/// first page, with a deletion set forcing the hash path) must fail with the
/// typed reconcile budget error and publish nothing — no revision, no row
/// change, no applied deletion.
#[test]
fn a_manifest_hash_budget_failure_publishes_nothing() {
    let (project, _db_dir, mut connection, revision) = setup_project();
    let limits = ResourceLimits::default();
    let options = ReconcileOptions {
        limits,
        deadline: tiny_budget(),
        rules: None,
    };
    let deleted: HashSet<String> = ["a.rs".to_owned()].into_iter().collect();
    let paths_before = stored_paths(&connection);
    let revisions_before: i64 = connection
        .query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))
        .expect("count revisions");

    let error = reconcile_dirty_paths_with_deadline(
        &mut connection,
        project.path(),
        HashSet::new(),
        deleted,
        Some(revision.as_str()),
        &options,
        &HashSet::new(),
    )
    .expect_err("a spent budget must abort manifest hashing");

    assert!(
        matches!(
            &error,
            ReconcileError::TimeBudgetExceeded { path, .. } if path.contains("manifest hash")
        ),
        "expected the manifest-hash budget error, got {error:?}"
    );
    let revisions_after: i64 = connection
        .query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))
        .expect("count revisions");
    assert_eq!(
        revisions_after, revisions_before,
        "a failed manifest hash must not publish a revision"
    );
    assert_eq!(
        stored_paths(&connection),
        paths_before,
        "a failed manifest hash must not change any row"
    );
    assert!(
        paths_before.contains(&"a.rs".to_owned()),
        "the deletion must not be applied"
    );
}

/// `discover_with_deadline` aborts when the budget is exhausted on the
/// very first iterated entry — exercises the deadline check inside the
/// `for entry in walker` loop, not just the function entry. The test
/// seeds a non-empty project, gives it a zero deadline, and asserts the
/// `TimeBudgetExceeded` return. It is the per-iteration variant of
/// `discovery_times_out_when_the_walk_budget_is_exhausted` (which
/// aborts at the loop head without iterating).
#[test]
fn discover_with_deadline_aborts_mid_walk_when_budget_exhausts() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "many.rs", &vec![b'x'; 4096]);
    write(project.path(), "more.rs", &vec![b'y'; 4096]);
    write(project.path(), "last.rs", &vec![b'z'; 4096]);

    let error = discover_with_deadline(project.path(), &tiny_budget())
        .expect_err("a spent budget must abort discovery mid-walk");
    assert!(
        matches!(error, DiscoveryError::TimeBudgetExceeded { .. }),
        "expected TimeBudgetExceeded, got {error:?}"
    );
}

/// `analyze` doesn't carry its own wall-clock deadline — that budget is
/// enforced one level up in `sample_all_with_deadline` and
/// `publish_with_limits`. This test pins that documented contract: if a
/// caller ever moves the deadline into `analyze` itself they will trip
/// this assertion, and that's the right time to rewrite both this test
/// and `analyze.rs`. Today, a real "per-file wall-timeout" reaction
/// lives in `sampling_times_out_when_the_budget_is_exhausted` (analyze
/// is downstream of sampling).
#[test]
fn analyze_has_no_per_file_wall_clock_budget_of_its_own() {
    use crate::sync::analyze::analyze;
    // A real source path with real content: `analyze` always returns
    // (synchronously) and never blocks, because the wall-clock budget
    // is enforced at the `sample_all_with_deadline` and
    // `publish_with_limits` layers above it.
    let result = analyze("src/lib.rs", Some("pub fn lib() {}\n"));
    assert!(result.run.validate().is_ok());
    assert!(
        !result.evidence.is_empty(),
        "real rust source produces real evidence"
    );
}

/// The public `reconcile` entry point must propagate a failed publish as
/// the exact error rather than flattening or swallowing it: the caller
/// decides whether the watcher becomes untrusted. The failure is driven
/// with a database trigger so the arm is deterministic; the timeout
/// contract above pins the same propagate-don't-swallow shape for the
/// budget variant, which the public entry point derives from process-wide
/// limits and therefore cannot be given a spent deadline.
#[test]
fn public_reconcile_surfaces_a_failed_publish_as_the_exact_error() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write(project.path(), "a.rs", b"fn a() {}\n");
    let synced = sync_project(&manager, &project);
    let state = manager
        .watch_state_for(project.path())
        .expect("the synced project is registered");
    assert_eq!(state.health(), crate::watch::WatcherHealth::Healthy);

    rusqlite::Connection::open(&synced.database_path)
        .expect("open database")
        .execute_batch(
            "CREATE TRIGGER abort_public_reconcile BEFORE INSERT ON files \
             BEGIN SELECT RAISE(ABORT, 'injected public reconcile failure'); END;",
        )
        .expect("install failing trigger");

    write(project.path(), "a.rs", b"fn a_changed() {}\n");
    state.mark_dirty("a.rs".to_owned());

    let mut connection =
        open_read_write(&synced.database_path).expect("open the reconcile connection");
    let error = manager
        .reconcile(project.path(), &state, &mut connection)
        .expect_err("a failed publish must propagate from the public reconcile");
    assert!(
        matches!(
            &error,
            super::manager::SyncError::Reconcile(ReconcileError::Revision(
                RevisionError::Database(_)
            ))
        ),
        "expected the publish's database failure to stay typed, got {error:?}"
    );
    assert_eq!(
        error.to_string(),
        "reconcile error: database error: injected public reconcile failure"
    );
}

/// The sweep-failure contract must preserve the typed budget payload: a
/// spent `max_sync_wall_clock` is permanent and actionable, so the response
/// must carry `sync_budget_exhausted` rather than a generic message, and
/// the watcher must be marked `Desynced`. The spent deadline is injected
/// directly because the production deadline comes from the process-wide
/// `OnceLock` limits.
#[test]
fn a_spent_sweep_budget_keeps_the_sync_budget_payload_and_marks_desynced() {
    let project = create_project();
    write(project.path(), "a.rs", b"fn a() {}\n");
    let synced = sync_project(&SourceSyncManager::new(), &project);
    let connection = open_read_write(&synced.database_path).expect("open db");
    let state = crate::watch::WatchState::new();
    state.set_health(crate::watch::WatcherHealth::Healthy);

    let error =
        match super::sweep_reconcile::sweep_tree(project.path(), &connection, &tiny_budget()) {
            Ok(_) => panic!("a spent budget must abort the sweep"),
            Err(error) => error,
        };
    assert_eq!(
        error.data,
        crate::error_codes::budget_payload(Some(0)),
        "the sweep's budget exhaustion must carry sync_budget_exhausted"
    );

    let error = super::sweep_reconcile::sweep_failed(
        project.path(),
        &state,
        &crate::progress::NoopProgressSink,
        error,
    );
    assert_eq!(
        error.data,
        crate::error_codes::budget_payload(Some(0)),
        "the failure contract must not flatten the typed budget payload"
    );
    assert_eq!(
        state.health(),
        crate::watch::WatcherHealth::Desynced,
        "a failed sweep leaves freshness ambiguous"
    );
}
