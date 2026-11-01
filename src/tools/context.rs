use crate::error_codes::{self, ErrorCode};
use crate::progress::ProgressSink;
use crate::store;
use crate::sync;
use rmcp::ErrorData;
use rusqlite::{Transaction, TransactionBehavior};

pub(crate) use sync::SyncedProject;

pub use super::paginate::{
    PageContext, PageRequest, PageSpec, Produced, RowSink, Served, decode_rows, page_request,
    paginate,
};

#[cfg(test)]
pub(crate) use crate::util::override_session_id_for_test;
pub(crate) use crate::util::session_id;

/// Runs `f` inside an immediate transaction on the project database,
/// verifying that the database revision still matches `synced.revision_id`.
pub(crate) fn with_verified_write<T>(
    synced: &SyncedProject,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, ErrorData>,
) -> Result<T, ErrorData> {
    let mut connection =
        store::open_existing_read_write(&synced.database_path).map_err(|error| match error {
            // The directory is gone: `off` ran after the sync, so the project
            // is off. A directory without a database is deleted scratch, and
            // the next call rebuilds it.
            store::StoreError::Missing
                if synced
                    .database_path
                    .parent()
                    .is_some_and(|dir| !dir.is_dir()) =>
            {
                sync::map_activation_error(crate::project::ActivationError::NotActive)
            }
            error => internal_error("opening the project database for write", error),
        })?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| internal_error("starting a write transaction", error))?;
    verify_revision_matches(&tx, &synced.revision_id)?;
    let result = f(&tx)?;
    tx.commit()
        .map_err(|error| internal_error("committing the write transaction", error))?;
    Ok(result)
}

/// Runs `f` inside a deferred read-only transaction on the project database,
/// verifying that the database revision still matches `synced.revision_id`.
pub(crate) fn with_verified_read<T>(
    synced: &SyncedProject,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, ErrorData>,
) -> Result<T, ErrorData> {
    let mut connection = store::open_existing(&synced.database_path)
        .map_err(|error| internal_error("opening the project database", error))?;
    let tx = start_read_transaction(&mut connection)?;
    verify_revision_matches(&tx, &synced.revision_id)?;
    let result = f(&tx)?;
    tx.commit()
        .map_err(|error| internal_error("committing the read transaction", error))?;
    Ok(result)
}

/// Starts the read transaction `with_verified_read` runs in. Split out so
/// the start-failure mapping is testable with a connection that already
/// has a transaction open — the one state in which SQLite refuses a second
/// `BEGIN DEFERRED`.
pub(super) fn start_read_transaction(
    connection: &mut rusqlite::Connection,
) -> Result<Transaction<'_>, ErrorData> {
    connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(|error| internal_error("starting a read transaction", error))
}

/// Wraps a low-level store/rusqlite error with which operation failed and
/// a generic recovery hint, instead of surfacing the bare `Display` text
/// alone — that names no operation and gives the caller no next step.
/// Mirrors `tools::query::describe_error`, which does the same kind of
/// translation for query-specific errors; this is the equivalent for the
/// sync/revision-verification errors that flow through every tool call.
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

/// Resolves the active project from `path`, ensures it's synchronized
/// (hashing the tree and publishing only what changed), and returns a
/// handle to its current revision. A concurrent publish is
/// caught by the revision check in `with_verified_read` / `with_verified_write`.
///
/// `manager` is the per-server `SourceSyncManager` instance owned by the
/// calling `SlugAuditServer` — passing it explicitly (rather than
/// reaching for a process global) means two servers in the same process
/// would each track their own last-sync stamps, session state, and paged
/// results. `sync::SourceSyncManager::default()` is fine in tests.
///
/// `sink` carries progress events from the sync layer (per-file sampling
/// during publish, plus `Started`/`Completed` markers for the overall
/// `ensuring_current` phase) to the MCP-aware server, which turns the
/// events into `/notifications/progress` notifications. Tests pass
/// `&NoopProgressSink`; callers that don't have an MCP connection
/// should as well.
///
/// # Errors
///
/// Returns an error if `path` isn't inside an active project, or if sync
/// itself fails.
pub fn ensure_synced(
    path: &str,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<SyncedProject, ErrorData> {
    manager.ensure_current(path, sink)
}

/// Backwards-compatible shim that calls [`ensure_synced`] with a no-op
/// sink. Tests and CLI paths that predate progress sinks use this so they
/// don't have to thread `&NoopProgressSink` by hand. Exposed under the
/// `cfg(test)` gate so production callers must take a real progress
/// sink and commit to the wire-point contract.
#[cfg(test)]
pub(crate) fn ensure_synced_no_progress(
    path: &str,
    manager: &sync::SourceSyncManager,
) -> Result<SyncedProject, ErrorData> {
    ensure_synced(path, &crate::progress::NoopProgressSink, manager)
}

fn verify_revision_matches(
    connection: &rusqlite::Connection,
    expected_revision_id: &str,
) -> Result<(), ErrorData> {
    let current: String = connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| internal_error("reading the current revision", error))?;
    if current != expected_revision_id {
        return Err(ErrorData::internal_error(
            format!(
                "revision changed concurrently (expected {expected_revision_id}, now {current}); retry the call"
            ),
            error_codes::payload(ErrorCode::StaleRevision),
        ));
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
