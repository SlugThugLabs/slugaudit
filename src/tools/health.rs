// slugaudit-line-exception: approved-by=slugthug; reason=the health contract: its request and response shapes and the project-state derivation share one read-only snapshot
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
//! The default answer carries only what a caller acts on: `phase`,
//! `remedy`, `watcher.health`, and `database.revision_id`/`file_count`.
//! Everything else is operator detail, returned under `detail` only when the
//! request sets `verbose: true`. Each group is either wholly present or
//! `null` — there is no state in which half of one is knowable:
//!
//! 1. **`watcher`** — what the filesystem watcher knows about the
//!    supervised project (`detail.watcher` adds the unreconciled dirty and
//!    deleted counts and the sequences). `null` when no project is
//!    registered with this server.
//! 2. **`database`** — what the project database holds (`detail.database`
//!    adds the parser pack version and the skipped-file count). `null` when
//!    the project has never been synced or the database could not be read;
//!    `detail.database_read_failed` says which.
//! 3. **`detail.server`** — what this process has done since it started:
//!    tool call counts and total milliseconds, last sync time and duration,
//!    and the consecutive-full-publish counter that signals an untrusted
//!    watcher. Instrumentation; nothing here drives `phase` or `remedy`.
//!
//! `phase` and `remedy` summarize the first two groups.
//! `detail.system_memory` is host state, reported because it sizes the
//! adaptive import budget, and `detail.mechanic_cache` the registered-mechanic
//! scan cache.

use crate::store;
use crate::sync;
use crate::watch;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PATH_BYTES: usize = 32 * 1024;

use super::ToolCounters;
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
    /// Also return `detail`: the operator view (unreconciled watcher
    /// counts and sequences, parser pack version, skipped files, whether the
    /// database read failed, process counters, host memory, the mechanic
    /// scan cache). Defaults to false.
    #[serde(default)]
    pub verbose: bool,
}

/// Response from the `health` tool.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HealthResponse {
    /// Coarse classification of what the server is doing right now.
    /// Derivation rule documented on `HealthPhase`.
    pub phase: HealthPhase,
    /// The next action when no project is being served by this answer: the
    /// same sentence the `project_not_enabled` error carries as its remedy.
    /// Present exactly when `phase` is `disabled`, or when no `path` was
    /// supplied and this server has no project; null whenever a project is
    /// served.
    pub remedy: Option<String>,
    /// The watcher's health for the supervised project; null when no project
    /// is registered with this server.
    pub watcher: Option<WatcherSummary>,
    /// What the project database holds; null when the project has never
    /// been synced or its database could not be read
    /// (`detail.database_read_failed` says which).
    pub database: Option<DatabaseSummary>,
    /// Operator detail, present only with `verbose: true`; absent otherwise.
    /// Beyond the summaries above it adds `watcher` counts and sequences,
    /// `database` pack version and skipped files, `database_read_failed`,
    /// `server` counters, `system_memory` and `mechanic_cache`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "HealthDetail", default)]
    pub detail: Option<HealthDetail>,
}

/// The watcher's health, as the default answer carries it.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct WatcherSummary {
    /// Current watcher health.
    pub health: watch::WatcherHealth,
}

/// The current revision and file count, as the default answer carries them.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DatabaseSummary {
    /// `revision_id` of the current revision; null if nothing has been
    /// published yet.
    pub revision_id: Option<String>,
    /// Files in the database, including discovered files that were skipped
    /// (oversize or unreadable) and hold no content.
    pub file_count: i64,
}

/// The operator view, present only with `verbose: true`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HealthDetail {
    /// The rest of the watcher group; null exactly when `watcher` is.
    pub watcher: Option<WatcherDetail>,
    /// The rest of the database group; null exactly when `database` is.
    pub database: Option<DatabaseDetail>,
    /// True when the project database exists but could not be read (an
    /// unopenable file, corruption, a lock or timeout, schema drift):
    /// `database` is then null and `phase` is `unknown`, never `importing`.
    /// False when the database was read or does not exist yet.
    pub database_read_failed: bool,
    /// What this server process has done since it started.
    pub server: ServerCounters,
    /// Host physical and available memory; it sizes the adaptive profile's
    /// import budget (`max_total_import_bytes`).
    pub system_memory: crate::model::SystemMemoryInfo,
    /// The per-revision cache of registered mechanics' scan results.
    pub mechanic_cache: MechanicCacheState,
}

/// Unreconciled watcher counts and sequences.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct WatcherDetail {
    /// Paths modified or created since the last successful reconcile.
    pub pending_dirty: usize,
    /// Paths deleted since the last successful reconcile.
    pub pending_deleted: usize,
    /// The watcher's monotonic event sequence.
    pub sequence: u64,
    /// The sequence acknowledged by the most recent reconcile.
    pub last_verified_sequence: u64,
}

/// The parser pack and skipped-file count of the current revision.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DatabaseDetail {
    /// Parser pack version stored on the current revision; null if nothing
    /// has been published yet.
    pub parser_pack_version: Option<String>,
    /// Of `database.file_count`, the rows discovered but never read
    /// (`file_kind = 'skipped'`), which hold no content.
    pub skipped_file_count: i64,
}

/// The registered-mechanic scan cache, as `health` reports it. Sizes are
/// estimates (path and detail bytes plus a fixed overhead per row), not
/// measured heap use.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MechanicCacheState {
    /// Cached scan results held now.
    pub entries: usize,
    /// Estimated bytes they hold.
    pub bytes: usize,
    /// The byte budget; the oldest entries are evicted past it.
    pub budget: usize,
    /// Entries evicted to make room since the server started.
    pub evictions: u64,
    /// Scans not cached because their rows alone outgrew the budget.
    pub skipped_too_large: u64,
    /// Lookups answered from the cache since the server started.
    pub hits: u64,
    /// Lookups that had to scan since the server started.
    pub misses: u64,
}

impl MechanicCacheState {
    fn snapshot(manager: &sync::SourceSyncManager) -> Self {
        let stats = manager.mechanic_scan_cache().stats();
        Self {
            entries: stats.entries,
            bytes: stats.bytes,
            budget: stats.budget,
            evictions: stats.evictions,
            skipped_too_large: stats.skipped_too_large,
            hits: stats.hits,
            misses: stats.misses,
        }
    }
}

/// What the project database holds, split on the wire into
/// [`DatabaseSummary`] and [`DatabaseDetail`].
///
/// Read as one unit: one failed query leaves the whole object absent rather
/// than producing a revision alongside a count that didn't come back, so a
/// caller never has to detect a half-read answer — there isn't one to
/// detect. That is also why no field here is a sentinel: absence is the
/// `None` on [`HealthResponse::database`], not a magic number inside it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DatabaseState {
    /// `revision_id` of the current revision. `None` if nothing has been
    /// published yet.
    pub(crate) revision_id: Option<String>,
    /// Parser pack version stored on the current revision. `None` if no
    /// revision has been published yet.
    pub(crate) parser_pack_version: Option<String>,
    /// Files in the database — every discovered file, including rows with
    /// `file_kind = 'skipped'`; `skipped_file_count` breaks those out.
    pub(crate) file_count: i64,
    /// Of `file_count`, how many rows were discovered but never read
    /// (`file_kind = 'skipped'`: oversize or unreadable), so they hold no
    /// content. `file_count - skipped_file_count` is the number of files
    /// that were actually read.
    pub(crate) skipped_file_count: i64,
}

/// The watcher's view of the supervised project, split on the wire into
/// [`WatcherSummary`] and [`WatcherDetail`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WatcherState {
    /// Current watcher health.
    pub(crate) health: watch::WatcherHealth,
    /// Unreconciled dirty paths — modified or created since the last
    /// successful reconcile.
    pub(crate) pending_dirty: usize,
    /// Unreconciled deleted paths.
    pub(crate) pending_deleted: usize,
    /// The watcher's monotonic event sequence.
    pub(crate) sequence: u64,
    /// The sequence acknowledged by the most recent reconcile.
    pub(crate) last_verified_sequence: u64,
}

/// The outcome of one read-only database inspection.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DatabaseRead {
    /// No database file: the project has never been synced.
    NeverSynced,
    /// The database exists but could not be inspected, opened, or queried.
    Unreadable,
    /// The database was opened and every field was read.
    Read(DatabaseState),
}

/// Reads the database state read-only, degrading gracefully when the
/// database doesn't exist yet (project enabled but never synced) or can't
/// be read.
pub(crate) fn read_database(database_path: &std::path::Path) -> DatabaseRead {
    match database_path.try_exists() {
        Ok(true) => {}
        Ok(false) => return DatabaseRead::NeverSynced,
        Err(error) => {
            tracing::warn!(
                database_id = %crate::util::redacted_path(database_path),
                error = %error,
                "health: failed to inspect the project database path; reporting empty database fields",
            );
            return DatabaseRead::Unreadable;
        }
    }
    let mut connection = match store::open_read_only(database_path) {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(
                database_id = %crate::util::redacted_path(database_path),
                error = %error,
                "health: failed to open the project database read-only; reporting empty database fields",
            );
            return DatabaseRead::Unreadable;
        }
    };
    match query_database_state(&mut connection) {
        Ok(state) => DatabaseRead::Read(state),
        Err(error) => {
            tracing::warn!(
                database_id = %crate::util::redacted_path(database_path),
                error = %error,
                "health: failed to read the project database; reporting empty database fields",
            );
            DatabaseRead::Unreadable
        }
    }
}

fn query_database_state(connection: &mut rusqlite::Connection) -> rusqlite::Result<DatabaseState> {
    let (revision_id, parser_pack_version) = read_revision_fields(connection)?;
    Ok(DatabaseState {
        revision_id,
        parser_pack_version,
        file_count: count_files(connection)?,
        skipped_file_count: count_skipped_files(connection)?,
    })
}

pub(super) fn read_revision_fields(
    connection: &mut rusqlite::Connection,
) -> rusqlite::Result<(Option<String>, Option<String>)> {
    Ok(connection
        .query_row(
            "SELECT revision_id, parser_pack_version FROM revisions WHERE is_current = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .unwrap_or((None, None)))
}

pub(super) fn count_files(connection: &mut rusqlite::Connection) -> rusqlite::Result<i64> {
    connection.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
}

fn count_skipped_files(connection: &mut rusqlite::Connection) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT count(*) FROM files WHERE file_kind = 'skipped'",
        [],
        |row| row.get(0),
    )
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
    let HealthRequest { path, verbose } = &request.0;
    if path
        .as_deref()
        .is_some_and(|value| crate::util::exceeds_input_bytes(value, MAX_PATH_BYTES))
    {
        return Err(rmcp::ErrorData::invalid_params(
            "path exceeds 32768 bytes",
            None,
        ));
    }

    let fields = match path.as_deref() {
        Some(p) => compute_project_state(p, manager)?,
        None => compute_global_only_state(manager),
    };
    let detail = verbose.then(|| HealthDetail {
        watcher: fields.watcher.as_ref().map(|watcher| WatcherDetail {
            pending_dirty: watcher.pending_dirty,
            pending_deleted: watcher.pending_deleted,
            sequence: watcher.sequence,
            last_verified_sequence: watcher.last_verified_sequence,
        }),
        database: fields.database.as_ref().map(|database| DatabaseDetail {
            parser_pack_version: database.parser_pack_version.clone(),
            skipped_file_count: database.skipped_file_count,
        }),
        database_read_failed: fields.database_read_failed,
        server: server_counters(manager),
        system_memory: crate::model::detect_system_memory(),
        mechanic_cache: MechanicCacheState::snapshot(manager),
    });

    Ok(Json(HealthResponse {
        phase: fields.phase,
        remedy: fields.remedy,
        watcher: fields.watcher.map(|watcher| WatcherSummary {
            health: watcher.health,
        }),
        database: fields.database.map(|database| DatabaseSummary {
            revision_id: database.revision_id,
            file_count: database.file_count,
        }),
        detail,
    }))
}

/// This process's counters, read once for a verbose answer.
fn server_counters(manager: &sync::SourceSyncManager) -> ServerCounters {
    let counters = ToolCounters::snapshot();
    ServerCounters {
        tool_call_count: counters.call_count,
        tool_call_error_count: counters.error_count,
        tool_call_total_ms: counters.total_ms,
        last_sync_unix_seconds: manager.last_sync_unix_seconds(),
        last_sync_duration_ms: manager.last_sync_duration_ms(),
        consecutive_full_publishes: manager.consecutive_full_publishes(),
    }
}

#[cfg(test)]
#[path = "health_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "health_state_tests.rs"]
mod state_tests;
