//! Fresh-request execution for [`super::paginate`]: syncs the project, runs
//! the producer into a [`SinkTarget`], and serves page one either directly from
//! memory or by spilling into [`ResultStore`].

use super::context::{ensure_synced, internal_error, synced_root, with_verified_read};
use super::paginate::{PageSpec, Produced};
use super::paginate_cursor::{Stored, serve_stored};
use super::paginate_cut::{CutStep, PageContext, ServedMeta, ServedRows, stored_row_from_raw};
use super::paginate_sink::{RowSink, SinkMode, SinkTarget};
use super::paginate_store::StoreSession;
use crate::error_codes::store_error;
use crate::progress::ProgressSink;
use crate::store::{Cell, ResultStore, ResultWriter, StoredRow};
use crate::sync::{DatabaseDiscard, SourceSyncManager};
use rmcp::ErrorData;
use rusqlite::Transaction;
use serde::{Serialize, de::DeserializeOwned};
use std::sync::{Arc, OnceLock};

pub(super) fn fresh_rows<E, R, P, C>(
    manager: &SourceSyncManager,
    spec: &PageSpec,
    path: &str,
    progress: &dyn ProgressSink,
    produce: P,
    cut_for: impl FnOnce(&PageContext<'_, E>) -> Result<C, ErrorData>,
) -> Result<ServedRows<R, E>, ErrorData>
where
    E: Serialize + DeserializeOwned,
    P: FnOnce(&Transaction<'_>, &mut RowSink<'_, '_>) -> Result<Produced<E>, ErrorData>,
    C: FnMut(&StoredRow, usize, usize) -> Result<CutStep<R>, ErrorData>,
{
    let synced = ensure_synced(path, progress, manager)?;
    let root = synced_root(&synced)?;
    let runtime_dir = synced
        .database_path
        .parent()
        .ok_or_else(|| internal_error("locating the runtime directory", "no parent directory"))?;
    let store_limits = crate::store::ResultStoreLimits::from(&super::limits_for_root(root)?);
    let opened_store: OnceLock<Arc<ResultStore>> = OnceLock::new();
    let store_session = StoreSession::new(
        manager,
        runtime_dir,
        store_limits,
        &opened_store,
        spec.tool,
        crate::util::redacted_path(root),
        synced.revision_id.clone(),
    );
    let mut target = SinkTarget::new(&store_session, &spec.columns, spec.page_bytes)?;
    let produced = with_verified_read(&synced, |tx| {
        produce(
            tx,
            &mut RowSink {
                target: &mut target,
            },
        )
    })?;
    let finished = target.finish()?;
    let stored = Stored {
        completeness: produced.completeness,
        extra: produced.extra,
        page_bytes: spec.page_bytes,
        ceiling: spec.ceiling,
        max_rows: spec.max_rows,
        database_path: Some(synced.database_path.clone()),
    };

    match finished.mode {
        SinkMode::Buffered { rows, .. } => serve_buffered(
            &store_session,
            stored,
            &finished.columns,
            &rows,
            synced.revision_id,
            synced.discard,
            cut_for,
        ),
        SinkMode::Spilled(writer) => {
            serve_spilled(&opened_store, *writer, stored, synced.discard, cut_for)
        }
    }
}

fn serve_buffered<E, R, C>(
    store_session: &StoreSession<'_>,
    stored: Stored<E>,
    columns: &[String],
    raw_rows: &[Vec<Cell>],
    revision_id: String,
    database_discard: Option<DatabaseDiscard>,
    cut_for: impl FnOnce(&PageContext<'_, E>) -> Result<C, ErrorData>,
) -> Result<ServedRows<R, E>, ErrorData>
where
    E: Serialize,
    C: FnMut(&StoredRow, usize, usize) -> Result<CutStep<R>, ErrorData>,
{
    let (mut cutter, at_ceiling) = stored.cutter(columns, cut_for)?;
    let mut next_ord = None;
    for (index, raw) in raw_rows.iter().enumerate() {
        let ord = u64::try_from(index)
            .map_err(|error| internal_error("numbering a buffered row", error))?;
        if !cutter.accept(&stored_row_from_raw(ord, raw))? {
            next_ord = Some(ord);
            break;
        }
    }
    let total_rows = u64::try_from(raw_rows.len())
        .map_err(|error| internal_error("counting result rows", error))?;
    let cursor = match next_ord {
        None => None,
        Some(ord) => {
            let encoded = stored.encode()?;
            let (store, handle) =
                store_session.materialize_buffered(columns, raw_rows, &encoded)?;
            Some(store.cursor(handle.id, ord).to_string())
        }
    };
    Ok(cutter.into_served(ServedMeta {
        total_rows,
        cursor,
        completeness: stored.completeness,
        page_bytes: stored.page_bytes,
        at_ceiling,
        revision_id,
        current_disk_revision_id: None,
        database_discard,
        extra: stored.extra,
        #[cfg(test)]
        columns: columns.to_vec(),
    }))
}

fn serve_spilled<E, R, C>(
    opened_store: &OnceLock<Arc<ResultStore>>,
    writer: ResultWriter<'_>,
    stored: Stored<E>,
    database_discard: Option<DatabaseDiscard>,
    cut_for: impl FnOnce(&PageContext<'_, E>) -> Result<C, ErrorData>,
) -> Result<ServedRows<R, E>, ErrorData>
where
    E: Serialize,
    C: FnMut(&StoredRow, usize, usize) -> Result<CutStep<R>, ErrorData>,
{
    let encoded = stored.encode()?;
    let handle = writer.finish(&encoded).map_err(|e| store_error(&e))?;
    let store = opened_store
        .get()
        .ok_or_else(|| internal_error("storing the result", "spilled store missing"))?;
    let meta = store.meta_of(handle.id).map_err(|e| store_error(&e))?;
    let mut served = serve_stored(store, handle.id, 0, stored, meta.as_ref(), cut_for)?;
    served.page.database_discard = database_discard;
    Ok(served)
}
