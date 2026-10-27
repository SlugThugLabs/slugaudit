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
    Cell, Materialize, ResultSpec, ResultStore, ResultStoreError, ResultWriter, StoredRow, Take,
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
    /// Column names, stored with the result.
    pub columns: Vec<String>,
    /// The byte budget the caller's `accept` closure enforces, echoed in the
    /// page.
    pub page_bytes: usize,
}

impl PageSpec {
    /// A spec whose columns are those of the typed row `R`.
    pub fn typed<R: Materialize>(tool: &'static str, page_bytes: usize) -> Self {
        Self {
            tool,
            columns: R::columns().iter().map(|name| (*name).to_owned()).collect(),
            page_bytes,
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
    writer: &'w mut ResultWriter<'s>,
}

impl RowSink<'_, '_> {
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
        self.writer.push(cells).map_err(|e| store_error(&e))
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
}

/// Completeness and extras as stored in the result's metadata.
#[derive(Serialize, Deserialize)]
struct Stored<E> {
    completeness: Completeness,
    extra: E,
}

/// Serves one page; see the module docs.
///
/// `produce` runs inside the verified read transaction and streams rows
/// through the [`RowSink`] (it never needs the whole result in memory).
/// `accept` decides where the byte-limited page ends; the first row is always
/// taken.
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
    accept: A,
) -> Result<Served<E>, ErrorData>
where
    E: Serialize + DeserializeOwned,
    P: FnOnce(&Transaction<'_>, &mut RowSink<'_, '_>) -> Result<Produced<E>, ErrorData>,
    A: FnMut(&StoredRow) -> Take,
{
    match request {
        PageRequest::Cursor(token) => continue_page(manager, spec, token, accept),
        PageRequest::Fresh { path, sink } => {
            let synced = ensure_synced(path, sink, manager)?;
            let runtime_dir = synced.database_path.parent().ok_or_else(|| {
                internal_error("locating the runtime directory", "no parent directory")
            })?;
            let store = manager
                .result_stores()
                .for_dir(runtime_dir)
                .map_err(|e| store_error(&e))?;
            let mut writer = store
                .begin(ResultSpec {
                    tool: spec.tool.to_owned(),
                    project: "default".to_owned(),
                    revision_id: synced.revision_id.clone(),
                    columns: spec.columns.clone(),
                })
                .map_err(|e| store_error(&e))?;
            let produced = with_verified_read(&synced, |tx| {
                produce(
                    tx,
                    &mut RowSink {
                        writer: &mut writer,
                    },
                )
            })?;
            let stored = serde_json::to_value(Stored {
                completeness: produced.completeness,
                extra: produced.extra,
            })
            .map_err(|error| internal_error("encoding the result extras", error))?;
            let handle = writer.finish(&stored).map_err(|e| store_error(&e))?;
            serve(&store, spec, handle.id, 0, accept)
        }
    }
}

fn continue_page<E: DeserializeOwned>(
    manager: &SourceSyncManager,
    spec: &PageSpec,
    token: &str,
    accept: impl FnMut(&StoredRow) -> Take,
) -> Result<Served<E>, ErrorData> {
    let Some(store) = manager.result_stores().current() else {
        tracing::info!("cursor refused: no result was ever stored");
        return Err(store_error(&ResultStoreError::Unknown));
    };
    let cursor = store.decode_cursor(token).map_err(|e| store_error(&e))?;
    // Before any read: a read renews the result and may mark it delivered,
    // which another tool's cursor must never do.
    if store.tool_of(cursor.result).map_err(|e| store_error(&e))? != spec.tool {
        tracing::info!(
            tool = spec.tool,
            "cursor refused: it belongs to another tool"
        );
        return Err(store_error(&ResultStoreError::Unknown));
    }
    serve(&store, spec, cursor.result, cursor.ord, accept)
}

fn serve<E: DeserializeOwned>(
    store: &ResultStore,
    spec: &PageSpec,
    id: crate::store::ResultId,
    from_ord: u64,
    accept: impl FnMut(&StoredRow) -> Take,
) -> Result<Served<E>, ErrorData> {
    let read = store
        .read_page(id, from_ord, accept)
        .map_err(|e| store_error(&e))?;
    let stored: Stored<E> = serde_json::from_value(read.meta.completeness.clone())
        .map_err(|error| internal_error("decoding the stored result extras", error))?;
    let complete = read.next_ord.is_none();
    let page = Page {
        returned: read.rows.len(),
        total: Some(read.row_count),
        complete,
        next_offset: None,
        cursor: read.next_ord.map(|ord| store.cursor(id, ord).to_string()),
        completeness: Some(stored.completeness),
        page_bytes: spec.page_bytes,
        bytes_used: 0,
        limited_by: if complete {
            PageLimit::None
        } else {
            PageLimit::PageBytes
        },
        revision_id: read.meta.revision_id.clone(),
    };
    Ok(Served {
        rows: read.rows,
        page,
        meta: stored.extra,
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

#[cfg(test)]
#[path = "paginate_tests.rs"]
mod tests;
