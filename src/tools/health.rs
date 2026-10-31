// slugaudit-line-exception: approved-by=slugthug; reason=the health contract: request and response shapes and the read-only database snapshot share one file
//! `health` MCP tool — what state is this in, and what should the caller do
//! next.
//!
//! Answers without writing through to the database or running a sync, so a
//! caller can ask at any point — including when nothing else seems to work.
//! Every failure mode degrades to an absent field rather than an error,
//! because a diagnostic tool that fails when there is nothing to diagnose is
//! useless. A `path` that is not an enabled project is one of those: the
//! answer is `phase: "disabled"` plus a `remedy` naming the call that
//! enables it, never an error.
//!
//! Every other state-bearing tool hashes the source before answering;
//! `health` is the exception. It reports what the database holds, not what
//! the source holds, and says so through its fields rather than repairing
//! anything.

use crate::store;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PATH_BYTES: usize = 32 * 1024;

use super::ToolCounters;
use super::health_phase::{compute_global_only_state, compute_project_state};
use super::health_result_store::ResultStoreState;

/// Request for the `health` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HealthRequest {
    /// Optional project root. When supplied, returns the database state for
    /// that project. When omitted, reports that no project is selected and
    /// names the call that enables one. Reading is strictly read-only — this
    /// tool never syncs, never mutates, and never writes, so a health check
    /// can never perturb the state it is reporting on.
    #[schemars(default)]
    pub path: Option<String>,
    /// Also return `detail`: the operator view (parser pack version, skipped
    /// files, whether the database read failed, process counters, host
    /// memory, the mechanic scan cache). Defaults to false.
    #[serde(default)]
    pub verbose: bool,
}

/// Response from the `health` tool.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HealthResponse {
    /// Coarse classification of what the server is doing right now.
    pub phase: HealthPhase,
    /// The next action when no project is being served by this answer: the
    /// same sentence the `project_not_enabled` error carries as its remedy.
    /// Present exactly when `phase` is `disabled` or `no_active_project`;
    /// null whenever a project is served.
    pub remedy: Option<String>,
    /// What the project database holds; null when the project has never been
    /// synced or its database could not be read
    /// (`detail.database_read_failed` says which).
    pub database: Option<DatabaseSummary>,
    /// Operator detail, present only with `verbose: true`; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "HealthDetail", default)]
    pub detail: Option<HealthDetail>,
}

/// The current revision and file count, as the default answer carries them.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DatabaseSummary {
    /// `revision_id` of the current revision; null if nothing has been
    /// published yet.
    pub revision_id: Option<String>,
    /// Files in the database, including discovered files that were skipped
    /// (oversize, unreadable, non-UTF-8) and hold no content.
    pub file_count: i64,
}

/// The operator view, present only with `verbose: true`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HealthDetail {
    /// The rest of the database group; null exactly when `database` is.
    #[schemars(with = "Option<serde_json::Map<String, serde_json::Value>>")]
    pub database: Option<DatabaseDetail>,
    /// True when the database exists but could not be read; `database` is
    /// then null and `phase` is `unknown`.
    pub database_read_failed: bool,
    /// Server process counters since start.
    #[schemars(with = "serde_json::Map<String, serde_json::Value>")]
    pub server: ServerCounters,
    /// Host physical and available memory as detected; null when detection
    /// failed (`system_memory_error` says why), in which case the limits come
    /// from an explicitly chosen profile, not from the host.
    #[schemars(with = "Option<serde_json::Map<String, serde_json::Value>>")]
    pub system_memory: Option<crate::model::SystemMemoryInfo>,
    /// Why host memory could not be detected; null when it was.
    pub system_memory_error: Option<String>,
    /// Mechanic scan cache counters.
    #[schemars(with = "serde_json::Map<String, serde_json::Value>")]
    pub mechanic_cache: MechanicCacheState,
    /// Paged-result store: results and bytes by class, budget, reclaims,
    /// refusals, refused cursors and the startup sweep. Null until the first
    /// paged result is made in this process.
    #[schemars(with = "Option<serde_json::Map<String, serde_json::Value>>")]
    pub result_store: Option<ResultStoreState>,
}

/// The parser pack and skipped-file count of the current revision.
#[derive(Debug, Serialize, PartialEq, Eq)]
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
#[derive(Debug, Serialize)]
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
    /// (`file_kind = 'skipped'`: oversize, unreadable, non-UTF-8), so they
    /// hold no content. `file_count - skipped_file_count` is the number of
    /// files that were actually read.
    pub(crate) skipped_file_count: i64,
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
                path = %database_path.display(),
                error = %error,
                "health: failed to inspect the project database path; reporting empty database fields",
            );
            return DatabaseRead::Unreadable;
        }
    }
    let mut connection = match store::open_existing(database_path) {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(
                path = %database_path.display(),
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
                path = %database_path.display(),
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
#[derive(Debug, Serialize, PartialEq, Eq)]
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
}

/// Coarse-grained operational phase. Derived from existing state rather
/// than tracked separately so it can't drift.
///
/// - `Importing`: no current revision — the first sync has never succeeded.
/// - `SteadyState`: a current revision exists and the database is readable.
/// - `NoActiveProject`: no `path` was supplied, so no project is selected
///   for this answer.
/// - `Unknown`: the project database exists but could not be read, so the
///   revision is unknowable (`database_read_failed` is true).
/// - `Disabled`: `request.path` resolved to no enabled project — the
///   activation marker is absent — so nothing can serve that path until
///   `project_control` enables it. `remedy` names that call. This is a
///   state, not an error: a diagnostic tool that fails when there is
///   nothing to diagnose is useless.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum HealthPhase {
    Importing,
    SteadyState,
    NoActiveProject,
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
/// failed, no current revision, and *no project enabled for that path* — is
/// surfaced as a field so the caller still gets an actionable response. The
/// not-enabled case is a successful answer with `phase: disabled` and a
/// `remedy`, because it is the state a cold caller is most likely to be in
/// and the one thing `health` must be able to report.
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
        Some(p) => compute_project_state(p)?,
        None => compute_global_only_state(),
    };
    let detail = verbose.then(|| {
        let (system_memory, system_memory_error) = match crate::model::detect_system_memory() {
            Ok(info) => (Some(info), None),
            Err(error) => (None, Some(error.to_string())),
        };
        HealthDetail {
            database: fields.database.as_ref().map(|database| DatabaseDetail {
                parser_pack_version: database.parser_pack_version.clone(),
                skipped_file_count: database.skipped_file_count,
            }),
            database_read_failed: fields.database_read_failed,
            server: server_counters(manager),
            system_memory,
            system_memory_error,
            mechanic_cache: MechanicCacheState::snapshot(manager),
            result_store: super::health_result_store::snapshot(manager),
        }
    });

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

/// This process's counters, read once for a verbose answer.
fn server_counters(manager: &sync::SourceSyncManager) -> ServerCounters {
    let counters = ToolCounters::snapshot();
    ServerCounters {
        tool_call_count: counters.call_count,
        tool_call_error_count: counters.error_count,
        tool_call_total_ms: counters.total_ms,
        last_sync_unix_seconds: manager.last_sync_unix_seconds(),
        last_sync_duration_ms: manager.last_sync_duration_ms(),
    }
}

#[cfg(test)]
#[path = "health_tests.rs"]
mod tests;
