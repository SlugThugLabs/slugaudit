//! The process's result store handle: one `ResultStore`, opened lazily on the
//! first materialisation and keyed by the runtime directory it lives in.
//!
//! Held on `SourceSyncManager` because the manager is the per-server state
//! every tool already receives. There is deliberately one slot, not a map: a
//! project switch (`project_control`) asks for another runtime directory, the
//! previous store is dropped (its lock released, its results closed) and a new
//! one is opened, so a store of another project is never kept alive and the
//! disk budget stays one budget. A cursor issued before the switch then reads
//! as `unknown_cursor`. A continuation carries no project path, so it uses
//! whichever store is current; it never opens one.

use crate::model::process_limits;
use crate::store::{ResultStore, ResultStoreError, ResultStoreLimits};
use crate::util::lock_or_recover;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The open store and the runtime directory it was opened for.
type Slot = Option<(PathBuf, Arc<ResultStore>)>;

/// See the module docs. Clones share the slot.
#[derive(Clone, Default)]
pub struct ResultStoreSlot {
    limits: Option<ResultStoreLimits>,
    current: Arc<Mutex<Slot>>,
}

impl ResultStoreSlot {
    /// A slot whose stores obey `limits` instead of the process limits.
    #[cfg(test)]
    pub(crate) fn with_limits(limits: ResultStoreLimits) -> Self {
        Self {
            limits: Some(limits),
            current: Arc::default(),
        }
    }

    /// The store for `runtime_dir`, opening it on first use and replacing a
    /// store opened for another directory.
    ///
    /// # Errors
    ///
    /// Whatever `ResultStore::open` reports (already logged by the store).
    pub fn for_dir(&self, runtime_dir: &Path) -> Result<Arc<ResultStore>, ResultStoreError> {
        // The slot lock is held across the open on purpose: two first calls
        // must not both open (and lock) the same directory. The open touches
        // only the results directory, never a result's rows.
        let mut slot = lock_or_recover(&self.current, "result_store_slot");
        if let Some((dir, store)) = slot.as_ref()
            && dir == runtime_dir
        {
            return Ok(Arc::clone(store));
        }
        // Release the previous store (and its owner lock) before opening the
        // next one.
        *slot = None;
        let limits = match self.limits {
            Some(limits) => limits,
            None => ResultStoreLimits::from(process_limits().map_err(|error| {
                tracing::error!(%error, "process limits environment is invalid");
                ResultStoreError::Unavailable {
                    context: "the process limits are invalid",
                    source: Box::new(error),
                }
            })?),
        };
        let store = Arc::new(ResultStore::open(
            runtime_dir,
            crate::util::session_id(),
            limits,
        )?);
        *slot = Some((runtime_dir.to_path_buf(), Arc::clone(&store)));
        Ok(store)
    }

    /// Drops the store held for `runtime_dir`, if that is the one in the slot,
    /// so the next `for_dir` opens a fresh one and recreates `results/`.
    ///
    /// Called when a project is enabled: a disable removed the results
    /// directory under the slot, and the held store would keep pointing at
    /// it. A store held for another directory is left alone.
    pub fn release_dir(&self, runtime_dir: &Path) {
        let mut slot = lock_or_recover(&self.current, "result_store_slot");
        if slot.as_ref().is_some_and(|(dir, _)| dir == runtime_dir) {
            *slot = None;
        }
    }

    /// The open store, if any result was ever materialised in this process.
    pub fn current(&self) -> Option<Arc<ResultStore>> {
        lock_or_recover(&self.current, "result_store_slot")
            .as_ref()
            .map(|(_, store)| Arc::clone(store))
    }
}

#[cfg(test)]
#[path = "result_store_proof_exclusion_tests.rs"]
mod proof_exclusion_tests;

#[cfg(test)]
#[path = "result_store_proof_replacement_tests.rs"]
mod proof_replacement_tests;
