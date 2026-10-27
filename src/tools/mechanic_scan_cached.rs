//! The registered mechanics' scan with the per-revision cache in front of it.
//!
//! One `leads` call needs the complete count of every entry it selects. For
//! each such entry this looks the scan up by (project, revision, file filter,
//! entry identity); the entries that miss are scanned together in one pass,
//! and each complete result is kept (rows and all) so the next call at the
//! same revision parses nothing and a page is cut from the cached rows
//! exactly as it would be from a fresh scan.
//!
//! Only a complete scan is cached: a spent time budget ends the pass early
//! ([`Scanned::stop`]) and nothing it found is stored, though the call may
//! still use the rows it holds. Rows are retained while the pass runs only up to the
//! cache budget in total across the pass's entries; an entry whose rows outgrow it is counted (`skipped_too_large`
//! in `health`), not cached, and its pages stream from the database as they
//! did before the cache existed.

use super::leads_query::MAX_DETAIL_CHARS;
use super::mechanic::MechanicSource;
use super::mechanic_catalog::Registered;
use super::mechanic_scan::{self, ScanEnd, Stop, Tally};
use crate::project::{CachedRow, MechanicScanCache, ScanKey};
use rmcp::ErrorData;
use rusqlite::Transaction;
use std::sync::Arc;
use std::time::Instant;

/// The cache and the call's place in it: which project and revision the
/// call is answering for.
#[derive(Clone, Copy)]
pub(super) struct CacheScope<'a> {
    pub(super) cache: &'a MechanicScanCache,
    /// The project database the call reads.
    pub(super) project: &'a str,
    /// The call's resolved revision.
    pub(super) revision: &'a str,
}

/// The cached rows of one entry, shared with the cache.
pub(super) type Rows = Arc<Vec<CachedRow>>;

/// The scan outcome for every entry of the catalogue.
pub(super) struct Scanned {
    /// Indexed like the catalogue's `custom`; an entry not selected keeps an
    /// empty tally.
    pub(super) tallies: Vec<Tally>,
    /// Indexed like `custom`: the entry's rows when they are held by the
    /// cache (or were just cached), `None` when not scanned or too large.
    pub(super) rows: Vec<Option<Rows>>,
    /// Set when the call deadline stopped the pass: the tallies and rows of
    /// the entries it scanned cover only the files read before the stop.
    pub(super) stop: Option<Stop>,
}

fn source_tag(source: MechanicSource) -> &'static str {
    match source {
        MechanicSource::Session => "session",
        MechanicSource::ProjectConfig => "project_config",
        MechanicSource::Builtin => "builtin",
    }
}

fn key_of(scope: &CacheScope<'_>, registered: &Registered, glob: Option<&str>) -> ScanKey {
    ScanKey {
        project: scope.project.to_owned(),
        revision: scope.revision.to_owned(),
        glob: glob.map(str::to_owned),
        id: registered.entry.id.clone(),
        language: registered.entry.language.clone(),
        query: registered.entry.query.clone(),
        source: source_tag(registered.source),
    }
}

/// A tally derived from cached rows, which are file-contiguous in path order.
fn tally_of(rows: &[CachedRow]) -> Tally {
    let mut tally = Tally::default();
    for row in rows {
        tally.record(&row.file);
    }
    tally
}

/// Rows being retained for one entry while the pass runs.
#[derive(Default)]
struct Retained {
    rows: Vec<CachedRow>,
    bytes: usize,
    overflowed: bool,
}

/// The rows one scan pass retains for caching, with one budget shared by
/// every entry of the pass: a call selecting many uncached entries holds at
/// most the cache budget in total, not the budget per entry. An entry whose
/// row would take the pass past the budget gives up its rows (and releases
/// their bytes); it is still counted, just not cached.
pub(super) struct Pass {
    budget: usize,
    bytes: usize,
    entries: Vec<Retained>,
}

impl Pass {
    pub(super) fn new(budget: usize, entries: usize) -> Self {
        Self {
            budget,
            bytes: 0,
            entries: (0..entries).map(|_| Retained::default()).collect(),
        }
    }

    /// Offers one row of entry `index` for retention.
    pub(super) fn offer(&mut self, index: usize, row: CachedRow) {
        let Some(keep) = self.entries.get_mut(index) else {
            return;
        };
        if keep.overflowed {
            return;
        }
        let size = row.estimated_bytes();
        if self.bytes.saturating_add(size) > self.budget {
            self.bytes = self.bytes.saturating_sub(keep.bytes);
            keep.bytes = 0;
            keep.overflowed = true;
            keep.rows = Vec::new();
        } else {
            self.bytes = self.bytes.saturating_add(size);
            keep.bytes = keep.bytes.saturating_add(size);
            keep.rows.push(row);
        }
    }

    /// Estimated bytes of the rows retained right now, over all entries
    /// (the seam the retention test reads).
    #[cfg(test)]
    pub(super) fn retained_bytes(&self) -> usize {
        self.bytes
    }

    /// Entry `index`'s rows, or `None` when it gave up retaining them.
    fn take(&mut self, index: usize) -> Option<Vec<CachedRow>> {
        let keep = self.entries.get_mut(index)?;
        if keep.overflowed {
            return None;
        }
        self.bytes = self.bytes.saturating_sub(keep.bytes);
        keep.bytes = 0;
        Some(std::mem::take(&mut keep.rows))
    }
}

/// Counts the entries of `custom` listed in `selected` under `glob`, serving
/// each from the cache when it can and scanning the rest in one pass.
///
/// # Errors
///
/// Fails like [`mechanic_scan::scan`]: a database read error or a grammar
/// that no longer loads. Nothing is cached on failure or when the deadline
/// stopped the pass.
pub(super) fn tally_cached(
    tx: &Transaction<'_>,
    custom: &[Registered],
    selected: &[usize],
    glob: Option<&str>,
    deadline: Option<Instant>,
    scope: &CacheScope<'_>,
) -> Result<Scanned, ErrorData> {
    let mut scanned = Scanned {
        tallies: vec![Tally::default(); custom.len()],
        rows: vec![None; custom.len()],
        stop: None,
    };
    let mut missed = Vec::new();
    for index in selected {
        let Some(registered) = custom.get(*index) else {
            continue;
        };
        match scope.cache.get(&key_of(scope, registered, glob)) {
            Some(rows) => {
                if let Some(slot) = scanned.tallies.get_mut(*index) {
                    *slot = tally_of(&rows);
                }
                if let Some(slot) = scanned.rows.get_mut(*index) {
                    *slot = Some(rows);
                }
            }
            None => missed.push(*index),
        }
    }
    if missed.is_empty() {
        return Ok(scanned);
    }
    let budget = scope.cache.budget();
    let mut pass = Pass::new(budget, custom.len());
    let end = mechanic_scan::scan(tx, custom, &missed, glob, deadline, &mut |hit| {
        let tally = scanned.tallies.get_mut(hit.entry).ok_or_else(|| {
            super::context::internal_error("counting mechanics", "a hit named no entry")
        })?;
        tally.record(hit.file);
        pass.offer(
            hit.entry,
            CachedRow {
                file: hit.file.to_owned(),
                line: hit.line,
                text: hit.text.chars().take(MAX_DETAIL_CHARS + 1).collect(),
            },
        );
        Ok(())
    })?;
    let stopped = match end {
        ScanEnd::Finished => false,
        ScanEnd::Stopped(stop) => {
            scanned.stop = Some(stop);
            true
        }
    };
    for index in missed {
        let Some(registered) = custom.get(index) else {
            continue;
        };
        let Some(rows) = pass.take(index) else {
            if !stopped {
                scope.cache.note_too_large();
            }
            continue;
        };
        let rows = Arc::new(rows);
        if !stopped {
            scope
                .cache
                .insert(key_of(scope, registered, glob), Arc::clone(&rows));
        }
        if let Some(slot) = scanned.rows.get_mut(index) {
            *slot = Some(rows);
        }
    }
    Ok(scanned)
}
