//! The `health` tool's per-project field derivation: resolve the project,
//! read the database state, and map it to a phase. Kept apart from `health.rs`
//! so the response schema stays separate from the derivation that fills it.
//! It never calls `ensure_current`, so a health call can never trigger a
//! publish.

use super::health::{DatabaseRead, DatabaseState, HealthPhase, read_database};
use crate::error_codes::ErrorCode;
use rmcp::ErrorData;

/// The per-project half of a [`super::health::HealthResponse`]: everything
/// that depends on which project is being served. The two builders below
/// return one of these and `health` fills in the process-wide remainder, so
/// neither builder has to know about counters or host memory.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ProjectHealthFields {
    pub(super) phase: HealthPhase,
    pub(super) remedy: Option<String>,
    pub(super) database: Option<DatabaseState>,
    /// True when the database exists but could not be read. `database` is
    /// then `None` and the phase never claims the index is empty.
    pub(super) database_read_failed: bool,
}

/// Computes every per-project field of the health response without
/// publishing: it resolves the project root and reads the database
/// read-only if it exists.
pub(super) fn compute_project_state(path: &str) -> Result<ProjectHealthFields, ErrorData> {
    let Some((_root, database_path)) = resolve_active_project(path)? else {
        return Ok(inactive_fields(HealthPhase::Disabled));
    };

    let (database, revision_id, database_read_failed) = match read_database(&database_path) {
        DatabaseRead::Read(state) => {
            let revision_id = state.revision_id.clone();
            (Some(state), revision_id, false)
        }
        DatabaseRead::NeverSynced => (None, None, false),
        DatabaseRead::Unreadable => (None, None, true),
    };

    Ok(ProjectHealthFields {
        phase: project_phase(revision_id.as_deref(), database_read_failed),
        remedy: None,
        database,
        database_read_failed,
    })
}

/// Resolves the project root and its database path. `Ok(None)` means the
/// path has no activation directory: `health` reports that state instead of
/// failing on it, because "this path is not an enabled project, here is the
/// call that enables it" must be an answer. Any *other* resolution failure
/// (an unreadable parent, a symlinked activation component) stays an error:
/// enabling the project would not fix it.
fn resolve_active_project(
    path: &str,
) -> Result<Option<(crate::project::ProjectRoot, std::path::PathBuf)>, ErrorData> {
    match crate::project::resolve_project(std::path::Path::new(path)) {
        Ok(resolved) => Ok(Some(resolved)),
        Err(crate::project::ActivationError::NotActive) => Ok(None),
        Err(error) => Err(ErrorData::invalid_params(error.to_string(), None)),
    }
}

/// Maps the database state to a phase. An unreadable database makes the
/// revision unknowable and must never read as "the first sync never
/// succeeded".
fn project_phase(revision_id: Option<&str>, database_read_failed: bool) -> HealthPhase {
    if database_read_failed {
        return HealthPhase::Unknown;
    }
    if revision_id.is_some() {
        HealthPhase::SteadyState
    } else {
        HealthPhase::Importing
    }
}

/// The field set for a project this answer is not serving: nothing to
/// report but the phase and the way out of it. Both per-project groups are
/// absent for the same reason — there is no project to open a database for.
///
/// The two callers reach this with the same absence and differ only in
/// which phase names it: `Disabled` when the path has no activation
/// directory, `NoActiveProject` when no path was supplied at all. The remedy
/// is the same either way.
fn inactive_fields(phase: HealthPhase) -> ProjectHealthFields {
    ProjectHealthFields {
        phase,
        remedy: Some(ErrorCode::ProjectNotEnabled.remedy().to_owned()),
        database: None,
        database_read_failed: false,
    }
}

/// Builds a per-project field set from the global state only — no
/// `ensure_synced`, no database call. Used when the caller passed no
/// `path`.
pub(super) fn compute_global_only_state() -> ProjectHealthFields {
    inactive_fields(HealthPhase::NoActiveProject)
}
