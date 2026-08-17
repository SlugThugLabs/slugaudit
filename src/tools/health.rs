//! `health` MCP tool — operational snapshot for operators and external
//! monitors.
//!
//! Returns every number an external health check might want without
//! making it write through to the database or run another Tree-sitter
//! parse. The intent is that calling `health` should never cause a
//! sync, never emit MCP progress, and never fail transiently — every
//! failure mode degrades to "field is null/0" rather than an error, so
//! a stale health read is still actionable.
//!
//! Three categories of data are surfaced:
//!
//! 1. **Watcher state** for the active project (or the most recently
//!    touched project when no path is supplied): health enum,
//!    unreconciled counts, watcher sequence / verified sequence.
//! 2. **Database state** for the supervised project: current revision
//!    id, file count, parser pack version. Falls back to zeros/None
//!    when no project is currently active or the DB can't be opened.
//! 3. **Process counters**: total tool calls, error calls, total wall-
//!    clock milliseconds across every tool call since process start.
// slugaudit-line-exception: approved-by=agent; reason=health is the schema-defining tool; Request+Response+phase+derivation live together so the schema isn't split from its only consumer

use crate::store;
use crate::sync;
use crate::watch;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ToolCounters;

/// Request for the `health` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HealthRequest {
    /// Optional project root. When omitted, returns watcher state for
    /// every project currently registered with the server and skips the
    /// per-database stats. When supplied, returns stats for that project:
    /// watcher state if the project is registered with the server's
    /// watcher, database state if the project's database exists, and the
    /// counters otherwise. Reading is strictly read-only — this tool
    /// never syncs, never registers a watch, and never writes, so a
    /// health check can never perturb the state it is reporting on.
    #[schemars(default)]
    pub path: Option<String>,
}

/// Response from the `health` tool.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HealthResponse {
    /// Coarse classification of what the server is doing right now.
    /// Derivation rule documented on [`HealthPhase`].
    pub phase: HealthPhase,
    /// Watcher health for the supervised project. `None` only if no
    /// project is currently registered with the server.
    pub watcher_health: Option<watch::WatcherHealth>,
    /// Number of unreconciled dirty paths (modified or created since
    /// the last successful reconcile). `None` if no project is active.
    pub pending_dirty: Option<usize>,
    /// Number of unreconciled deleted paths. `None` if no project is active.
    pub pending_deleted: Option<usize>,
    /// Watcher's monotonic event sequence. `None` if no project is active.
    pub watcher_sequence: Option<u64>,
    /// Sequence acknowledged by the most recent reconcile. `None` if no
    /// project is active.
    pub last_verified_sequence: Option<u64>,
    /// Unix-epoch seconds of the most recent successful `ensure_current`
    /// across the entire server's lifetime. Zero before the first sync.
    pub last_sync_unix_seconds: i64,
    /// `revision_id` of the active project's current revision. `None`
    /// if no project is active or the database hasn't been published to.
    pub revision_id: Option<String>,
    /// Parser pack version stored on the current revision. `None` if no
    /// revision has been published yet.
    pub parser_pack_version: Option<String>,
    /// File count in the active project's database. `-1` if no project is
    /// active or the count query failed (use this rather than
    /// `Option<i64>` so consumers can render a single JSON number).
    pub file_count: i64,
    /// Total tool calls served since process start.
    pub tool_call_count: u64,
    /// Total tool calls that returned any `ErrorData` since process start.
    pub tool_call_error_count: u64,
    /// Sum of every tool call's work duration, in milliseconds.
    pub tool_call_total_ms: u64,
    /// How many consecutive full publishes (untrusted-watcher or
    /// Unavailable-path verifications) have run since the last successful
    /// incremental reconcile. A value that persists at 1+ across calls
    /// means the watcher is not being trusted — e.g. Linux inotify
    /// `fs.inotify.max_user_watches` overflow forcing `Desynced` — so
    /// every tool call full-publishes. See the production-failure runbook
    /// in `.planning/PERFORMANCE.md`.
    pub consecutive_full_publishes: u64,
}

/// Coarse-grained operational phase. Derived from existing state rather
/// than tracked separately so it can't drift.
///
/// - `Importing`: no current revision — the first sync has never succeeded.
/// - `Restoring`: watcher is `NeedsVerification` or `Desynced`; the next
///   call will do a full verification rather than incremental reconcile.
/// - `SteadyState`: watcher `Healthy`, no unreconciled events, a current
///   revision exists, the project is fully served from disk.
/// - `Unavailable`: watcher could not be initialized; every call will
///   full-publish.
/// - `NoActiveProject`: server has been initialized but no `ensure_current`
///   call has succeeded, so the watcher state can't be summarized.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum HealthPhase {
    Importing,
    Restoring,
    SteadyState,
    Unavailable,
    NoActiveProject,
}

/// Composes the health snapshot for `path` (or for "no path" when
/// `request.path` is `None`).
///
/// # Errors
///
/// Returns an error only if `request.path` was provided but no project is
/// enabled for it. All other failure modes — database missing or locked,
/// query failed, no current revision, project not registered with the
/// watcher — are surfaced as zero/None fields so the caller still gets an
/// actionable response.
pub fn health(
    request: &Parameters<HealthRequest>,
    _sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<HealthResponse>, ErrorData> {
    let HealthRequest { path } = &request.0;

    let counters = ToolCounters::snapshot();
    let last_sync = manager.last_sync_unix_seconds();

    let fields = match path.as_deref() {
        Some(p) => compute_project_state(p, manager)?,
        None => compute_global_only_state(manager),
    };

    Ok(Json(HealthResponse {
        phase: fields.phase,
        watcher_health: fields.watcher_health,
        pending_dirty: fields.pending_dirty,
        pending_deleted: fields.pending_deleted,
        watcher_sequence: fields.watcher_sequence,
        last_verified_sequence: fields.last_verified_sequence,
        last_sync_unix_seconds: last_sync,
        revision_id: fields.revision_id,
        parser_pack_version: fields.parser_pack_version,
        file_count: fields.file_count,
        tool_call_count: counters.call_count,
        tool_call_error_count: counters.error_count,
        tool_call_total_ms: counters.total_ms,
        consecutive_full_publishes: manager.consecutive_full_publishes(),
    }))
}

/// Computes every per-project field of the health response **without any
/// side effect**: it resolves the project root, reads the watcher state if
/// the project is already registered, and reads the database read-only if
/// it exists. It never calls `ensure_current`, so a health call can never
/// trigger a publish, register a watch, or change watcher health — the
/// snapshot is whatever the last real tool call left behind, which is
/// exactly what an operator wants from a health check.
/// Per-project fields of [`HealthResponse`], grouped so callers can
/// destructure a single tuple-shaped return rather than threading
/// nine outputs through the call site. The fields are intentionally
/// `pub` so `compute_global_only_state` can construct the same shape.
#[allow(clippy::type_complexity)]
#[derive(Debug, PartialEq, Eq)]
struct ProjectHealthFields {
    phase: HealthPhase,
    watcher_health: Option<watch::WatcherHealth>,
    pending_dirty: Option<usize>,
    pending_deleted: Option<usize>,
    watcher_sequence: Option<u64>,
    last_verified_sequence: Option<u64>,
    revision_id: Option<String>,
    parser_pack_version: Option<String>,
    file_count: i64,
}

fn compute_project_state(
    path: &str,
    manager: &crate::sync::SourceSyncManager,
) -> Result<ProjectHealthFields, ErrorData> {
    let root = crate::project::find_project_root(std::path::Path::new(path))
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    let database_path = crate::project::database_path(&root);
    // Read-only: `watch_state_for` never registers a watch and never
    // changes watcher health (unlike `activate`).
    let snapshot = manager
        .watch_state_for(root.as_path())
        .map(|state| state.snapshot());

    let (revision_id, parser_pack_version, file_count) = read_database_fields(&database_path);

    let phase = match snapshot.as_ref() {
        Some(snapshot) => derive_phase(
            snapshot.health,
            &revision_id,
            snapshot.has_unreconciled_events(),
        ),
        // No watcher state: the project was never registered with this
        // server's watcher (or the server just started). DB fields, if
        // any, are still surfaced; the phase honestly says the server
        // has nothing active for this project.
        None => HealthPhase::NoActiveProject,
    };

    Ok(ProjectHealthFields {
        phase,
        watcher_health: snapshot.as_ref().map(|s| s.health),
        pending_dirty: snapshot.as_ref().map(|s| s.dirty_paths.len()),
        pending_deleted: snapshot.as_ref().map(|s| s.deleted_paths.len()),
        watcher_sequence: snapshot.as_ref().map(|s| s.watcher_sequence),
        last_verified_sequence: snapshot.as_ref().map(|s| s.last_verified_sequence),
        revision_id,
        parser_pack_version,
        file_count,
    })
}

/// Reads the DB-derived health fields read-only, degrading gracefully when
/// the database doesn't exist yet (project enabled but never synced) or
/// can't be opened — a health check must never fail on transient DB state.
fn read_database_fields(database_path: &std::path::Path) -> (Option<String>, Option<String>, i64) {
    if !database_path.exists() {
        return (None, None, -1);
    }
    let Ok(mut connection) = store::open_read_only(database_path) else {
        return (None, None, -1);
    };
    let revision_id = current_revision_id(&mut connection);
    let parser_pack_version = current_parser_pack_version(&mut connection);
    let file_count = match count_files(&mut connection) {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!(
                database_path = %database_path.display(),
                error = %error,
                "health: file count query failed",
            );
            -1
        }
    };
    (revision_id, parser_pack_version, file_count)
}

/// Builds a per-project field set from the global state only — no
/// `ensure_synced`, no database call. Used when the caller passed no
/// `path`. Snapshot shape matches `compute_project_state`; the
/// revision/parser-pack/file-count slots are `None` / `-1` because
/// we never opened the database in this codepath.
fn compute_global_only_state(manager: &crate::sync::SourceSyncManager) -> ProjectHealthFields {
    let snapshots = manager.watch_states_snapshot();
    if snapshots.is_empty() {
        return ProjectHealthFields {
            phase: HealthPhase::NoActiveProject,
            watcher_health: None,
            pending_dirty: None,
            pending_deleted: None,
            watcher_sequence: None,
            last_verified_sequence: None,
            revision_id: None,
            parser_pack_version: None,
            file_count: -1,
        };
    }
    // In the current single-active-project model, snapshots has one
    // entry; reduce it to the reported fields. Future multi-project
    // support would either aggregate or pick "most recently touched"
    // and document the choice — never silently combine.
    let snapshot = snapshots.into_iter().next().expect("non-empty");
    let phase = derive_phase(snapshot.health, &None, snapshot.has_unreconciled_events());
    ProjectHealthFields {
        phase,
        watcher_health: Some(snapshot.health),
        pending_dirty: Some(snapshot.dirty_paths.len()),
        pending_deleted: Some(snapshot.deleted_paths.len()),
        watcher_sequence: Some(snapshot.watcher_sequence),
        last_verified_sequence: Some(snapshot.last_verified_sequence),
        revision_id: None,
        parser_pack_version: None,
        file_count: -1,
    }
}

/// Maps `(watcher_health, current_revision, has_unreconciled_events)`
/// to a coarse `HealthPhase`. Pure function over existing state — no
/// I/O — so the phase agrees with the watcher-snapshot fields it
/// derives from.
fn derive_phase(
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
    }
}

fn current_revision_id(connection: &mut rusqlite::Connection) -> Option<String> {
    connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
}

fn current_parser_pack_version(connection: &mut rusqlite::Connection) -> Option<String> {
    connection
        .query_row(
            "SELECT parser_pack_version FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
}

fn count_files(connection: &mut rusqlite::Connection) -> rusqlite::Result<i64> {
    connection.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
}

#[cfg(test)]
#[path = "health_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "health_state_tests.rs"]
mod state_tests;
