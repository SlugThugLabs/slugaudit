use super::env_error::EnvConfigError;
use super::limit_overrides::nonzero_override;
use super::system_ram::{MemoryDetectError, SystemMemoryInfo, detect_system_memory};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

/// Bounds applied when normalizing pack output into evidence rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceLimits {
    /// Largest number of evidence items kept per file; excess items are
    /// discarded and recorded in a truncation `Diagnostic`.
    pub max_items_per_file: usize,
    /// Largest serialized JSON payload kept for one evidence item; larger
    /// items are discarded.
    pub max_payload_bytes_per_item: usize,
    /// Largest cumulative serialized payload kept per file; once reached,
    /// remaining items are discarded.
    pub max_payload_bytes_per_file: usize,
}

impl Default for EvidenceLimits {
    fn default() -> Self {
        Self {
            max_items_per_file: 100_000,
            max_payload_bytes_per_item: 256 * 1024,
            max_payload_bytes_per_file: 64 * 1024 * 1024,
        }
    }
}

/// Largest TEXT/BLOB the bundled SQLite can store (`SQLITE_MAX_LENGTH`).
///
/// `libsqlite3-sys` 0.35.0 compiles SQLite with the default
/// `SQLITE_MAX_LENGTH` of 1,000,000,000 bytes (`sqlite3.c:14137-14138`).
/// A content or query value over this cannot be inserted or read, so the
/// per-file/query limits are clamped here: without the clamp, a file in
/// (1e9, max_file] would fail the whole publish at insert instead of
/// becoming a reported skipped row. Verified 2026-10-19.
pub(crate) const SQLITE_MAX_VALUE_BYTES: u64 = 1_000_000_000;

/// Process-wide resource ceilings for import, query, and structure work.
/// Defaults are intentionally conservative for a single-project MCP server.
///
/// Every field except `evidence` can be overridden at startup via an
/// environment variable (see [`process_limits`]). When no override is
/// present, the compile-time default is used. The pattern is
/// `SLUGAUDIT_<FIELD>` where `<FIELD>` is the snake_case name of the field —
/// e.g. `SLUGAUDIT_MAX_FILE_BYTES=134217728` doubles the per-file size cap.
/// Duration fields accept a number of seconds (whole seconds only).
/// Only explicitly-set variables override; defaults are never altered. A
/// variable that is set but cannot be read or parsed logs a warning naming
/// the variable, and the default applies. Single-value overrides are
/// clamped to `SQLITE_MAX_VALUE_BYTES` with a warning, because SQLite
/// cannot store a larger value. A `0` for any of these variables is refused
/// with a warning naming the variable (once per process) and the value
/// already in place stays: no limit here means "off" at zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Largest single file that will be fully read into memory for hashing
    /// and indexing. Larger files are rejected rather than loaded. Clamped
    /// to `SQLITE_MAX_VALUE_BYTES` so a stored content value cannot exceed
    /// what the bundled SQLite can hold.
    /// Env: `SLUGAUDIT_MAX_FILE_BYTES` (u64).
    pub max_file_bytes: u64,
    /// Largest file content, in bytes, a grammar is run on. A file above it
    /// (and within `max_file_bytes`) is still stored and searchable through
    /// `query`, but no grammar is fetched, loaded or run over it: it is
    /// recorded `ContentOnly` with `parse_error_reason = 'oversize_for_parse'`,
    /// counted in `report.completeness` and in every `leads` response's
    /// `files_not_parsed`, and `structure` skips it. Bounds the parse's
    /// memory and time, which grow with input size (a 50 MB Rust file took
    /// 185 s and 7.5 GB RSS before this cap). 2 MiB in every profile. Clamped
    /// to `max_file_bytes`, with a warning, when an override sets it higher.
    /// A `0` override is refused with a warning and the default stays.
    /// Env: `SLUGAUDIT_MAX_PARSE_BYTES` (u64); project `config.json` key
    /// `max_parse_bytes`.
    pub max_parse_bytes: u64,
    /// Wall-clock budget, in milliseconds, for every parse of one file (the
    /// pack's `process()` and each walker, call and macro-body re-parse
    /// share it). Each file runs under the lesser of this and what is left of
    /// the sync budget (`max_sync_wall_clock`). When this allowance is what
    /// runs out, only that file is affected: it is stored `ContentOnly` (or
    /// `Partial` when only a re-parse was cut) with
    /// `parse_error_reason = 'parse_timeout'`, counted in
    /// `report.completeness` and in every `leads` response's
    /// `files_not_parsed`, and the sync goes on with the next file. When the
    /// sync budget runs out first, the sync fails as before. A cut file with
    /// unchanged content is not re-parsed on later syncs (like any unchanged
    /// file); editing it parses it again. 5000 (5 s) in every profile; a `0`
    /// override is refused with a warning and the default stays.
    /// Env: `SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS` (milliseconds, u64); project
    /// `config.json` key `max_parse_wall_clock_ms`.
    pub max_parse_wall_clock_ms: u64,
    /// Cap on the sum of sampled file sizes in one publish. This is a
    /// **work budget**, not a process memory bound: peak memory also includes
    /// in-flight samples, evidence payloads, and parse transients, so a tree
    /// can fit this budget and still exceed available RAM. A separate memory
    /// bound is a pending architectural decision (PLAN/DECISIONS 2026-10-19).
    /// Env: `SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES` (u64).
    pub max_total_import_bytes: u64,
    /// Cap on serialized JSON bytes returned by one read call — the rows of
    /// `query` and the `rows` of `leads`. Enforced per row as the response
    /// is assembled: the row that would cross the cap stops the page, and
    /// both tools report the budget and the bytes they spent.
    /// Env: `SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES` (usize).
    pub max_query_response_bytes: usize,
    /// Default page size, in serialized JSON bytes, for one `query` or
    /// `leads` call that does not pass its own `page_bytes`. A page is cut at
    /// this size on a row boundary and resumed with `page.cursor`; a request's
    /// `page_bytes` overrides it, and the effective page is always clamped to
    /// `max_query_response_bytes` (the hard ceiling), with the clamp reported
    /// in the response. An oversized row has its individual cells clipped and
    /// reported in `clipped_cells`. Env: `SLUGAUDIT_DEFAULT_PAGE_BYTES` (usize); project
    /// `config.json` key `default_page_bytes`.
    pub default_page_bytes: usize,
    /// Maximum SQL text length accepted by `query` (UTF-8 bytes).
    /// Env: `SLUGAUDIT_MAX_QUERY_SQL_BYTES` (usize).
    pub max_query_sql_bytes: usize,
    /// Soft cap on SQLite virtual-machine steps per `query` execution.
    /// Enforced via a progress handler when available.
    /// Env: `SLUGAUDIT_MAX_QUERY_VM_STEPS` (u32).
    pub max_query_vm_steps: u32,
    /// Wall-clock budget for one `query` execution, independent of the VM
    /// step budget — bounds queries whose individual steps are each slow
    /// (e.g. disk I/O stalls) rather than merely numerous.
    /// Env: `SLUGAUDIT_MAX_QUERY_WALL_CLOCK_SECS` (seconds, u64).
    pub max_query_wall_clock: Duration,
    /// Largest single TEXT or BLOB column value `query` will return, checked
    /// against the raw value before it is cloned or hex-expanded into JSON.
    /// Env: `SLUGAUDIT_MAX_QUERY_VALUE_BYTES` (usize).
    pub max_query_value_bytes: usize,
    /// Maximum tree-sitter query text length for `structure` (UTF-8 bytes).
    /// `Query::new` has no deadline of its own, so a longer query is refused
    /// with `invalid_params` (stating its size and this limit, never its
    /// text) before it is compiled. 16 KiB in every profile; a `0` override is
    /// refused with a warning and the default stays.
    /// Env: `SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES` (usize); project
    /// `config.json` key `max_structure_query_bytes`.
    pub max_structure_query_bytes: usize,
    /// Maximum capture matches returned by one `structure` call.
    /// Env: `SLUGAUDIT_MAX_STRUCTURE_MATCHES` (usize).
    pub max_structure_matches: usize,
    /// Wall-clock budget for one `structure` query's Tree-sitter execution,
    /// enforced natively via `QueryCursorOptions::progress_callback` so a
    /// pathological pattern (deep nesting, wildcard-heavy captures) can be
    /// aborted mid-query rather than only after it returns.
    /// Env: `SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS` (seconds, u64).
    pub max_structure_execution_time: Duration,
    /// Wall-clock budget for one sync operation — a full publish (including
    /// all CAS retries) or one barrier-sync reconcile pass (including all
    /// barrier iterations). Enforced cooperatively inside the hot loops
    /// (per discovered file, per dirty path, per barrier iteration), so a
    /// pathological repo (huge tree, hung network filesystem) fails closed
    /// with a `TimeBudgetExceeded` error instead of stalling the tool call
    /// indefinitely. Generous by default — the measured baseline for a
    /// 200-file import is ~160 ms — because legitimate large imports must
    /// not be cut off; this bounds hangs, not real work.
    /// Env: `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS` (seconds, u64).
    pub max_sync_wall_clock: Duration,
    /// Largest number of filesystem entries one watch-scope walk may yield.
    /// The scope decides how many directories carry a `notify` watch, so
    /// every counted entry is one potential kernel watch descriptor plus the
    /// path set's per-directory memory; bounding the walk bounds both. A tree
    /// past the cap produces a partial scope: the directories beyond it are
    /// unwatched, the scope reports the limit (`WatcherHealth::Partial`), and
    /// the stat sweep still runs on every call, so freshness does not depend
    /// on the cap. Env: `SLUGAUDIT_MAX_WATCH_ENTRIES` (usize).
    pub max_watch_entries: usize,
    /// Deepest directory level one watch-scope walk may descend, with the
    /// project root at depth 0. Bounds a pathologically deep tree that the
    /// entry cap alone would let the walk explore deeply before stopping; a
    /// directory at the limit with unwalked children also marks the scope
    /// partial, and the per-call stat sweep remains the freshness guarantee
    /// for the unwatched subtrees. Env: `SLUGAUDIT_MAX_WATCH_DEPTH` (usize).
    pub max_watch_depth: usize,
    /// Total on-disk bytes of paged results the result store keeps. A new
    /// result that does not fit reclaims delivered results, then idle ones,
    /// never active ones; one that still does not fit is refused with
    /// `result_store_full`, and a single result above the whole budget with
    /// `result_too_large`. Defaults to 4 times the effective
    /// `max_query_response_bytes`, including its env override (1 GiB on
    /// `audit`, 128 MiB on `lean`), unless this limit is set itself; a `0`
    /// override is refused with a warning and the default stays.
    /// Env: `SLUGAUDIT_MAX_RESULT_STORE_BYTES` (u64); project `config.json`
    /// key `max_result_store_bytes`.
    pub max_result_store_bytes: u64,
    /// Seconds after its last served page that an unfinished paged result
    /// stays protected from reclaim (900 in every profile). The grace never
    /// deletes anything by itself, and continuing a result renews it. A `0`
    /// override is refused with a warning and the default stays.
    /// Env: `SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS` (seconds, u64); project
    /// `config.json` key `result_active_grace_secs`.
    pub result_active_grace_secs: u64,
    /// Per-file bounds applied when normalizing evidence into rows.
    pub evidence: EvidenceLimits,
}

/// Validates the environment and installs the process-wide resource limits.
/// The server entry point calls this before serving, so a set-but-invalid
/// `SLUGAUDIT_*` variable (or undetectable memory without an explicit
/// profile) stops startup instead of running with other limits.
///
/// # Errors
///
/// Returns [`EnvConfigError`] naming the offending variable.
pub fn init_process_limits() -> Result<&'static ResourceLimits, EnvConfigError> {
    if let Some(limits) = LIMITS.get() {
        return Ok(limits);
    }
    let limits = ResourceLimits::from_env()?;
    Ok(LIMITS.get_or_init(|| limits))
}

static LIMITS: OnceLock<ResourceLimits> = OnceLock::new();

/// Returns the process-wide resource limits: those [`init_process_limits`]
/// installed at startup. Reached without that call (library embedding,
/// tests) the environment is validated here; an invalid one is logged as an
/// error and the test-only [`Default`] limits apply, because this accessor
/// cannot fail. The shipped binary validates before serving, so it never
/// takes that path.
/// Tests continue to use `ResourceLimits::default()` directly for
/// deterministic behavior independent of the test runner's environment.
#[must_use]
pub fn process_limits() -> &'static ResourceLimits {
    LIMITS.get_or_init(|| match ResourceLimits::from_env() {
        Ok(limits) => limits,
        Err(error) => {
            tracing::error!(%error, "process limits environment is invalid and was not validated at startup; using the default limits");
            ResourceLimits::default()
        }
    })
}

impl ResourceLimits {
    /// Creates limits by taking the profile baseline and overriding any
    /// fields that have a corresponding `SLUGAUDIT_*` env var set. Unset
    /// variables leave the default unchanged, so a typo in a variable
    /// *name* is harmless; a variable that is set but cannot be read or
    /// parsed fails with an error naming it (never its value). Duration
    /// fields are parsed as whole seconds.
    ///
    /// # Errors
    ///
    /// Returns [`EnvConfigError`] for a set-but-invalid variable or an
    /// Adaptive profile on a host whose memory cannot be detected.
    pub(super) fn from_env() -> Result<Self, EnvConfigError> {
        let mut limits = base_limits(super::profile::profile_from_env()?, detect_system_memory)?;
        apply_import_overrides(&mut limits)?;
        apply_query_overrides(&mut limits)?;
        apply_structure_overrides(&mut limits)?;
        apply_sync_and_watch_overrides(&mut limits)?;
        // The store budget follows the effective response ceiling (after the
        // env override above) unless its own variable sets it below.
        limits.max_result_store_bytes =
            super::profile::result_store_budget(limits.max_query_response_bytes);
        apply_result_store_overrides(&mut limits)?;
        limits.clamp_parse_to_file_bytes();
        Ok(limits)
    }
}

/// The baseline the overrides are applied to: the chosen profile, or
/// Adaptive when none is set. Adaptive needs the host memory, so when that
/// cannot be detected the operator must choose a profile explicitly.
pub(super) fn base_limits(
    profile: Option<super::profile::AuditProfile>,
    detect: impl FnOnce() -> Result<SystemMemoryInfo, MemoryDetectError>,
) -> Result<ResourceLimits, EnvConfigError> {
    super::profile::limits_for_profile_with(profile.unwrap_or_default(), detect)
        .map_err(|source| EnvConfigError::MemoryUndetectable { source })
}

/// Applies the file and publish work-budget overrides.
fn apply_import_overrides(limits: &mut ResourceLimits) -> Result<(), EnvConfigError> {
    if let Some(val) = nonzero_override::<u64>("SLUGAUDIT_MAX_FILE_BYTES")? {
        limits.max_file_bytes = clamp_sqlite_value("SLUGAUDIT_MAX_FILE_BYTES", val);
    }
    if let Some(val) = nonzero_override("SLUGAUDIT_MAX_PARSE_BYTES")? {
        limits.max_parse_bytes = val;
    }
    if let Some(val) = nonzero_override("SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS")? {
        limits.max_parse_wall_clock_ms = val;
    }
    if let Some(val) = nonzero_override::<u64>("SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES")? {
        limits.max_total_import_bytes = val;
    }
    Ok(())
}

/// Applies the page, SQL, VM-step, wall-clock, and value-cap overrides.
fn apply_query_overrides(limits: &mut ResourceLimits) -> Result<(), EnvConfigError> {
    if let Some(val) = nonzero_override::<usize>("SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES")? {
        limits.max_query_response_bytes = val;
    }
    if let Some(val) = nonzero_override::<usize>("SLUGAUDIT_DEFAULT_PAGE_BYTES")? {
        limits.default_page_bytes = val;
    }
    if let Some(val) = nonzero_override::<usize>("SLUGAUDIT_MAX_QUERY_SQL_BYTES")? {
        limits.max_query_sql_bytes = val;
    }
    if let Some(val) = nonzero_override::<u32>("SLUGAUDIT_MAX_QUERY_VM_STEPS")? {
        limits.max_query_vm_steps = val;
    }
    if let Some(val) = nonzero_override::<u64>("SLUGAUDIT_MAX_QUERY_WALL_CLOCK_SECS")? {
        limits.max_query_wall_clock = Duration::from_secs(val);
    }
    if let Some(val) = nonzero_override::<usize>("SLUGAUDIT_MAX_QUERY_VALUE_BYTES")? {
        limits.max_query_value_bytes =
            clamp_sqlite_value("SLUGAUDIT_MAX_QUERY_VALUE_BYTES", val as u64) as usize;
    }
    Ok(())
}

/// Applies the Tree-sitter `structure` overrides.
fn apply_structure_overrides(limits: &mut ResourceLimits) -> Result<(), EnvConfigError> {
    if let Some(val) = nonzero_override::<usize>("SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES")? {
        limits.max_structure_query_bytes = val;
    }
    if let Some(val) = nonzero_override::<usize>("SLUGAUDIT_MAX_STRUCTURE_MATCHES")? {
        limits.max_structure_matches = val;
    }
    if let Some(val) = nonzero_override::<u64>("SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS")? {
        limits.max_structure_execution_time = Duration::from_secs(val);
    }
    Ok(())
}

/// Applies the sync budget and watch-scope walk overrides.
fn apply_sync_and_watch_overrides(limits: &mut ResourceLimits) -> Result<(), EnvConfigError> {
    if let Some(val) = nonzero_override::<u64>("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS")? {
        limits.max_sync_wall_clock = Duration::from_secs(val);
    }
    if let Some(val) = nonzero_override::<usize>("SLUGAUDIT_MAX_WATCH_ENTRIES")? {
        limits.max_watch_entries = val;
    }
    if let Some(val) = nonzero_override::<usize>("SLUGAUDIT_MAX_WATCH_DEPTH")? {
        limits.max_watch_depth = val;
    }
    Ok(())
}

/// Applies the result store budget and grace overrides.
fn apply_result_store_overrides(limits: &mut ResourceLimits) -> Result<(), EnvConfigError> {
    if let Some(val) = nonzero_override::<u64>("SLUGAUDIT_MAX_RESULT_STORE_BYTES")? {
        limits.max_result_store_bytes = val;
    }
    if let Some(val) = nonzero_override::<u64>("SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS")? {
        limits.result_active_grace_secs = val;
    }
    Ok(())
}

/// Clamps one single-value limit to what the bundled SQLite can store.
///
/// An override above the cap is unusable: the value would be rejected at
/// insert/read. Clamp it and say so rather than letting a per-file skip turn
/// into a failed publish.
fn clamp_sqlite_value(name: &str, value: u64) -> u64 {
    if value > SQLITE_MAX_VALUE_BYTES {
        tracing::warn!(
            variable = name,
            requested = value,
            cap = SQLITE_MAX_VALUE_BYTES,
            "limit exceeds SQLite's maximum storable value; clamping",
        );
        SQLITE_MAX_VALUE_BYTES
    } else {
        value
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        super::profile::limits_for_profile(super::profile::AuditProfile::Adaptive).unwrap_or_else(
            |error| {
                tracing::error!(%error, "default limits cannot be sized from host memory; using the lean profile (production startup rejects this)");
                super::profile::lean_limits()
            },
        )
    }
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
