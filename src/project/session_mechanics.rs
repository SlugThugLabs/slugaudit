//! The mechanics an auditor registered for one server's session, held in
//! memory only: this set dies with the process.

use super::mechanic_entry::MechanicEntry;
use crate::util::LockPoisoned;
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
    ///
    /// # Errors
    ///
    /// Returns [`LockPoisoned`] when the entries were poisoned.
    pub fn entries(&self) -> Result<Vec<MechanicEntry>, LockPoisoned> {
        Ok(crate::util::lock(&self.entries, "session_mechanics")?.clone())
    }

    /// Runs `change` with the session entries locked, so a check and the
    /// change it guards (collision, cap) are one step for every
    /// `mechanic` call on this server.
    ///
    /// # Errors
    ///
    /// Returns [`LockPoisoned`] when the entries were poisoned.
    pub fn with_entries<T>(
        &self,
        change: impl FnOnce(&mut Vec<MechanicEntry>) -> T,
    ) -> Result<T, LockPoisoned> {
        Ok(change(&mut *crate::util::lock(
            &self.entries,
            "session_mechanics",
        )?))
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
