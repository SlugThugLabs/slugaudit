//! Filesystem watching infrastructure for SlugAudit.
//!
//! The watcher tracks filesystem changes (creates, modifies, deletes) and
//! records them in per-project `WatchState`. It never parses or indexes —
//! that's the sync layer's job. The watcher's only responsibility is to
//! answer: "what changed since you last asked?"
//!
//! # Health states
//!
//! - **Healthy**: Watcher is active and events are being recorded.
//! - **Partial**: Watcher is active, but the scope walk hit a documented
//!   limit (`max_watch_entries` / `max_watch_depth`); directories beyond it
//!   are unwatched. The stat sweep still runs on every call, so freshness is
//!   preserved — the watcher is an incomplete hint, not a gap in the
//!   guarantee.
//! - **NeedsVerification**: Watcher history is untrustworthy (e.g. after
//!   restart). The current filesystem must be reconciled against stored
//!   state before the database can be considered synchronized.
//! - **Desynced**: Watcher detected an integrity problem (queue overflow,
//!   watch removed, etc.). Full verification required.
//! - **Unavailable**: Watcher couldn't be initialized. Sync layer does
//!   full verification on every call.
//!
//! # Barrier synchronization
//!
//! Before serving evidence, the sync layer takes a "barrier" — a snapshot
//! of the current watcher sequence. It then reconciles all dirty/deleted
//! paths up to that barrier. After reconciliation, it checks if the
//! watcher sequence advanced (meaning more events arrived during
//! reconciliation). If so, it takes another barrier and reconciles again.
//! This loops until the watcher sequence stabilizes.

mod errors;
mod event_filter;
mod manager;
mod path;
mod scope;
mod scope_apply;
mod state;
mod types;

pub use manager::WatchManager;
pub use path::normalize_relative_path;
pub use scope::{WatchScope, WatchScopeLimit};
pub use state::WatchState;
pub use types::{ProjectWatchState, WatcherHealth};

/// A `#[cfg(test)]` hook invoked once inside a scope walk on the thread
/// that started it. It exists so a test can prove, deterministically and
/// without timing guesses, that the manager lock is free while the walk
/// runs. The thread id keeps a concurrently running test's walk from
/// consuming a hook installed for another thread.
#[cfg(test)]
type WalkHook = (std::thread::ThreadId, Box<dyn FnOnce() + Send>);

#[cfg(test)]
static WALK_HOOK: std::sync::Mutex<Option<WalkHook>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_walk_hook(hook: Box<dyn FnOnce() + Send>) {
    *WALK_HOOK.lock().expect("walk hook lock") = Some((std::thread::current().id(), hook));
}

#[cfg(test)]
fn run_walk_hook() {
    let hook = {
        let mut slot = WALK_HOOK.lock().expect("walk hook lock");
        match slot.as_ref() {
            Some((thread, _)) if *thread == std::thread::current().id() => {
                slot.take().map(|(_, hook)| hook)
            }
            _ => None,
        }
    };
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
impl WatchManager {
    /// Test-only entry point: registers `root` with an explicit scope cap so
    /// the truncation transitions can be exercised without building a
    /// 200k-directory tree.
    pub(crate) fn watch_with_scope_limits_for_test(
        &self,
        root: &std::path::Path,
        max_entries: usize,
        max_depth: usize,
    ) -> WatchState {
        self.watch_with_scope_limits(root, max_entries, max_depth)
    }

    /// Test-only entry point: recomputes `root`'s scope under an explicit
    /// cap, bypassing the process-wide limits.
    pub(crate) fn refresh_scope_with_limits_for_test(
        &self,
        root: &std::path::Path,
        max_entries: usize,
        max_depth: usize,
    ) {
        self.refresh_scope_with_limits(root, max_entries, max_depth);
    }
}

#[cfg(test)]
mod tests;
