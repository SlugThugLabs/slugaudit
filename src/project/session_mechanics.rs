//! The mechanics an auditor registered for one server's session, held in
//! memory only. Split from `mechanics_config` (the persisted section) because
//! the two have different lifetimes: this set dies with the process, the
//! file outlives it.

use super::mechanics_config::MechanicEntry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The mechanics registered for this server's session: held in memory by the
/// server's `SourceSyncManager` (clones share them), never written anywhere,
/// and gone when the process exits. Also records whether this server has
/// already delivered the one-time `report` mechanics note.
#[derive(Debug, Clone, Default)]
pub struct SessionMechanics {
    entries: Arc<Mutex<Vec<MechanicEntry>>>,
    report_note_delivered: Arc<AtomicBool>,
}

/// Most session entries one server holds; a further `add` is refused.
pub const MAX_SESSION_MECHANICS: usize = 64;

impl SessionMechanics {
    /// A snapshot of the session entries, oldest first.
    #[must_use]
    pub fn entries(&self) -> Vec<MechanicEntry> {
        crate::util::lock_or_recover(&self.entries, "session_mechanics").clone()
    }

    /// Runs `change` with the session entries locked, so a check and the
    /// change it guards (collision, cap, persist) are one step for every
    /// `mechanic` call on this server.
    pub fn with_entries<T>(&self, change: impl FnOnce(&mut Vec<MechanicEntry>) -> T) -> T {
        change(&mut crate::util::lock_or_recover(
            &self.entries,
            "session_mechanics",
        ))
    }

    /// True exactly once per server: for the first caller that claims the
    /// `report` mechanics note.
    #[must_use]
    pub fn claim_report_note(&self) -> bool {
        !self.report_note_delivered.swap(true, Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_report_note_is_true_once_per_session_set() {
        let session = SessionMechanics::default();
        let shared = session.clone();

        let first = session.claim_report_note();
        let second = shared.claim_report_note();

        assert!(first, "the first claim delivers the note");
        assert!(!second, "a clone shares the claim");
    }
}
