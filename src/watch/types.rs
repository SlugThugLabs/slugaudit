//! Pure data types for the filesystem watcher: the health enum and the
//! per-project state record. These types hold no synchronization
//! primitives — the concurrent wrapper `WatchState` (in `state.rs`)
//! stores them inside an `Arc<Mutex<...>>`. Splitting them out keeps the
//! data definitions in one place where they're easy to compare and
//! reason about, and keeps the locking layer next to its actual locking
//! logic.

use schemars::JsonSchema;
use serde::Serialize;
use std::collections::HashSet;

/// Health of a project's filesystem watcher. Never silently assume healthy —
/// the caller must check before trusting the dirty set.
///
/// Derived `Serialize` so the `health` MCP tool can return this enum as
/// part of its `Json<HealthResponse>` response. Derived `JsonSchema` so
/// the same schema is surfaced through MCP to consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
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
    ///
    /// "Unreconciled" means either dirty or deleted sets contain a path,
    /// or the watcher sequence is strictly ahead of the last-verified
    /// sequence. The first component handles "events arrived and are
    /// sitting unacknowledged"; the second handles "reconciliation moved
    /// the verified mark but the sequence has since advanced again",
    /// which can still happen after `snapshot_dirty` cleared the sets
    /// but before `acknowledge_through` was called.
    pub fn has_unreconciled_events(&self) -> bool {
        self.watcher_sequence > self.last_verified_sequence
            || !self.dirty_paths.is_empty()
            || !self.deleted_paths.is_empty()
    }

    /// Returns the number of unreconciled events (dirty + deleted). Useful
    /// to expose through the `health` MCP tool so an operator can see how
    /// many events are queued without enumerating them.
    pub fn unreconciled_count(&self) -> usize {
        self.dirty_paths.len() + self.deleted_paths.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_has_no_unreconciled_events() {
        let state = ProjectWatchState::default();
        assert!(!state.has_unreconciled_events());
        assert_eq!(state.unreconciled_count(), 0);
    }

    #[test]
    fn dirty_paths_count_as_unreconciled() {
        let mut state = ProjectWatchState::default();
        state.dirty_paths.insert("a.rs".to_owned());
        state.dirty_paths.insert("b.rs".to_owned());
        assert!(state.has_unreconciled_events());
        assert_eq!(state.unreconciled_count(), 2);
    }

    #[test]
    fn deleted_paths_count_as_unreconciled() {
        let mut state = ProjectWatchState::default();
        state.deleted_paths.insert("a.rs".to_owned());
        assert!(state.has_unreconciled_events());
        assert_eq!(state.unreconciled_count(), 1);
    }

    /// After `snapshot_dirty` clears the sets but before `acknowledge_through`
    /// advances `last_verified_sequence`, the watcher sequence mismatch is
    /// the only thing still flagging unreconciled events. This is the case
    /// where the lock_or_recovered state must continue to surface
    /// "still dirty" — otherwise a recovered critical section could hide
    /// pending reconciliation from a quick health check.
    #[test]
    fn a_sequence_advance_with_cleared_sets_still_flags_unreconciled() {
        let mut state = ProjectWatchState::default();
        state.dirty_paths.insert("a.rs".to_owned());
        state.watcher_sequence = 5;
        // Simulate `snapshot_dirty` clearing the sets:
        state.dirty_paths.clear();
        state.deleted_paths.clear();
        // Sequence now outpaces the verified mark — there could be a race
        // where events arrived between snapshot and acknowledge.
        assert_eq!(state.last_verified_sequence, 0);
        assert!(state.has_unreconciled_events());
        assert_eq!(
            state.unreconciled_count(),
            0,
            "but pending count is zero this exact instant"
        );
    }
}
