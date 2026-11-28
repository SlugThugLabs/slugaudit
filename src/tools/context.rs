use crate::error_codes::{self, ErrorCode};
use crate::progress::ProgressSink;
use crate::store;
use crate::sync;
use rmcp::ErrorData;
use rusqlite::{Transaction, TransactionBehavior};
use std::path::Path;

pub(crate) use sync::SyncedProject;

/// Derives the canonical project root from `<root>/.planning/slugaudit/project.db`.
pub(in crate::tools) fn synced_root(synced: &SyncedProject) -> Result<&Path, ErrorData> {
    synced
        .database_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| {
            operation_error(
                "locating the project root",
                "project database path has fewer than three parent components",
            )
        })
}

/// Runs `f` inside a deferred read-only transaction on the project database,
/// verifying that the database revision still matches `synced.revision_id`.
pub(crate) fn with_verified_read<T>(
    synced: &SyncedProject,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, ErrorData>,
) -> Result<T, ErrorData> {
    with_verified_read_setup(synced, |_| Ok(()), f)
}

/// Like [`with_verified_read`], but runs `setup` on `&mut Connection` before
/// starting the deferred read-only transaction.
pub(crate) fn with_verified_read_setup<T>(
    synced: &SyncedProject,
    setup: impl FnOnce(&mut rusqlite::Connection) -> Result<(), ErrorData>,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, ErrorData>,
) -> Result<T, ErrorData> {
    let mut connection = store::open_existing(&synced.database_path)
        .map_err(|error| internal_error("opening the project database", error))?;
    setup(&mut connection)?;
    let tx = start_read_transaction(&mut connection)?;
    verify_revision_matches(&tx, synced)?;
    let result = f(&tx)?;
    tx.commit()
        .map_err(|error| operation_error("committing the read transaction", error))?;
    Ok(result)
}

pub(super) fn start_read_transaction(
    connection: &mut rusqlite::Connection,
) -> Result<Transaction<'_>, ErrorData> {
    connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(|error| operation_error("starting a read transaction", error))
}

/// Wraps a low-level store/rusqlite error with which operation failed and
/// a retry hint for failures a concurrent sync, publish or disable can cause.
pub(super) fn internal_error(context: &str, error: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(
        format!(
            "{context}: {error}. This is often transient (a concurrent \
             sync, publish, or disable) — retry the call. If it keeps \
             happening, the project may need to be disabled and re-enabled."
        ),
        None,
    )
}

/// Wraps a low-level error with the operation that failed and no retry
/// advice, for failures a concurrent sync, publish or disable does not cause.
pub(super) fn operation_error(context: &str, error: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(format!("{context}: {error}"), None)
}

/// Resolves the active project from `path`, ensures it is synchronized, and
/// returns a handle to its current revision.
///
/// # Errors
///
/// Returns an error if `path` isn't inside an active project, or if sync fails.
pub fn ensure_synced(
    path: &str,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<SyncedProject, ErrorData> {
    manager.ensure_current(path, sink)
}

/// Shim that calls [`ensure_synced`] with a no-op sink in tests.
#[cfg(test)]
pub(crate) fn ensure_synced_no_progress(
    path: &str,
    manager: &sync::SourceSyncManager,
) -> Result<SyncedProject, ErrorData> {
    ensure_synced(path, &crate::progress::NoopProgressSink, manager)
}

pub(super) fn current_revision(connection: &rusqlite::Connection) -> Result<String, ErrorData> {
    sync::current_revision_id(connection)
        .and_then(|found| found.ok_or(rusqlite::Error::QueryReturnedNoRows))
        .map_err(|error| internal_error("reading the current revision", error))
}

fn verify_revision_matches(
    connection: &rusqlite::Connection,
    synced: &SyncedProject,
) -> Result<(), ErrorData> {
    let current = current_revision(connection)?;
    if current != synced.revision_id {
        return Err(ErrorData::internal_error(
            format!(
                "revision changed concurrently (expected {}, now {current}); retry the call",
                synced.revision_id
            ),
            Some(error_codes::payload(ErrorCode::StaleRevision)),
        ));
    }
    if let Some(expected_instance) = synced.db_instance_id.as_deref() {
        let actual_instance = sync::project_instance_id(connection)
            .map_err(|error| internal_error("reading the project instance id", error))?;
        if actual_instance.as_deref() != Some(expected_instance) {
            return Err(ErrorData::internal_error(
                format!(
                    "project database was rebuilt concurrently (expected instance {expected_instance}); retry the call"
                ),
                Some(error_codes::payload(ErrorCode::StaleRevision)),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "context_race_tests.rs"]
mod race_tests;

#[cfg(test)]
#[path = "context_transaction_tests.rs"]
mod transaction_tests;
