//! The `health` tool's per-project field derivation: resolve the project,
//! read watcher and database state read-only, and map that state to a phase.
//! Split from `health.rs` so the response schema stays separate from the
//! derivation that fills it. Nothing here has a side effect — in particular
//! it never calls `ensure_current`, so a health call can never trigger a
//! publish, register a watch, or change watcher health.

use super::health::{DatabaseState, HealthPhase, WatcherState};
use super::health_database::read_database;
use crate::error_codes::ErrorCode;
use crate::sync;
use crate::watch;
use rmcp::ErrorData;

/// The per-project half of a [`super::health::HealthResponse`]: everything
/// that depends on which project is being served. The two builders below
/// return one of these and `health` fills in the process-wide remainder, so
/// neither builder has to know about counters or host memory.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ProjectHealthFields {
    pub(super) phase: HealthPhase,
    pub(super) remedy: Option<String>,
    pub(super) watcher: Option<WatcherState>,
    pub(super) database: Option<DatabaseState>,
}

/// Computes every per-project field of the health response **without any
/// side effect**: it resolves the project root, reads the watcher state if
/// the project is already registered, and reads the database read-only if
/// it exists.
pub(super) fn compute_project_state(
    path: &str,
    manager: &sync::SourceSyncManager,
) -> Result<ProjectHealthFields, ErrorData> {
    let Some((root, database_path)) = resolve_active_project(path)? else {
        return Ok(inactive_fields(HealthPhase::Disabled));
    };
    // Read-only: `watch_state_for` never registers a watch and never
    // changes watcher health (unlike `activate`).
    let snapshot = manager
        .watch_state_for(root.as_path())
        .map(|state| state.snapshot());

    let database = read_database(&database_path);
    let revision_id = database
        .as_ref()
        .and_then(|state| state.revision_id.clone());

    Ok(ProjectHealthFields {
        phase: project_phase(snapshot.as_ref(), &revision_id),
        remedy: None,
        watcher: snapshot.as_ref().map(watcher_state),
        database,
    })
}

/// Resolves the project root and its database path. `Ok(None)` means the
/// path has no activation marker: `health` reports that state instead of
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

/// Maps a watcher snapshot to a phase. No watcher state means the project
/// was never registered with this server's watcher (or the server just
/// started); database fields, if any, are still surfaced, distinguishing a
/// queryable indexed project from one that has never published a revision.
fn project_phase(
    snapshot: Option<&watch::ProjectWatchState>,
    revision_id: &Option<String>,
) -> HealthPhase {
    match snapshot {
        Some(snapshot) => derive_phase(
            snapshot.health,
            revision_id,
            snapshot.has_unreconciled_events(),
        ),
        None if revision_id.is_some() => HealthPhase::IndexedUnwatched,
        None => HealthPhase::NoActiveProject,
    }
}

/// The reported fields of one watcher snapshot.
fn watcher_state(snapshot: &watch::ProjectWatchState) -> WatcherState {
    WatcherState {
        health: snapshot.health,
        pending_dirty: snapshot.dirty_paths.len(),
        pending_deleted: snapshot.deleted_paths.len(),
        sequence: snapshot.watcher_sequence,
        last_verified_sequence: snapshot.last_verified_sequence,
    }
}

/// The field set for a project this server is not serving: nothing to
/// report but the phase and the way out of it. Both per-project groups are
/// absent for the same reason — there is no watcher to ask and no database
/// to open.
///
/// The two callers reach this with the same absence and differ only in
/// which phase names it: `Disabled` when the path has no activation
/// marker, `NoActiveProject` when no path was supplied and nothing is
/// registered. The remedy is the same either way.
fn inactive_fields(phase: HealthPhase) -> ProjectHealthFields {
    ProjectHealthFields {
        phase,
        remedy: Some(ErrorCode::ProjectNotEnabled.remedy().to_owned()),
        watcher: None,
        database: None,
    }
}

/// Builds a per-project field set from the global state only — no
/// `ensure_synced`, no database call. Used when the caller passed no
/// `path`.
pub(super) fn compute_global_only_state(manager: &sync::SourceSyncManager) -> ProjectHealthFields {
    let snapshots = manager.watch_states_snapshot();
    if snapshots.is_empty() {
        return inactive_fields(HealthPhase::NoActiveProject);
    }
    // In the current single-active-project model, snapshots has one
    // entry; reduce it to the reported fields. With more than one
    // registered project, one snapshot is reported and which one is
    // unspecified (the manager stores them in a `HashMap`; see the
    // `health` tool description). Never silently combine them.
    //
    // `is_empty()` was checked above, so `None` here is unreachable by
    // construction — but matching instead of `expect` keeps the "no
    // panic in production code" invariant literal rather than relying
    // on the guard staying in place.
    let Some(snapshot) = snapshots.into_iter().next() else {
        return inactive_fields(HealthPhase::NoActiveProject);
    };
    ProjectHealthFields {
        phase: derive_global_phase(snapshot.health, snapshot.has_unreconciled_events()),
        remedy: None,
        watcher: Some(watcher_state(&snapshot)),
        database: None,
    }
}

/// Maps the watcher's health to a phase for the pathless snapshot.
///
/// This answer has no project root and therefore no revision to consult, so
/// it must not claim one: a `Healthy` (or `Partial`) watcher with no
/// unreconciled events is [`HealthPhase::Unknown`], never `Importing`
/// ("the first sync has never succeeded") and never `SteadyState` ("a
/// current revision exists"). The watcher-only conditions carry over from
/// [`derive_phase`]: `Unavailable` and the two untrusted states, plus
/// pending reconciliation for a trusted watcher.
fn derive_global_phase(
    watcher_health: watch::WatcherHealth,
    has_unreconciled_events: bool,
) -> HealthPhase {
    match watcher_health {
        watch::WatcherHealth::Unavailable => HealthPhase::Unavailable,
        watch::WatcherHealth::NeedsVerification | watch::WatcherHealth::Desynced => {
            HealthPhase::Restoring
        }
        watch::WatcherHealth::Healthy | watch::WatcherHealth::Partial
            if has_unreconciled_events =>
        {
            HealthPhase::Restoring
        }
        watch::WatcherHealth::Healthy | watch::WatcherHealth::Partial => HealthPhase::Unknown,
    }
}

/// Maps `(watcher_health, current_revision, has_unreconciled_events)`
/// to a coarse `HealthPhase`. Pure function over existing state — no
/// I/O — so the phase agrees with the watcher-snapshot fields it
/// derives from.
pub(super) fn derive_phase(
    watcher_health: watch::WatcherHealth,
    revision_id: &Option<String>,
    has_unreconciled_events: bool,
) -> HealthPhase {
    match watcher_health {
        watch::WatcherHealth::Unavailable => HealthPhase::Unavailable,
        watch::WatcherHealth::NeedsVerification | watch::WatcherHealth::Desynced => {
            HealthPhase::Restoring
        }
        watch::WatcherHealth::Healthy => {
            if revision_id.is_none() {
                HealthPhase::Importing
            } else if has_unreconciled_events {
                HealthPhase::Restoring
            } else {
                HealthPhase::SteadyState
            }
        }
        watch::WatcherHealth::Partial => {
            // SteadyState means the project is fully served from disk; a
            // partial watch scope means part of the tree has no watcher, so
            // the same revision/event conditions yield Degraded instead.
            if revision_id.is_none() {
                HealthPhase::Importing
            } else if has_unreconciled_events {
                HealthPhase::Restoring
            } else {
                HealthPhase::Degraded
            }
        }
    }
}
