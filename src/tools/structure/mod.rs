//! The `structure` tool contract: request/response types, input validation,
//! and cursor-paged Tree-sitter query execution.

mod exec;
mod match_node;
mod runner;
pub(in crate::tools) mod source;
mod stored;
mod types;
mod validate;
mod window;

use super::page::PageSize;
use super::paginate::{PageSpec, page_request, paginate_rows};
use crate::model::ResourceLimits;
use crate::store::StoredRow;
use crate::sync;
use exec::produce;
#[cfg(test)]
use match_node::MAX_TEXT_BYTES;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use source::MAX_MULTI_FILE_TOTAL_BYTES;
use stored::{StoredMatch, cut_match_row, structure_response};
pub(super) use types::StructureMatch;
pub use types::{StructureRequest, StructureResponse};
use validate::{validate_content, validate_entry};

/// # Errors
///
/// Returns an error if `request.path` isn't an active project, `request.file`
/// isn't indexed with a detected, pack-supported language, the query text
/// is empty/too large or fails to compile, the execution budget is already
/// exhausted, the request carries `offset` or `expected_revision`, or `cursor`
/// names no readable stored result.
pub fn structure(
    request: &Parameters<StructureRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<StructureResponse>, ErrorData> {
    let request_path = request.0.path.as_str();
    validate_entry(&request.0)?;
    let limits = if request.0.cursor.is_none() && !request_path.is_empty() {
        super::resolve_limits(request_path)?
    } else {
        ResourceLimits::default()
    };
    serve(request, &limits, MAX_MULTI_FILE_TOTAL_BYTES, sink, manager)
}

#[cfg(test)]
pub(super) fn structure_with_limits(
    request: &Parameters<StructureRequest>,
    limits: &ResourceLimits,
    window_bytes: usize,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<StructureResponse>, ErrorData> {
    validate_entry(&request.0)?;
    serve(request, limits, window_bytes, sink, manager)
}

fn serve(
    request: &Parameters<StructureRequest>,
    limits: &ResourceLimits,
    window_bytes: usize,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<StructureResponse>, ErrorData> {
    let request = &request.0;
    let page = PageSize::resolve(request.page_bytes, limits)?;
    let path = (!request.path.is_empty()).then_some(request.path.as_str());
    let page_request = page_request(request.cursor.as_deref(), path, sink)?;
    if request.cursor.is_none() {
        validate_content(request, limits)?;
    }
    let mut spec = PageSpec::typed::<StoredMatch>("structure", page.bytes);
    spec.requested_page_bytes = request.page_bytes;
    spec.ceiling = limits.max_query_response_bytes;
    spec.max_rows = Some(limits.max_structure_matches.max(1));
    spec.store_limits = matches!(page_request, super::paginate::PageRequest::Fresh { .. })
        .then(|| crate::store::ResultStoreLimits::from(limits));
    let served = paginate_rows(
        manager,
        &spec,
        page_request,
        |tx, rows| produce(tx, rows, request, limits, window_bytes),
        |ctx| {
            let budget = ctx.page_bytes;
            Ok(move |row: &StoredRow, taken: usize, bytes_so_far: usize| {
                cut_match_row(row, taken, bytes_so_far, budget)
            })
        },
    )?;
    Ok(structure_response(served))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod refusal_tests;

#[cfg(test)]
mod limit_tests;

#[cfg(test)]
mod limit_query_tests;

#[cfg(test)]
mod parse_bounds_tests;

#[cfg(test)]
mod language_tests;

#[cfg(test)]
mod paged_tests;
