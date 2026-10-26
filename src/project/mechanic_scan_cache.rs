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
//! removed-and-re-added id with a different query, or a persisted entry
//! edited by another process, never meets an old result.
//!
//! The cache is bounded by a byte budget. The size of a result is estimated,
//! not measured: for every row its path bytes plus its detail bytes plus
//! [`ROW_OVERHEAD_BYTES`], and for the entry the key's string bytes plus
//! [`ENTRY_OVERHEAD_BYTES`]. An insert evicts oldest-first until the new
//! entry fits; an entry larger than the whole budget is not cached and is
//! counted instead.

use crate::util::lock_or_recover;
use std::collections::VecDeque;
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
        self.file
            .len()
            .saturating_add(self.text.len())
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
    /// Where the entry came from (`session` or `project_config`).
    pub source: &'static str,
}

impl ScanKey {
    fn estimated_bytes(&self) -> usize {
        [
            self.project.len(),
            self.revision.len(),
            self.glob.as_ref().map_or(0, String::len),
            self.id.len(),
            self.language.len(),
            self.query.len(),
            self.source.len(),
        ]
        .into_iter()
        .fold(ENTRY_OVERHEAD_BYTES, usize::saturating_add)
    }
}

/// A snapshot of the cache for `health`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: usize,
    pub budget: usize,
    pub evictions: u64,
    pub skipped_too_large: u64,
    pub hits: u64,
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
            self.bytes = self.entries.iter().map(|stored| stored.bytes).sum();
        }
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
                bytes: 0,
                budget,
                evictions: 0,
                skipped_too_large: 0,
                hits: 0,
                misses: 0,
            })),
        }
    }

    /// The byte budget; a caller collecting rows to cache stops keeping them
    /// once their estimate passes it.
    #[must_use]
    pub fn budget(&self) -> usize {
        lock_or_recover(&self.inner, "mechanic_scan_cache").budget
    }

    /// The rows cached for exactly `key`, if any. A lookup at a new revision
    /// of the key's project first drops that project's older entries.
    #[must_use]
    pub fn get(&self, key: &ScanKey) -> Option<Arc<Vec<CachedRow>>> {
        let mut inner = lock_or_recover(&self.inner, "mechanic_scan_cache");
        inner.drop_stale(&key.project, &key.revision);
        let found = inner
            .entries
            .iter()
            .find(|stored| stored.key == *key)
            .map(|stored| Arc::clone(&stored.rows));
        if found.is_some() {
            inner.hits = inner.hits.saturating_add(1);
        } else {
            inner.misses = inner.misses.saturating_add(1);
        }
        found
    }

    /// Caches the complete result of one scan, evicting oldest-first to fit;
    /// a result larger than the whole budget is counted and not cached.
    pub fn insert(&self, key: ScanKey, rows: Arc<Vec<CachedRow>>) {
        let bytes = rows
            .iter()
            .map(CachedRow::estimated_bytes)
            .fold(key.estimated_bytes(), usize::saturating_add);
        let mut inner = lock_or_recover(&self.inner, "mechanic_scan_cache");
        inner.drop_stale(&key.project, &key.revision);
        if bytes > inner.budget {
            inner.skipped_too_large = inner.skipped_too_large.saturating_add(1);
            return;
        }
        if let Some(position) = inner.entries.iter().position(|stored| stored.key == key)
            && let Some(old) = inner.entries.remove(position)
        {
            inner.bytes = inner.bytes.saturating_sub(old.bytes);
        }
        while inner.bytes.saturating_add(bytes) > inner.budget {
            let Some(oldest) = inner.entries.pop_front() else {
                break;
            };
            inner.bytes = inner.bytes.saturating_sub(oldest.bytes);
            inner.evictions = inner.evictions.saturating_add(1);
        }
        inner.bytes = inner.bytes.saturating_add(bytes);
        inner.entries.push_back(Stored { key, rows, bytes });
    }

    /// Counts a scan whose rows outgrew the budget while being collected, so
    /// it was never offered to [`MechanicScanCache::insert`].
    pub fn note_too_large(&self) {
        let mut inner = lock_or_recover(&self.inner, "mechanic_scan_cache");
        inner.skipped_too_large = inner.skipped_too_large.saturating_add(1);
    }

    /// Drops every entry of mechanic `id`, whatever its revision or identity.
    pub fn remove_id(&self, id: &str) {
        let mut inner = lock_or_recover(&self.inner, "mechanic_scan_cache");
        inner.entries.retain(|stored| stored.key.id != id);
        inner.bytes = inner.entries.iter().map(|stored| stored.bytes).sum();
    }

    /// A snapshot of the counters.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        let inner = lock_or_recover(&self.inner, "mechanic_scan_cache");
        CacheStats {
            entries: inner.entries.len(),
            bytes: inner.bytes,
            budget: inner.budget,
            evictions: inner.evictions,
            skipped_too_large: inner.skipped_too_large,
            hits: inner.hits,
            misses: inner.misses,
        }
    }
}

#[cfg(test)]
#[path = "mechanic_scan_cache_tests.rs"]
mod tests;
