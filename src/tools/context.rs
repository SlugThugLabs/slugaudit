use crate::progress::ProgressSink;
use crate::sync;
use rmcp::ErrorData;
use std::sync::OnceLock;

#[path = "context_transactions.rs"]
mod context_transactions;
pub(crate) use context_transactions::{with_verified_read, with_verified_write};

// Re-export `SyncedProject` from `sync` so there is exactly one definition.
// `SourceSyncManager::ensure_current` returns it, and `ensure_synced`
// delegates to it — both must agree on the type.
pub use sync::SyncedProject;

/// Global `SourceSyncManager` used by `ensure_synced`. Initialized lazily on
/// the first call. The manager owns a filesystem watcher and uses it to
/// avoid full publishes when the project is quiescent.
static SYNC_MANAGER: OnceLock<sync::SourceSyncManager> = OnceLock::new();

fn get_sync_manager() -> &'static sync::SourceSyncManager {
    SYNC_MANAGER.get_or_init(sync::SourceSyncManager::with_watcher)
}

/// Returns the process-wide `SourceSyncManager`. Same instance every
/// call; `tools::health` uses this to read counters and iterate the
/// watcher's per-project `WatchState`s without going through the
/// ensure-current side effect of `ensure_synced`.
pub(crate) fn sync_manager() -> &'static sync::SourceSyncManager {
    get_sync_manager()
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
pub fn ensure_synced(path: &str, sink: &dyn ProgressSink) -> Result<SyncedProject, ErrorData> {
    get_sync_manager().ensure_current(path, sink)
}

/// Backwards-compatible shim that calls [`ensure_synced`] with a no-op
/// sink. Tests and CLI paths seeded before Phase 2.2 use this so they
/// don't have to thread `&NoopProgressSink` by hand. Exposed under the
/// `cfg(test)` gate so production callers must take a real progress
/// sink and commit to the wire-point contract documented in
/// `ARCHITECTURE.md`.
#[cfg(test)]
pub(crate) fn ensure_synced_no_progress(path: &str) -> Result<SyncedProject, ErrorData> {
    ensure_synced(path, &crate::progress::NoopProgressSink)
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
            None,
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
