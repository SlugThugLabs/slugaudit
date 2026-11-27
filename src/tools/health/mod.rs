//! `health` MCP tool: read-only server, database, host-memory, and per-project
//! result-store snapshot without triggering a sync.

mod detail;

use detail::HealthDetail;

use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::project::ProjectRoot;
use crate::store;
use crate::sync;
use detail::collect_detail;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Request for the `health` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HealthRequest {
    /// Optional project root. When supplied, returns the database state for
    /// that project. When omitted, reports that no project is selected and
    /// names the call that enables one. Read-only: never syncs or mutates.
    #[schemars(default)]
    pub path: Option<String>,
    /// Also return `detail`: the operator view (parser pack version, skipped
    /// files, whether the database read failed, process counters, host
    /// memory, result store). Defaults to false.
    #[serde(default)]
    pub verbose: bool,
}

/// Response from the `health` tool.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HealthResponse {
    /// Coarse classification of what the server is doing right now.
    pub phase: HealthPhase,
    /// The next action when no project is being served by this answer.
    pub remedy: Option<String>,
    /// What the project database holds; null when never synced or unreadable.
    pub database: Option<DatabaseSummary>,
    /// Operator detail, present only with `verbose: true`; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "HealthDetail", default)]
    pub detail: Option<HealthDetail>,
}

/// The current revision and file count, as the default answer carries them.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DatabaseSummary {
    /// `revision_id` of the current revision; null if nothing has been published yet.
    pub revision_id: Option<String>,
    /// Files in the database, including discovered files that were skipped.
    pub file_count: i64,
}

/// Coarse-grained operational phase.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum HealthPhase {
    /// Enabled project exists, no revision published yet.
    Importing,
    /// Published revision is readable on disk.
    SteadyState,
    /// `path` was omitted.
    NoActiveProject,
    /// Enabled project's database exists on disk but failed to read.
    Unknown,
    /// Supplied `path` is not inside an enabled project.
    Disabled,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DatabaseState {
    pub(crate) revision_id: Option<String>,
    pub(crate) parser_pack_version: Option<String>,
    pub(crate) file_count: i64,
    pub(crate) skipped_file_count: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DatabaseRead {
    NeverSynced,
    Unreadable(String),
    Read(DatabaseState),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ProjectHealthFields {
    pub(super) active_root: Option<ProjectRoot>,
    pub(super) phase: HealthPhase,
    pub(super) remedy: Option<String>,
    pub(super) database: Option<DatabaseState>,
    pub(super) database_read_error: Option<String>,
}

fn unreadable(database_path: &Path, step: &str, error: impl std::fmt::Display) -> DatabaseRead {
    tracing::warn!(
        path = %database_path.display(),
        error = %error,
        step,
        "health: failed to inspect or read project database; reporting empty database fields",
    );
    DatabaseRead::Unreadable(format!("{step}: {error}"))
}

pub(crate) fn read_database(database_path: &Path) -> DatabaseRead {
    match database_path.try_exists() {
        Ok(true) => {}
        Ok(false) => return DatabaseRead::NeverSynced,
        Err(error) => {
            return unreadable(database_path, "inspecting the project database path", error);
        }
    }
    let mut connection = match store::open_existing(database_path) {
        Ok(connection) => connection,
        Err(error) => {
            return unreadable(
                database_path,
                "opening the project database read-only",
                error,
            );
        }
    };
    match query_database_state(&mut connection) {
        Ok(state) => DatabaseRead::Read(state),
        Err(error) => unreadable(database_path, "reading the project database", error),
    }
}

fn query_database_state(connection: &mut rusqlite::Connection) -> rusqlite::Result<DatabaseState> {
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
    let (revision_id, parser_pack_version) = match sync::current_revision_row(&tx)? {
        Some((rev, pack)) => (Some(rev), Some(pack)),
        None => (None, None),
    };
    let file_count: i64 = tx.query_row("SELECT count(*) FROM files", [], |row| row.get(0))?;
    let skipped_file_count: i64 = tx.query_row(
        "SELECT count(*) FROM files WHERE file_kind = 'skipped'",
        [],
        |row| row.get(0),
    )?;
    tx.commit()?;
    Ok(DatabaseState {
        revision_id,
        parser_pack_version,
        file_count,
        skipped_file_count,
    })
}

pub(super) fn compute_project_state(path: &str) -> Result<ProjectHealthFields, ErrorData> {
    let (root, database_path) = match crate::project::resolve_project(Path::new(path)) {
        Ok(resolved) => resolved,
        Err(crate::project::ActivationError::NotActive) => {
            return Ok(inactive_fields(HealthPhase::Disabled));
        }
        Err(crate::project::ActivationError::Lock(poisoned)) => return Err(poisoned.into()),
        Err(error) => {
            return Err(invalid_params_with_code(
                error.to_string(),
                ErrorCode::InvalidInput,
            ));
        }
    };

    let (database, database_read_error) = match read_database(&database_path) {
        DatabaseRead::Read(state) => (Some(state), None),
        DatabaseRead::NeverSynced => (None, None),
        DatabaseRead::Unreadable(error) => (None, Some(error)),
    };
    let phase = if database_read_error.is_some() {
        HealthPhase::Unknown
    } else if database
        .as_ref()
        .and_then(|d| d.revision_id.as_deref())
        .is_some()
    {
        HealthPhase::SteadyState
    } else {
        HealthPhase::Importing
    };

    Ok(ProjectHealthFields {
        active_root: Some(root),
        phase,
        remedy: None,
        database,
        database_read_error,
    })
}

fn inactive_fields(phase: HealthPhase) -> ProjectHealthFields {
    ProjectHealthFields {
        active_root: None,
        phase,
        remedy: Some(ErrorCode::ProjectNotEnabled.remedy().to_owned()),
        database: None,
        database_read_error: None,
    }
}

pub(super) fn compute_global_only_state() -> ProjectHealthFields {
    inactive_fields(HealthPhase::NoActiveProject)
}

/// Composes the read-only health snapshot for `path` (or global-only when `path` is `None`).
///
/// # Errors
///
/// Returns `invalid_params` when `path` exceeds `MAX_PATH_BYTES` or cannot be
/// resolved, or an internal error when a server lock is poisoned.
pub fn health(
    request: &Parameters<HealthRequest>,
    _sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<HealthResponse>, ErrorData> {
    let HealthRequest { path, verbose } = &request.0;
    super::check_optional_path_bytes(path.as_deref())?;

    let fields = match path.as_deref() {
        Some(p) => compute_project_state(p)?,
        None => compute_global_only_state(),
    };
    let detail = if *verbose {
        Some(collect_detail(manager, &fields, fields.database.as_ref())?)
    } else {
        None
    };

    Ok(Json(HealthResponse {
        phase: fields.phase,
        remedy: fields.remedy,
        database: fields.database.map(|database| DatabaseSummary {
            revision_id: database.revision_id,
            file_count: database.file_count,
        }),
        detail,
    }))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod detail_tests;
