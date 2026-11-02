// slugaudit-line-exception: approved-by=slugthuglabs; reason=the project-metadata row, the revision read and the publish error mapping form one cohesive helper set
//! Project-metadata helpers used by `SourceSyncManager`.
//!
//! Kept apart from `manager.rs` so the sync orchestrator's hot path is
//! shorter than the small-file rule cap. These helpers are not part
//! of the orchestrator's runtime concerns — they mediate the project's
//! SQLite metadata row and the revision lookup that always succeeds or
//! always returns `None`, respectively.

use super::SyncedProject;
use super::publish;
use super::publish_log;
use super::revision::current_revision_id;
use crate::error_codes::{self, ErrorCode};
use crate::parse;
use crate::project;
use crate::store;
use rmcp::ErrorData;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Reads the revision id the database holds after a sync. Every field is
/// present or the call errors with a named reason — a successful sync that
/// cannot name its revision is a bug an operator must be able to see. The
/// error names the database, and is logged once with it by
/// `server_runner::log_completion`.
pub(crate) fn revision_after_sync(
    connection: &Connection,
    database_path: &Path,
) -> Result<String, ErrorData> {
    current_revision_id(connection)
        .map_err(|error| {
            ErrorData::internal_error(
                format!(
                    "reading the current revision of {}: {error}",
                    database_path.display()
                ),
                None,
            )
        })?
        .ok_or_else(|| {
            ErrorData::internal_error(
                format!(
                    "no current revision found after sync in {} — this is unexpected; \
                     try disabling and re-enabling the project",
                    database_path.display()
                ),
                None,
            )
        })
}

/// Errors from verifying a project's metadata row.
#[derive(Debug)]
pub(crate) enum ProjectMetaError {
    /// The database at this path was built by a newer/incompatible
    /// SlugAudit (contract/schema version no longer supported). The
    /// derived data is stale, so the caller should discard and rebuild it
    /// from source rather than failing the tool call.
    IncompatibleVersion {
        contract_version: i64,
        schema_version: i64,
    },
    /// The database at this path was previously published for a different
    /// project root — the repo was moved, copied, or re-extracted from an
    /// archive. The derived database is stale, so the caller should discard
    /// and rebuild it from source rather than failing the tool call.
    RootMismatch { stored_root: String },
    /// Any other metadata failure, surfaced to the tool call as-is.
    Other(ErrorData),
}

impl From<ProjectMetaError> for ErrorData {
    fn from(value: ProjectMetaError) -> Self {
        match value {
            ProjectMetaError::Other(error) => error,
            // A standalone surface for stale-database variants, used where
            // the caller chose not to auto-recover. The recovery path in
            // `SourceSyncManager::ensure_current` never produces these, so
            // they only show up in direct callers.
            ProjectMetaError::RootMismatch { stored_root } => ErrorData::invalid_params(
                format!(
                    "database at this location was built for a different project root \
                     ({stored_root}); discard the .planning/slugaudit directory (or \
                     re-enable the project) so it is rebuilt from source"
                ),
                None,
            ),
            ProjectMetaError::IncompatibleVersion {
                contract_version,
                schema_version,
            } => ErrorData::invalid_params(
                format!(
                    "database at this location was built by an unsupported SlugAudit \
                     contract/schema version ({contract_version}/{schema_version}); discard \
                     the .planning/slugaudit directory (or re-enable the project) so it is \
                     rebuilt from source"
                ),
                None,
            ),
        }
    }
}

/// Creates the project's singleton metadata row on first sync if it
/// doesn't already exist, then verifies the stored `root_path` still
/// matches the canonical root this process resolved on later syncs.
///
/// This function is metadata-only.
///
/// The `INSERT OR IGNORE` is safe across concurrent first syncs because
/// the row is keyed on `id = 1`, a fixed constant. The subsequent
/// `SELECT` is the actual verification, comparing the stored root
/// against the canonical one. A mismatch returns
/// [`ProjectMetaError::RootMismatch`] so the caller can recover by
/// discarding the disposable database and re-publishing — see
/// `SourceSyncManager::ensure_current`.
pub(crate) fn ensure_project_row(
    connection: &mut Connection,
    root: &Path,
) -> Result<(), ProjectMetaError> {
    const CONTRACT_VERSION: i64 = 1;
    const SCHEMA_VERSION: i64 = 1;

    let root_path = root.to_string_lossy();
    let created_at =
        crate::util::now_unix().map_err(|error| ProjectMetaError::Other(error.into()))?;

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
            ProjectMetaError::Other(ErrorData::internal_error(
                format!("recording project metadata: {error}"),
                None,
            ))
        })?;

    let (stored_root_path, contract_version, schema_version): (String, i64, i64) = connection
        .query_row(
            "SELECT root_path, contract_version, schema_version FROM project WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| {
            ProjectMetaError::Other(ErrorData::internal_error(
                format!("reading project metadata: {error}"),
                None,
            ))
        })?;

    if contract_version != CONTRACT_VERSION || schema_version != SCHEMA_VERSION {
        return Err(ProjectMetaError::IncompatibleVersion {
            contract_version,
            schema_version,
        });
    }
    if stored_root_path != root_path {
        return Err(ProjectMetaError::RootMismatch {
            stored_root: stored_root_path,
        });
    }
    Ok(())
}

/// Single source of truth for "publish returned `Err` -> attach the
/// manager operation's context and return an `ErrorData::internal_error`".
/// Used by every `ensure_current` branch that calls `publish` so a
/// wire-shape change lands in one place rather than three.
///
/// Nothing is logged here: the message carries the cause, the manager
/// `operation`, the project root, the failure category and the file it names
/// ([`publish_log::failure_context`]), and `server_runner::log_completion`
/// logs that message once when the tool call returns it.
pub(crate) fn map_publish_error(
    root: &Path,
    error: &publish::PublishError,
    operation: &str,
) -> ErrorData {
    // The two publish failures a caller would do something different about.
    // A spent budget is permanent on a repo too big for the limit, so its
    // remedy names the knob rather than advising the retry that fails
    // identically; the CAS race is the opposite — retrying re-samples and
    // succeeds.
    let budget = error.budget_elapsed_ms();
    let data = if budget.is_some() {
        error_codes::budget_payload(budget)
    } else if matches!(error, publish::PublishError::ChangedDuringSample { .. }) {
        error_codes::payload(ErrorCode::StaleRevision)
    } else {
        None
    };
    ErrorData::internal_error(
        format!(
            "publishing a new revision: {error} ({operation}; {})",
            publish_log::failure_context(root, error)
        ),
        data,
    )
}

/// Rebuilds a project's database from scratch after the previous one was
/// discarded as corrupt. Kept here (not in `manager.rs`) so the sync
/// orchestrator stays under the small-file rule cap.
pub(crate) fn publish_from_scratch(
    root: &project::ProjectRoot,
    database_path: PathBuf,
    sink: &dyn crate::progress::ProgressSink,
) -> Result<SyncedProject, ErrorData> {
    let mut connection = store::open_read_write(&database_path).map_err(|error| {
        // Only `project_control on` creates `.planning/slugaudit`. If it is
        // gone (`off` ran, or the user deleted it) the file cannot be
        // created and the directory must stay gone: report that, not a bare
        // I/O error. The error is returned to the tool call, which logs it
        // once (`server_runner::log_completion`).
        if matches!(error, store::StoreError::Create(_))
            && database_path.parent().is_some_and(|dir| !dir.is_dir())
        {
            return ErrorData::invalid_params(
                "the project was turned off during this call: .planning/slugaudit no longer \
                 exists. Call project_control with action \"on\" to enable it again",
                error_codes::payload(ErrorCode::ProjectNotEnabled),
            );
        }
        ErrorData::internal_error(format!("recreating the project database: {error}"), None)
    })?;
    ensure_project_row(&mut connection, root.as_path())?;
    let report =
        publish(&mut connection, root.as_path(), parse::PACK_VERSION, sink).map_err(|error| {
            map_publish_error(root.as_path(), &error, "republishing after corruption")
        })?;
    drop(connection);
    Ok(SyncedProject {
        database_path,
        revision_id: report.revision_id,
        discard: None,
    })
}

#[cfg(test)]
#[path = "manager_meta_tests.rs"]
mod tests;
