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
use rusqlite::{Connection, OptionalExtension};
use std::path::{Path, PathBuf};

/// Reads the per-database instance token (`project.project_id`) from the
/// singleton `project` metadata row, or `None` if no metadata row exists yet.
pub(crate) fn project_instance_id(
    connection: &Connection,
) -> Result<Option<String>, rusqlite::Error> {
    connection
        .query_row("SELECT project_id FROM project WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()
}

/// Reads the database instance token after a sync, shaping any SQLite error
/// with `database_path`.
pub(crate) fn instance_id_after_sync(
    connection: &Connection,
    database_path: &Path,
) -> Result<Option<String>, ErrorData> {
    project_instance_id(connection).map_err(|error| {
        ErrorData::internal_error(
            format!(
                "reading the project instance id of {}: {error}",
                database_path.display()
            ),
            None,
        )
    })
}

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
    /// The database returned a SQLite corruption error while reading or
    /// recording the project metadata row. The derived database is damaged,
    /// so the caller should discard and rebuild it from source rather than
    /// failing the tool call.
    Corrupt(rusqlite::Error),
    /// Any other metadata failure, surfaced to the tool call as-is.
    Other(ErrorData),
}

impl std::fmt::Display for ProjectMetaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncompatibleVersion {
                contract_version,
                schema_version,
            } => write!(
                f,
                "database has an unsupported contract/schema version \
                 ({contract_version}/{schema_version})"
            ),
            Self::RootMismatch { stored_root } => {
                write!(
                    f,
                    "database was built for a different project root ({stored_root})"
                )
            }
            Self::Corrupt(error) => write!(f, "project database is corrupt: {error}"),
            Self::Other(error) => write!(f, "{}", error.message),
        }
    }
}

impl From<ProjectMetaError> for ErrorData {
    fn from(value: ProjectMetaError) -> Self {
        match value {
            ProjectMetaError::Other(error) => error,
            ProjectMetaError::Corrupt(error) => ErrorData::internal_error(
                format!("project database is corrupt while verifying metadata: {error}"),
                None,
            ),
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

const PROJECT_CONTRACT_VERSION: i64 = 1;
const PROJECT_SCHEMA_VERSION: i64 = 1;
const PROJECT_META_SELECT_SQL: &str =
    "SELECT contract_version, schema_version, root_path FROM project WHERE id = 1";

/// Verifies the stored metadata row matches the current contract/schema
/// versions and canonical project root.
fn verify_project_row(
    contract_version: i64,
    schema_version: i64,
    stored_root: String,
    expected_root: &str,
) -> Result<(), ProjectMetaError> {
    if contract_version != PROJECT_CONTRACT_VERSION || schema_version != PROJECT_SCHEMA_VERSION {
        return Err(ProjectMetaError::IncompatibleVersion {
            contract_version,
            schema_version,
        });
    }
    if stored_root != expected_root {
        return Err(ProjectMetaError::RootMismatch { stored_root });
    }
    Ok(())
}

/// Maps a SQLite failure from reading or recording project metadata into
/// [`ProjectMetaError::Corrupt`] or [`ProjectMetaError::Other`].
fn map_metadata_sql_error(error: rusqlite::Error, context: &str) -> ProjectMetaError {
    if store::is_rusqlite_corruption(&error) {
        return ProjectMetaError::Corrupt(error);
    }
    ProjectMetaError::Other(ErrorData::internal_error(
        format!("{context}: {error}"),
        None,
    ))
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
    let expected_root = root.to_string_lossy();
    match connection
        .query_row(PROJECT_META_SELECT_SQL, [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .optional()
    {
        Ok(Some((contract_version, schema_version, stored_root))) => verify_project_row(
            contract_version,
            schema_version,
            stored_root,
            expected_root.as_ref(),
        ),
        Ok(None) => {
            let created_at =
                crate::util::now_unix().map_err(|error| ProjectMetaError::Other(error.into()))?;
            let instance_id = uuid::Uuid::new_v4().to_string();
            connection
                .execute(
                    "INSERT OR IGNORE INTO project (\
                        id, project_id, root_path, contract_version, schema_version, created_at_unix\
                     ) VALUES (1, ?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        instance_id,
                        expected_root.as_ref(),
                        PROJECT_CONTRACT_VERSION,
                        PROJECT_SCHEMA_VERSION,
                        created_at
                    ],
                )
                .map_err(|error| map_metadata_sql_error(error, "recording project metadata"))?;
            let (contract_version, schema_version, stored_root) = connection
                .query_row(PROJECT_META_SELECT_SQL, [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .map_err(|error| map_metadata_sql_error(error, "reading project metadata"))?;
            verify_project_row(
                contract_version,
                schema_version,
                stored_root,
                expected_root.as_ref(),
            )
        }
        Err(error) => Err(map_metadata_sql_error(error, "reading project metadata")),
    }
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
    } else if matches!(
        error,
        publish::PublishError::ChangedDuringSample { .. }
            | publish::PublishError::Revision(super::revision::RevisionError::StaleBaseline { .. })
    ) {
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
        if matches!(error, store::StoreError::Open(_))
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
    let db_instance_id = instance_id_after_sync(&connection, &database_path)?;
    drop(connection);
    Ok(SyncedProject {
        database_path,
        revision_id: report.revision_id,
        db_instance_id,
        discard: None,
    })
}

#[cfg(test)]
#[path = "manager_meta_tests.rs"]
mod tests;
