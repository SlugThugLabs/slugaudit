//! `health` MCP tool — what state is this in, and what should the caller
//! do next.
//!
//! Answers without writing through to the database or running another
//! Tree-sitter parse, so a caller can ask at any point — including when
//! nothing else seems to work. Calling `health` never causes a sync, never
//! emits MCP progress, and never fails transiently: every failure mode
//! degrades to an absent field rather than an error, because a diagnostic
//! tool that fails when there is nothing to diagnose is useless. The state
//! a cold caller is most likely to hit — a `path` that is not an enabled
//! project — is one of those: the answer is `phase: "disabled"` plus a
//! `remedy` naming the call that enables it, never an error.
//!
//! The response is grouped by where each fact comes from, so that absence
//! reads as absence. A group is either wholly present or `null` — there is
//! no state in which half of one is knowable:
//!
//! 1. **`watcher`** — what the filesystem watcher knows about the
//!    supervised project: health enum, unreconciled dirty and deleted
//!    counts, watcher sequence, last verified sequence. `null` when no
//!    project is registered with this server.
//! 2. **`database`** — what the project database holds: current revision
//!    id, parser pack version, file count, skipped-file count. `null` when
//!    the project has never been synced or the database could not be read.
//! 3. **`server`** — what this process has done since it started: tool
//!    call counts and total milliseconds, last sync time and duration, and
//!    the consecutive-full-publish counter that signals an untrusted
//!    watcher. Instrumentation, present so the tool can be measured over
//!    time; nothing here drives `phase` or `remedy`.
//!
//! `phase` and `remedy` summarize the first two groups. `system_memory` is
//! host state, reported because it sizes the adaptive import budget.
// slugaudit-line-exception: approved-by=slugthug; reason=health is the schema-defining tool; Request+Response+phase+derivation live together so the schema isn't split from its only consumer

use crate::error_codes::ErrorCode;
use crate::sync;
use crate::watch;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PATH_BYTES: usize = 32 * 1024;

use super::ToolCounters;
use super::health_database::read_database;
#[cfg(test)]
use super::health_database::{count_files, read_revision_fields};

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
    /// The next action to take when no project is being served by this
    /// answer: the same static sentence the `project_not_enabled` error
    /// carries in `ErrorData.data.remedy`, so a caller can treat "health
    /// says `disabled`" and "every tool fails with `project_not_enabled`"
    /// as one instruction. `Some` exactly when `phase` is `disabled`, or
    /// when no `path` was supplied and this server has no project at all;
    /// `None` whenever a project is actually being served.
    pub remedy: Option<String>,
    /// What the filesystem watcher knows about the supervised project.
    /// `None` only if no project is currently registered with the server.
    /// The fields are one object because they become unknowable together —
    /// there is no state in which half of them exist.
    pub watcher: Option<WatcherState>,
    /// What the project database holds. `None` if the project has never
    /// been synced, or if the database could not be read. Either every
    /// field inside is present or the whole object is absent: a partial
    /// read is not representable, so no number here is ever a placeholder.
    pub database: Option<DatabaseState>,
    /// What this server process has done since it started.
    pub server: ServerCounters,
    /// Host physical and available memory. Reported because it sets the
    /// adaptive profile's import budget (`src/model/profile.rs`), so a
    /// caller that trips `max_total_import_bytes` can see what sized it.
    pub system_memory: crate::model::SystemMemoryInfo,
}

/// What the project database holds.
///
/// Read as one unit: one failed query leaves the whole object absent rather
/// than producing a revision alongside a count that didn't come back, so a
/// caller never has to detect a half-read answer — there isn't one to
/// detect. That is also why no field here is a sentinel: absence is the
/// `None` on [`HealthResponse::database`], not a magic number inside it.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DatabaseState {
    /// `revision_id` of the current revision. `None` if nothing has been
    /// published yet.
    pub revision_id: Option<String>,
    /// Parser pack version stored on the current revision. `None` if no
    /// revision has been published yet.
    pub parser_pack_version: Option<String>,
    /// Files in the database — every discovered file, including rows with
    /// `file_kind = 'skipped'`; `skipped_file_count` breaks those out.
    pub file_count: i64,
    /// Of `file_count`, how many rows were discovered but never read
    /// (`file_kind = 'skipped'`: oversize or unreadable), so they hold no
    /// content. `file_count - skipped_file_count` is the number of files
    /// that were actually read.
    pub skipped_file_count: i64,
}

/// The watcher's view of the supervised project.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct WatcherState {
    /// Current watcher health.
    pub health: watch::WatcherHealth,
    /// Unreconciled dirty paths — modified or created since the last
    /// successful reconcile.
    pub pending_dirty: usize,
    /// Unreconciled deleted paths.
    pub pending_deleted: usize,
    /// The watcher's monotonic event sequence.
    pub sequence: u64,
    /// The sequence acknowledged by the most recent reconcile.
    pub last_verified_sequence: u64,
}

/// Counters for this server process since it started.
///
/// Operational instrumentation: these exist so the tool's own behaviour can
/// be measured over time. Nothing here is something a caller acts on, which
/// is why they share one object and drive no `phase` or `remedy`.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ServerCounters {
    /// Total tool calls served since process start.
    pub tool_call_count: u64,
    /// Total tool calls that returned any `ErrorData` since process start.
    pub tool_call_error_count: u64,
    /// Sum of every tool call's work duration, in milliseconds.
    pub tool_call_total_ms: u64,
    /// Unix-epoch seconds of the most recent successful `ensure_current`
    /// across the entire server's lifetime. Zero before the first sync.
    pub last_sync_unix_seconds: i64,
    /// Duration of the most recent successful `ensure_current`, in
    /// milliseconds. Zero before the first sync.
    pub last_sync_duration_ms: u64,
    /// How many consecutive full publishes (untrusted-watcher or
    /// Unavailable-path verifications) have run since the last successful
    /// incremental reconcile. A value that persists at 1+ across calls
    /// means the watcher is not being trusted — e.g. Linux inotify
    /// `fs.inotify.max_user_watches` overflow forcing `Desynced` — so
    /// every tool call full-publishes. See the full-publish runbook
    /// in `docs/TROUBLESHOOTING.md`.
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
/// - `IndexedUnwatched`: the enabled project has a published revision in
///   its database, but this server has no registered watcher for that path.
///   Indexed evidence remains readable; no watcher health is implied.
/// - `Disabled`: `request.path` resolved to no enabled project — the
///   activation marker is absent — so nothing can serve that path until
///   `project_control` enables it. `remedy` names that call. This is a
///   state, not an error: a diagnostic tool that fails when there is
///   nothing to diagnose is useless.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum HealthPhase {
    Importing,
    Restoring,
    SteadyState,
    Unavailable,
    NoActiveProject,
    IndexedUnwatched,
    Disabled,
}

/// Composes the health snapshot for `path` (or for "no path" when
/// `request.path` is `None`).
///
/// # Errors
///
/// Returns an error only if `request.path` was provided and cannot be
/// resolved at all. Every other state — database missing or locked, query
/// failed, no current revision, project not registered with the watcher,
/// and *no project enabled for that path* — is surfaced as a field so the
/// caller still gets an actionable response. The not-enabled case is a
/// successful answer with `phase: disabled` and a `remedy`, because it is
/// the state a cold caller is most likely to be in and the one thing
/// `health` must be able to report.
pub fn health(
    request: &Parameters<HealthRequest>,
    _sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<HealthResponse>, ErrorData> {
    let HealthRequest { path } = &request.0;
    if path
        .as_deref()
        .is_some_and(|value| crate::util::exceeds_input_bytes(value, MAX_PATH_BYTES))
    {
        return Err(rmcp::ErrorData::invalid_params(
            "path exceeds 32768 bytes",
            None,
        ));
    }

    let counters = ToolCounters::snapshot();
    let last_sync = manager.last_sync_unix_seconds();
    let last_sync_duration_ms = manager.last_sync_duration_ms();

    let fields = match path.as_deref() {
        Some(p) => compute_project_state(p, manager)?,
        None => compute_global_only_state(manager),
    };

    Ok(Json(HealthResponse {
        phase: fields.phase,
        remedy: fields.remedy,
        watcher: fields.watcher,
        database: fields.database,
        server: ServerCounters {
            tool_call_count: counters.call_count,
            tool_call_error_count: counters.error_count,
            tool_call_total_ms: counters.total_ms,
            last_sync_unix_seconds: last_sync,
            last_sync_duration_ms,
            consecutive_full_publishes: manager.consecutive_full_publishes(),
        },
        system_memory: crate::model::detect_system_memory(),
    }))
}

/// The per-project half of a [`HealthResponse`]: everything that depends on
/// which project is being served. The two builders below return one of
/// these and `health` fills in the process-wide remainder, so neither
/// builder has to know about counters or host memory.
#[derive(Debug, PartialEq, Eq)]
struct ProjectHealthFields {
    phase: HealthPhase,
    remedy: Option<String>,
    watcher: Option<WatcherState>,
    database: Option<DatabaseState>,
}

/// Computes every per-project field of the health response **without any
/// side effect**: it resolves the project root, reads the watcher state if
/// the project is already registered, and reads the database read-only if
/// it exists. It never calls `ensure_current`, so a health call can never
/// trigger a publish, register a watch, or change watcher health — the
/// snapshot is whatever the last real tool call left behind, which is
/// exactly what a caller wants from a health check.
fn compute_project_state(
    path: &str,
    manager: &sync::SourceSyncManager,
) -> Result<ProjectHealthFields, ErrorData> {
    let (root, database_path) = match crate::project::resolve_project(std::path::Path::new(path)) {
        Ok(resolved) => resolved,
        // No activation marker: report the state instead of failing on it.
        // `health` is the tool a caller reaches for precisely when nothing
        // works, so "this path is not an enabled project, here is the call
        // that enables it" must be an answer, not another opaque failure.
        // Any *other* resolution failure (an unreadable parent, a symlinked
        // activation component) stays an error: enabling the project would
        // not fix it.
        Err(crate::project::ActivationError::NotActive) => {
            return Ok(inactive_fields(HealthPhase::Disabled));
        }
        Err(error) => return Err(ErrorData::invalid_params(error.to_string(), None)),
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

    let phase = match snapshot.as_ref() {
        Some(snapshot) => derive_phase(
            snapshot.health,
            &revision_id,
            snapshot.has_unreconciled_events(),
        ),
        // No watcher state: the project was never registered with this
        // server's watcher (or the server just started). Database fields,
        // if any, are still surfaced; distinguish a queryable indexed
        // project from one that has never published a revision.
        None if revision_id.is_some() => HealthPhase::IndexedUnwatched,
        None => HealthPhase::NoActiveProject,
    };

    Ok(ProjectHealthFields {
        phase,
        remedy: None,
        watcher: snapshot.as_ref().map(|s| WatcherState {
            health: s.health,
            pending_dirty: s.dirty_paths.len(),
            pending_deleted: s.deleted_paths.len(),
            sequence: s.watcher_sequence,
            last_verified_sequence: s.last_verified_sequence,
        }),
        database,
    })
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
/// `path`. `database` is absent because this codepath never opens one.
fn compute_global_only_state(manager: &sync::SourceSyncManager) -> ProjectHealthFields {
    let snapshots = manager.watch_states_snapshot();
    if snapshots.is_empty() {
        return inactive_fields(HealthPhase::NoActiveProject);
    }
    // In the current single-active-project model, snapshots has one
    // entry; reduce it to the reported fields. Future multi-project
    // support would either aggregate or pick "most recently touched"
    // and document the choice — never silently combine.
    //
    // `is_empty()` was checked above, so `None` here is unreachable by
    // construction — but matching instead of `expect` keeps the "no
    // panic in production code" invariant literal rather than relying
    // on the guard staying in place.
    let Some(snapshot) = snapshots.into_iter().next() else {
        return inactive_fields(HealthPhase::NoActiveProject);
    };
    let phase = derive_phase(snapshot.health, &None, snapshot.has_unreconciled_events());
    ProjectHealthFields {
        phase,
        remedy: None,
        watcher: Some(WatcherState {
            health: snapshot.health,
            pending_dirty: snapshot.dirty_paths.len(),
            pending_deleted: snapshot.deleted_paths.len(),
            sequence: snapshot.watcher_sequence,
            last_verified_sequence: snapshot.last_verified_sequence,
        }),
        database: None,
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

#[cfg(test)]
#[path = "health_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "health_state_tests.rs"]
mod state_tests;
