//! Verbose operator diagnostics (`HealthDetail`) collected for `health` when
//! `verbose: true` is requested.

use super::{DatabaseState, ProjectHealthFields};
use crate::project::ProjectRoot;
use crate::store::{ClassStats, StoreStats, SweepOutcome};
use crate::sync;
use crate::tools::ToolCounters;
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Serialize;

/// The operator view, present only with `verbose: true`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HealthDetail {
    /// The rest of the database group; null exactly when `database` is.
    pub database: Option<DatabaseDetail>,
    /// True when the database exists but could not be read.
    pub database_read_failed: bool,
    /// Why the database could not be read; present iff `database_read_failed` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_read_error: Option<String>,
    /// Server process counters since start.
    pub server: ServerCounters,
    /// Host physical and available memory as detected.
    pub system_memory: Option<crate::model::SystemMemoryInfo>,
    /// Why host memory could not be detected; null when it was.
    pub system_memory_error: Option<String>,
    /// Paged-result store for the selected active project; null when no project
    /// is selected or until a multi-page result is stored for that project.
    pub result_store: Option<ResultStoreState>,
}

/// The parser pack and skipped-file count of the current revision.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DatabaseDetail {
    /// Parser pack version stored on the current revision; null if none published yet.
    pub parser_pack_version: Option<String>,
    /// Rows discovered but never read (`file_kind = 'skipped'`).
    pub skipped_file_count: i64,
}

/// Results and bytes held in one retention class.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ResultClassState {
    /// Number of results in the class.
    pub results: usize,
    /// Their measured bytes.
    pub bytes: u64,
}

impl From<ClassStats> for ResultClassState {
    fn from(stats: ClassStats) -> Self {
        Self {
            results: stats.results,
            bytes: stats.bytes,
        }
    }
}

/// What the startup sweep of leftover result files did.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ResultSweepState {
    /// Files removed (dead owners' databases and lock files, orphans).
    pub removed_files: usize,
    /// Sessions another live process holds; their files were left.
    pub live_owners_skipped: usize,
    /// Operations that failed while the sweep carried on.
    pub errors: usize,
    /// The first failure's message; null when none failed.
    pub first_error: Option<String>,
}

impl From<SweepOutcome> for ResultSweepState {
    fn from(sweep: SweepOutcome) -> Self {
        Self {
            removed_files: sweep.removed_files,
            live_owners_skipped: sweep.live_owners_skipped,
            errors: sweep.errors,
            first_error: sweep.first_error,
        }
    }
}

/// The result store for one active project, as `health` reports it.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ResultStoreState {
    /// Unfinished results served within the grace period; never reclaimed.
    pub active: ResultClassState,
    /// Unfinished results older than the grace period; reclaimed second.
    pub idle: ResultClassState,
    /// Fully read results kept for a final-page retry; reclaimed first.
    pub delivered: ResultClassState,
    /// `max_result_store_bytes`: the most the store admits.
    pub budget_bytes: u64,
    /// Bytes of all admitted results; never above the budget.
    pub admitted_bytes: u64,
    /// Delivered results reclaimed since the server started.
    pub reclaimed_delivered: u64,
    /// Idle results reclaimed since the server started.
    pub reclaimed_idle: u64,
    /// New results refused with `result_store_full`.
    pub refused_full: u64,
    /// New results refused with `result_too_large`.
    pub refused_too_large: u64,
    /// Cursors refused as `unknown_cursor`.
    pub unknown_cursors: u64,
    /// Cursors refused as `result_expired`.
    pub expired_cursors: u64,
    /// What the startup sweep did.
    pub sweep: ResultSweepState,
}

impl From<StoreStats> for ResultStoreState {
    fn from(stats: StoreStats) -> Self {
        Self {
            active: stats.active.into(),
            idle: stats.idle.into(),
            delivered: stats.delivered.into(),
            budget_bytes: stats.budget_bytes,
            admitted_bytes: stats.admitted_bytes,
            reclaimed_delivered: stats.reclaimed_delivered,
            reclaimed_idle: stats.reclaimed_idle,
            refused_full: stats.refused_full,
            refused_too_large: stats.refused_too_large,
            unknown_cursors: stats.unknown_cursors,
            expired_cursors: stats.expired_cursors,
            sweep: stats.sweep.into(),
        }
    }
}

/// Counters for this server process since it started.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ServerCounters {
    /// Total tool calls served since process start.
    pub tool_call_count: u64,
    /// Total tool calls that returned any `ErrorData` since process start.
    pub tool_call_error_count: u64,
    /// Sum of every tool call's work duration, in milliseconds.
    pub tool_call_total_ms: u64,
    /// Unix-epoch seconds of the most recent successful `ensure_current`.
    pub last_sync_unix_seconds: i64,
    /// Duration of the most recent successful `ensure_current`, in milliseconds.
    pub last_sync_duration_ms: u64,
}

fn snapshot_result_store(
    manager: &sync::SourceSyncManager,
    active_root: Option<&ProjectRoot>,
) -> Result<Option<ResultStoreState>, ErrorData> {
    let Some(root) = active_root else {
        return Ok(None);
    };
    let runtime_dir = crate::project::activation_dir(root);
    match manager.result_stores().get_dir(&runtime_dir)? {
        Some(store) => Ok(Some(
            store
                .stats()
                .map_err(|error| crate::error_codes::store_error(&error))?
                .into(),
        )),
        None => Ok(None),
    }
}

pub(super) fn collect_detail(
    manager: &sync::SourceSyncManager,
    fields: &ProjectHealthFields,
    database: Option<&DatabaseState>,
) -> Result<HealthDetail, ErrorData> {
    let (system_memory, system_memory_error) = match crate::model::detect_system_memory() {
        Ok(info) => (Some(info), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let counters = ToolCounters::snapshot();
    Ok(HealthDetail {
        database: database.map(|db| DatabaseDetail {
            parser_pack_version: db.parser_pack_version.clone(),
            skipped_file_count: db.skipped_file_count,
        }),
        database_read_failed: fields.database_read_error.is_some(),
        database_read_error: fields.database_read_error.clone(),
        server: ServerCounters {
            tool_call_count: counters.call_count,
            tool_call_error_count: counters.error_count,
            tool_call_total_ms: counters.total_ms,
            last_sync_unix_seconds: manager.last_sync_unix_seconds(),
            last_sync_duration_ms: manager.last_sync_duration_ms(),
        },
        system_memory,
        system_memory_error,
        result_store: snapshot_result_store(manager, fields.active_root.as_ref())?,
    })
}
