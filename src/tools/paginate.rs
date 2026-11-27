//! The one paging entry every paged tool calls.
//!
//! Fresh calls stream rows into an in-memory buffer first and serve single-page
//! results directly from memory with zero `ResultStore` disk I/O. Multi-page
//! results materialise into `ResultStore` and return a `cursor` whose
//! continuations read the immutable stored snapshot without syncing.

#[cfg(test)]
use super::context::internal_error;
use super::page::Completeness;
use super::paginate_cursor::continue_rows;
pub use super::paginate_cut::PageContext;
pub(in crate::tools) use super::paginate_cut::byte_cut_step;
#[cfg(test)]
pub(in crate::tools) use super::paginate_cut::compute_limited_by;
pub(crate) use super::paginate_cut::{CutStep, ServedRows};
use super::paginate_fresh::fresh_rows;
pub use super::paginate_sink::RowSink;
#[cfg(test)]
use crate::error_codes::store_error;
use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::progress::ProgressSink;
use crate::store::{Materialize, StoredRow};
use crate::sync::SourceSyncManager;
use rmcp::ErrorData;
use rusqlite::Transaction;
use serde::{Serialize, de::DeserializeOwned};

/// Longest `cursor` a tool accepts before decoding.
pub(crate) const MAX_CURSOR_BYTES: usize = 1024;

/// What the call is: a new request or the continuation of a stored one.
#[derive(Clone, Copy)]
pub enum PageRequest<'a> {
    /// No cursor: sync `path` and serve page one (materialising if multi-page).
    Fresh {
        /// A path inside the project.
        path: &'a str,
        /// Where sync progress goes.
        sink: &'a dyn ProgressSink,
    },
    /// A cursor exactly as a previous page returned it.
    Cursor(&'a str),
}

/// What the calling tool declares about its result.
pub struct PageSpec {
    /// The tool name; a cursor only continues the tool that issued it.
    pub tool: &'static str,
    /// Column names, stored with the result (empty when declared via [`RowSink::set_columns`]).
    pub columns: Vec<String>,
    /// Effective byte budget of page one, echoed and stored for continuations.
    pub page_bytes: usize,
    /// Explicit `page_bytes` on a continuation request, if any.
    pub requested_page_bytes: Option<usize>,
    /// Hard byte ceiling (`max_query_response_bytes`) stored with the result.
    pub ceiling: usize,
    /// Optional per-page row cap (`max_structure_matches` on `structure`),
    /// enforced on page one and persisted for every cursor continuation.
    pub max_rows: Option<usize>,
}

impl PageSpec {
    /// A spec whose columns are those of the typed row `R`.
    #[must_use]
    pub fn typed<R: Materialize>(tool: &'static str, page_bytes: usize) -> Self {
        Self {
            tool,
            columns: R::columns().iter().map(|name| (*name).to_owned()).collect(),
            page_bytes,
            requested_page_bytes: None,
            ceiling: usize::MAX,
            max_rows: None,
        }
    }
}

/// The part of a result the producer reports after its last row.
pub struct Produced<E> {
    /// Whether the scan covered everything it set out to.
    pub completeness: Completeness,
    /// Tool-level totals and extras, stored with the result and returned on every page.
    pub extra: E,
}

/// Serves one page of decoded rows `R`, decoding and measuring each offered row
/// once and computing `bytes_used` and `limited_by` in one shared place.
pub(crate) fn paginate_rows<E, R, P, C>(
    manager: &SourceSyncManager,
    spec: &PageSpec,
    request: PageRequest<'_>,
    produce: P,
    cut_for: impl FnOnce(&PageContext<'_, E>) -> Result<C, ErrorData>,
) -> Result<ServedRows<R, E>, ErrorData>
where
    E: Serialize + DeserializeOwned,
    P: FnOnce(&Transaction<'_>, &mut RowSink<'_, '_>) -> Result<Produced<E>, ErrorData>,
    C: FnMut(&StoredRow, usize, usize) -> Result<CutStep<R>, ErrorData>,
{
    match request {
        PageRequest::Cursor(token) => continue_rows(manager, spec, token, cut_for),
        PageRequest::Fresh { path, sink } => {
            fresh_rows(manager, spec, path, sink, produce, cut_for)
        }
    }
}

/// Resolves the cursor/path pair every paged tool takes into a [`PageRequest`].
///
/// # Errors
///
/// `invalid_params` when `cursor` exceeds `MAX_CURSOR_BYTES`, or when neither
/// `cursor` nor `path` is supplied.
pub fn page_request<'a>(
    cursor: Option<&'a str>,
    path: Option<&'a str>,
    sink: &'a dyn ProgressSink,
) -> Result<PageRequest<'a>, ErrorData> {
    match (cursor, path) {
        (Some(token), _) => {
            if crate::util::exceeds_input_bytes(token, MAX_CURSOR_BYTES) {
                return Err(invalid_params_with_code(
                    format!("cursor exceeds {MAX_CURSOR_BYTES} bytes"),
                    ErrorCode::InvalidInput,
                ));
            }
            Ok(PageRequest::Cursor(token))
        }
        (None, Some(path)) => Ok(PageRequest::Fresh { path, sink }),
        (None, None) => Err(invalid_params_with_code(
            "path is required when no cursor is given",
            ErrorCode::InvalidInput,
        )),
    }
}

#[cfg(test)]
#[path = "paginate_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "paginate_cursor_tests.rs"]
mod cursor_tests;

#[cfg(test)]
#[path = "paginate_late_tests.rs"]
mod late_tests;

#[cfg(test)]
#[path = "paginate_store_limit_tests.rs"]
mod store_limit_tests;

#[cfg(test)]
#[path = "paginate_context_store_tests.rs"]
mod context_store_tests;
