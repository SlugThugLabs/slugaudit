//! `leads` — unranked mechanical observations about an indexed project.

// Owner decision (2026-10-31): when the tool hands the model findings or a checklist, the model starts to treat running the tool as doing the audit, so built-in leads only count and never rank or give verdicts.

mod coverage;
pub(in crate::tools) mod custom;
pub(in crate::tools) mod mechanics;
pub(in crate::tools) mod page;
mod produce;
pub(in crate::tools) mod query;
mod stored;
mod types;

use super::mechanic::catalog::{self as mechanic_catalog, Catalog};
#[cfg(test)]
use super::page::LegacyPaging;
use super::page::PageSize;
use super::paginate::{PageRequest, page_request};
use super::tool_limits::deadline_after;
use super::{MAX_CURSOR_BYTES, check_bytes_limit, check_optional_path_bytes};
use crate::model::ResourceLimits;
use crate::progress::ProgressSink;
use crate::sync;
use mechanics::resolve_selected;
use page::Cut;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
pub use types::{
    CompactLeadRow, FileCount, LeadMode, LeadRow, LeadRows, LeadsRequest, LeadsResponse,
    MechanicCount,
};
pub(in crate::tools) use types::{MAX_DETAIL_CHARS, clip_detail};

const MAX_FILTER_BYTES: usize = 8 * 1024;

/// Unranked mechanical observations from the project's current revision.
///
/// # Errors
///
/// Returns an error on invalid paging/filter arguments, inactive project paths,
/// unknown mechanic ids, unknown/expired cursors, or database failures.
pub fn leads(
    request: &Parameters<LeadsRequest>,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<LeadsResponse>, ErrorData> {
    leads_with_optional_limits(&request.0, None, sink, manager).map(Json)
}

#[cfg(test)]
pub(super) fn leads_with_limits(
    request: &Parameters<LeadsRequest>,
    limits: &ResourceLimits,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<LeadsResponse>, ErrorData> {
    leads_with_optional_limits(&request.0, Some(limits), sink, manager).map(Json)
}

fn leads_with_optional_limits(
    request: &LeadsRequest,
    limits: Option<&ResourceLimits>,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<LeadsResponse, ErrorData> {
    validate_leads_request(request)?;
    let paging = page_request(request.cursor.as_deref(), request.path.as_deref(), sink)?;
    let (resolved_limits, catalog) = match paging {
        PageRequest::Cursor(_) => (ResourceLimits::default(), Catalog { custom: Vec::new() }),
        PageRequest::Fresh { path, .. } => {
            let root = super::project_root(path)?;
            let loaded_limits = match limits {
                Some(explicit) => *explicit,
                None => super::limits_for_root(&root)?,
            };
            let catalog = mechanic_catalog::load(manager.session_mechanics(), root.as_path())?;
            (loaded_limits, catalog)
        }
    };
    run_leads(request, paging, &resolved_limits, &catalog, false, manager)
}

#[cfg(test)]
pub(in crate::tools) fn leads_with_catalog(
    request: &LeadsRequest,
    limits: &ResourceLimits,
    catalog: &Catalog,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<LeadsResponse, ErrorData> {
    validate_leads_request(request)?;
    let paging = page_request(request.cursor.as_deref(), request.path.as_deref(), sink)?;
    run_leads(request, paging, limits, catalog, false, manager)
}

pub(in crate::tools) fn trial_leads_with_catalog(
    request: &LeadsRequest,
    limits: &ResourceLimits,
    catalog: &Catalog,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<LeadsResponse, ErrorData> {
    validate_leads_request(request)?;
    let paging = page_request(request.cursor.as_deref(), request.path.as_deref(), sink)?;
    run_leads(request, paging, limits, catalog, true, manager)
}

fn run_leads(
    request: &LeadsRequest,
    paging: PageRequest<'_>,
    limits: &ResourceLimits,
    catalog: &Catalog,
    discard_on_partial: bool,
    manager: &sync::SourceSyncManager,
) -> Result<LeadsResponse, ErrorData> {
    let size = PageSize::resolve(request.page_bytes, limits)?;
    let cut = Cut {
        limit: request.limit.map(|rows| rows.max(1)),
        size,
        requested_bytes: request.page_bytes,
        ceiling: limits.max_query_response_bytes,
        store_limits: matches!(paging, PageRequest::Fresh { .. })
            .then(|| crate::store::ResultStoreLimits::from(limits)),
        discard_on_partial,
    };
    let scope = match paging {
        PageRequest::Cursor(_) => None,
        PageRequest::Fresh { .. } => Some(page::Scope {
            catalog,
            selected: resolve_selected(request.mechanic.as_deref(), catalog)?,
            mode: request.mode,
            pattern: request.pattern.as_deref(),
            deadline: deadline_after(limits.max_structure_execution_time),
        }),
    };
    let response = page::page(manager, paging, cut, scope.as_ref())?;
    tracing::info!(
        revision_id = response.page.revision_id,
        row_count = response.page.returned,
        total = response.page.total,
        complete = response.page.complete,
        registered = catalog.custom.len(),
        "leads built"
    );
    Ok(response)
}

fn validate_leads_request(request: &LeadsRequest) -> Result<(), ErrorData> {
    request.legacy_paging.refuse()?;
    check_optional_path_bytes(request.path.as_deref())?;
    for (name, value, max_bytes) in [
        ("mechanic", request.mechanic.as_deref(), MAX_FILTER_BYTES),
        ("pattern", request.pattern.as_deref(), MAX_FILTER_BYTES),
        ("cursor", request.cursor.as_deref(), MAX_CURSOR_BYTES),
    ] {
        if let Some(text) = value {
            check_bytes_limit(text, name, max_bytes)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod not_parsed_tests;

#[cfg(test)]
mod page_tests;

#[cfg(test)]
mod line_tests;

#[cfg(test)]
mod marker_tests;

#[cfg(test)]
mod language_tests;

#[cfg(test)]
mod paged_tests;

#[cfg(test)]
mod paged_limit_tests;

#[cfg(test)]
mod detail_tests;
