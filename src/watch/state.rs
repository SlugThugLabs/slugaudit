//! Thread-safe wrapper around per-project watch state. The `WatchManager`
//! holds one of these per active project.
//!
//! The lock in `inner` is acquired through [`crate::util::lock_or_recover`]
//! rather than a panicking `Mutex::lock()`: a `Mutex` whose previous guard
//! panicked inside a critical section is recovered (the inner value —
//! possibly in a logically inconsistent state — is returned to the next
//! caller) instead of panicking again. Without that recovery, a single
//! bug in any watcher state mutation would crash the entire MCP server on
//! the next filesystem event. The data definitions live in [`types`],
//! the path-normalization helper lives in [`path`]; this module owns the
//! concurrency wrapper behavior.

use super::types::{ProjectWatchState, WatcherHealth};
use crate::util::lock_or_recover;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Thread-safe wrapper around per-project watch state. The `WatchManager`
/// holds one of these per active project. Cloneable — clones share the
/// same underlying `Arc<Mutex<ProjectWatchState>>`, so an event recorded
/// by one clone is visible to every other clone.
#[derive(Debug, Clone, Default)]
pub struct WatchState {
    inner: Arc<Mutex<ProjectWatchState>>,
}

impl WatchState {
    /// An empty state: no dirty paths, no deleted paths.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a modify or create event for `relative_path`. Returns the
    /// new watcher sequence, which callers can use to barrier-synchronize
    /// — see [`crate::sync::SourceSyncManager::reconcile`].
    pub fn mark_dirty(&self, relative_path: String) -> u64 {
        let mut guard = lock_or_recover(&self.inner, "watch_state_inner");
        guard.dirty_paths.insert(relative_path);
        guard.watcher_sequence += 1;
        guard.watcher_sequence
    }

    /// Record a delete event for `relative_path`. If the path was also in
    /// `dirty_paths` (i.e. it was modified since the last reconcile), it's
    /// moved from there into `deleted_paths` — a delete supersedes a modify
    /// for the same path in the same window, so the next reconcile removes
    /// it cleanly rather than re-sampling a file that's gone.
    pub fn mark_deleted(&self, relative_path: String) -> u64 {
        let mut guard = lock_or_recover(&self.inner, "watch_state_inner");
        guard.dirty_paths.remove(&relative_path);
        guard.deleted_paths.insert(relative_path);
        guard.watcher_sequence += 1;
        guard.watcher_sequence
    }

    /// Snapshot the current dirty and deleted sets for reconciliation,
    /// clearing them so new events can be recorded separately. Does NOT
    /// advance `last_verified_sequence` — the caller must call
    /// `acknowledge_through` after reconciliation succeeds. Returns the
    /// current sequence so the caller can later acknowledge through it.
    pub fn snapshot_dirty(&self) -> (u64, HashSet<String>, HashSet<String>) {
        let mut guard = lock_or_recover(&self.inner, "watch_state_inner");
        let seq = guard.watcher_sequence;
        let dirty = std::mem::take(&mut guard.dirty_paths);
        let deleted = std::mem::take(&mut guard.deleted_paths);
        (seq, dirty, deleted)
    }

    /// Acknowledge that reconciliation through `seq` succeeded. Advances
    /// `last_verified_sequence` to `seq` only if the watcher hasn't since
    /// advanced past it (i.e. no new events need reconciliation). A stale
    /// seq is silently ignored — keeping it quiet is intentional, since
    /// a stale sequence carries no actionable information for the caller
    /// beyond "more events arrived during my reconcile, loop again".
    pub fn acknowledge_through(&self, seq: u64) {
        let mut guard = lock_or_recover(&self.inner, "watch_state_inner");
        if guard.watcher_sequence == seq {
            guard.last_verified_sequence = seq;
        }
    }

    /// Returns the current watcher sequence.
    pub fn current_sequence(&self) -> u64 {
        lock_or_recover(&self.inner, "watch_state_inner").watcher_sequence
    }

    /// Returns the current health.
    pub fn health(&self) -> WatcherHealth {
        lock_or_recover(&self.inner, "watch_state_inner").health
    }

    /// Sets the health.
    pub fn set_health(&self, health: WatcherHealth) {
        lock_or_recover(&self.inner, "watch_state_inner").health = health;
    }

    /// Returns true if there are unreconciled events.
    pub fn has_unreconciled_events(&self) -> bool {
        lock_or_recover(&self.inner, "watch_state_inner").has_unreconciled_events()
    }

    /// True when the watcher is `Healthy` and holds no unreconciled event,
    /// decided under one lock acquisition so an event or a health change
    /// cannot slip between the two checks. A caller that skips its stat
    /// sweep on this answer misses only events that arrive after the
    /// decision, which the next call sees.
    pub fn is_healthy_and_quiet(&self) -> bool {
        let guard = lock_or_recover(&self.inner, "watch_state_inner");
        guard.health == WatcherHealth::Healthy && !guard.has_unreconciled_events()
    }

    /// Returns a snapshot of the current state.
    pub fn snapshot(&self) -> ProjectWatchState {
        lock_or_recover(&self.inner, "watch_state_inner").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_and_quiet_requires_healthy_health_and_no_events() {
        let state = WatchState::new();
        assert!(state.is_healthy_and_quiet());

        state.mark_dirty("a.rs".to_owned());
        assert!(!state.is_healthy_and_quiet(), "a dirty path is not quiet");

        let (seq, _, _) = state.snapshot_dirty();
        assert!(
            !state.is_healthy_and_quiet(),
            "drained but unacknowledged events are not quiet"
        );
        state.acknowledge_through(seq);
        assert!(state.is_healthy_and_quiet());

        for health in [
            WatcherHealth::Partial,
            WatcherHealth::NeedsVerification,
            WatcherHealth::Desynced,
            WatcherHealth::Unavailable,
        ] {
            state.set_health(health);
            assert!(!state.is_healthy_and_quiet(), "{health:?} is not Healthy");
        }
    }

    #[test]
    fn dirty_then_deleted_collapses_correctly() {
        let state = WatchState::new();
        state.mark_dirty("src/lib.rs".to_owned());
        state.mark_deleted("src/lib.rs".to_owned());

        let (seq, dirty, deleted) = state.snapshot_dirty();
        assert_eq!(seq, 2);
        assert!(dirty.is_empty());
        assert!(deleted.contains("src/lib.rs"));
        assert_eq!(deleted.len(), 1);

        // last_verified_sequence should NOT have advanced yet
        let snapshot = state.snapshot();
        assert_eq!(snapshot.last_verified_sequence, 0);

        // After acknowledge_through, it should advance
        state.acknowledge_through(seq);
        let snapshot2 = state.snapshot();
        assert_eq!(snapshot2.last_verified_sequence, 2);
    }

    #[test]
    fn multiple_events_for_same_file_collapse_to_one() {
        let state = WatchState::new();
        for _ in 0..5 {
            state.mark_dirty("src/lib.rs".to_owned());
        }

        let (seq, dirty, _) = state.snapshot_dirty();
        assert_eq!(seq, 5);
        assert_eq!(dirty.len(), 1);
        assert!(dirty.contains("src/lib.rs"));

        // last_verified_sequence should NOT have advanced yet
        let snapshot = state.snapshot();
        assert_eq!(snapshot.last_verified_sequence, 0);
    }

    #[test]
    fn snapshot_dirty_does_not_advance_last_verified_sequence() {
        let state = WatchState::new();
        state.mark_dirty("a.rs".to_owned());
        state.mark_dirty("b.rs".to_owned());

        let (seq, _, _) = state.snapshot_dirty();
        assert_eq!(seq, 2);

        // last_verified_sequence should still be 0
        assert_eq!(state.snapshot().last_verified_sequence, 0);

        // has_unreconciled_events should still be true
        assert!(state.has_unreconciled_events());

        // After acknowledge, it should be false
        state.acknowledge_through(seq);
        assert!(!state.has_unreconciled_events());
    }

    #[test]
    fn acknowledge_through_ignores_stale_seq() {
        let state = WatchState::new();
        state.mark_dirty("a.rs".to_owned());
        let (seq, _, _) = state.snapshot_dirty();

        // New event arrives after snapshot but before acknowledge
        state.mark_dirty("b.rs".to_owned());

        // Acknowledging through the old seq should NOT advance
        state.acknowledge_through(seq);
        assert_eq!(state.snapshot().last_verified_sequence, 0);

        // has_unreconciled_events should still be true
        assert!(state.has_unreconciled_events());
    }

    /// After poison recovery, a `WatchState` continues to record events
    /// instead of forever rejecting the next caller with `PoisonError`.
    /// This is the WatchState-specific recovery test — the generic
    /// `Mutex<T>` recovery tests for `lock_or_recover` itself live in
    /// `crate::util::tests`. It poisons the state's own inner mutex, so a
    /// regression that swaps `lock_or_recover` for a panicking `lock()` in
    /// `mark_dirty` or `snapshot` fails here.
    #[test]
    fn watch_state_recovers_from_a_poisoned_inner_mutex() {
        let state = WatchState::new();
        state.mark_dirty("before-panic".to_owned());

        let inner = Arc::clone(&state.inner);
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = inner.lock().expect("unpoisoned");
            // Hold the guard through a deliberate panic so the state's own
            // mutex ends up poisoned after this closure returns.
            std::panic::panic_any("simulated panic inside WatchState's critical section");
        }));
        assert!(panic_result.is_err());

        // A std `lock()` would now fail with `PoisonError`; both the
        // mutator and the reader must recover the underlying state.
        let seq = state.mark_dirty("after-panic".to_owned());
        let snapshot = state.snapshot();

        assert_eq!(seq, 2, "the recovered state must keep recording events");
        assert!(
            snapshot.dirty_paths.contains("before-panic"),
            "the dirty path inserted before the panic must survive"
        );
        assert!(
            snapshot.dirty_paths.contains("after-panic"),
            "an event recorded after the poison must be visible"
        );
    }
}
