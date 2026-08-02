use crate::{parse, project, store, sync};
use rmcp::ErrorData;
use std::path::{Path, PathBuf};

/// A project brought fully up to date, ready for a tool to query. Every
/// tool calls this first — there is no separate sync/rebuild entry point
/// for a human or an AI to remember to call.
pub struct SyncedProject {
    pub database_path: PathBuf,
    pub revision_id: String,
}

/// Resolves the active project from `path`, publishes a fresh revision,
/// and returns where its database lives. If a sync is effectively already
/// current (nothing changed on disk), this still verifies that rather than
/// trusting a cached assumption.
pub fn ensure_synced(path: &str) -> Result<SyncedProject, ErrorData> {
    let root = project::find_project_root(Path::new(path))
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    let database_path = project::database_path(&root);

    let mut connection = store::open_read_write(&database_path)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let report = sync::publish(&mut connection, root.as_path(), parse::PACK_VERSION)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    drop(connection);

    Ok(SyncedProject {
        database_path,
        revision_id: report.revision_id,
    })
}
