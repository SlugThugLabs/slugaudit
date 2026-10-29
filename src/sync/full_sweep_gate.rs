//! The backstop that bounds how long a Healthy watcher may go without a
//! full-tree verification.
//!
//! On a Healthy watcher with an empty dirty queue the per-call stat sweep is
//! skipped: the watcher already says nothing changed. That trust is bounded.
//! A notify event can be dropped without any error surfacing (a kernel that
//! loses an inotify event, a network mount that never reports), so the gate
//! forces a full sweep once [`FULL_RECONCILE_INTERVAL`] has passed since the
//! last *completed* full verification of the project. Anything that makes the
//! previous verification untrustworthy forces a sweep too: no verification
//! recorded yet (first call after start), a changed set of ignore rules, or a
//! failed sweep (which invalidates the record).
//!
//! The freshness contract for a skipped call is therefore: the answer
//! reflects every change the watcher reported, and every other change within
//! [`FULL_RECONCILE_INTERVAL`] of the call that last swept the whole tree.

use crate::ignore_rules::IgnoreRules;
use crate::util::lock_or_recover;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Longest a Healthy, quiet watcher is trusted without a full sweep. A
/// change the watcher never reports is picked up by the first call after
/// this much monotonic time has passed since the last completed full sweep.
pub(crate) const FULL_RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

/// Source of monotonic time; replaced in tests so the interval can be
/// crossed without sleeping.
type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// One project's last completed full verification.
struct Verified {
    /// When the verifying walk *started*, so a long walk never extends the
    /// trust window past what it actually observed.
    started: Instant,
    /// The ignore rules in force for that walk. A different set means files
    /// may have become (un)ignored since, which only a sweep sees.
    rules: Arc<IgnoreRules>,
}

/// Per-project record of the last completed full verification, plus a
/// production-safe counter of Healthy-path whole-tree walks.
#[derive(Clone)]
pub(super) struct FullSweepGate {
    clock: Clock,
    verified: Arc<Mutex<HashMap<PathBuf, Verified>>>,
    healthy_walks: Arc<AtomicU64>,
}

impl Default for FullSweepGate {
    fn default() -> Self {
        Self::with_clock(Arc::new(Instant::now))
    }
}

impl FullSweepGate {
    pub(super) fn with_clock(clock: Clock) -> Self {
        Self {
            clock,
            verified: Arc::default(),
            healthy_walks: Arc::default(),
        }
    }

    /// The gate's current monotonic time; capture it before a walk starts and
    /// pass it to [`Self::record`].
    pub(super) fn now(&self) -> Instant {
        (self.clock)()
    }

    /// True when a full sweep may be skipped for `root`: a verification is on
    /// record, it used exactly these ignore rules, and fewer than
    /// [`FULL_RECONCILE_INTERVAL`] have passed since it started.
    pub(super) fn may_skip(&self, root: &Path, rules: Option<&Arc<IgnoreRules>>) -> bool {
        let Some(rules) = rules else {
            return false;
        };
        let guard = lock_or_recover(&self.verified, "full_sweep_gate");
        guard.get(root).is_some_and(|verified| {
            Arc::ptr_eq(&verified.rules, rules)
                && self.now().saturating_duration_since(verified.started) < FULL_RECONCILE_INTERVAL
        })
    }

    /// Records a completed, successful full verification of `root` that began
    /// at `started`. Never call this for a failed walk or apply. Without
    /// ignore rules there is nothing to compare later, so no record is kept
    /// and the next call sweeps.
    pub(super) fn record(&self, root: &Path, started: Instant, rules: Option<Arc<IgnoreRules>>) {
        let mut guard = lock_or_recover(&self.verified, "full_sweep_gate");
        match rules {
            Some(rules) => {
                guard.insert(root.to_path_buf(), Verified { started, rules });
            }
            None => {
                guard.remove(root);
            }
        }
    }

    /// Forgets `root`'s verification so the next call sweeps.
    pub(super) fn invalidate(&self, root: &Path) {
        lock_or_recover(&self.verified, "full_sweep_gate").remove(root);
    }

    /// Counts one Healthy-path whole-tree walk.
    pub(super) fn count_healthy_walk(&self) {
        self.healthy_walks.fetch_add(1, Ordering::Relaxed);
    }

    /// Whole-tree walks the Healthy path has run so far.
    #[cfg(test)]
    pub(super) fn healthy_walks(&self) -> u64 {
        self.healthy_walks.load(Ordering::Relaxed)
    }
}
