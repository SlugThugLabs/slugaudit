//! Persisted [`Stored`] result metadata, `ResultStore` page reading, cursor
//! continuation, and current disk revision probing for `paginate`.

use super::context::internal_error;
use super::page::Completeness;
use super::paginate::PageSpec;
use super::paginate_cut::{CutStep, PageContext, RowCutter, ServedMeta, ServedRows};
use crate::error_codes::store_error;
use crate::store::{ResultId, ResultMeta, ResultStore, ResultStoreError, StoredRow, Take};
use crate::sync::SourceSyncManager;
use rmcp::ErrorData;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
pub(super) struct Stored<E> {
    pub(super) completeness: Completeness,
    pub(super) extra: E,
    pub(super) page_bytes: usize,
    pub(super) ceiling: usize,
    #[serde(default)]
    pub(super) max_rows: Option<usize>,
    #[serde(default)]
    pub(super) database_path: Option<PathBuf>,
}

impl<E: Serialize> Stored<E> {
    pub(super) fn encode(&self) -> Result<serde_json::Value, ErrorData> {
        serde_json::to_value(self)
            .map_err(|error| internal_error("encoding the result extras", error))
    }
}

impl<E> Stored<E> {
    pub(super) fn cutter<R, C>(
        &self,
        columns: &[String],
        cut_for: impl FnOnce(&PageContext<'_, E>) -> Result<C, ErrorData>,
    ) -> Result<(RowCutter<R, C>, bool), ErrorData>
    where
        C: FnMut(&StoredRow, usize, usize) -> Result<CutStep<R>, ErrorData>,
    {
        let at_ceiling = self.page_bytes >= self.ceiling;
        let cut = cut_for(&PageContext {
            page_bytes: self.page_bytes,
            at_ceiling,
            columns,
            extra: &self.extra,
        })?;
        Ok((RowCutter::new(cut, self.max_rows), at_ceiling))
    }
}

pub(super) fn continue_rows<E, R, C>(
    manager: &SourceSyncManager,
    spec: &PageSpec,
    token: &str,
    cut_for: impl FnOnce(&PageContext<'_, E>) -> Result<C, ErrorData>,
) -> Result<ServedRows<R, E>, ErrorData>
where
    E: DeserializeOwned,
    C: FnMut(&StoredRow, usize, usize) -> Result<CutStep<R>, ErrorData>,
{
    let Some(store) = manager.result_stores().store_for_continuation()? else {
        tracing::info!("cursor refused: no result was ever stored");
        return Err(store_error(&ResultStoreError::Unknown));
    };
    let cursor = store.decode_cursor(token).map_err(|e| store_error(&e))?;
    let meta = store.meta_of(cursor.result).map_err(|e| store_error(&e))?;
    if meta.tool != spec.tool {
        tracing::info!(
            tool = spec.tool,
            "cursor refused: it belongs to another tool"
        );
        return Err(store_error(&store.refuse_foreign_tool()));
    }
    let mut stored: Stored<E> = serde::Deserialize::deserialize(&meta.completeness)
        .map_err(|error| internal_error("decoding the stored result extras", error))?;
    if let Some(requested) = spec.requested_page_bytes {
        stored.page_bytes = requested.min(stored.ceiling);
    }
    let disk_revision = current_disk_revision_of(stored.database_path.as_deref());
    let mut served = serve_stored(
        &store,
        cursor.result,
        cursor.ord,
        stored,
        meta.as_ref(),
        cut_for,
    )?;
    served.page.current_disk_revision_id = disk_revision;
    Ok(served)
}

pub(super) fn serve_stored<E, R, C>(
    store: &ResultStore,
    id: ResultId,
    from_ord: u64,
    stored: Stored<E>,
    meta: &ResultMeta,
    cut_for: impl FnOnce(&PageContext<'_, E>) -> Result<C, ErrorData>,
) -> Result<ServedRows<R, E>, ErrorData>
where
    C: FnMut(&StoredRow, usize, usize) -> Result<CutStep<R>, ErrorData>,
{
    let (mut cutter, at_ceiling) = stored.cutter(&meta.columns, cut_for)?;
    let mut cut_error: Option<ErrorData> = None;

    let read = store
        .read_page(id, from_ord, |row| {
            if cut_error.is_some() {
                return Take::No;
            }
            match cutter.accept(row) {
                Ok(true) => Take::Yes,
                Ok(false) => Take::No,
                Err(error) => {
                    cut_error = Some(error);
                    Take::No
                }
            }
        })
        .map_err(|e| store_error(&e))?;
    if let Some(error) = cut_error {
        return Err(error);
    }
    Ok(cutter.into_served(ServedMeta {
        total_rows: read.row_count,
        cursor: read.next_ord.map(|ord| store.cursor(id, ord).to_string()),
        completeness: stored.completeness,
        page_bytes: stored.page_bytes,
        at_ceiling,
        revision_id: read.meta.revision_id.clone(),
        current_disk_revision_id: None,
        database_discard: None,
        extra: stored.extra,
        #[cfg(test)]
        columns: read.meta.columns.clone(),
    }))
}

fn current_disk_revision_of(database_path: Option<&Path>) -> Option<String> {
    let path = database_path?;
    let connection = match crate::store::open_existing(path) {
        Ok(connection) => connection,
        Err(error) => {
            tracing::debug!(error = %error, "continuation could not open project database");
            return None;
        }
    };
    match crate::sync::current_revision_id(&connection) {
        Ok(revision) => revision,
        Err(error) => {
            tracing::debug!(error = %error, "continuation could not read current_disk_revision_id");
            None
        }
    }
}
