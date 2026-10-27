//! Resource limit profiles (Lean, Audit, Adaptive).

use super::limits::{EvidenceLimits, ResourceLimits, SQLITE_MAX_VALUE_BYTES};
use super::system_ram::detect_system_memory;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
/// Named resource-limit presets an operator selects via `SLUGAUDIT_PROFILE`
/// or the project's `config.json`.
pub enum AuditProfile {
    /// Everyday coding assistant mode: fast, responsive, conservative budgets.
    Lean,
    /// Deep security audit and due-diligence mode: large file limits, uncapped AST blocks, generous time budgets.
    Audit,
    /// Adaptive mode: automatically scales limits based on host physical RAM and available memory.
    #[default]
    Adaptive,
}

impl AuditProfile {
    #[must_use]
    /// Case-insensitive parse of a profile name; anything unrecognized is `None`
    /// (`profile_from_env` reports the discarded setting).
    pub fn parse_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "lean" => Some(Self::Lean),
            "audit" => Some(Self::Audit),
            "adaptive" => Some(Self::Adaptive),
            _ => None,
        }
    }
}

/// Resolves `SLUGAUDIT_PROFILE`, reporting why a set value is discarded.
///
/// An unset variable means the operator chose no profile — the default
/// applies and nothing needs saying. A variable that IS set but is not a
/// known profile (or cannot be read) is operator intent that would
/// otherwise vanish silently, so it warns — the same rule
/// `process_limits` applies to the numeric overrides, and the same rule
/// AGENTS.md non-negotiable #3 states: reported, or it is a bug.
#[must_use]
pub(crate) fn profile_from_env() -> Option<AuditProfile> {
    match std::env::var("SLUGAUDIT_PROFILE") {
        Err(std::env::VarError::NotPresent) => None,
        // `VarError::NotUnicode`'s Display prints the raw value, which may be
        // sensitive, so the variable name and the failure kind are logged
        // instead of the error itself.
        Err(std::env::VarError::NotUnicode(_)) => {
            tracing::warn!(
                variable = "SLUGAUDIT_PROFILE",
                error = "value is not valid unicode",
                "profile override could not be read; using the default profile",
            );
            None
        }
        Ok(raw) => match AuditProfile::parse_str(&raw) {
            Some(profile) => Some(profile),
            None => {
                tracing::warn!(
                    variable = "SLUGAUDIT_PROFILE",
                    "profile override is not a known profile; using the default profile",
                );
                None
            }
        },
    }
}

/// Default `query`/`leads` page size in every profile: small enough that a
/// client which passes a tool result to the model unbounded is not flooded,
/// large enough for a useful page. Clamped to `max_query_response_bytes`.
pub(crate) const DEFAULT_PAGE_BYTES: usize = 64 * 1024;

/// Largest file content a grammar is run on, in every profile (see
/// `ResourceLimits::max_parse_bytes`). Parse time and memory grow with the
/// input, so this is what bounds them; it is not scaled with RAM because the
/// cost it bounds is per parse, not per host.
pub(crate) const DEFAULT_MAX_PARSE_BYTES: u64 = 2 * 1024 * 1024;

/// Wall-clock allowance for every parse of one file, in milliseconds, in
/// every profile (see `ResourceLimits::max_parse_wall_clock_ms`). Well above
/// a normal parse (a 2 MiB file parses in well under a second) and far below
/// the sync budget, so one pathological file costs seconds, not the sync.
pub(crate) const DEFAULT_MAX_PARSE_WALL_CLOCK_MS: u64 = 5_000;

/// Longest tree-sitter query text `structure` compiles, in every profile (see
/// `ResourceLimits::max_structure_query_bytes`). Query compilation cost grows
/// with the text and has no deadline of its own, so the text is bounded
/// instead; 16 KiB is far above any query a model writes by hand.
pub(crate) const DEFAULT_MAX_STRUCTURE_QUERY_BYTES: usize = 16 * 1024;

/// How many times `max_query_response_bytes` the result store may hold (see
/// `ResourceLimits::max_result_store_bytes`): four maximal responses.
const RESULT_STORE_BUDGET_FACTOR: u64 = 4;

/// Seconds an unfinished paged result stays protected after its last page, in
/// every profile (see `ResourceLimits::result_active_grace_secs`).
pub(crate) const DEFAULT_RESULT_ACTIVE_GRACE_SECS: u64 = 900;

/// The result store budget derived from a profile's response ceiling.
fn result_store_budget(max_query_response_bytes: usize) -> u64 {
    (max_query_response_bytes as u64).saturating_mul(RESULT_STORE_BUDGET_FACTOR)
}

/// Watch-scope walk caps. The walk enumerates the directories that carry a
/// `notify` watch, so its size is bounded in both entries (one potential
/// kernel watch descriptor and one entry in the scope's path set each) and
/// depth (a deep tree can be expensive to explore before the entry cap
/// trips). Lean runs bounded tightly enough for a laptop; Audit and Adaptive
/// allow a larger tree. Either way a truncated scope is reported as
/// `WatcherHealth::Partial`, and the stat sweep still runs on every call —
/// the sweep is the freshness guarantee, not the watcher.
const LEAN_MAX_WATCH_ENTRIES: usize = 50_000;
const LEAN_MAX_WATCH_DEPTH: usize = 32;
const AUDIT_MAX_WATCH_ENTRIES: usize = 200_000;
const AUDIT_MAX_WATCH_DEPTH: usize = 64;

#[must_use]
/// The complete limit set for one profile. `Adaptive` scales the import
/// budget from detected host memory.
pub fn limits_for_profile(profile: AuditProfile) -> ResourceLimits {
    match profile {
        AuditProfile::Lean => lean_limits(),
        AuditProfile::Audit => audit_limits(),
        AuditProfile::Adaptive => adaptive_limits(),
    }
}

/// Conservative laptop budgets: fast to satisfy, bounded in every dimension.
fn lean_limits() -> ResourceLimits {
    ResourceLimits {
        max_file_bytes: 32 * 1024 * 1024,
        max_parse_bytes: DEFAULT_MAX_PARSE_BYTES,
        max_parse_wall_clock_ms: DEFAULT_MAX_PARSE_WALL_CLOCK_MS,
        max_total_import_bytes: 2 * 1024 * 1024 * 1024,
        max_query_response_bytes: 32 * 1024 * 1024,
        default_page_bytes: DEFAULT_PAGE_BYTES,
        max_query_sql_bytes: 50_000,
        max_query_vm_steps: 5_000_000,
        max_query_wall_clock: Duration::from_secs(10),
        max_query_value_bytes: 32 * 1024 * 1024,
        max_structure_query_bytes: DEFAULT_MAX_STRUCTURE_QUERY_BYTES,
        max_structure_matches: 2_000,
        max_structure_execution_time: Duration::from_secs(10),
        max_sync_wall_clock: Duration::from_secs(120),
        max_watch_entries: LEAN_MAX_WATCH_ENTRIES,
        max_watch_depth: LEAN_MAX_WATCH_DEPTH,
        max_result_store_bytes: result_store_budget(32 * 1024 * 1024),
        result_active_grace_secs: DEFAULT_RESULT_ACTIVE_GRACE_SECS,
        evidence: EvidenceLimits {
            max_items_per_file: 50_000,
            max_payload_bytes_per_item: 128 * 1024,
            max_payload_bytes_per_file: 32 * 1024 * 1024,
        },
    }
}

/// Deep-audit budgets: large files, uncapped AST blocks, generous time.
fn audit_limits() -> ResourceLimits {
    ResourceLimits {
        max_file_bytes: 256 * 1024 * 1024,
        max_parse_bytes: DEFAULT_MAX_PARSE_BYTES,
        max_parse_wall_clock_ms: DEFAULT_MAX_PARSE_WALL_CLOCK_MS,
        max_total_import_bytes: 32 * 1024 * 1024 * 1024,
        max_query_response_bytes: 256 * 1024 * 1024,
        default_page_bytes: DEFAULT_PAGE_BYTES,
        max_query_sql_bytes: 200_000,
        max_query_vm_steps: 50_000_000,
        max_query_wall_clock: Duration::from_secs(60),
        max_query_value_bytes: 256 * 1024 * 1024,
        max_structure_query_bytes: DEFAULT_MAX_STRUCTURE_QUERY_BYTES,
        max_structure_matches: 20_000,
        max_structure_execution_time: Duration::from_secs(60),
        max_sync_wall_clock: Duration::from_secs(1200),
        max_watch_entries: AUDIT_MAX_WATCH_ENTRIES,
        max_watch_depth: AUDIT_MAX_WATCH_DEPTH,
        max_result_store_bytes: result_store_budget(256 * 1024 * 1024),
        result_active_grace_secs: DEFAULT_RESULT_ACTIVE_GRACE_SECS,
        evidence: EvidenceLimits {
            max_items_per_file: 200_000,
            max_payload_bytes_per_item: 512 * 1024,
            max_payload_bytes_per_file: 256 * 1024 * 1024,
        },
    }
}

/// Budgets scaled from detected host memory.
fn adaptive_limits() -> ResourceLimits {
    let ram = detect_system_memory();
    let import_budget = (ram.available_bytes / 2).max(4 * 1024 * 1024 * 1024);
    // SQLite cannot store a value larger than `SQLITE_MAX_LENGTH`;
    // clamping here keeps a per-file skip from becoming a failed
    // publish at the insert.
    let max_file = (ram.total_bytes / 10).clamp(256 * 1024 * 1024, SQLITE_MAX_VALUE_BYTES);
    ResourceLimits {
        max_file_bytes: max_file,
        max_parse_bytes: DEFAULT_MAX_PARSE_BYTES,
        max_parse_wall_clock_ms: DEFAULT_MAX_PARSE_WALL_CLOCK_MS,
        max_total_import_bytes: import_budget,
        max_query_response_bytes: max_file as usize,
        default_page_bytes: DEFAULT_PAGE_BYTES,
        max_query_sql_bytes: 200_000,
        max_query_vm_steps: 50_000_000,
        max_query_wall_clock: Duration::from_secs(60),
        max_query_value_bytes: max_file as usize,
        max_structure_query_bytes: DEFAULT_MAX_STRUCTURE_QUERY_BYTES,
        max_structure_matches: 20_000,
        max_structure_execution_time: Duration::from_secs(60),
        max_sync_wall_clock: Duration::from_secs(1200),
        max_watch_entries: AUDIT_MAX_WATCH_ENTRIES,
        max_watch_depth: AUDIT_MAX_WATCH_DEPTH,
        max_result_store_bytes: result_store_budget(max_file as usize),
        result_active_grace_secs: DEFAULT_RESULT_ACTIVE_GRACE_SECS,
        evidence: EvidenceLimits {
            max_items_per_file: 200_000,
            max_payload_bytes_per_item: 512 * 1024,
            max_payload_bytes_per_file: max_file as usize,
        },
    }
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
