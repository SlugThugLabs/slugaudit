use crate::progress::ProgressSink;
use crate::sync;
use rmcp::ErrorData;
use std::sync::Mutex;
use uuid::Uuid;

#[path = "context_transactions.rs"]
mod context_transactions;
pub(crate) use context_transactions::{with_verified_read, with_verified_write};

// Re-export `SyncedProject` from `sync` so there is exactly one definition.
// `SourceSyncManager::ensure_current` returns it, and `ensure_synced`
// delegates to it — both must agree on the type.
pub use sync::SyncedProject;

/// UUID generated once per `slugaudit-mcp` process boot, stamped onto
/// every `findings` row written by this process and used by
/// `sync::manager_meta::purge_prior_session_findings` to delete findings
/// left by a prior agent session at the next `ensure_current`. A
/// `Mutex<Option<…>>` (not `OnceLock<…>`) so parallel-running tests can
/// override the active value mid-suite; the production code path takes
/// the lock once per call and holds it only long enough to read or
/// initialize the inner `Option<Uuid>`.
static SESSION_ID: Mutex<Option<Uuid>> = Mutex::new(None);

/// Returns the current session UUID, allocating a fresh v4 on first
/// call. After the first call the inner `Option` is `Some`, so every
/// subsequent call is a single lock + clone and never regenerates.
#[must_use]
pub(crate) fn session_id() -> Uuid {
    let mut guard = SESSION_ID.lock().expect("session id mutex poisoned");
    if guard.is_none() {
        *guard = Some(Uuid::new_v4());
    }
    guard.expect("just initialized")
}

/// Tests override the live session ID to simulate a fresh process boot
/// without spawning a new binary. Production code never calls this.
#[cfg(test)]
pub(crate) fn override_session_id_for_test(id: Uuid) {
    let mut guard = SESSION_ID.lock().expect("session id mutex poisoned");
    *guard = Some(id);
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

#[cfg(test)]
#[path = "context_transaction_tests.rs"]
mod transaction_tests;
