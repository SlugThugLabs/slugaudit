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
//!    the project has never been synced or the database could not be read;
//!    `database_read_failed` says which, because the two states have
//!    different meanings.
//! 3. **`server`** — what this process has done since it started: tool
//!    call counts and total milliseconds, last sync time and duration, and
//!    the consecutive-full-publish counter that signals an untrusted
//!    watcher. Instrumentation, present so the tool can be measured over
//!    time; nothing here drives `phase` or `remedy`.
//!
//! `phase` and `remedy` summarize the first two groups. `system_memory` is
//! host state, reported because it sizes the adaptive import budget.

use crate::sync;
use crate::watch;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PATH_BYTES: usize = 32 * 1024;

use super::ToolCounters;
#[cfg(test)]
use super::health_database::{count_files, read_database, read_revision_fields};
#[cfg(test)]
use super::health_phase::derive_phase;
use super::health_phase::{compute_global_only_state, compute_project_state};

/// Request for the `health` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HealthRequest {
    /// Optional project root. When omitted, returns the watcher state of a
    /// project registered with the server (one is registered in the current
    /// single-project model; if several are, one is reported and which one
    /// is unspecified) and skips the per-database stats. When supplied,
    /// returns stats for that project: watcher state if the project is
    /// registered with the server's watcher, database state if the
    /// project's database exists, and the counters otherwise. Reading is
    /// strictly read-only — this tool never syncs, never registers a watch,
    /// and never writes, so a health check can never perturb the state it
    /// is reporting on.
    #[schemars(default)]
    pub path: Option<String>,
}

/// Response from the `health` tool.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HealthResponse {
    /// Coarse classification of what the server is doing right now.
    /// Derivation rule documented on `HealthPhase`.
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
    /// `database_read_failed` distinguishes the two cases.
    pub database: Option<DatabaseState>,
    /// True when the project database exists but could not be read — an
    /// unopenable file (permissions, symlink, network filesystem),
    /// corruption, a lock or timeout, or schema drift. `database` is then
    /// `null` and `phase` is `unknown`: the index may still hold a current
    /// revision, but this answer cannot see it, and the phase never claims
    /// the first sync failed. False when the database was read or does not
    /// exist yet (never synced); `database: null` with this false means
    /// exactly "never synced".
    pub database_read_failed: bool,
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
/// - `Degraded`: watcher `Partial`, no unreconciled events, a current
///   revision exists. The project is served and the stat sweep keeps it
///   fresh, but part of the tree (beyond the documented watch-scope limit)
///   carries no watcher — this is not full steady state.
/// - `Unavailable`: watcher could not be initialized; every call will
///   full-publish.
/// - `NoActiveProject`: server has been initialized but no `ensure_current`
///   call has succeeded, so the watcher state can't be summarized.
/// - `IndexedUnwatched`: the enabled project has a published revision in
///   its database, but this server has no registered watcher for that path.
///   Indexed evidence remains readable; no watcher health is implied.
/// - `Unknown`: whether a current revision exists is not knowable from
///   this answer — either no `path` was supplied (so there is no project
///   root to open a database with), or the project database exists but
///   could not be read (`database_read_failed` is true). The watcher's own
///   health is still reported; in particular this never claims the first
///   sync has failed.
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
    Degraded,
    Unavailable,
    NoActiveProject,
    IndexedUnwatched,
    Unknown,
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
        database_read_failed: fields.database_read_failed,
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

#[cfg(test)]
#[path = "health_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "health_state_tests.rs"]
mod state_tests;
