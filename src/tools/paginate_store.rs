//! Lazy [`ResultStore`] session for [`super::paginate_fresh`] and
//! [`super::paginate_sink`]: opens the per-project `ResultStore` only when a
//! fresh result spills past the in-memory page buffer or requires a cursor.

use crate::error_codes::store_error;
use crate::store::{
    Cell, ResultHandle, ResultSpec, ResultStore, ResultStoreError, ResultStoreLimits, ResultWriter,
};
use crate::sync::SourceSyncManager;
use rmcp::ErrorData;
use std::path::Path;
use std::sync::{Arc, OnceLock};

pub(super) fn map_store<T>(result: Result<T, ResultStoreError>) -> Result<T, ErrorData> {
    result.map_err(|error| store_error(&error))
}

/// Lazy [`ResultStore`] handle and metadata for one fresh `paginate_rows` call.
pub(super) struct StoreSession<'s> {
    manager: &'s SourceSyncManager,
    runtime_dir: &'s Path,
    store_limits: ResultStoreLimits,
    opened_store: &'s OnceLock<Arc<ResultStore>>,
    tool: &'static str,
    project: String,
    revision_id: String,
}

impl<'s> StoreSession<'s> {
    pub(super) fn new(
        manager: &'s SourceSyncManager,
        runtime_dir: &'s Path,
        store_limits: ResultStoreLimits,
        opened_store: &'s OnceLock<Arc<ResultStore>>,
        tool: &'static str,
        project: String,
        revision_id: String,
    ) -> Self {
        Self {
            manager,
            runtime_dir,
            store_limits,
            opened_store,
            tool,
            project,
            revision_id,
        }
    }

    fn open_store(&self) -> Result<&'s ResultStore, ErrorData> {
        if let Some(store) = self.opened_store.get() {
            return Ok(store.as_ref());
        }
        let opened = map_store(
            self.manager
                .result_stores()
                .for_dir_with_limits(self.runtime_dir, self.store_limits),
        )?;
        Ok(self.opened_store.get_or_init(|| opened).as_ref())
    }

    pub(super) fn begin_and_drain(
        &self,
        columns: &[String],
        buffered_rows: &[Vec<Cell>],
    ) -> Result<(&'s ResultStore, ResultWriter<'s>), ErrorData> {
        let store = self.open_store()?;
        let mut writer = map_store(store.begin(ResultSpec {
            tool: self.tool.to_owned(),
            project: self.project.clone(),
            revision_id: self.revision_id.clone(),
            columns: columns.to_vec(),
        }))?;
        for row in buffered_rows {
            map_store(writer.push(row))?;
        }
        Ok((store, writer))
    }

    pub(super) fn materialize_buffered(
        &self,
        columns: &[String],
        rows: &[Vec<Cell>],
        completeness: &serde_json::Value,
    ) -> Result<(&'s ResultStore, ResultHandle), ErrorData> {
        let (store, writer) = self.begin_and_drain(columns, rows)?;
        let handle = map_store(writer.finish(completeness))?;
        Ok((store, handle))
    }
}
