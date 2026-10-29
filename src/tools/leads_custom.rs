//! Registered mechanics inside `leads`: their counts, and their rows fed into
//! the result being stored.
//!
//! Split from `leads_page.rs` because a registered mechanic's rows come from
//! a Tree-sitter scan (`mechanic_scan`), not from one SQL statement: they are
//! the rows the counting pass held (or the cache kept), or, for an entry too
//! large to hold, a second scan of that one entry.

use super::leads::{CompactLeadRow, LeadMode, LeadRow, MechanicCount};
use super::leads_covers::LanguagesRead;
use super::leads_page::{Feed, Scope};
use super::leads_query::{MAX_DETAIL_CHARS, clip_detail, compile_pattern};
use super::mechanic_catalog::Catalog;
use super::mechanic_scan::{self, ScanEnd, Tally, budget_error};
use super::mechanic_scan_cached::Scanned;
use rmcp::ErrorData;
use rusqlite::Transaction;

/// One count per scanned registered entry, in catalogue order (session,
/// then project_config), from the scan's tallies. An entry that was not
/// scanned is not listed: an empty tally is not a count of zero. When the
/// scan was stopped (`page.completeness` says so) a count covers only the
/// files read before the stop.
pub(super) fn counts(
    catalog: &Catalog,
    scanned: &[usize],
    tallies: &[Tally],
    read: &LanguagesRead,
) -> Vec<MechanicCount> {
    scanned
        .iter()
        .filter_map(|index| Some((catalog.custom.get(*index)?, tallies.get(*index)?)))
        .map(|(registered, tally)| MechanicCount {
            mechanic: registered.entry.id.clone(),
            source: registered.source,
            note: Some(registered.entry.note.clone()),
            count: tally.count,
            files: tally.files,
            covers: read.covers_language(&registered.entry.language),
        })
        .collect()
}

fn full_row(file: &str, line: i64, text: &str) -> LeadRow {
    // One character past the cap is enough for `clip_detail` to flag it.
    let mut row = LeadRow {
        file: file.to_owned(),
        line,
        detail: text.chars().take(MAX_DETAIL_CHARS + 1).collect(),
        detail_clipped: false,
    };
    clip_detail(&mut row);
    row
}

/// Feeds one registered entry's rows, in path then position order, into the
/// result: from the rows the counting pass held or the cache kept, else by
/// scanning that entry again.
///
/// # Errors
///
/// A store failure, a scan failure, or `mechanic_budget_exhausted` when the
/// second scan of an entry too large to hold is stopped by the call
/// deadline: its rows cannot be reported, and a result missing them would not
/// say so.
pub(super) fn rows(
    tx: &Transaction<'_>,
    scope: &Scope<'_>,
    index: usize,
    scanned: &Scanned,
    feed: &mut Feed<'_, '_, '_>,
) -> Result<(), ErrorData> {
    let registered = scope.catalog.custom.get(index).ok_or_else(|| {
        super::context::internal_error("reading a registered mechanic", "no such entry")
    })?;
    let id = registered.entry.id.as_str();
    let mode = scope.mode;
    let mut emit = |file: &str, line: i64, text: &str| match mode {
        LeadMode::Full => feed.full(full_row(file, line, text)),
        LeadMode::Compact => feed.compact(CompactLeadRow {
            mechanic: id.to_owned(),
            file: file.to_owned(),
            line,
        }),
    };
    if let Some(Some(held)) = scanned.rows.get(index) {
        for row in held.iter() {
            emit(&row.file, row.line, &row.text)?;
        }
        return Ok(());
    }
    rescan(tx, scope, index, &mut emit)
}

/// Scans one registered entry again, handing each hit to `emit` in path then
/// position order.
///
/// # Errors
///
/// A scan or `emit` failure, or `mechanic_budget_exhausted` when the call
/// deadline stops the scan: the rows it did not reach cannot be reported.
pub(super) fn rescan(
    tx: &Transaction<'_>,
    scope: &Scope<'_>,
    index: usize,
    emit: &mut dyn FnMut(&str, i64, &str) -> Result<(), ErrorData>,
) -> Result<(), ErrorData> {
    let registered = scope.catalog.custom.get(index).ok_or_else(|| {
        super::context::internal_error("rescanning a registered mechanic", "no such entry")
    })?;
    let glob = compile_pattern(scope.pattern);
    let end = mechanic_scan::scan(
        tx,
        &scope.catalog.custom,
        &[index],
        glob.as_deref(),
        scope.deadline,
        &mut |hit| emit(hit.file, hit.line, hit.text),
    )?;
    match end {
        ScanEnd::Finished => Ok(()),
        ScanEnd::Stopped(_) => Err(budget_error(&[registered.entry.id.as_str()])),
    }
}
