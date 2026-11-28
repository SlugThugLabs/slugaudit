//! Single-project result store slot: holds at most one active [`ResultStore`]
//! so switching projects drops the previous project's unlinked `.db`
//! descriptors and bounds scratch usage to one project's `max_result_store_bytes`.

#[cfg(test)]
use crate::model::ResourceLimits;
use crate::store::{ResultStore, ResultStoreError, ResultStoreLimits};
use crate::util::{LockPoisoned, lock};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

type ActiveSlot = Option<(PathBuf, Arc<ResultStore>)>;

fn canonical_or_raw(runtime_dir: &Path) -> PathBuf {
    match runtime_dir.canonicalize() {
        Ok(canonical) => canonical,
        Err(_) => runtime_dir.to_path_buf(),
    }
}

fn same_runtime_dir(held_dir: &Path, candidate: &Path) -> bool {
    held_dir == candidate || held_dir == canonical_or_raw(candidate).as_path()
}

/// Shared single-project slot for the active project's [`ResultStore`]. Clones
/// share the underlying slot.
#[derive(Clone, Default)]
pub struct ResultStoreSlot {
    pinned_limits: Option<ResultStoreLimits>,
    state: Arc<Mutex<ActiveSlot>>,
}

impl ResultStoreSlot {
    /// A slot whose stores always obey `limits` (test helper).
    #[cfg(test)]
    pub(crate) fn with_limits(limits: ResultStoreLimits) -> Self {
        Self {
            pinned_limits: Some(limits),
            state: Arc::default(),
        }
    }

    /// Opens or retrieves the store for `runtime_dir` using
    /// [`ResourceLimits::default`] (test helper).
    ///
    /// # Errors
    ///
    /// Whatever [`ResultStore::open`] reports.
    #[cfg(test)]
    pub fn for_dir(&self, runtime_dir: &Path) -> Result<Arc<ResultStore>, ResultStoreError> {
        self.for_dir_with_limits(
            runtime_dir,
            ResultStoreLimits::from(&ResourceLimits::default()),
        )
    }

    /// Returns the store for `runtime_dir`, reusing and updating limits when
    /// the slot already holds `runtime_dir` and its `results` directory still
    /// exists; otherwise drops any previous store and opens a fresh one.
    ///
    /// # Errors
    ///
    /// Whatever [`ResultStore::open`] reports.
    pub fn for_dir_with_limits(
        &self,
        runtime_dir: &Path,
        limits: ResultStoreLimits,
    ) -> Result<Arc<ResultStore>, ResultStoreError> {
        let effective = match self.pinned_limits {
            Some(pinned) => pinned,
            None => limits,
        };
        let mut state = lock(&self.state, "result_store_slot").map_err(|error| {
            ResultStoreError::unavailable("the result store slot is poisoned", error)
        })?;
        if let Some((held_dir, existing)) = state.as_ref()
            && same_runtime_dir(held_dir, runtime_dir)
            && runtime_dir.join("results").is_dir()
        {
            existing.update_limits(effective);
            return Ok(Arc::clone(existing));
        }
        *state = None;
        let store = Arc::new(ResultStore::open(
            runtime_dir,
            uuid::Uuid::new_v4(),
            effective,
        )?);
        let canonical = canonical_or_raw(runtime_dir);
        *state = Some((canonical, Arc::clone(&store)));
        Ok(store)
    }

    /// Drops the held store if it belongs to `runtime_dir`, closing its lock
    /// and open result descriptors.
    ///
    /// # Errors
    ///
    /// Returns `LockPoisoned` when the slot was poisoned.
    pub fn release_dir(&self, runtime_dir: &Path) -> Result<(), LockPoisoned> {
        let mut state = lock(&self.state, "result_store_slot")?;
        if state
            .as_ref()
            .is_some_and(|(held_dir, _)| same_runtime_dir(held_dir, runtime_dir))
        {
            *state = None;
        }
        Ok(())
    }

    /// Returns the open store for `runtime_dir` without opening a new one,
    /// only when the held slot matches `runtime_dir` and its `results`
    /// directory still exists.
    ///
    /// # Errors
    ///
    /// Returns `LockPoisoned` when the slot was poisoned.
    pub fn get_dir(&self, runtime_dir: &Path) -> Result<Option<Arc<ResultStore>>, LockPoisoned> {
        let state = lock(&self.state, "result_store_slot")?;
        Ok(state
            .as_ref()
            .filter(|(held_dir, _)| {
                same_runtime_dir(held_dir, runtime_dir) && runtime_dir.join("results").is_dir()
            })
            .map(|(_, store)| Arc::clone(store)))
    }

    /// Returns the currently held [`ResultStore`] (if any and its `results`
    /// directory still exists) so `decode_cursor` either matches its nonce or
    /// increments `unknown_cursors`.
    ///
    /// # Errors
    ///
    /// Returns `LockPoisoned` when the slot was poisoned.
    pub fn store_for_continuation(&self) -> Result<Option<Arc<ResultStore>>, LockPoisoned> {
        self.current()
    }

    /// Returns the currently held store when its `results` directory still
    /// exists on disk.
    ///
    /// # Errors
    ///
    /// Returns `LockPoisoned` when the slot was poisoned.
    pub fn current(&self) -> Result<Option<Arc<ResultStore>>, LockPoisoned> {
        let state = lock(&self.state, "result_store_slot")?;
        Ok(state
            .as_ref()
            .filter(|(held_dir, _)| held_dir.join("results").is_dir())
            .map(|(_, store)| Arc::clone(store)))
    }
}

#[cfg(test)]
#[path = "result_stores_tests.rs"]
mod single_project_tests;

#[cfg(test)]
#[path = "result_store_proof_exclusion_tests.rs"]
mod proof_exclusion_tests;

#[cfg(test)]
#[path = "result_store_proof_replacement_tests.rs"]
mod proof_replacement_tests;
