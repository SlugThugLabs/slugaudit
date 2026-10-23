use crate::error_codes::{self, ErrorCode};
use crate::progress::ProgressSink;
use crate::store;
use crate::sync;
use rmcp::ErrorData;
use rusqlite::{Transaction, TransactionBehavior};

pub(crate) use sync::SyncedProject;

#[cfg(test)]
pub(crate) use crate::util::override_session_id_for_test;
pub(crate) use crate::util::session_id;

/// Runs `f` inside an immediate transaction on the project database,
/// verifying that the database revision still matches `synced.revision_id`.
pub(crate) fn with_verified_write<T>(
    synced: &SyncedProject,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, ErrorData>,
) -> Result<T, ErrorData> {
    let mut connection = store::open_read_write(&synced.database_path)
        .map_err(|error| internal_error("opening the project database for write", error))?;
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
    let mut connection = store::open_read_only(&synced.database_path)
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
/// (using the filesystem watcher to avoid full publishes when possible),
/// and returns a handle to its current revision. A concurrent publish is
/// caught by the revision check in `with_verified_read` / `with_verified_write`.
///
/// `manager` is the per-server `SourceSyncManager` instance owned by the
/// calling `SlugAuditServer` — passing it explicitly (rather than
/// reaching for a process global) means two servers in the same process
/// would each manage their own watchers, their own last-sync stamps, and
/// their own ignore rule sets. `sync::SourceSyncManager::default()` is
/// fine in tests when the watcher's full mode is not needed.
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
/// sink. Tests and CLI paths seeded before Phase 2.2 use this so they
/// don't have to thread `&NoopProgressSink` by hand. Exposed under the
/// `cfg(test)` gate so production callers must take a real progress
/// sink and commit to the wire-point contract documented in
/// `ARCHITECTURE.md`.
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

/// Refuses a read whose caller-supplied `expected_revision` is no longer
/// current.
///
/// `verify_revision_matches` catches a publish that races *this* call; this
/// catches the caller's own pin, which by construction can only be compared
/// against the revision `ensure_synced` has just re-established. The two
/// share the `stale_revision` code and differ in the remedy: a raced read is
/// retried, a resumed page must restart from offset 0 because the rows the
/// caller already saw came from a different ordering
/// ([`error_codes::stale_page_payload`]).
///
/// # Errors
///
/// Returns `invalid_params` carrying the `stale_revision` code when
/// `expected` is `Some` and differs from `current`.
pub(super) fn guard_expected_revision(
    current: &str,
    expected: Option<&str>,
) -> Result<(), ErrorData> {
    let Some(expected) = expected else {
        return Ok(());
    };
    if expected == current {
        return Ok(());
    }
    Err(ErrorData::invalid_params(
        format!(
            "expected_revision `{expected}` is not the current revision (`{current}`); the index \
             changed since the page this call resumes"
        ),
        error_codes::stale_page_payload(),
    ))
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
