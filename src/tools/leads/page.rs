//! Stored-result page cutting and [`LeadsResponse`] assembly for `leads`.

pub(in crate::tools) use super::produce::Scope;
use super::produce::produce;
use super::stored::{Extras, StoredLead};
use super::{FileCount, LeadRows, LeadsResponse};
use crate::error_codes::store_error;
use crate::store::{Materialize, StoredRow};
use crate::sync::SourceSyncManager;
use crate::tools::context::internal_error;
use crate::tools::page::{PageLimit, PageSize};
use crate::tools::paginate::{
    CutStep, PageRequest, PageSpec, ServedRows, byte_cut_step, paginate_rows,
};
use rmcp::ErrorData;
use std::collections::HashMap;

const TOOL: &str = "leads";

#[derive(Clone, Copy)]
pub(super) struct Cut {
    pub(super) limit: Option<usize>,
    pub(super) size: PageSize,
    pub(super) requested_bytes: Option<usize>,
    pub(super) ceiling: usize,
    pub(super) store_limits: Option<crate::store::ResultStoreLimits>,
    pub(super) discard_on_partial: bool,
}

pub(super) fn page(
    manager: &SourceSyncManager,
    request: PageRequest<'_>,
    cut: Cut,
    scope: Option<&Scope<'_>>,
) -> Result<LeadsResponse, ErrorData> {
    let mut spec = PageSpec::typed::<StoredLead>(TOOL, cut.size.bytes);
    spec.requested_page_bytes = cut.requested_bytes;
    spec.ceiling = cut.ceiling;
    spec.store_limits = cut.store_limits;
    spec.discard_on_partial = cut.discard_on_partial;
    let served = paginate_rows::<Extras, StoredLead, _, _>(
        manager,
        &spec,
        request,
        |tx, sink| match scope {
            Some(scope) => produce(tx, sink, scope, cut.limit),
            None => Err(internal_error(
                "serving a leads continuation",
                "a continuation produces no rows",
            )),
        },
        |ctx| {
            let limit = cut.limit.or(ctx.extra.limit);
            let budget = ctx.page_bytes;
            Ok(move |row: &StoredRow, taken: usize, bytes_so_far: usize| {
                cut_lead_row(row, taken, bytes_so_far, limit, budget)
            })
        },
    )?;
    respond(served)
}

fn respond(served: ServedRows<StoredLead, Extras>) -> Result<LeadsResponse, ErrorData> {
    let rows = served.rows;
    let extras = served.meta;
    let page = served.page;
    let oversized_row = page.bytes_used > page.page_bytes;
    let files_covered = if page.complete {
        extras.files_total
    } else {
        match rows.last() {
            Some(row) => row.files_seen,
            None => 0,
        }
    };
    let page_file_counts: Vec<FileCount> = page_file_counts(rows.iter().map(|r| r.file.as_str()))?;
    let rows = if extras.compact {
        LeadRows::Compact(rows.into_iter().map(StoredLead::into_compact).collect())
    } else {
        LeadRows::Full(rows.into_iter().map(StoredLead::into_full).collect())
    };
    Ok(LeadsResponse {
        oversized_row,
        page,
        mechanic_counts: extras.counts,
        notices: extras.notices,
        files_not_parsed: extras.files_not_parsed,
        rows,
        files_total: extras.files_total,
        files_covered,
        page_file_counts,
    })
}

fn cut_lead_row(
    row: &StoredRow,
    taken: usize,
    bytes_so_far: usize,
    limit: Option<usize>,
    budget: usize,
) -> Result<CutStep<StoredLead>, ErrorData> {
    if taken > 0 && limit.is_some_and(|max_rows| taken >= max_rows) {
        return Ok(CutStep::Stop {
            limit: Some(PageLimit::Limit),
        });
    }
    let stored = StoredLead::from_values(&row.cells).map_err(|error| store_error(&error))?;
    let size = stored.response_bytes()?;
    Ok(byte_cut_step(stored, size, taken, bytes_so_far, budget))
}

pub(super) fn page_file_counts<'a>(
    files: impl Iterator<Item = &'a str>,
) -> Result<Vec<FileCount>, ErrorData> {
    let mut counts: Vec<FileCount> = Vec::new();
    let mut index_by_file: HashMap<&'a str, usize> = HashMap::new();
    for file in files {
        if let Some(&index) = index_by_file.get(file)
            && let Some(entry) = counts.get_mut(index)
        {
            entry.count = entry.count.checked_add(1).ok_or_else(|| {
                ErrorData::internal_error("a page's per-file row count overflowed", None)
            })?;
        } else {
            index_by_file.insert(file, counts.len());
            counts.push(FileCount {
                file: file.to_owned(),
                count: 1,
            });
        }
    }
    Ok(counts)
}
