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

mod manager;
mod path;
mod scope;
mod state;
mod types;

#[cfg(test)]
mod tests;

pub use manager::WatchManager;
pub use path::normalize_relative_path;
pub use scope::WatchScope;
pub use state::WatchState;
pub use types::{ProjectWatchState, WatcherHealth};
