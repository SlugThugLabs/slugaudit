//! The result store handle: limits, open, cursor resolution, admission, and
//! statistics.

use super::cursor::{Cursor, ResultId};
use super::error::ResultStoreError;
use super::owner::{self, OwnerLock};
use super::registry::{Entry, Refused, Registry, ResultMeta, StoreStats};
use super::sweep::SweepOutcome;
use crate::util::lock;
use crate::util::tally;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use uuid::Uuid;

/// The two resource bounds enforced by [`ResultStore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultStoreLimits {
    /// Total bytes of admitted results (`max_result_store_bytes`).
    pub max_bytes: u64,
    /// Seconds an unfinished result stays protected after its last page
    /// (`result_active_grace_secs`).
    pub active_grace_secs: u64,
}

impl From<&crate::model::ResourceLimits> for ResultStoreLimits {
    fn from(limits: &crate::model::ResourceLimits) -> Self {
        Self {
            max_bytes: limits.max_result_store_bytes,
            active_grace_secs: limits.result_active_grace_secs,
        }
    }
}

/// Seconds since the Unix epoch, injectable so tests control time.
pub type Clock = Arc<dyn Fn() -> Result<i64, crate::util::ClockError> + Send + Sync>;

/// Immutable paged results, one unlinked scratch SQLite file each.
pub struct ResultStore {
    dir: PathBuf,
    session: Uuid,
    nonce: Uuid,
    max_bytes: AtomicU64,
    active_grace_secs: AtomicU64,
    clock: Clock,
    registry: Mutex<Registry>,
    sweep: SweepOutcome,
    /// Declared after `registry` so all open result files close and unlink
    /// before the directory lock is released on drop.
    _lock: OwnerLock,
}

impl ResultStore {
    /// Opens the store under `runtime_dir`, using the system clock.
    ///
    /// # Errors
    ///
    /// `Unavailable` when the directory or lock cannot be prepared. A failed
    /// startup sweep does not fail the open; it shows in
    /// [`ResultStore::stats`].
    pub fn open(
        runtime_dir: &Path,
        session: Uuid,
        limits: ResultStoreLimits,
    ) -> Result<Self, ResultStoreError> {
        let clock: Clock = Arc::new(|| crate::clock::unix_seconds(SystemTime::now()));
        Self::open_with_clock(runtime_dir, session, limits, clock)
    }

    /// [`ResultStore::open`] with an explicit clock.
    ///
    /// # Errors
    ///
    /// As [`ResultStore::open`].
    pub fn open_with_clock(
        runtime_dir: &Path,
        session: Uuid,
        limits: ResultStoreLimits,
        clock: Clock,
    ) -> Result<Self, ResultStoreError> {
        let owner = owner::open_owner(runtime_dir, session)?;
        Ok(Self {
            dir: owner.dir,
            session,
            nonce: Uuid::new_v4(),
            max_bytes: AtomicU64::new(limits.max_bytes),
            active_grace_secs: AtomicU64::new(limits.active_grace_secs),
            clock,
            registry: Mutex::new(Registry::default()),
            sweep: owner.sweep,
            _lock: owner.lock,
        })
    }

    /// Updates the store's byte ceiling and active grace period so mid-session
    /// config or profile changes apply to subsequent admissions and sweeps.
    pub fn update_limits(&self, limits: ResultStoreLimits) {
        self.max_bytes.store(limits.max_bytes, Ordering::Relaxed);
        self.active_grace_secs
            .store(limits.active_grace_secs, Ordering::Relaxed);
    }

    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(super) fn session(&self) -> Uuid {
        self.session
    }

    pub(crate) fn nonce(&self) -> Uuid {
        self.nonce
    }

    pub(super) fn limits(&self) -> ResultStoreLimits {
        ResultStoreLimits {
            max_bytes: self.max_bytes.load(Ordering::Relaxed),
            active_grace_secs: self.active_grace_secs.load(Ordering::Relaxed),
        }
    }

    /// The store clock's Unix seconds; an unusable clock is `Clock`.
    pub(super) fn now(&self) -> Result<i64, ResultStoreError> {
        (self.clock)().map_err(ResultStoreError::clock)
    }

    /// The registry, locked. A poisoned registry is a `Lock` error (logged
    /// once, by `util::lock`): the store cannot serve results.
    fn registry(&self) -> Result<std::sync::MutexGuard<'_, Registry>, ResultStoreError> {
        Ok(lock(&self.registry, "result_registry")?)
    }

    fn tally_counter(&self, pick: fn(&mut Registry) -> &mut u64) -> Result<(), ResultStoreError> {
        let mut registry = self.registry()?;
        tally(pick(&mut registry));
        Ok(())
    }

    /// Parses `token` and checks it was issued by this store instance.
    ///
    /// # Errors
    ///
    /// `Unknown` for a malformed token or another store's nonce.
    pub fn decode_cursor(&self, token: &str) -> Result<Cursor, ResultStoreError> {
        match Cursor::parse(token) {
            Some(cursor) if cursor.nonce == self.nonce => Ok(cursor),
            _ => Err(self.unknown_cursor()),
        }
    }

    /// Counts an unknown cursor and returns `Unknown` (logged here) or the
    /// registry lock error.
    pub(super) fn unknown_cursor(&self) -> ResultStoreError {
        if let Err(error) = self.tally_counter(|reg| &mut reg.counters.unknown_cursors) {
            return error;
        }
        tracing::info!("cursor refused: unknown");
        ResultStoreError::Unknown
    }

    /// Counts a cursor refused because it belongs to another tool and returns
    /// `Unknown`, or the registry lock error. The caller logs the refusal.
    #[must_use]
    pub fn refuse_foreign_tool(&self) -> ResultStoreError {
        if let Err(error) = self.tally_counter(|reg| &mut reg.counters.unknown_cursors) {
            return error;
        }
        ResultStoreError::Unknown
    }

    pub(super) fn lookup(&self, id: ResultId) -> Result<Arc<Entry>, ResultStoreError> {
        let mut registry = self.registry()?;
        if let Some(entry) = registry.entries.get(&id) {
            return Ok(Arc::clone(entry));
        }
        tally(&mut registry.counters.expired_cursors);
        drop(registry);
        tracing::info!(result = %id, "cursor refused: the result was reclaimed");
        Err(ResultStoreError::Expired)
    }

    /// What `id` is (tool, columns and the tool's stored record), read from
    /// the registry. Renews nothing and marks nothing.
    ///
    /// # Errors
    ///
    /// `Expired` when the result was reclaimed (counted once, as for a read).
    pub fn meta_of(&self, id: ResultId) -> Result<Arc<ResultMeta>, ResultStoreError> {
        Ok(Arc::clone(&self.lookup(id)?.meta))
    }

    /// Admits a committed result, reclaiming under pressure. One log line for
    /// the materialisation and one per reclaim; never rows.
    pub(super) fn admit(&self, id: ResultId, entry: Entry) -> Result<(), ResultStoreError> {
        let meta = Arc::clone(&entry.meta);
        let bytes = entry.bytes;
        let limits = self.limits();
        let (now, grace) = (self.now()?, limits.active_grace_secs);
        let budget = limits.max_bytes;
        let admitted = self.registry()?.admit(id, entry, budget, now, grace);
        let reclaimed = match admitted {
            Ok(reclaimed) => reclaimed,
            Err(refused) => {
                if let ResultStoreError::Full { active_count } = &refused.error {
                    tracing::info!(
                        active_count,
                        bytes = refused.entry.bytes,
                        budget,
                        "result refused: the store is full"
                    );
                } else {
                    tracing::info!(
                        result = %id,
                        bytes = refused.entry.bytes,
                        budget,
                        "result refused: larger than the whole budget"
                    );
                }
                let Refused { error, entry } = refused;
                drop(entry);
                return Err(error);
            }
        };
        for victim in &reclaimed {
            tracing::info!(
                result = %victim.id,
                tool = %victim.entry.meta.tool,
                revision = %victim.entry.meta.revision_id,
                bytes = victim.entry.bytes,
                class = ?victim.class,
                "result reclaimed"
            );
        }
        drop(reclaimed);
        tracing::info!(result = %id, tool = %meta.tool, revision = %meta.revision_id, bytes, class = "Active", "result materialised");
        Ok(())
    }

    /// Counts a writer that outgrew the whole budget and returns `TooLarge`,
    /// or the registry lock error.
    pub(super) fn refuse_too_large(&self, id: ResultId, bytes: u64) -> ResultStoreError {
        if let Err(error) = self.tally_counter(|reg| &mut reg.counters.refused_too_large) {
            return error;
        }
        let budget = self.limits().max_bytes;
        tracing::info!(result = %id, bytes, budget, "result aborted: larger than the whole budget");
        ResultStoreError::TooLarge
    }

    /// A snapshot of what the store holds and has decided.
    ///
    /// # Errors
    ///
    /// `Lock` when the registry is poisoned, `Clock` when the clock is
    /// unusable.
    pub fn stats(&self) -> Result<StoreStats, ResultStoreError> {
        let now = self.now()?;
        let limits = self.limits();
        self.registry()?
            .snapshot(limits.max_bytes, now, limits.active_grace_secs, &self.sweep)
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
