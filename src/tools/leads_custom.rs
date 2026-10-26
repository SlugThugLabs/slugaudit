// slugaudit-line-exception: approved-by=slugthug; reason=registered mechanics in leads: their counts and the one streaming page cut shared by the single-mechanic and whole-surface pages
//! Registered mechanics inside `leads`: their counts, one registered
//! mechanic's page, and the compact whole-surface page when registered
//! entries come before the built-ins.
//!
//! Split from `leads_page.rs` because a registered mechanic's rows come from
//! a Tree-sitter scan (`mechanic_scan`), not from one SQL statement, so its
//! paging is a streaming cut instead of `LIMIT`/`OFFSET`. The cut keeps the
//! built-in contract exactly: the caller's `limit` and the byte budget stop
//! the page, the first row is always returned, one row past `limit` is only
//! looked at to report `limited_by`, and `files_covered` counts the distinct
//! files of rows `0..offset + returned`.

use super::leads::{CompactLeadRow, LeadMode, LeadRow, LeadRows, MechanicCount, PageLimit};
use super::leads_compact;
use super::leads_covers::LanguagesRead;
use super::leads_page::{Paging, Prepared, Scope, count_of, files_of, seen};
use super::leads_query::{MAX_DETAIL_CHARS, clip_detail, compile_pattern, page_file_counts};
use super::mechanic_catalog::Catalog;
use super::mechanic_scan::{self, Flow, Hit, Tally};
use super::mechanic_scan_cached::Scanned;
use crate::util::serialized_len;
use rmcp::ErrorData;
use rusqlite::Transaction;
use std::collections::BTreeSet;
use std::ops::ControlFlow;

/// One count per scanned registered entry, in catalogue order (session,
/// then project_config), from the scan's tallies. An entry that was not
/// scanned is not listed: an empty tally is not a count of zero.
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

/// A streaming page cut with the same rules as `leads_query::fill_page`.
struct Collector<R> {
    skip: usize,
    limit: Option<usize>,
    budget: usize,
    rows: Vec<R>,
    bytes: usize,
    limited_by: PageLimit,
    done: bool,
    /// Distinct files of the rows consumed (skipped or kept), in order; rows
    /// of one mechanic are file-contiguous, so equal paths are adjacent.
    consumed_files: Vec<String>,
}

impl<R: serde::Serialize> Collector<R> {
    fn new(paging: Paging) -> Self {
        Self {
            skip: paging.offset,
            limit: paging.limit,
            budget: paging.budget_bytes,
            rows: Vec::new(),
            bytes: 0,
            limited_by: PageLimit::None,
            done: false,
            consumed_files: Vec::new(),
        }
    }

    /// Offers the next row of the scope; `Break` once the page is cut.
    fn offer(&mut self, file: &str, row: impl FnOnce() -> R) -> Flow {
        if self.done {
            return Ok(ControlFlow::Break(()));
        }
        if self.skip > 0 {
            self.skip -= 1;
            self.consume(file);
            return Ok(ControlFlow::Continue(()));
        }
        if self.limit.is_some_and(|limit| self.rows.len() >= limit) {
            self.limited_by = PageLimit::Limit;
            self.done = true;
            return Ok(ControlFlow::Break(()));
        }
        let row = row();
        let row_bytes = serialized_len(&row)
            .map_err(|error| super::context::internal_error("measuring a leads row", error))?;
        if !self.rows.is_empty() && self.bytes.saturating_add(row_bytes) > self.budget {
            self.limited_by = PageLimit::Bytes;
            self.done = true;
            return Ok(ControlFlow::Break(()));
        }
        self.bytes = self.bytes.saturating_add(row_bytes);
        self.rows.push(row);
        self.consume(file);
        Ok(ControlFlow::Continue(()))
    }

    fn consume(&mut self, file: &str) {
        if self.consumed_files.last().map(String::as_str) != Some(file) {
            self.consumed_files.push(file.to_owned());
        }
    }
}

/// Feeds one registered entry's hits, in path then position order, into
/// `collector` until the page is cut: from the cached rows when the scan
/// pass kept them, else by scanning again (an entry too large to cache).
fn collect<R: serde::Serialize>(
    tx: &Transaction<'_>,
    scope: &Scope<'_>,
    index: usize,
    scanned: &Scanned,
    collector: &mut Collector<R>,
    make: impl Fn(&Hit<'_>) -> R,
) -> Result<(), ErrorData> {
    if let Some(Some(cached)) = scanned.rows.get(index) {
        for row in cached.iter() {
            let hit = Hit {
                entry: index,
                file: &row.file,
                line: row.line,
                text: &row.text,
            };
            if collector.offer(&row.file, || make(&hit))?.is_break() {
                break;
            }
        }
        return Ok(());
    }
    let glob = compile_pattern(scope.pattern);
    let custom = &scope.catalog.custom;
    mechanic_scan::scan(
        tx,
        custom,
        &[index],
        glob.as_deref(),
        scope.deadline,
        &mut |hit| collector.offer(hit.file, || make(&hit)),
    )
}

fn full_row(hit: &Hit<'_>) -> LeadRow {
    // One character past the cap is enough for `clip_detail` to flag it.
    let mut row = LeadRow {
        file: hit.file.to_owned(),
        line: hit.line,
        detail: hit.text.chars().take(MAX_DETAIL_CHARS + 1).collect(),
        detail_clipped: false,
    };
    clip_detail(&mut row);
    row
}

/// One page of one registered mechanic, in either mode.
pub(super) fn single(
    tx: &Transaction<'_>,
    scope: &Scope<'_>,
    index: usize,
    mode: LeadMode,
    scanned: &Scanned,
    counts: &[MechanicCount],
) -> Result<Prepared, ErrorData> {
    let registered = scope.catalog.custom.get(index).ok_or_else(|| {
        super::context::internal_error("paging a registered mechanic", "no such entry")
    })?;
    let id = registered.entry.id.as_str();
    let (rows, consumed, limited_by, rows_bytes) = match mode {
        LeadMode::Full => {
            let mut collector = Collector::new(scope.paging);
            collect(tx, scope, index, scanned, &mut collector, full_row)?;
            let rows = LeadRows::Full(collector.rows);
            (
                rows,
                collector.consumed_files,
                collector.limited_by,
                collector.bytes,
            )
        }
        LeadMode::Compact => {
            let mut collector = Collector::new(scope.paging);
            collect(tx, scope, index, scanned, &mut collector, |hit| {
                CompactLeadRow {
                    mechanic: id.to_owned(),
                    file: hit.file.to_owned(),
                    line: hit.line,
                }
            })?;
            let rows = LeadRows::Compact(collector.rows);
            (
                rows,
                collector.consumed_files,
                collector.limited_by,
                collector.bytes,
            )
        }
    };
    let total = count_of(counts, id)?;
    let files_total = files_of(counts, id)?;
    let returned = row_count(&rows);
    let complete = seen(scope.paging.offset.saturating_add(returned))? >= total;
    let files_covered = if complete {
        files_total
    } else {
        to_i64(consumed.len())?
    };
    Ok(Prepared {
        page_file_counts: file_counts(&rows),
        rows,
        returned,
        total,
        files_total,
        files_covered,
        limited_by,
        rows_bytes,
    })
}

/// The compact page over every mechanic: registered entries in catalogue
/// order, then the built-ins in declaration order, cut as one sequence.
pub(super) fn whole_surface(
    tx: &Transaction<'_>,
    scope: &Scope<'_>,
    scanned: &Scanned,
    counts: &[MechanicCount],
) -> Result<Prepared, ErrorData> {
    let mut collector: Collector<CompactLeadRow> = Collector::new(scope.paging);
    let mut covered = BTreeSet::new();
    let mut all_files = BTreeSet::new();
    let mut registered_rows = 0_usize;
    for (index, (registered, tally)) in scope
        .catalog
        .custom
        .iter()
        .zip(&scanned.tallies)
        .enumerate()
    {
        all_files.extend(tally.file_list.iter().cloned());
        let count = usize::try_from(tally.count)
            .map_err(|error| super::context::internal_error("counting registered rows", error))?;
        registered_rows = registered_rows.saturating_add(count);
        if collector.done {
            continue;
        }
        if collector.skip >= count {
            collector.skip -= count;
            covered.extend(tally.file_list.iter().cloned());
            continue;
        }
        collector.consumed_files.clear();
        let id = registered.entry.id.as_str();
        collect(tx, scope, index, scanned, &mut collector, |hit| {
            CompactLeadRow {
                mechanic: id.to_owned(),
                file: hit.file.to_owned(),
                line: hit.line,
            }
        })?;
        covered.extend(collector.consumed_files.drain(..));
    }
    let builtins = leads_compact::scope(None);
    if !collector.done {
        let offset = std::mem::take(&mut collector.skip);
        let fetch = collector
            .limit
            .map(|limit| limit.saturating_sub(collector.rows.len()).saturating_add(1));
        leads_compact::stream(tx, &builtins, scope.pattern, offset, fetch, &mut |row| {
            let file = row.file.clone();
            collector.offer(&file, || row)
        })?;
    }
    all_files.extend(leads_compact::distinct_paths(
        tx,
        &builtins,
        scope.pattern,
        None,
    )?);
    let total = counts
        .iter()
        .fold(0_i64, |sum, count| sum.saturating_add(count.count));
    let files_total = to_i64(all_files.len())?;
    let returned = collector.rows.len();
    let through = scope.paging.offset.saturating_add(returned);
    let complete = seen(through)? >= total;
    let files_covered = if complete {
        files_total
    } else {
        let builtin_through = through.saturating_sub(registered_rows);
        if builtin_through > 0 {
            covered.extend(leads_compact::distinct_paths(
                tx,
                &builtins,
                scope.pattern,
                Some(builtin_through),
            )?);
        }
        to_i64(covered.len())?
    };
    let rows = LeadRows::Compact(collector.rows);
    Ok(Prepared {
        page_file_counts: file_counts(&rows),
        rows,
        returned,
        total,
        files_total,
        files_covered,
        limited_by: collector.limited_by,
        rows_bytes: collector.bytes,
    })
}

fn row_count(rows: &LeadRows) -> usize {
    match rows {
        LeadRows::Full(rows) => rows.len(),
        LeadRows::Compact(rows) => rows.len(),
    }
}

fn file_counts(rows: &LeadRows) -> Vec<super::leads::FileCount> {
    match rows {
        LeadRows::Full(rows) => page_file_counts(rows.iter().map(|row| row.file.as_str())),
        LeadRows::Compact(rows) => page_file_counts(rows.iter().map(|row| row.file.as_str())),
    }
}

fn to_i64(value: usize) -> Result<i64, ErrorData> {
    i64::try_from(value)
        .map_err(|error| super::context::internal_error("counting leads files", error))
}
