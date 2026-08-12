//! Project-metadata helpers used by `SourceSyncManager`.
//!
//! Split out from `manager.rs` so the sync orchestrator's hot path is
//! shorter than the small-file rule cap. These helpers are not part
//! of the orchestrator's runtime concerns — they mediate the project's
//! SQLite metadata row, the session-scoped cleanup of prior
//! agent's findings, and the revision lookup that always succeeds or
//! always returns `None`, respectively.

use crate::tools::context::session_id;
use rmcp::ErrorData;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
#[cfg(test)]
use uuid::Uuid;

/// Deletes every `findings` row whose `session_id` does not match the
/// caller's current session UUID. This is the cross-session
/// poisoning defense: a fresh `slugaudit-mcp` process sees the prior
/// session's reasoning conclusions wiped on its first `ensure_current`,
/// so a new agent (new chat, new model, new reasoning context) is not
/// silently inheriting the audit conclusions of whoever was here last.
///
/// Side-effect-free against an empty `findings` table; against a large
/// table the cleanup is bounded to the index-assisted `DELETE` whose
/// cost matches a `DELETE WHERE session_id = ?` — both run on the
/// existing `idx_findings_session` index. Caller controls concurrency
/// (`&mut Connection`), so this composes naturally into the same
/// transaction that `ensure_project_row` opens.
pub(crate) fn purge_prior_session_findings(connection: &mut Connection) -> Result<(), ErrorData> {
    connection
        .execute(
            "DELETE FROM findings WHERE session_id != ?1",
            rusqlite::params![session_id().to_string()],
        )
        .map_err(|error| {
            ErrorData::internal_error(format!("purging prior-session findings: {error}"), None)
        })?;
    Ok(())
}

/// Same semantics as [`purge_prior_session_findings`] but takes the
/// session UUID explicitly so tests can drive the cleanup without
/// touching the live `SESSION_ID` mutex.
/// Production-only callers should use the no-arg form.
#[cfg(test)]
pub(crate) fn purge_prior_session_findings_with(
    connection: &mut Connection,
    current_session_id: Uuid,
) -> Result<(), ErrorData> {
    connection
        .execute(
            "DELETE FROM findings WHERE session_id != ?1",
            rusqlite::params![current_session_id.to_string()],
        )
        .map_err(|error| {
            ErrorData::internal_error(format!("purging prior-session findings: {error}"), None)
        })?;
    Ok(())
}

/// Returns the `revision_id` of the current revision, or `None` if no
/// revision has been published yet.
///
/// Used by both `ensure_current` (after a healthy/incremental pass)
/// and `reconcile` (to detect concurrent publish/revoke and report a
/// stale-baseline error). Kept here because both call sites need
/// identical SQL against identical schema.
pub(crate) fn current_revision_id(
    connection: &Connection,
) -> Result<Option<String>, rusqlite::Error> {
    connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .optional()
}

/// Creates the project's singleton metadata row on first sync if it
/// doesn't already exist, then verifies the stored `root_path` still
/// matches the canonical root this process resolved on later syncs.
///
/// Also purges findings left by any prior agent session before doing
/// either — a fresh `slugaudit-mcp` boot sees the prior reasoning
/// session's findings wiped on its first sync so a different agent
/// (different chat, different model, different chain of thought) does
/// not silently inherit the audit conclusions of whoever was here
/// last. Both the metadata row creation and the session-scoped finding
/// cleanup run on the same `&mut Connection`, so callers get a single
/// all-or-nothing session-start state transition.
///
/// The `INSERT OR IGNORE` is safe across concurrent first syncs because
/// the row is keyed on `id = 1`, a fixed constant. The subsequent
/// `SELECT` is the actual verification, comparing the stored root
/// against the canonical one — a mismatch is reported as `invalid_params`
/// rather than `internal_error` since the user can recover by disabling
/// and re-enabling.
pub(crate) fn ensure_project_row(
    connection: &mut Connection,
    root: &Path,
) -> Result<(), ErrorData> {
    const CONTRACT_VERSION: i64 = 1;
    const SCHEMA_VERSION: i64 = 1;

    // Session-scoped finding cleanup runs first so any violation of
    // the prior-session invariant is surfaced before we touch the
    // project metadata row. A failure here aborts the whole
    // session-start, matching the rest of `ensure_project_row`'s
    // all-or-nothing semantics.
    purge_prior_session_findings(connection)?;

    let root_path = root.to_string_lossy();
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        });

    connection
        .execute(
            "INSERT OR IGNORE INTO project (\
                id, project_id, root_path, contract_version, schema_version, created_at_unix\
             ) VALUES (1, 'default', ?1, ?2, ?3, ?4)",
            rusqlite::params![
                root_path.as_ref(),
                CONTRACT_VERSION,
                SCHEMA_VERSION,
                created_at
            ],
        )
        .map_err(|error| {
            ErrorData::internal_error(format!("recording project metadata: {error}"), None)
        })?;

    let (stored_root_path, contract_version, schema_version): (String, i64, i64) = connection
        .query_row(
            "SELECT root_path, contract_version, schema_version FROM project WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| {
            ErrorData::internal_error(format!("reading project metadata: {error}"), None)
        })?;

    if contract_version != CONTRACT_VERSION || schema_version != SCHEMA_VERSION {
        return Err(ErrorData::internal_error(
            format!("unsupported contract/schema version ({contract_version}/{schema_version})"),
            None,
        ));
    }
    if stored_root_path != root_path {
        return Err(ErrorData::invalid_params(
            format!(
                "database at this location belongs to a different project root \
                 (expected {root_path}, found {stored_root_path})"
            ),
            None,
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "manager_meta_tests.rs"]
mod tests;
