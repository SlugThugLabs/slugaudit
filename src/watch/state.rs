//! Per-project watcher state: dirty/deleted paths, sequence numbers, and
//! watcher health. The watcher never parses or indexes — it only records
//! that something changed and lets the sync layer decide what to do.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Health of a project's filesystem watcher. Never silently assume healthy —
/// the caller must check before trusting the dirty set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WatcherHealth {
    /// Watcher is active and events are being recorded.
    #[default]
    Healthy,
    /// Watcher history is untrustworthy (e.g. after restart). The current
    /// filesystem must be reconciled against the stored state before the
    /// database can be considered synchronized.
    NeedsVerification,
    /// Watcher detected an integrity problem (queue overflow, watch removed,
    /// etc.). A full verification is required before serving evidence.
    Desynced,
    /// Watcher could not be initialized for this project (e.g. platform
    /// doesn't support it, or the project root isn't watchable).
    Unavailable,
}

/// Watch state for a single active project. All mutation happens under the
/// lock in `WatchManager`; this struct is the pure data.
#[derive(Debug, Clone, Default)]
pub struct ProjectWatchState {
    /// Project-relative paths that have been created or modified since the
    /// last reconciliation. Multiple events for the same path collapse into
    /// one entry.
    pub dirty_paths: HashSet<String>,
    /// Project-relative paths that have been deleted since the last
    /// reconciliation.
    pub deleted_paths: HashSet<String>,
    /// Monotonically increasing sequence number. Incremented for every
    /// filesystem event (after collapsing). Used for barrier synchronization.
    pub watcher_sequence: u64,
    /// The sequence last reconciled by the sync layer. When this equals
    /// `watcher_sequence`, the dirty/deleted sets are empty and the
    /// database is fully current.
    pub last_verified_sequence: u64,
    /// Current watcher health.
    pub health: WatcherHealth,
}

impl ProjectWatchState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if there are unreconciled events.
    pub fn has_unreconciled_events(&self) -> bool {
        self.watcher_sequence > self.last_verified_sequence
            || !self.dirty_paths.is_empty()
            || !self.deleted_paths.is_empty()
    }

    /// Returns the number of unreconciled events (dirty + deleted).
    pub fn unreconciled_count(&self) -> usize {
        self.dirty_paths.len() + self.deleted_paths.len()
    }
}

/// Thread-safe wrapper around per-project watch state. The `WatchManager`
/// holds one of these per active project.
#[derive(Debug, Clone, Default)]
pub struct WatchState {
    inner: Arc<Mutex<ProjectWatchState>>,
}

impl WatchState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a modify or create event for `relative_path`.
    pub fn mark_dirty(&self, relative_path: String) -> u64 {
        let mut guard = self.inner.lock().unwrap();
        guard.dirty_paths.insert(relative_path);
        guard.watcher_sequence += 1;
        guard.watcher_sequence
    }

    /// Record a delete event for `relative_path`.
    pub fn mark_deleted(&self, relative_path: String) -> u64 {
        let mut guard = self.inner.lock().unwrap();
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
        let mut guard = self.inner.lock().unwrap();
        let seq = guard.watcher_sequence;
        let dirty = std::mem::take(&mut guard.dirty_paths);
        let deleted = std::mem::take(&mut guard.deleted_paths);
        (seq, dirty, deleted)
    }

    /// Acknowledge that reconciliation through `seq` succeeded. Advances
    /// `last_verified_sequence` to `seq` only if the watcher hasn't since
    /// advanced past it (i.e. no new events need reconciliation).
    pub fn acknowledge_through(&self, seq: u64) {
        let mut guard = self.inner.lock().unwrap();
        if guard.watcher_sequence == seq {
            guard.last_verified_sequence = seq;
        }
    }

    /// Returns the current watcher sequence.
    pub fn current_sequence(&self) -> u64 {
        self.inner.lock().unwrap().watcher_sequence
    }

    /// Returns the current health.
    pub fn health(&self) -> WatcherHealth {
        self.inner.lock().unwrap().health
    }

    /// Sets the health.
    pub fn set_health(&self, health: WatcherHealth) {
        self.inner.lock().unwrap().health = health;
    }

    /// Returns true if there are unreconciled events.
    pub fn has_unreconciled_events(&self) -> bool {
        self.inner.lock().unwrap().has_unreconciled_events()
    }

    /// Returns a snapshot of the current state.
    pub fn snapshot(&self) -> ProjectWatchState {
        self.inner.lock().unwrap().clone()
    }
}

/// Normalizes a filesystem path to a project-relative, forward-slash path
/// string. Returns `None` if the path isn't under `root`.
pub fn normalize_relative_path(root: &Path, absolute: &Path) -> Option<String> {
    absolute
        .strip_prefix(root)
        .ok()?
        .to_str()
        .map(|s| s.replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
