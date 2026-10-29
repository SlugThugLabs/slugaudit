// slugaudit-line-exception: approved-by=slugthug; reason=one paging entry: request/spec types, the producer's row sink, fresh materialisation and continuation share the cursor, budget and stored-extras contract
//! The one paging entry every paged tool calls.
//!
//! A call without a cursor is a new request: it syncs, streams the tool's rows
//! out of the verified read transaction into the result store, and serves page
//! one from the stored copy. A call with a cursor is a continuation: it reads
//! the stored copy and nothing else (no sync, no revision check, no `path`),
//! so the rows and `revision_id` are those of the first call however the tree
//! changed since.
//!
//! This function is synchronous; callers run it inside the server's existing
//! blocking-operation semaphore, which also bounds concurrent
//! materialisations.

use super::context::{ensure_synced, internal_error, with_verified_read};
use super::page::{Completeness, Page, PageLimit};
use crate::error_codes::store_error;
use crate::progress::ProgressSink;
use crate::store::{
    Cell, Materialize, ResultId, ResultMeta, ResultSpec, ResultStore, ResultStoreError,
    ResultWriter, StoredRow, Take,
};
use crate::sync::SourceSyncManager;
use rmcp::ErrorData;
use rusqlite::Transaction;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// What the call is: a new request or the continuation of a stored one.
pub enum PageRequest<'a> {
    /// No cursor: sync `path` and materialise a new result.
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
    /// Column names, stored with the result. Empty when the producer declares
    /// them with [`RowSink::set_columns`] (the columns of a `query` come from
    /// its prepared statement).
    pub columns: Vec<String>,
    /// The byte budget the caller's `accept` closure enforces, echoed in the
    /// page. For a new request it is also stored with the result, as the
    /// budget its continuations default to.
    pub page_bytes: usize,
    /// The `page_bytes` a continuation's request carried (already clamped to
    /// the ceiling by the tool), if any: a continuation uses it, else the
    /// budget stored with the result.
    pub requested_page_bytes: Option<usize>,
}

impl PageSpec {
    /// A spec whose columns are those of the typed row `R`.
    pub fn typed<R: Materialize>(tool: &'static str, page_bytes: usize) -> Self {
        Self {
            tool,
            columns: R::columns().iter().map(|name| (*name).to_owned()).collect(),
            page_bytes,
            requested_page_bytes: None,
        }
    }
}

/// The part of a result the producer reports after its last row.
pub struct Produced<E> {
    /// Whether the scan covered everything it set out to.
    pub completeness: Completeness,
    /// Tool-level totals and extras, stored with the result and returned on
    /// every page.
    pub extra: E,
}

/// The producer's handle for streaming rows into the store.
pub struct RowSink<'w, 's> {
    store: &'s ResultStore,
    /// The spec with empty columns, used when the producer declares them.
    pending: &'w ResultSpec,
    writer: &'w mut Option<ResultWriter<'s>>,
}

/// A misuse of [`RowSink`] by its producer.
#[derive(Debug, thiserror::Error)]
enum SinkError {
    /// `set_columns` after the columns were already known.
    #[error("the result columns were already declared")]
    ColumnsAlreadyDeclared,
    /// A row pushed before any columns were declared.
    #[error("a row was pushed before the result columns were declared")]
    ColumnsNotDeclared,
}

impl RowSink<'_, '_> {
    /// Declares the result's columns. Callable once, before the first push,
    /// and only when [`PageSpec::columns`] was empty.
    ///
    /// # Errors
    ///
    /// An internal error when the columns are already known (declared twice,
    /// supplied by the spec, or a row was pushed), or a store failure mapped
    /// to its wire code.
    pub fn set_columns(&mut self, columns: Vec<String>) -> Result<(), ErrorData> {
        if self.writer.is_some() {
            return Err(internal_error(
                "declaring the result columns",
                SinkError::ColumnsAlreadyDeclared,
            ));
        }
        let mut spec = self.pending.clone();
        spec.columns = columns;
        *self.writer = Some(self.store.begin(spec).map_err(|e| store_error(&e))?);
        Ok(())
    }

    /// Stores one typed row.
    ///
    /// # Errors
    ///
    /// A store failure mapped to its wire code.
    pub fn push<R: Materialize>(&mut self, row: &R) -> Result<(), ErrorData> {
        self.push_cells(&row.to_values())
    }

    /// Stores one raw row (`query`).
    ///
    /// # Errors
    ///
    /// A store failure mapped to its wire code.
    pub fn push_cells(&mut self, cells: &[Cell]) -> Result<(), ErrorData> {
        let Some(writer) = self.writer.as_mut() else {
            return Err(internal_error(
                "storing a result row",
                SinkError::ColumnsNotDeclared,
            ));
        };
        writer.push(cells).map_err(|e| store_error(&e))
    }
}

/// One served page.
pub struct Served<E> {
    /// The rows of this page, in order.
    pub rows: Vec<StoredRow>,
    /// The shared page block. `bytes_used` is 0 and `limited_by` is
    /// `page_bytes` (or `none` on the last page); the tool, which measured the
    /// bytes in `accept`, overwrites them when it knows better.
    pub page: Page,
    /// The tool-level extras stored with the result.
    pub meta: E,
    /// The column names stored with the result.
    pub columns: Vec<String>,
}

/// Completeness and extras as stored in the result's metadata.
#[derive(Serialize, Deserialize)]
struct Stored<E> {
    completeness: Completeness,
    extra: E,
    /// The effective page budget of the request that made the result.
    page_bytes: usize,
}

/// What the page cut may know about the result it cuts, identical on a new
/// request (after `produce`) and on a continuation (decoded from the stored
/// result): the tool never reads a process limit or the store to learn it.
pub struct PageContext<'a, E> {
    /// The effective byte budget of this page: the request's own (already
    /// clamped to the ceiling by the tool) or the one the result's first page
    /// used. Also echoed in `page.page_bytes`.
    pub page_bytes: usize,
    /// The column names stored with the result.
    pub columns: &'a [String],
    /// The tool-level extras the producer reported, as stored.
    pub extra: &'a E,
}

/// Serves one page; see the module docs.
///
/// `produce` runs inside the verified read transaction and streams rows
/// through the [`RowSink`] (it never needs the whole result in memory).
/// `accept_for` receives one [`PageContext`] for both branches: the effective
/// budget (a continuation's `requested_page_bytes` when it carries one, else
/// the budget stored with the result; a new request's is `spec.page_bytes`),
/// the stored column names and the stored extras. It returns the `accept`
/// closure, which must own what it needs from the context. Every row of the
/// page is offered to `accept`, the first too so the tool's byte accounting
/// sees it; the first is always taken whatever the verdict, so a page always
/// advances, and the page ends at the first later `No`.
///
/// # Errors
///
/// Sync failures, producer failures, and store failures mapped to
/// `unknown_cursor`, `result_expired`, `result_store_full`, `result_too_large`
/// or `result_store_unavailable`. A failed materialisation leaves no cursor
/// and no stored result.
pub fn paginate<E, P, A>(
    manager: &SourceSyncManager,
    spec: &PageSpec,
    request: PageRequest<'_>,
    produce: P,
    accept_for: impl FnOnce(&PageContext<'_, E>) -> A,
) -> Result<Served<E>, ErrorData>
where
    E: Serialize + DeserializeOwned,
    P: FnOnce(&Transaction<'_>, &mut RowSink<'_, '_>) -> Result<Produced<E>, ErrorData>,
    A: FnMut(&StoredRow) -> Take,
{
    match request {
        PageRequest::Cursor(token) => continue_page(manager, spec, token, accept_for),
        PageRequest::Fresh { path, sink } => {
            let synced = ensure_synced(path, sink, manager)?;
            let runtime_dir = synced.database_path.parent().ok_or_else(|| {
                internal_error("locating the runtime directory", "no parent directory")
            })?;
            let store = manager
                .result_stores()
                .for_dir(runtime_dir)
                .map_err(|e| store_error(&e))?;
            let pending = ResultSpec {
                tool: spec.tool.to_owned(),
                project: project_label(runtime_dir)?,
                revision_id: synced.revision_id.clone(),
                columns: Vec::new(),
            };
            // With columns in the spec the writer starts now; otherwise the
            // producer declares them before its first row.
            let mut writer = if spec.columns.is_empty() {
                None
            } else {
                let mut known = pending.clone();
                known.columns.clone_from(&spec.columns);
                Some(store.begin(known).map_err(|e| store_error(&e))?)
            };
            let produced = with_verified_read(&synced, |tx| {
                produce(
                    tx,
                    &mut RowSink {
                        store: &store,
                        pending: &pending,
                        writer: &mut writer,
                    },
                )
            })?;
            let Some(writer) = writer else {
                return Err(internal_error(
                    "storing the result",
                    SinkError::ColumnsNotDeclared,
                ));
            };
            let stored = Stored {
                completeness: produced.completeness,
                extra: produced.extra,
                page_bytes: spec.page_bytes,
            };
            let encoded = serde_json::to_value(&stored)
                .map_err(|error| internal_error("encoding the result extras", error))?;
            let handle = writer.finish(&encoded).map_err(|e| store_error(&e))?;
            let meta = store.meta_of(handle.id).map_err(|e| store_error(&e))?;
            let mut served = serve(&store, handle.id, 0, stored, meta.as_ref(), accept_for)?;
            served.page.database_discard = synced.discard;
            Ok(served)
        }
    }
}

/// The label stored with a result: the project root's path id, as the logs
/// show it. The runtime directory is `<root>/.planning/slugaudit`.
fn project_label(runtime_dir: &std::path::Path) -> Result<String, ErrorData> {
    let root = runtime_dir
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or_else(|| internal_error("locating the project root", "no project root"))?;
    Ok(crate::util::redacted_path(root))
}

fn continue_page<E, A>(
    manager: &SourceSyncManager,
    spec: &PageSpec,
    token: &str,
    accept_for: impl FnOnce(&PageContext<'_, E>) -> A,
) -> Result<Served<E>, ErrorData>
where
    E: DeserializeOwned,
    A: FnMut(&StoredRow) -> Take,
{
    let Some(store) = manager.result_stores().current() else {
        tracing::info!("cursor refused: no result was ever stored");
        return Err(store_error(&ResultStoreError::Unknown));
    };
    let cursor = store.decode_cursor(token).map_err(|e| store_error(&e))?;
    // Before any read: a read renews the result and may mark it delivered,
    // which another tool's cursor must never do.
    let meta = store.meta_of(cursor.result).map_err(|e| store_error(&e))?;
    if meta.tool != spec.tool {
        tracing::info!(
            tool = spec.tool,
            "cursor refused: it belongs to another tool"
        );
        return Err(store_error(&store.refuse_foreign_tool()));
    }
    let mut stored: Stored<E> = serde_json::from_value(meta.completeness.clone())
        .map_err(|error| internal_error("decoding the stored result extras", error))?;
    if let Some(requested) = spec.requested_page_bytes {
        stored.page_bytes = requested;
    }
    serve(
        &store,
        cursor.result,
        cursor.ord,
        stored,
        meta.as_ref(),
        accept_for,
    )
}

/// Reads the page at `from_ord` of result `id`. `stored` carries the extras
/// and the effective budget (`stored.page_bytes`), `meta` the stored columns.
fn serve<E, A>(
    store: &ResultStore,
    id: ResultId,
    from_ord: u64,
    stored: Stored<E>,
    meta: &ResultMeta,
    accept_for: impl FnOnce(&PageContext<'_, E>) -> A,
) -> Result<Served<E>, ErrorData>
where
    A: FnMut(&StoredRow) -> Take,
{
    let page_bytes = stored.page_bytes;
    let accept = accept_for(&PageContext {
        page_bytes,
        columns: &meta.columns,
        extra: &stored.extra,
    });
    let read = store
        .read_page(id, from_ord, accept)
        .map_err(|e| store_error(&e))?;
    let complete = read.next_ord.is_none();
    let page = Page {
        returned: read.rows.len(),
        total: Some(read.row_count),
        complete,
        cursor: read.next_ord.map(|ord| store.cursor(id, ord).to_string()),
        completeness: Some(stored.completeness),
        page_bytes,
        bytes_used: 0,
        limited_by: if complete {
            PageLimit::None
        } else {
            PageLimit::PageBytes
        },
        revision_id: read.meta.revision_id.clone(),
        database_discard: None,
    };
    Ok(Served {
        rows: read.rows,
        page,
        meta: stored.extra,
        columns: read.meta.columns.clone(),
    })
}

/// Rebuilds typed rows from a served page.
///
/// # Errors
///
/// An internal error when the stored cells do not match `R`'s layout.
pub fn decode_rows<R: Materialize>(rows: &[StoredRow]) -> Result<Vec<R>, ErrorData> {
    rows.iter()
        .map(|row| R::from_values(&row.cells).map_err(|error| store_error(&error)))
        .collect()
}

/// Resolves the cursor/path pair every migrated tool takes into a
/// [`PageRequest`]. With a cursor the call continues a stored result: `path`
/// and every request-shaping parameter are ignored (only `page_bytes` is
/// honoured). Without one it is a new request and `path` is required.
///
/// # Errors
///
/// `invalid_params` when there is neither a cursor nor a path.
pub fn page_request<'a>(
    cursor: Option<&'a str>,
    path: Option<&'a str>,
    sink: &'a dyn ProgressSink,
) -> Result<PageRequest<'a>, ErrorData> {
    match (cursor, path) {
        (Some(token), _) => Ok(PageRequest::Cursor(token)),
        (None, Some(path)) => Ok(PageRequest::Fresh { path, sink }),
        (None, None) => Err(ErrorData::invalid_params(
            "path is required when no cursor is given",
            None,
        )),
    }
}

#[cfg(test)]
#[path = "paginate_tests.rs"]
mod tests;
