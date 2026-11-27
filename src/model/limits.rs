use super::system_ram::SystemMemoryInfo;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Bounds applied when normalizing AST output into `evidence` rows.
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

#[cfg(test)]
impl Default for EvidenceLimits {
    fn default() -> Self {
        Self {
            max_items_per_file: 100_000,
            max_payload_bytes_per_item: 256 * 1024,
            max_payload_bytes_per_file: 64 * 1024 * 1024,
        }
    }
}

/// Largest TEXT/BLOB the bundled SQLite can store (`SQLITE_MAX_LENGTH` = 1e9).
pub(crate) const SQLITE_MAX_VALUE_BYTES: u64 = 1_000_000_000;

/// Resource ceilings for sync, SQL query, Tree-sitter structure, and paging.
///
/// Every field except `evidence` can be overridden by the human in
/// `~/.slugthug/slugaudit/config.json` or via `SLUGAUDIT_<FIELD>` environment
/// variables. Set-but-invalid values or `0` fail closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Largest single file read into memory for hashing and indexing.
    pub max_file_bytes: u64,
    /// Largest file content a grammar is run on (`oversize_for_parse` above it).
    pub max_parse_bytes: u64,
    /// Wall-clock budget in milliseconds for parsing one file.
    pub max_parse_wall_clock_ms: u64,
    /// Cap on the sum of sampled file sizes in one publish.
    pub max_total_import_bytes: u64,
    /// Cap on serialized JSON bytes returned by one `query` or `leads` page.
    pub max_query_response_bytes: usize,
    /// Default page size in serialized JSON bytes when `page_bytes` is omitted.
    pub default_page_bytes: usize,
    /// Maximum SQL text length accepted by `query` (UTF-8 bytes).
    pub max_query_sql_bytes: usize,
    /// Cap on SQLite virtual-machine steps per `query` execution.
    pub max_query_vm_steps: u32,
    /// Wall-clock budget for one `query` execution.
    pub max_query_wall_clock: Duration,
    /// Largest single TEXT or BLOB column value `query` returns.
    pub max_query_value_bytes: usize,
    /// Maximum Tree-sitter query text length for `structure` (UTF-8 bytes).
    pub max_structure_query_bytes: usize,
    /// Maximum capture matches returned on one `structure` page.
    pub max_structure_matches: usize,
    /// Wall-clock budget for one `structure` execution.
    pub max_structure_execution_time: Duration,
    /// Wall-clock budget for one sync operation across discovery, parse, and publish.
    pub max_sync_wall_clock: Duration,
    /// Total bytes of paged results retained in the result store.
    pub max_result_store_bytes: u64,
    /// Seconds an unfinished paged result stays protected from reclaim after its last page.
    pub result_active_grace_secs: u64,
    /// Per-file bounds applied when normalizing evidence into rows.
    pub evidence: EvidenceLimits,
}

const DEFAULT_BASELINE_MEMORY: SystemMemoryInfo = SystemMemoryInfo {
    total_bytes: 16 * 1024 * 1024 * 1024,
    available_bytes: 16 * 1024 * 1024 * 1024,
};

impl Default for ResourceLimits {
    fn default() -> Self {
        super::profile::adaptive_limits(DEFAULT_BASELINE_MEMORY)
    }
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
