// slugaudit-line-exception: approved-by=slugthuglabs; reason=cohesive per-revision mechanic scan cache with byte-budget eviction and database-instance invalidation
//! The scan results of registered mechanics, kept for the revision they were
//! computed at so a second `leads` call selecting the same entry parses
//! nothing.
//!
//! Held by the server's `SourceSyncManager` beside the session mechanics
//! (clones share it) and never written anywhere. An entry is valid for
//! exactly one project revision: a lookup or an insert at a different
//! revision of the same project drops that project's older entries first, so
//! a stale result can neither be served nor linger. The key also carries the
//! entry's identity (id, language, query, source) and the file filter, so a
//! removed-and-re-added id with a different query never meets an old
//! result.
//!
//! The cache is bounded by a byte budget. The size of a result is estimated,
//! not measured: for every row its path bytes plus its detail bytes plus
//! [`ROW_OVERHEAD_BYTES`], and for the entry the key's string bytes plus
//! [`ENTRY_OVERHEAD_BYTES`]. An insert evicts oldest-first until the new
//! entry fits; an entry larger than the whole budget is not cached and is
//! counted instead.

use crate::util::{LockPoisoned, lock};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Default budget: the same size as the default query response ceiling.
pub const DEFAULT_MECHANIC_CACHE_BYTES: usize = 32 * 1024 * 1024;

/// Fixed bytes charged per cached row on top of its path and detail text:
/// two `String` headers, the line number and allocator slack.
pub const ROW_OVERHEAD_BYTES: usize = 64;

/// Fixed bytes charged per cached entry on top of its key text and rows.
pub const ENTRY_OVERHEAD_BYTES: usize = 256;

/// One captured node of a cached scan, in the scan's path then position order.
#[derive(Debug, Clone)]
pub struct CachedRow {
    /// Project-relative path.
    pub file: String,
    /// One-based start line of the captured node.
    pub line: i64,
    /// The captured node's text, already clipped to the detail cap.
    pub text: String,
}

impl CachedRow {
    /// The estimated bytes this row occupies in the cache.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        // KEEP saturating: an estimate past usize::MAX is far over any budget and is only ever compared with one
        self.file
            .len()
            .saturating_add(self.text.len())
            // KEEP saturating: same clamp as the addition above
            .saturating_add(ROW_OVERHEAD_BYTES)
    }
}

/// What a cached scan is valid for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanKey {
    /// The project database the scan read (one manager can serve several).
    pub project: String,
    /// The revision the scan answered for.
    pub revision: String,
    /// The `pattern` glob the scan was limited to, if any.
    pub glob: Option<String>,
    /// The entry's id.
    pub id: String,
    /// The entry's language.
    pub language: String,
    /// The entry's query source.
    pub query: String,
    /// Where the entry came from: `session` for a registered mechanic, `builtin` for a shipped one.
    pub source: &'static str,
}

impl ScanKey {
    fn estimated_bytes(&self) -> usize {
        // KEEP saturating: an estimate past usize::MAX is far over any budget and is only ever compared with one
        [
            self.project.len(),
            self.revision.len(),
            match &self.glob {
                Some(glob) => glob.len(),
                None => 0,
            },
            self.id.len(),
            self.language.len(),
            self.query.len(),
            self.source.len(),
        ]
        .into_iter()
        // KEEP saturating: same clamp as the estimate above
        .fold(ENTRY_OVERHEAD_BYTES, usize::saturating_add)
    }
}

/// A snapshot of the cache for `health`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Number of cached scans currently held.
    pub entries: usize,
    /// Total bytes the held scans account for.
    pub bytes: usize,
    /// Byte budget the cache evicts down to.
    pub budget: usize,
    /// Scans evicted to stay within the budget since the cache was created.
    pub evictions: u64,
    /// Scans not cached because a single scan exceeded the budget.
    pub skipped_too_large: u64,
    /// Lookups answered from the cache.
    pub hits: u64,
    /// Lookups that found no usable cached scan.
    pub misses: u64,
}

struct Stored {
    key: ScanKey,
    rows: Arc<Vec<CachedRow>>,
    bytes: usize,
}

struct Inner {
    /// Oldest first.
    entries: VecDeque<Stored>,
    /// Last observed per-database instance token (`project.project_id`) per
    /// project database path. When a database is rebuilt externally (producing
    /// a fresh `"rev-1"` with a new instance token), noticing the change drops
    /// every cached entry for that project so stale rows cannot be served.
    project_instances: HashMap<String, String>,
    bytes: usize,
    budget: usize,
    evictions: u64,
    skipped_too_large: u64,
    hits: u64,
    misses: u64,
}

impl Inner {
    /// Drops this project's entries of any revision other than `revision`.
    fn drop_stale(&mut self, project: &str, revision: &str) {
        let before = self.entries.len();
        self.entries
            .retain(|stored| stored.key.project != project || stored.key.revision == revision);
        if self.entries.len() != before {
            self.recount_bytes();
        }
    }

    /// Sets `bytes` to the sum of the held entries' sizes.
    fn recount_bytes(&mut self) {
        self.bytes = self.entries.iter().map(|stored| stored.bytes).sum();
    }
}

/// The shared cache; clones share the same state.
#[derive(Clone)]
pub struct MechanicScanCache {
    inner: Arc<Mutex<Inner>>,
}

impl Default for MechanicScanCache {
    fn default() -> Self {
        Self::with_budget(DEFAULT_MECHANIC_CACHE_BYTES)
    }
}

impl MechanicScanCache {
    /// A cache that holds at most `budget` estimated bytes.
    #[must_use]
    pub fn with_budget(budget: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                entries: VecDeque::new(),
                project_instances: HashMap::new(),
                bytes: 0,
                budget,
                evictions: 0,
                skipped_too_large: 0,
                hits: 0,
                misses: 0,
            })),
        }
    }

    /// Drops every cached entry for `project` (for example after a database
    /// rebuild or `project_control off`/`on`, where SQLite's `rev-1` sequence
    /// starts over at the same database path).
    ///
    /// # Errors
    ///
    /// Returns [`LockPoisoned`] when the cache was poisoned.
    pub fn clear_project(&self, project: &Path) -> Result<(), LockPoisoned> {
        let mut inner = lock(&self.inner, "mechanic_scan_cache")?;
        let project_key = project.to_string_lossy();
        inner.project_instances.remove(project_key.as_ref());
        inner
            .entries
            .retain(|stored| stored.key.project != project_key);
        inner.recount_bytes();
        Ok(())
    }

    /// Records the database instance token (`project.project_id`) currently
    /// backing `project`. If a different instance token was previously seen
    /// for `project` (e.g. another process rebuilt `project.db` and restarted
    /// at `"rev-1"`), every cached entry for `project` is dropped immediately.
    ///
    /// # Errors
    ///
    /// Returns [`LockPoisoned`] when the cache was poisoned.
    pub fn sync_project_instance(
        &self,
        project: &Path,
        instance_id: &str,
    ) -> Result<(), LockPoisoned> {
        let mut inner = lock(&self.inner, "mechanic_scan_cache")?;
        let project_key = project.to_string_lossy();
        let changed = inner
            .project_instances
            .get(project_key.as_ref())
            .is_some_and(|previous| previous != instance_id);
        if changed {
            inner
                .entries
                .retain(|stored| stored.key.project != project_key);
            inner.recount_bytes();
        }
        inner
            .project_instances
            .insert(project_key.into_owned(), instance_id.to_owned());
        Ok(())
    }

    /// The byte budget; a caller collecting rows to cache stops keeping them
    /// once their estimate passes it.
    ///
    /// # Errors
    ///
    /// Returns [`LockPoisoned`] when the cache was poisoned.
    pub fn budget(&self) -> Result<usize, LockPoisoned> {
        Ok(lock(&self.inner, "mechanic_scan_cache")?.budget)
    }

    /// The rows cached for exactly `key`, if any. A lookup at a new revision
    /// of the key's project first drops that project's older entries.
    ///
    /// # Errors
    ///
    /// Returns [`LockPoisoned`] when the cache was poisoned.
    pub fn get(&self, key: &ScanKey) -> Result<Option<Arc<Vec<CachedRow>>>, LockPoisoned> {
        let mut inner = lock(&self.inner, "mechanic_scan_cache")?;
        inner.drop_stale(&key.project, &key.revision);
        let found = inner
            .entries
            .iter()
            .find(|stored| stored.key == *key)
            .map(|stored| Arc::clone(&stored.rows));
        if found.is_some() {
            crate::util::tally(&mut inner.hits);
        } else {
            crate::util::tally(&mut inner.misses);
        }
        Ok(found)
    }

    /// Caches the complete result of one scan, evicting oldest-first to fit;
    /// a result larger than the whole budget is counted and not cached.
    ///
    /// # Errors
    ///
    /// Returns [`LockPoisoned`] when the cache was poisoned.
    pub fn insert(&self, key: ScanKey, rows: Arc<Vec<CachedRow>>) -> Result<(), LockPoisoned> {
        // KEEP saturating: an estimate past usize::MAX exceeds the budget, so the scan is skipped as too large and never stored
        let bytes = rows
            .iter()
            .map(CachedRow::estimated_bytes)
            // KEEP saturating: same clamp as the estimate above
            .fold(key.estimated_bytes(), usize::saturating_add);
        let mut inner = lock(&self.inner, "mechanic_scan_cache")?;
        inner.drop_stale(&key.project, &key.revision);
        if bytes > inner.budget {
            crate::util::tally(&mut inner.skipped_too_large);
            return Ok(());
        }
        if let Some(position) = inner.entries.iter().position(|stored| stored.key == key) {
            inner.entries.remove(position);
            inner.recount_bytes();
        }
        // `bytes` fits the budget alone, so once the cache is empty the sum
        // always fits; a sum past `usize::MAX` is over the budget like any other.
        let total = loop {
            match inner.bytes.checked_add(bytes) {
                Some(total) if total <= inner.budget => break total,
                _ => {}
            }
            if inner.entries.pop_front().is_some() {
                crate::util::tally(&mut inner.evictions);
            }
            inner.recount_bytes();
            if inner.entries.is_empty() {
                break bytes;
            }
        };
        inner.bytes = total;
        inner.entries.push_back(Stored { key, rows, bytes });
        Ok(())
    }

    /// Counts a scan whose rows outgrew the budget while being collected, so
    /// it was never offered to [`MechanicScanCache::insert`].
    ///
    /// # Errors
    ///
    /// Returns [`LockPoisoned`] when the cache was poisoned.
    pub fn note_too_large(&self) -> Result<(), LockPoisoned> {
        let mut inner = lock(&self.inner, "mechanic_scan_cache")?;
        crate::util::tally(&mut inner.skipped_too_large);
        Ok(())
    }

    /// Drops every entry of mechanic `id`, whatever its revision or identity.
    ///
    /// # Errors
    ///
    /// Returns [`LockPoisoned`] when the cache was poisoned.
    pub fn remove_id(&self, id: &str) -> Result<(), LockPoisoned> {
        let mut inner = lock(&self.inner, "mechanic_scan_cache")?;
        inner.entries.retain(|stored| stored.key.id != id);
        inner.bytes = inner.entries.iter().map(|stored| stored.bytes).sum();
        Ok(())
    }

    /// A snapshot of the counters.
    ///
    /// # Errors
    ///
    /// Returns [`LockPoisoned`] when the cache was poisoned.
    pub fn stats(&self) -> Result<CacheStats, LockPoisoned> {
        let inner = lock(&self.inner, "mechanic_scan_cache")?;
        Ok(CacheStats {
            entries: inner.entries.len(),
            bytes: inner.bytes,
            budget: inner.budget,
            evictions: inner.evictions,
            skipped_too_large: inner.skipped_too_large,
            hits: inner.hits,
            misses: inner.misses,
        })
    }
}

#[cfg(test)]
#[path = "mechanic_scan_cache_tests.rs"]
mod tests;
