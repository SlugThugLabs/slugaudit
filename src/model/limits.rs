use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

/// Bounds applied when normalizing pack output into evidence rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceLimits {
    pub max_items_per_file: usize,
    pub max_payload_bytes_per_item: usize,
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

/// Process-wide resource ceilings for import, query, and structure work.
/// Defaults are intentionally conservative for a single-project MCP server.
///
/// Every field can be overridden at startup via an environment variable
/// (see [`ResourceLimits::from_env`]). When no override is present, the
/// compile-time default is used. The pattern is `SLUGAUDIT_<FIELD>` where
/// `<FIELD>` is the snake_case name of the field — e.g.
/// `SLUGAUDIT_MAX_FILE_BYTES=134217728` doubles the per-file size cap.
/// Duration fields accept a number of seconds (whole seconds only).
/// Only explicitly-set variables override; defaults are never altered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Largest single file that will be fully read into memory for hashing
    /// and indexing. Larger files are rejected rather than loaded.
    /// Env: `SLUGAUDIT_MAX_FILE_BYTES` (u64).
    pub max_file_bytes: u64,
    /// Cap on the sum of sampled file sizes in one publish. Prevents one
    /// oversized tree from exhausting process memory during import.
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
    /// this size on a row boundary and resumed with `next_offset`; a request's
    /// `page_bytes` overrides it, and the effective page is always clamped to
    /// `max_query_response_bytes` (the hard ceiling), with the clamp reported
    /// in the response. A single row larger than the page is still returned
    /// whole. Env: `SLUGAUDIT_DEFAULT_PAGE_BYTES` (usize); project
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
    /// Env: `SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES` (usize).
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
    pub evidence: EvidenceLimits,
}

/// Returns the process-wide resource limits, computed once from env vars
/// on first call. Subsequent calls return the same cached value.
/// Callers that currently use `ResourceLimits::default()` should switch
/// to this so operators can tune limits at startup without recompiling.
/// Tests continue to use `ResourceLimits::default()` directly for
/// deterministic behavior independent of the test runner's environment.
#[must_use]
pub fn process_limits() -> &'static ResourceLimits {
    static LIMITS: OnceLock<ResourceLimits> = OnceLock::new();
    LIMITS.get_or_init(ResourceLimits::from_env)
}

impl ResourceLimits {
    /// Creates limits by taking the compile-time default and overriding
    /// any fields that have a corresponding `SLUGAUDIT_*` env var set.
    /// Unset or unparseable env vars are silently ignored (the default
    /// is used), so a typo in an env var name is harmless — the old
    /// limit applies. Duration fields are parsed as whole seconds.
    ///
    /// This is the intended production entry point: callers that
    /// currently use `ResourceLimits::default()` should switch to
    /// `ResourceLimits::from_env()`.
    #[must_use]
    pub fn from_env() -> Self {
        // A set-but-invalid SLUGAUDIT_PROFILE warns inside profile_from_env
        // rather than silently falling back, matching the numeric overrides.
        let mut limits = match super::profile::profile_from_env() {
            Some(profile) => super::profile::limits_for_profile(profile),
            None => Self::default(),
        };
        if let Some(val) = parse_override::<u64>("SLUGAUDIT_MAX_FILE_BYTES") {
            limits.max_file_bytes = val;
        }
        if let Some(val) = parse_override::<u64>("SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES") {
            limits.max_total_import_bytes = val;
        }
        if let Some(val) = parse_override::<usize>("SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES") {
            limits.max_query_response_bytes = val;
        }
        if let Some(val) = parse_override::<usize>("SLUGAUDIT_DEFAULT_PAGE_BYTES") {
            limits.default_page_bytes = val;
        }
        if let Some(val) = parse_override::<usize>("SLUGAUDIT_MAX_QUERY_SQL_BYTES") {
            limits.max_query_sql_bytes = val;
        }
        if let Some(val) = parse_override::<u32>("SLUGAUDIT_MAX_QUERY_VM_STEPS") {
            limits.max_query_vm_steps = val;
        }
        if let Some(val) = parse_override::<u64>("SLUGAUDIT_MAX_QUERY_WALL_CLOCK_SECS") {
            limits.max_query_wall_clock = Duration::from_secs(val);
        }
        if let Some(val) = parse_override::<usize>("SLUGAUDIT_MAX_QUERY_VALUE_BYTES") {
            limits.max_query_value_bytes = val;
        }
        if let Some(val) = parse_override::<usize>("SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES") {
            limits.max_structure_query_bytes = val;
        }
        if let Some(val) = parse_override::<usize>("SLUGAUDIT_MAX_STRUCTURE_MATCHES") {
            limits.max_structure_matches = val;
        }
        if let Some(val) = parse_override::<u64>("SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS") {
            limits.max_structure_execution_time = Duration::from_secs(val);
        }
        if let Some(val) = parse_override::<u64>("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS") {
            limits.max_sync_wall_clock = Duration::from_secs(val);
        }
        limits
    }
}

/// Parses one `SLUGAUDIT_<FIELD>` override.
///
/// An unset variable is not a failure — the caller keeps the default, and
/// that is the documented meaning of "only explicitly-set variables
/// override". A variable that IS set but does not parse is a different case,
/// and it used to be indistinguishable from unset: the operator's intended
/// limit was silently discarded. That is now logged. The fallback itself is
/// unchanged and deliberate; what changed is that it can be seen.
fn parse_override<T>(name: &str) -> Option<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let raw = match std::env::var(name) {
        Ok(raw) => raw,
        // NotPresent is the documented "no override" case.
        Err(std::env::VarError::NotPresent) => return None,
        // A variable the operator set that could not be read is not the same
        // thing and is reported. `NotUnicode`'s Display prints the raw value,
        // which may be sensitive, so the failure kind is logged instead.
        Err(std::env::VarError::NotUnicode(_)) => {
            tracing::warn!(
                variable = name,
                error = "value is not valid unicode",
                "could not read a limit override; using the default"
            );
            return None;
        }
    };
    match raw.parse::<T>() {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(
                variable = name,
                error = %error,
                "limit override is not a valid value and was ignored; the default applies",
            );
            None
        }
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        super::profile::limits_for_profile(super::profile::AuditProfile::Adaptive)
    }
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
