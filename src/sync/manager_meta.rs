//! Project-metadata helpers used by `SourceSyncManager`.
//!
//! Split out from `manager.rs` so the sync orchestrator's hot path is
//! shorter than the small-file rule cap. These helpers are not part
//! of the orchestrator's runtime concerns — they mediate the project's
//! SQLite metadata row and the revision lookup that always succeeds or
//! always returns `None`, respectively.

use rmcp::ErrorData;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

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
