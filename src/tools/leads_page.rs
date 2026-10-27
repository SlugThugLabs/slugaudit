// slugaudit-line-exception: approved-by=slugthug; reason=one leads result: the producer that fills the store from one read transaction and the page assembly that serves it share one envelope and one cut rule
//! Assembling one `leads` response: the producer that streams the rows and the
//! counts out of one verified read transaction into the result store, and the
//! page assembly that serves any page of a stored result.
//!
//! A call without a cursor produces the whole result (counts, then every row
//! of the requested scope in its stable order) and serves page one; a call
//! with a cursor serves the stored page and reads nothing else. Either way the
//! page is cut here, by the `limit` and the byte budget (each the request's,
//! else the one page one used), measuring every row it is offered. Everything
//! that is not per-page (the counts, the file denominators) comes back from
//! the extras stored with the result, so every page of one result agrees.

use super::context::{PageRequest, PageSpec, Produced, RowSink, decode_rows, internal_error};
use super::leads::{FileCount, LeadMode, LeadRows, LeadsResponse};
use super::leads_compact;
use super::leads_covers::languages_read;
use super::leads_custom;
use super::leads_mechanics::Selected;
use super::leads_query::{self, compile_pattern, count_all, page_file_counts};
use super::mechanic_catalog::Catalog;
use super::mechanic_scan_cached::{self, CacheScope};
use super::page::{Completeness, PageLimit};
use super::page_size::PageSize;
use super::paginate::paginate_with;
use crate::error_codes::store_error;
use crate::project::MechanicScanCache;
use crate::store::{Materialize, StoredRow, Take};
use crate::sync::SourceSyncManager;
use rmcp::ErrorData;
use rusqlite::Transaction;
use std::time::Instant;

#[path = "leads_stored.rs"]
mod stored;
pub(super) use stored::Feed;
use stored::{Extras, StoredLead, count_of, stored_of, to_i64};

/// The tool name a `leads` cursor is bound to.
const TOOL: &str = "leads";

/// Everything a new request asks of the producer.
pub(super) struct Scope<'a> {
    /// The registered entries (and failures) this call sees.
    pub(super) catalog: &'a Catalog,
    pub(super) selected: Option<Selected>,
    pub(super) mode: LeadMode,
    pub(super) pattern: Option<&'a str>,
    /// When the registered mechanics' scan must stop
    /// (`max_structure_execution_time`); `None` means no deadline.
    pub(super) deadline: Option<Instant>,
    /// Where this call's registered-mechanic scans are cached.
    pub(super) cache: &'a MechanicScanCache,
}

/// How a page is cut: the caller's `limit` and the response byte budget.
#[derive(Clone, Copy)]
pub(super) struct Cut {
    pub(super) limit: Option<usize>,
    /// The byte budget this call resolved to, and whether it is the ceiling.
    pub(super) size: PageSize,
    /// Whether the request asked for a budget. A continuation that did not
    /// uses the one page one used, and `size` is then unused.
    pub(super) explicit_bytes: bool,
}

/// What the page cut learns while the reader offers rows to it.
#[derive(Default)]
struct Cutting {
    failure: Option<ErrorData>,
    stopped_by: Option<PageLimit>,
    taken: usize,
    bytes_used: usize,
    /// The row limit in force, known once the first row is offered.
    limit: Option<Option<usize>>,
}

/// Serves one page: a new request when `scope` is given (its rows are
/// produced and stored first), the stored page when the request is a cursor.
///
/// # Errors
///
/// Propagates any failure of the sync, the counts, row, or coverage reads, the
/// registered mechanics' scan, and the result store; see [`paginate_with`].
pub(super) fn page(
    manager: &SourceSyncManager,
    request: PageRequest<'_>,
    cut: Cut,
    scope: Option<&Scope<'_>>,
) -> Result<LeadsResponse, ErrorData> {
    let mut cutting = Cutting::default();
    let mut spec = PageSpec::typed::<StoredLead>(TOOL, cut.size.bytes);
    spec.offer_first_row = true;
    spec.requested_page_bytes = cut.explicit_bytes.then_some(cut.size.bytes);
    let served = paginate_with::<Extras, _, _>(
        manager,
        &spec,
        request,
        |tx, sink| match scope {
            Some(scope) => produce(tx, sink, scope, &cut),
            None => Err(internal_error(
                "serving a leads continuation",
                "a continuation produces no rows",
            )),
        },
        |budget| {
            let cutting = &mut cutting;
            move |row: &StoredRow| accept(cutting, cut.limit, budget, row)
        },
    )?;
    if let Some(error) = cutting.failure.take() {
        return Err(error);
    }
    let rows: Vec<StoredLead> = decode_rows(&served.rows)?;
    let extras = served.meta;
    let mut page = served.page;
    let bytes_used = cutting.bytes_used;
    page.bytes_used = bytes_used;
    let at_ceiling = if scope.is_some() || cut.explicit_bytes {
        cut.size.at_ceiling
    } else {
        extras.at_ceiling
    };
    page.limited_by = match (page.complete, cutting.stopped_by) {
        (true, _) => PageLimit::None,
        (false, Some(PageLimit::Limit)) => PageLimit::Limit,
        (false, _) if at_ceiling => PageLimit::Ceiling,
        (false, _) => PageLimit::PageBytes,
    };
    let oversized_row = bytes_used > page.page_bytes;
    let files_covered = if page.complete {
        extras.files_total
    } else {
        rows.last().map_or(0, |row| row.files_seen)
    };
    let files = rows.iter().map(|row| row.file.as_str());
    let page_file_counts: Vec<FileCount> = page_file_counts(files);
    let rows = if extras.compact {
        LeadRows::Compact(rows.into_iter().map(StoredLead::into_compact).collect())
    } else {
        LeadRows::Full(rows.into_iter().map(StoredLead::into_full).collect())
    };
    Ok(LeadsResponse {
        oversized_row,
        page,
        mechanic_counts: extras.counts.into_iter().map(count_of).collect(),
        mechanics_invalid: extras.mechanics_invalid,
        files_not_parsed: extras.files_not_parsed,
        rows,
        files_total: extras.files_total,
        files_covered,
        page_file_counts,
    })
}

/// The page cut for one offered row. Every row is offered, the first too
/// (its verdict is ignored by the reader), so the bytes of the page are
/// exactly the bytes of the rows taken.
fn accept(cutting: &mut Cutting, asked: Option<usize>, budget: usize, row: &StoredRow) -> Take {
    let stored = match StoredLead::from_values(&row.cells) {
        Ok(stored) => stored,
        Err(error) => {
            cutting.failure = Some(store_error(&error));
            return Take::No;
        }
    };
    let size = match stored.response_bytes() {
        Ok(size) => size,
        Err(error) => {
            cutting.failure = Some(error);
            return Take::No;
        }
    };
    // The request's `limit`, else the one page one was asked for.
    let limit = *cutting.limit.get_or_insert_with(|| {
        asked.or_else(|| {
            usize::try_from(stored.limit)
                .ok()
                .filter(|limit| *limit > 0)
        })
    });
    if cutting.taken > 0 {
        if limit.is_some_and(|limit| cutting.taken >= limit) {
            cutting.stopped_by = Some(PageLimit::Limit);
            return Take::No;
        }
        if cutting.bytes_used.saturating_add(size) > budget {
            cutting.stopped_by = Some(PageLimit::PageBytes);
            return Take::No;
        }
    }
    cutting.taken += 1;
    cutting.bytes_used = cutting.bytes_used.saturating_add(size);
    Take::Yes
}

/// Counts and rows from one read transaction: the counts pass, then every
/// row of the requested scope in its stable order.
fn produce(
    tx: &Transaction<'_>,
    sink: &mut RowSink<'_, '_>,
    scope: &Scope<'_>,
    cut: &Cut,
) -> Result<Produced<Extras>, ErrorData> {
    let glob = compile_pattern(scope.pattern);
    let read = languages_read(tx, glob.as_deref())?;
    let revision: String = tx
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| internal_error("reading the current revision", error))?;
    let project = tx
        .path()
        .ok_or_else(|| internal_error("locating the project database", "no file path"))?;
    let cache = CacheScope {
        cache: scope.cache,
        project,
        revision: &revision,
    };
    let scanned = scanned_entries(scope);
    let found = mechanic_scan_cached::tally_cached(
        tx,
        &scope.catalog.custom,
        &scanned,
        glob.as_deref(),
        scope.deadline,
        &cache,
    )?;
    let mut counts = leads_custom::counts(scope.catalog, &scanned, &found.tallies, &read);
    counts.extend(count_all(tx, scope.pattern, &read)?);
    let files_not_parsed = leads_query::files_not_parsed(tx, scope.pattern)?;
    let mut feed = Feed::new(sink, cut.limit)?;
    match (scope.mode, scope.selected) {
        (LeadMode::Full, None) => {}
        (LeadMode::Full, Some(Selected::Builtin(mechanic))) => {
            leads_query::stream_rows(tx, mechanic, scope.pattern, &mut |row| feed.full(row))?;
        }
        (LeadMode::Compact, Some(Selected::Builtin(mechanic))) => {
            let only = leads_compact::scope(Some(mechanic));
            leads_compact::stream(tx, &only, scope.pattern, &mut |row| feed.compact(row))?;
        }
        (LeadMode::Compact, None) => {
            for index in 0..scope.catalog.custom.len() {
                leads_custom::rows(tx, scope, index, &found, &mut feed)?;
            }
            let all = leads_compact::scope(None);
            leads_compact::stream(tx, &all, scope.pattern, &mut |row| feed.compact(row))?;
        }
        (_, Some(Selected::Registered(index))) => {
            leads_custom::rows(tx, scope, index, &found, &mut feed)?;
        }
    }
    let completeness = match found.stop {
        None => Completeness::Complete,
        Some(stop) => Completeness::Partial {
            limited_by: PageLimit::TimeBudget,
            resume_after_file: stop.resume_after_file,
            files_scanned: stop.files_scanned,
            files_total: stop.files_total,
        },
    };
    Ok(Produced {
        completeness,
        extra: Extras {
            counts: counts.into_iter().map(stored_of).collect(),
            mechanics_invalid: scope.catalog.invalid.len(),
            files_not_parsed,
            files_total: to_i64(feed.files())?,
            compact: scope.mode == LeadMode::Compact,
            at_ceiling: cut.size.at_ceiling,
        },
    })
}

/// The registered entries this call scans: every one when no mechanic is
/// selected (counts only, or the compact whole surface), only the selected
/// one, and none when a built-in is selected — so an entry the caller did not
/// ask for can neither slow nor stop the call.
fn scanned_entries(scope: &Scope<'_>) -> Vec<usize> {
    match scope.selected {
        None => (0..scope.catalog.custom.len()).collect(),
        Some(Selected::Builtin(_)) => Vec::new(),
        Some(Selected::Registered(index)) => vec![index],
    }
}
