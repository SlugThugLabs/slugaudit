use serde::{Deserialize, Serialize};

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
            max_items_per_file: 10_000,
            max_payload_bytes_per_item: 64 * 1024,
            max_payload_bytes_per_file: 4 * 1024 * 1024,
        }
    }
}

/// Process-wide resource ceilings for import, query, and structure work.
/// Defaults are intentionally conservative for a single-project MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Largest single file that will be fully read into memory for hashing
    /// and indexing. Larger files are rejected rather than loaded.
    pub max_file_bytes: u64,
    /// Cap on the sum of sampled file sizes in one publish. Prevents one
    /// oversized tree from exhausting process memory during import.
    pub max_total_import_bytes: u64,
    /// Cap on serialized JSON bytes returned by one `query` call.
    pub max_query_response_bytes: usize,
    /// Maximum SQL text length accepted by `query` (UTF-8 bytes).
    pub max_query_sql_bytes: usize,
    /// Soft cap on SQLite virtual-machine steps per `query` execution.
    /// Enforced via a progress handler when available.
    pub max_query_vm_steps: u32,
    /// Wall-clock budget for one `query` execution, independent of the VM
    /// step budget — bounds queries whose individual steps are each slow
    /// (e.g. disk I/O stalls) rather than merely numerous.
    pub max_query_wall_clock: std::time::Duration,
    /// Largest single TEXT or BLOB column value `query` will return, checked
    /// against the raw value before it is cloned or hex-expanded into JSON.
    pub max_query_value_bytes: usize,
    /// Maximum tree-sitter query text length for `structure` (UTF-8 bytes).
    pub max_structure_query_bytes: usize,
    /// Maximum capture matches returned by one `structure` call.
    pub max_structure_matches: usize,
    /// Wall-clock budget for one `structure` query's Tree-sitter execution,
    /// enforced natively via `QueryCursorOptions::progress_callback` so a
    /// pathological pattern (deep nesting, wildcard-heavy captures) can be
    /// aborted mid-query rather than only after it returns.
    pub max_structure_execution_time: std::time::Duration,
    pub evidence: EvidenceLimits,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 8 * 1024 * 1024,
            max_total_import_bytes: 256 * 1024 * 1024,
            max_query_response_bytes: 2 * 1024 * 1024,
            max_query_sql_bytes: 10_000,
            max_query_vm_steps: 2_000_000,
            max_query_wall_clock: std::time::Duration::from_secs(5),
            max_query_value_bytes: 1024 * 1024,
            max_structure_query_bytes: 8_000,
            max_structure_matches: 500,
            max_structure_execution_time: std::time::Duration::from_secs(5),
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
    }
}
