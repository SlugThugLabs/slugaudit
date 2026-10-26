// slugaudit-line-exception: approved-by=slugthug; reason=one leads envelope: the counts pass, the page dispatch and the completeness fields every page shape shares
//! Assembling one `leads` response from the counts and the page a request
//! asked for — the counts-only reply, the full row page, or the compact
//! whole-surface (or single-mechanic) enumeration.
//!
//! Split out of `leads.rs` so that file stays the declarative contract
//! (request and response shapes), and out of the two row modules so the
//! envelope — which completeness field means what — lives in exactly one
//! place, shared by both `mode`s. Nothing here ranks or interprets a row.

use super::leads::{FileCount, LeadMode, LeadRows, LeadsResponse, MechanicCount, PageLimit};
use super::leads_compact;
use super::leads_covers::languages_read;
use super::leads_custom;
use super::leads_mechanics::{Mechanic, Selected};
use super::leads_query::{self, compile_pattern, count_all, page_file_counts};
use super::mechanic_catalog::Catalog;
use super::mechanic_scan;
use rmcp::ErrorData;
use rusqlite::Transaction;
use std::time::Instant;

/// The paging window one call asks for: the caller's `limit`, the rows to
/// skip, and the response byte budget.
#[derive(Clone, Copy)]
pub(super) struct Paging {
    pub(super) limit: Option<usize>,
    pub(super) offset: usize,
    pub(super) budget_bytes: usize,
    /// The larger page size that the ceiling clamped, when it did.
    pub(super) clamped_from: Option<usize>,
}

/// Everything one call asks of the page builder.
pub(super) struct Scope<'a> {
    /// The registered entries (and failures) this call sees.
    pub(super) catalog: &'a Catalog,
    pub(super) selected: Option<Selected>,
    pub(super) mode: LeadMode,
    pub(super) pattern: Option<&'a str>,
    pub(super) paging: Paging,
    /// When the registered mechanics' scan must stop
    /// (`max_structure_execution_time`); `None` means no deadline.
    pub(super) deadline: Option<Instant>,
}

/// Everything one page contributes to the envelope; `complete` and
/// `next_offset` are derived from `returned` and `total` in [`envelope`].
pub(super) struct Prepared {
    pub(super) rows: LeadRows,
    pub(super) returned: usize,
    pub(super) total: i64,
    pub(super) files_total: i64,
    pub(super) files_covered: i64,
    pub(super) page_file_counts: Vec<FileCount>,
    pub(super) limited_by: PageLimit,
    pub(super) rows_bytes: usize,
}

/// The complete counts pass, independent of paging.
struct Counts {
    mechanic_counts: Vec<MechanicCount>,
    files_not_parsed: i64,
    mechanics_invalid: usize,
}

/// Counts plus the page the request asked for.
///
/// # Errors
///
/// Propagates any failure of the counts, row, or coverage reads, and of the
/// registered mechanics' scan; see [`leads_query`], [`leads_compact`] and
/// [`mechanic_scan`].
pub(super) fn build(
    tx: &Transaction<'_>,
    revision_id: &str,
    scope: &Scope<'_>,
) -> Result<LeadsResponse, ErrorData> {
    let glob = compile_pattern(scope.pattern);
    let read = languages_read(tx, glob.as_deref())?;
    let whole_surface = scope.mode == LeadMode::Compact && scope.selected.is_none();
    let tallies = mechanic_scan::tally(
        tx,
        &scope.catalog.custom,
        glob.as_deref(),
        scope.deadline,
        whole_surface,
    )?;
    let mut mechanic_counts = leads_custom::counts(scope.catalog, &tallies, &read);
    mechanic_counts.extend(count_all(tx, scope.pattern, &read)?);
    let counts = Counts {
        mechanic_counts,
        files_not_parsed: leads_query::files_not_parsed(tx, scope.pattern)?,
        mechanics_invalid: scope.catalog.invalid.len(),
    };
    let (pattern, paging) = (scope.pattern, scope.paging);
    let listed = &counts.mechanic_counts;
    let prepared = match (scope.mode, scope.selected) {
        (LeadMode::Full, None) => return Ok(counts_only(revision_id, counts, paging)),
        (LeadMode::Full, Some(Selected::Builtin(mechanic))) => {
            full(tx, mechanic, pattern, paging, listed)?
        }
        (LeadMode::Compact, Some(Selected::Builtin(mechanic))) => {
            compact(tx, Some(mechanic), pattern, paging, listed)?
        }
        (LeadMode::Compact, None) if scope.catalog.custom.is_empty() => {
            compact(tx, None, pattern, paging, listed)?
        }
        (LeadMode::Compact, None) => leads_custom::whole_surface(tx, scope, &tallies, listed)?,
        (mode, Some(Selected::Registered(index))) => {
            leads_custom::single(tx, scope, index, mode, listed)?
        }
    };
    envelope(revision_id, counts, prepared, paging)
}

/// One page of a single mechanic's full rows.
fn full(
    tx: &Transaction<'_>,
    mechanic: &Mechanic,
    pattern: Option<&str>,
    paging: Paging,
    counts: &[MechanicCount],
) -> Result<Prepared, ErrorData> {
    let page = leads_query::rows_for(
        tx,
        mechanic,
        pattern,
        paging.limit,
        paging.offset,
        paging.budget_bytes,
    )?;
    let returned = page.rows.len();
    let total = count_of(counts, mechanic.id)?;
    let files_total = files_of(counts, mechanic.id)?;
    let through = paging.offset.saturating_add(returned);
    let complete = seen(through)? >= total;
    // The page that ends the mechanic has seen every file by construction;
    // earlier pages ask the index how many distinct files their rows reach.
    let files_covered = if complete {
        files_total
    } else {
        leads_query::files_covered(tx, mechanic, pattern, through)?
    };
    let file_counts = page_file_counts(page.rows.iter().map(|row| row.file.as_str()));
    Ok(Prepared {
        rows: LeadRows::Full(page.rows),
        returned,
        total,
        files_total,
        files_covered,
        page_file_counts: file_counts,
        limited_by: page.limited_by,
        rows_bytes: page.bytes,
    })
}

/// One compact page over the requested mechanic, or over the whole catalogue
/// when none was requested.
fn compact(
    tx: &Transaction<'_>,
    selected: Option<&Mechanic>,
    pattern: Option<&str>,
    paging: Paging,
    counts: &[MechanicCount],
) -> Result<Prepared, ErrorData> {
    let scope = leads_compact::scope(selected);
    let page = leads_compact::rows_for(
        tx,
        &scope,
        pattern,
        paging.limit,
        paging.offset,
        paging.budget_bytes,
    )?;
    let returned = page.rows.len();
    let total = scope.iter().try_fold(0_i64, |sum, mechanic| {
        count_of(counts, mechanic.id).map(|count| sum.saturating_add(count))
    })?;
    let files_total = leads_compact::files_total(tx, &scope, pattern)?;
    let through = paging.offset.saturating_add(returned);
    let complete = seen(through)? >= total;
    let files_covered = if complete {
        files_total
    } else {
        leads_compact::files_covered(tx, &scope, pattern, through)?
    };
    let file_counts = page_file_counts(page.rows.iter().map(|row| row.file.as_str()));
    Ok(Prepared {
        rows: LeadRows::Compact(page.rows),
        returned,
        total,
        files_total,
        files_covered,
        page_file_counts: file_counts,
        limited_by: page.limited_by,
        rows_bytes: page.bytes,
    })
}

/// The counts-only reply: no mechanic requested in full mode, so there is no
/// scope to page and every completeness field is trivially empty.
fn counts_only(revision_id: &str, counts: Counts, paging: Paging) -> LeadsResponse {
    LeadsResponse {
        revision_id: revision_id.to_owned(),
        mechanic_counts: counts.mechanic_counts,
        mechanics_invalid: counts.mechanics_invalid,
        files_not_parsed: counts.files_not_parsed,
        rows: LeadRows::Full(Vec::new()),
        returned: 0,
        total: 0,
        complete: true,
        files_total: 0,
        files_covered: 0,
        page_file_counts: Vec::new(),
        limited_by: PageLimit::None,
        row_budget_bytes: paging.budget_bytes,
        page_bytes_clamped_from: paging.clamped_from,
        oversized_row: false,
        rows_bytes: 0,
        next_offset: None,
    }
}

/// Wraps a prepared page in the envelope, deriving `complete` and
/// `next_offset` from what the page actually returned.
fn envelope(
    revision_id: &str,
    counts: Counts,
    prepared: Prepared,
    paging: Paging,
) -> Result<LeadsResponse, ErrorData> {
    let through = paging.offset.saturating_add(prepared.returned);
    let complete = seen(through)? >= prepared.total;
    Ok(LeadsResponse {
        revision_id: revision_id.to_owned(),
        mechanic_counts: counts.mechanic_counts,
        mechanics_invalid: counts.mechanics_invalid,
        files_not_parsed: counts.files_not_parsed,
        rows: prepared.rows,
        returned: prepared.returned,
        total: prepared.total,
        complete,
        files_total: prepared.files_total,
        files_covered: prepared.files_covered,
        page_file_counts: prepared.page_file_counts,
        limited_by: prepared.limited_by,
        row_budget_bytes: paging.budget_bytes,
        page_bytes_clamped_from: paging.clamped_from,
        oversized_row: prepared.rows_bytes > paging.budget_bytes,
        rows_bytes: prepared.rows_bytes,
        next_offset: (!complete).then_some(through),
    })
}

/// The counts-pass entry for `id`. The catalogue and the counts come from the
/// same table, so a missing entry is a defect: the call fails rather than
/// reporting `total = 0` and a `complete` page.
fn entry_of<'a>(counts: &'a [MechanicCount], id: &str) -> Result<&'a MechanicCount, ErrorData> {
    counts
        .iter()
        .find(|entry| entry.mechanic == id)
        .ok_or_else(|| {
            super::context::internal_error(
                "reading leads counts",
                format!("no count was recorded for mechanic `{id}`"),
            )
        })
}

/// The row count the counts pass recorded for `id`.
pub(super) fn count_of(counts: &[MechanicCount], id: &str) -> Result<i64, ErrorData> {
    entry_of(counts, id).map(|entry| entry.count)
}

/// The distinct-file count the counts pass recorded for `id`.
pub(super) fn files_of(counts: &[MechanicCount], id: &str) -> Result<i64, ErrorData> {
    entry_of(counts, id).map(|entry| entry.files)
}

/// `usize` rows as an `i64`; a count SQLite cannot hold fails the call.
pub(super) fn seen(rows: usize) -> Result<i64, ErrorData> {
    i64::try_from(rows)
        .map_err(|error| super::context::internal_error("counting leads rows seen", error))
}
