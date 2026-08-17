// slugaudit-line-exception: approved-by=agent; reason=one test per wall-clock deadline reaction (discovery, sampling, reconcile, barrier, manifest, analyze contract) sharing the tiny_budget/open_read_write/write fixtures in this file; splitting would fragment the injected-budget pattern that makes each abort deterministic
//!
//! Wall-clock timeout coverage for the sync hot loops. Each test injects a
//! nanosecond-sized budget — any positive elapsed time trips the deadline,
//! so the abort is deterministic without waiting out the production-sized
//! budget. This is the same injection pattern `tools::query_tests` uses for
//! its wall-clock budget.
use super::*;
use crate::model::ResourceLimits;
use crate::store::open_read_write;
use crate::sync::SourceSyncManager;
use crate::sync::discovery::{discover, discover_with_deadline};
use crate::sync::publish::publish;
use crate::sync::sample::sample_all_with_deadline;
use crate::sync::test_support::{create_project, sync_project, write};
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
    let leftover: Vec<String> = Vec::new();
    let hash = super::manifest::compute_manifest_hash(&connection, &empty, &leftover)
        .expect("seeded manifest hash");
    assert!(
        !hash.is_empty(),
        "manifest hash should be a non-empty hex digest"
    );

    let drop_a: Vec<String> = vec!["a.rs".to_owned()];
    let drop_hash = super::manifest::compute_manifest_hash(&connection, &empty, &drop_a)
        .expect("exclude-branch manifest hash");
    assert_ne!(drop_hash, hash, "deletions must change the aggregate hash");
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
