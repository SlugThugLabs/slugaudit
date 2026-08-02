use crate::{parse, project, store, sync};
use rmcp::ErrorData;
use rusqlite::Connection;
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
///
/// # Errors
///
/// Returns an error if `path` isn't inside an active project, or if sync
/// itself fails.
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

/// Opens a read-only connection and confirms it still sees the exact
/// revision `synced` verified — never the caller's job to remember. Sync
/// and read happen on separate connections (a read-only one can't run
/// `sync::publish`, which needs to write), so a concurrent publish from
/// another process in the gap between them is possible in principle. This
/// closes that gap by detecting it rather than silently returning data
/// under a stale label: if the current revision has moved on, the call
/// fails explicitly instead of returning a response whose `revision_id`
/// doesn't match what was actually read.
///
/// # Errors
///
/// Returns an error if the connection can't be opened, or if the current
/// revision no longer matches `synced.revision_id`.
pub fn open_verified_read_only(synced: &SyncedProject) -> Result<Connection, ErrorData> {
    let connection = store::open_read_only(&synced.database_path)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    verify_revision_matches(&connection, &synced.revision_id)?;
    Ok(connection)
}

/// The `finding` tool's counterpart to [`open_verified_read_only`] — writes
/// still need a read-write connection, but the same concurrent-publish gap
/// applies and gets the same explicit-failure treatment.
///
/// # Errors
///
/// Returns an error if the connection can't be opened, or if the current
/// revision no longer matches `synced.revision_id`.
pub fn open_verified_read_write(synced: &SyncedProject) -> Result<Connection, ErrorData> {
    let connection = store::open_read_write(&synced.database_path)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    verify_revision_matches(&connection, &synced.revision_id)?;
    Ok(connection)
}

fn verify_revision_matches(
    connection: &Connection,
    expected_revision_id: &str,
) -> Result<(), ErrorData> {
    let current: String = connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    if current != expected_revision_id {
        return Err(ErrorData::internal_error(
            format!(
                "revision changed concurrently (expected {expected_revision_id}, now {current}); retry the call"
            ),
            None,
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
