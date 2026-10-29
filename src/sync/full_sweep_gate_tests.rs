//! The Healthy-path stat-sweep skip and its time/transition backstop.
//! Events the watcher "missed" are simulated by draining the dirty queue
//! behind the manager's back, so the only way a change becomes visible is a
//! sweep or a full verification; the clock is injected so the backstop
//! interval is crossed without sleeping.

use super::SourceSyncManager;
use crate::sync::full_sweep_gate::{FULL_RECONCILE_INTERVAL, FullSweepGate};
use crate::sync::test_support::{TestClock, create_project, sync_project, write_file};
use crate::watch::{WatchState, WatcherHealth};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn stored_content(manager: &SourceSyncManager, project: &tempfile::TempDir, path: &str) -> String {
    let synced = sync_project(manager, project);
    let connection = crate::store::open_read_only(&synced.database_path).expect("open db");
    connection
        .query_row("SELECT content FROM files WHERE path = ?1", [path], |row| {
            row.get(0)
        })
        .expect("read stored content")
}

impl SourceSyncManager {
    /// A watcher-backed manager whose full-sweep backstop reads `clock`.
    pub(crate) fn with_watcher_and_clock(
        clock: Arc<dyn Fn() -> std::time::Instant + Send + Sync>,
    ) -> Self {
        Self {
            full_sweep_gate: crate::sync::full_sweep_gate::FullSweepGate::with_clock(clock),
            ..Self::with_watcher()
        }
    }

    /// Whole-tree walks the Healthy path has run. A Healthy, quiet watcher
    /// inside the backstop interval adds none.
    pub(crate) fn healthy_walks(&self) -> u64 {
        self.full_sweep_gate.healthy_walks()
    }
}

/// Waits for the watcher to deliver the write, then discards its record of
/// the event, as if notify had dropped it.
fn swallow_events(state: &WatchState) {
    thread::sleep(Duration::from_millis(300));
    let (seq, _, _) = state.snapshot_dirty();
    state.acknowledge_through(seq);
    assert!(!state.has_unreconciled_events());
}

fn watched(manager: &SourceSyncManager, project: &tempfile::TempDir) -> WatchState {
    manager
        .watch_state_for(project.path())
        .expect("the synced project is registered")
}

#[test]
fn a_healthy_quiet_watcher_does_not_walk_the_tree() {
    let clock = TestClock::new();
    let manager = SourceSyncManager::with_watcher_and_clock(clock.reader());
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let first = sync_project(&manager, &project);
    let state = watched(&manager, &project);
    assert_eq!(state.health(), WatcherHealth::Healthy);
    assert_eq!(
        manager.healthy_walks(),
        0,
        "verification is not a healthy walk"
    );

    clock.advance(Duration::from_secs(1));
    let second = sync_project(&manager, &project);
    clock.advance(Duration::from_secs(1));
    let third = sync_project(&manager, &project);

    assert_eq!(manager.healthy_walks(), 0, "quiet calls must not walk");
    assert_eq!(second.revision_id, first.revision_id);
    assert_eq!(third.revision_id, first.revision_id);
}

#[test]
fn a_watcher_detected_change_appears_in_the_next_call() {
    let clock = TestClock::new();
    let manager = SourceSyncManager::with_watcher_and_clock(clock.reader());
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let first = sync_project(&manager, &project);
    sync_project(&manager, &project);
    assert_eq!(manager.healthy_walks(), 0);

    write_file(&project, "lib.rs", b"pub fn b_changed() {}\n");
    thread::sleep(Duration::from_millis(300));
    let after = sync_project(&manager, &project);

    assert_ne!(after.revision_id, first.revision_id);
    assert_eq!(
        manager.healthy_walks(),
        1,
        "pending events take the walking path"
    );
    assert!(stored_content(&manager, &project, "lib.rs").contains("b_changed"));
}

#[test]
fn the_backstop_sweeps_once_the_interval_has_elapsed() {
    let clock = TestClock::new();
    let manager = SourceSyncManager::with_watcher_and_clock(clock.reader());
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let first = sync_project(&manager, &project);
    let state = watched(&manager, &project);

    // A change the watcher never reports stays invisible inside the interval.
    write_file(&project, "lib.rs", b"pub fn missed_by_notify() {}\n");
    swallow_events(&state);
    clock.advance(FULL_RECONCILE_INTERVAL - Duration::from_secs(1));
    let inside = sync_project(&manager, &project);
    assert_eq!(inside.revision_id, first.revision_id);
    assert_eq!(manager.healthy_walks(), 0);

    // Crossing the interval forces the sweep, which finds it.
    clock.advance(Duration::from_secs(1));
    let after = sync_project(&manager, &project);
    assert_eq!(
        manager.healthy_walks(),
        1,
        "the backstop fires exactly once"
    );
    assert_ne!(after.revision_id, first.revision_id);
    assert!(stored_content(&manager, &project, "lib.rs").contains("missed_by_notify"));

    // The completed sweep restarts the window: the next call skips again.
    sync_project(&manager, &project);
    assert_eq!(manager.healthy_walks(), 1);
}

#[test]
fn leaving_healthy_forces_a_full_verification_and_then_trusts_again() {
    let clock = TestClock::new();
    let manager = SourceSyncManager::with_watcher_and_clock(clock.reader());
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);
    let state = watched(&manager, &project);

    write_file(&project, "lib.rs", b"pub fn after_overflow() {}\n");
    swallow_events(&state);
    state.set_health(WatcherHealth::Desynced);
    let publishes_before = manager.consecutive_full_publishes();
    assert!(stored_content(&manager, &project, "lib.rs").contains("after_overflow"));
    assert_eq!(manager.consecutive_full_publishes(), publishes_before + 1);
    assert_eq!(state.health(), WatcherHealth::Healthy);

    sync_project(&manager, &project);
    assert_eq!(
        manager.healthy_walks(),
        0,
        "trusted again after re-verifying"
    );
}

#[test]
fn a_failed_sweep_does_not_restart_the_backstop_window() {
    let clock = TestClock::new();
    let manager = SourceSyncManager::with_watcher_and_clock(clock.reader());
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let synced = sync_project(&manager, &project);
    let state = watched(&manager, &project);

    clock.advance(FULL_RECONCILE_INTERVAL);
    {
        let connection = crate::store::open_read_write(&synced.database_path).expect("open db");
        connection
            .execute_batch("ALTER TABLE files RENAME TO files_backing")
            .expect("hide the files table from the sweep");
    }
    let error = manager
        .ensure_current(
            &project.path().to_string_lossy(),
            &crate::progress::NoopProgressSink,
        )
        .err()
        .expect("the failed sweep must surface");
    assert_eq!(error.message, "stat sweep: no such table: files");
    assert_eq!(state.health(), WatcherHealth::Desynced);
    assert_eq!(manager.healthy_walks(), 1);

    // Even if something put the watcher back to Healthy, the failed sweep
    // left no verification on record, so the next call walks again.
    {
        let connection = crate::store::open_read_write(&synced.database_path).expect("open db");
        connection
            .execute_batch("ALTER TABLE files_backing RENAME TO files")
            .expect("restore the files table");
    }
    state.set_health(WatcherHealth::Healthy);
    sync_project(&manager, &project);
    assert_eq!(manager.healthy_walks(), 2);
}

#[test]
fn a_partial_watcher_still_sweeps_every_call() {
    let clock = TestClock::new();
    let manager = SourceSyncManager::with_watcher_and_clock(clock.reader());
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);
    let state = watched(&manager, &project);

    state.set_health(WatcherHealth::Partial);
    sync_project(&manager, &project);
    sync_project(&manager, &project);
    assert_eq!(manager.healthy_walks(), 2);
}

#[test]
fn the_gate_requires_a_recent_record_with_the_same_ignore_rules() {
    let clock = TestClock::new();
    let gate = FullSweepGate::with_clock(clock.reader());
    let root = Path::new("/project");
    let rules = Arc::new(crate::ignore_rules::IgnoreRules::build(root, &[]));
    let other_rules = Arc::new(crate::ignore_rules::IgnoreRules::build(root, &[]));

    assert!(!gate.may_skip(root, Some(&rules)), "nothing recorded yet");
    gate.record(root, gate.now(), Some(Arc::clone(&rules)));
    assert!(gate.may_skip(root, Some(&rules)));
    assert!(!gate.may_skip(root, Some(&other_rules)), "rules changed");
    assert!(!gate.may_skip(root, None), "no rules to compare");
    assert!(!gate.may_skip(Path::new("/elsewhere"), Some(&rules)));

    clock.advance(FULL_RECONCILE_INTERVAL - Duration::from_millis(1));
    assert!(gate.may_skip(root, Some(&rules)));
    clock.advance(Duration::from_millis(1));
    assert!(!gate.may_skip(root, Some(&rules)), "interval elapsed");

    gate.record(root, gate.now(), Some(Arc::clone(&rules)));
    gate.invalidate(root);
    assert!(!gate.may_skip(root, Some(&rules)), "invalidated");
}
