//! The result store handle: open, cursor resolution, admission, statistics.

use super::cursor::{Cursor, ResultId};
use super::error::{ResultStoreError, UnsupportedPlatform};
use super::owner::{self, SweepOutcome};
use super::registry::{Entry, Refused, Registry, ResultMeta};
use super::stats::StoreStats;
use crate::util::lock_or_recover;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use uuid::Uuid;

/// Seconds since the Unix epoch, injectable so tests control time.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The two limits the store obeys.
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

/// Immutable paged results, one private SQLite file each. See the module docs.
pub struct ResultStore {
    dir: PathBuf,
    session: Uuid,
    nonce: Uuid,
    limits: ResultStoreLimits,
    clock: Clock,
    registry: Mutex<Registry>,
    sweep: SweepOutcome,
    _lock: File,
    lock_path: PathBuf,
}

impl ResultStore {
    /// Opens the store under `runtime_dir`, using the system clock.
    ///
    /// # Errors
    ///
    /// `Unavailable` when the directory or lock cannot be prepared, or on a
    /// platform without unlink-while-open semantics. A failed startup sweep
    /// does not fail the open; it shows in [`ResultStore::stats`].
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
        if !cfg!(unix) {
            return Err(ResultStoreError::unavailable(
                "open the result store",
                UnsupportedPlatform,
            ));
        }
        let owner = owner::open_owner(runtime_dir, session)?;
        Ok(Self {
            dir: owner.dir,
            session,
            nonce: Uuid::new_v4(),
            limits,
            clock,
            registry: Mutex::new(Registry::default()),
            sweep: owner.sweep,
            _lock: owner.lock,
            lock_path: owner.lock_path,
        })
    }

    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(super) fn session(&self) -> Uuid {
        self.session
    }

    pub(super) fn nonce(&self) -> Uuid {
        self.nonce
    }

    pub(super) fn limits(&self) -> ResultStoreLimits {
        self.limits
    }

    pub(super) fn now(&self) -> i64 {
        (self.clock)()
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        // Recovery is safe: every registry mutation is planned first and then
        // applied without a panicking step, so a poisoned lock never hides a
        // half-applied change.
        lock_or_recover(&self.registry, "result_registry")
    }

    /// Parses `token` and checks it was issued by this process.
    ///
    /// # Errors
    ///
    /// `Unknown` for a malformed token or another process's nonce.
    pub fn decode_cursor(&self, token: &str) -> Result<Cursor, ResultStoreError> {
        match Cursor::parse(token) {
            Some(cursor) if cursor.nonce == self.nonce => Ok(cursor),
            _ => Err(self.unknown_cursor()),
        }
    }

    pub(super) fn unknown_cursor(&self) -> ResultStoreError {
        self.registry().counters.unknown_cursors += 1;
        tracing::info!("cursor refused: unknown");
        ResultStoreError::Unknown
    }

    pub(super) fn lookup(&self, id: ResultId) -> Result<Arc<Entry>, ResultStoreError> {
        let mut registry = self.registry();
        if let Some(entry) = registry.entries.get(&id) {
            return Ok(Arc::clone(entry));
        }
        registry.counters.expired_cursors += 1;
        drop(registry);
        tracing::info!(result = %id, "cursor refused: the result was reclaimed");
        Err(ResultStoreError::Expired)
    }

    /// The name of the tool that produced `id`, read from the registry. It
    /// renews nothing, marks nothing and reads no SQL, so a caller can check
    /// a cursor's owner before any read touches the result.
    ///
    /// # Errors
    ///
    /// `Expired` when the result was reclaimed (counted once, as for a read).
    pub fn tool_of(&self, id: ResultId) -> Result<String, ResultStoreError> {
        Ok(self.lookup(id)?.meta.tool.clone())
    }

    /// What `id` is (tool, columns and the tool's stored record), read from
    /// the registry. Like [`ResultStore::tool_of`] it renews nothing and
    /// marks nothing.
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
        let (tool, revision, bytes) = (
            entry.meta.tool.clone(),
            entry.meta.revision_id.clone(),
            entry.bytes,
        );
        let (now, grace) = (self.now(), self.limits.active_grace_secs);
        let budget = self.limits.max_bytes;
        // The guard is a temporary of this statement: it is released before
        // anything is logged or dropped.
        let admitted = self.registry().admit(id, entry, budget, now, grace);
        let reclaimed = match admitted {
            Ok(reclaimed) => reclaimed,
            Err(refused) => {
                match &refused.error {
                    ResultStoreError::Full { active_count } => tracing::info!(
                        active_count,
                        bytes = refused.entry.bytes,
                        budget,
                        "result refused: the store is full"
                    ),
                    _ => tracing::info!(
                        result = %id,
                        bytes = refused.entry.bytes,
                        budget,
                        "result refused: larger than the whole budget"
                    ),
                }
                let Refused { error, entry } = refused;
                // Closes the refused entry's connection and file, lock released.
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
        // Dropping the entries closes their files, outside the registry lock.
        drop(reclaimed);
        tracing::info!(result = %id, tool = %tool, revision = %revision, bytes, class = "Active", "result materialised");
        Ok(())
    }

    /// Counts a writer that outgrew the whole budget.
    pub(super) fn refuse_too_large(&self, id: ResultId, bytes: u64) {
        self.registry().counters.refused_too_large += 1;
        tracing::info!(result = %id, bytes, budget = self.limits.max_bytes, "result aborted: larger than the whole budget");
    }

    /// A snapshot of what the store holds and has decided.
    #[must_use]
    pub fn stats(&self) -> StoreStats {
        let now = self.now();
        self.registry().snapshot(
            self.limits.max_bytes,
            now,
            self.limits.active_grace_secs,
            &self.sweep,
        )
    }
}

impl Drop for ResultStore {
    /// Removes the lock file while still holding the lock, then releases it.
    fn drop(&mut self) {
        match std::fs::remove_file(&self.lock_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(error = %error, "could not remove the result store lock file");
            }
        }
    }
}
