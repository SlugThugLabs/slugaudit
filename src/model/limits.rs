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
    /// Cap on serialized JSON bytes returned by one `query` call.
    /// Env: `SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES` (usize).
    pub max_query_response_bytes: usize,
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
        let mut limits = Self::default();
        if let Some(val) = parse_u64("SLUGAUDIT_MAX_FILE_BYTES") {
            limits.max_file_bytes = val;
        }
        if let Some(val) = parse_u64("SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES") {
            limits.max_total_import_bytes = val;
        }
        if let Some(val) = parse_usize("SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES") {
            limits.max_query_response_bytes = val;
        }
        if let Some(val) = parse_usize("SLUGAUDIT_MAX_QUERY_SQL_BYTES") {
            limits.max_query_sql_bytes = val;
        }
        if let Some(val) = parse_u32("SLUGAUDIT_MAX_QUERY_VM_STEPS") {
            limits.max_query_vm_steps = val;
        }
        if let Some(val) = parse_duration_secs("SLUGAUDIT_MAX_QUERY_WALL_CLOCK_SECS") {
            limits.max_query_wall_clock = val;
        }
        if let Some(val) = parse_usize("SLUGAUDIT_MAX_QUERY_VALUE_BYTES") {
            limits.max_query_value_bytes = val;
        }
        if let Some(val) = parse_usize("SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES") {
            limits.max_structure_query_bytes = val;
        }
        if let Some(val) = parse_usize("SLUGAUDIT_MAX_STRUCTURE_MATCHES") {
            limits.max_structure_matches = val;
        }
        if let Some(val) = parse_duration_secs("SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS") {
            limits.max_structure_execution_time = val;
        }
        if let Some(val) = parse_duration_secs("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS") {
            limits.max_sync_wall_clock = val;
        }
        limits
    }
}

fn parse_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}

fn parse_u32(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.parse().ok()
}

fn parse_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.parse().ok()
}

fn parse_duration_secs(name: &str) -> Option<Duration> {
    let secs: u64 = std::env::var(name).ok()?.parse().ok()?;
    Some(Duration::from_secs(secs))
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 64 * 1024 * 1024,
            max_total_import_bytes: 4 * 1024 * 1024 * 1024,
            max_query_response_bytes: 64 * 1024 * 1024,
            max_query_sql_bytes: 100_000,
            max_query_vm_steps: 10_000_000,
            max_query_wall_clock: Duration::from_secs(30),
            max_query_value_bytes: 64 * 1024 * 1024,
            max_structure_query_bytes: 64_000,
            max_structure_matches: 5_000,
            max_structure_execution_time: Duration::from_secs(30),
            max_sync_wall_clock: Duration::from_secs(600),
            evidence: EvidenceLimits::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_strictly_positive() {
        let limits = ResourceLimits::default();
        assert!(limits.max_file_bytes > 0);
        assert!(limits.max_total_import_bytes >= limits.max_file_bytes);
        assert!(limits.max_query_response_bytes > 0);
        assert!(limits.max_query_vm_steps > 0);
        assert!(limits.max_query_wall_clock.as_millis() > 0);
        assert!(limits.max_query_value_bytes > 0);
        assert!(limits.max_query_value_bytes <= limits.max_query_response_bytes);
        assert!(limits.max_structure_query_bytes > 0);
        assert!(limits.max_structure_matches > 0);
        assert!(limits.max_structure_execution_time.as_millis() > 0);
        assert!(limits.max_sync_wall_clock.as_millis() > 0);
    }

    #[test]
    fn from_env_uses_defaults_when_no_vars_are_set() {
        temp_env::with_var_unset("SLUGAUDIT_MAX_FILE_BYTES", || {
            let limits = ResourceLimits::from_env();
            assert_eq!(
                limits.max_file_bytes,
                ResourceLimits::default().max_file_bytes
            );
        });
    }

    #[test]
    fn from_env_overrides_u64_fields() {
        temp_env::with_var("SLUGAUDIT_MAX_FILE_BYTES", Some("16777216"), || {
            let limits = ResourceLimits::from_env();
            assert_eq!(limits.max_file_bytes, 16_777_216);
            // Other fields remain at defaults
            assert_eq!(
                limits.max_total_import_bytes,
                ResourceLimits::default().max_total_import_bytes
            );
        });
    }

    #[test]
    fn from_env_overrides_duration_fields() {
        temp_env::with_var("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS", Some("120"), || {
            let limits = ResourceLimits::from_env();
            assert_eq!(limits.max_sync_wall_clock, Duration::from_secs(120));
        });
    }

    #[test]
    fn unparseable_env_vars_are_silently_ignored() {
        temp_env::with_var("SLUGAUDIT_MAX_FILE_BYTES", Some("not-a-number"), || {
            let limits = ResourceLimits::from_env();
            assert_eq!(
                limits.max_file_bytes,
                ResourceLimits::default().max_file_bytes
            );
        });
    }

    #[test]
    fn multiple_overrides_compose() {
        temp_env::with_vars(
            [
                ("SLUGAUDIT_MAX_FILE_BYTES", Some("1048576")),
                ("SLUGAUDIT_MAX_QUERY_VM_STEPS", Some("5000000")),
                ("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS", Some("30")),
            ],
            || {
                let limits = ResourceLimits::from_env();
                assert_eq!(limits.max_file_bytes, 1_048_576);
                assert_eq!(limits.max_query_vm_steps, 5_000_000);
                assert_eq!(limits.max_sync_wall_clock, Duration::from_secs(30));
                // Unset fields stay at defaults
                assert_eq!(
                    limits.max_query_response_bytes,
                    ResourceLimits::default().max_query_response_bytes
                );
            },
        );
    }
}
