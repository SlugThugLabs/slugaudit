//! Wall-clock timeout coverage for the sync hot loops. Each test injects a
//! nanosecond-sized budget — any positive elapsed time trips the deadline,
//! so the abort is deterministic without waiting out the production-sized
//! budget. This is the same injection pattern `tools::query_tests` uses for
//! its wall-clock budget.
use super::*;
use crate::model::ResourceLimits;
use crate::store::open_read_write;
use crate::sync::discovery::{discover, discover_with_deadline};
use crate::sync::sample::sample_all_with_deadline;
use crate::util::Deadline;
use crate::watch::WatchState;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::Duration;

fn write(root: &Path, relative: &str, content: &[u8]) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, content).expect("write fixture file");
}

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
    let error = reconcile_dirty_paths_with_deadline(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        None,
        &limits,
        &tiny_budget(),
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
