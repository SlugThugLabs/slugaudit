//! The registered mechanics' scan with the per-revision cache in front of it.
//!
//! One `leads` call needs the complete count of every entry it selects. For
//! each such entry this looks the scan up by (project, revision, file filter,
//! entry identity); the entries that miss are scanned together in one pass,
//! and each complete result is kept (rows and all) so the next call at the
//! same revision parses nothing and a page is cut from the cached rows
//! exactly as it would be from a fresh scan.
//!
//! Only a complete scan is cached: a spent time budget fails the call before
//! anything is stored. Rows are retained while the pass runs only up to the
//! cache budget; an entry whose rows outgrow it is counted (`skipped_too_large`
//! in `health`), not cached, and its pages stream from the database as they
//! did before the cache existed.

use super::leads_query::MAX_DETAIL_CHARS;
use super::mechanic::MechanicSource;
use super::mechanic_catalog::Registered;
use super::mechanic_scan::{self, Tally};
use crate::project::{CachedRow, MechanicScanCache, ScanKey};
use rmcp::ErrorData;
use rusqlite::Transaction;
use std::ops::ControlFlow;
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
fn tally_of(rows: &[CachedRow], keep_files: bool) -> Tally {
    let mut tally = Tally::default();
    for row in rows {
        tally.record(&row.file, keep_files);
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

/// Counts the entries of `custom` listed in `selected` under `glob`, serving
/// each from the cache when it can and scanning the rest in one pass.
///
/// # Errors
///
/// Fails like [`mechanic_scan::scan`]: a database read error, a grammar that
/// no longer loads, or a spent budget. Nothing is cached on failure.
pub(super) fn tally_cached(
    tx: &Transaction<'_>,
    custom: &[Registered],
    selected: &[usize],
    glob: Option<&str>,
    deadline: Option<Instant>,
    keep_files: bool,
    scope: &CacheScope<'_>,
) -> Result<Scanned, ErrorData> {
    let mut scanned = Scanned {
        tallies: vec![Tally::default(); custom.len()],
        rows: vec![None; custom.len()],
    };
    let mut missed = Vec::new();
    for index in selected {
        let Some(registered) = custom.get(*index) else {
            continue;
        };
        match scope.cache.get(&key_of(scope, registered, glob)) {
            Some(rows) => {
                if let Some(slot) = scanned.tallies.get_mut(*index) {
                    *slot = tally_of(&rows, keep_files);
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
    let mut retained: Vec<Retained> = custom.iter().map(|_| Retained::default()).collect();
    mechanic_scan::scan(tx, custom, &missed, glob, deadline, &mut |hit| {
        let tally = scanned.tallies.get_mut(hit.entry).ok_or_else(|| {
            super::context::internal_error("counting mechanics", "a hit named no entry")
        })?;
        tally.record(hit.file, keep_files);
        if let Some(keep) = retained.get_mut(hit.entry)
            && !keep.overflowed
        {
            let row = CachedRow {
                file: hit.file.to_owned(),
                line: hit.line,
                text: hit.text.chars().take(MAX_DETAIL_CHARS + 1).collect(),
            };
            keep.bytes = keep.bytes.saturating_add(row.estimated_bytes());
            if keep.bytes > budget {
                keep.overflowed = true;
                keep.rows = Vec::new();
            } else {
                keep.rows.push(row);
            }
        }
        Ok(ControlFlow::Continue(()))
    })?;
    for index in missed {
        let (Some(registered), Some(keep)) = (custom.get(index), retained.get_mut(index)) else {
            continue;
        };
        if keep.overflowed {
            scope.cache.note_too_large();
            continue;
        }
        let rows = Arc::new(std::mem::take(&mut keep.rows));
        scope
            .cache
            .insert(key_of(scope, registered, glob), Arc::clone(&rows));
        if let Some(slot) = scanned.rows.get_mut(index) {
            *slot = Some(rows);
        }
    }
    Ok(scanned)
}
