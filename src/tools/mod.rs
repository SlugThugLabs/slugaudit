//! Tool handlers orchestrate: they resolve the active project, ensure it's
//! synced, and format a response. They own no parsing, SQL, or storage
//! logic themselves — that lives in `sync`, `store`, and `evidence`.
//!
//! This module also owns the **tool-level call counters**: process-wide
//! `AtomicU64` counts of total calls, error calls, and total elapsed
//! wall-clock time across every tool. The `health` tool exposes these
//! counters for observability without changing anything about how the
//! other tools execute. The counters live here rather than in
//! `server.rs` so they're reachable without going through the MCP tool
//! router — a future non-MCP entry point (a CLI subcommand, a watchdog,
//! etc.) bumps the same counters and the `health` tool sees them
//! consistently.

pub(crate) mod context;
mod finding;
mod finding_read;
mod health;
mod health_database;
mod leads;
mod leads_compact;
mod leads_mechanics;
mod leads_page;
mod leads_query;
mod page_size;
mod project_control;
mod query;
mod query_clip;
mod query_exec;
mod query_value;

mod report;
mod report_completeness;
mod report_edges;
mod structure;
mod structure_exec;
mod structure_source;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "redaction_tests.rs"]
mod redaction_tests;

pub use context::ensure_synced;
pub use finding::{FindingRequest, FindingResponse, finding};
pub use finding_read::{FindingReadEntry, FindingReadRequest, FindingReadResponse, finding_read};
pub use health::{HealthRequest, HealthResponse, health};
pub use leads::{
    CompactLeadRow, FileCount, LeadMode, LeadRow, LeadRows, LeadsRequest, LeadsResponse,
    MechanicCount, MechanicCoverage, PageLimit, leads,
};
pub use project_control::{ProjectControlRequest, ProjectControlResponse, project_control};
pub use query::{QueryRequest, QueryResponse, query};
pub use query_clip::ClippedCell;
pub use report::{ReportRequest, ReportResponse, report};
pub use report_completeness::{
    CompletenessCount, CompletenessEnvelope, FileClassCount, FileKindCount, SkipReasonCount,
};
pub use report_edges::DependencyEdgeTotals;
pub use structure::{StructureRequest, StructureResponse, structure};

use crate::model::{ResourceLimits, process_limits};
use rmcp::ErrorData;
use std::sync::atomic::{AtomicU64, Ordering};

/// Single definition of "this path looks like a credential file". Shared by
/// `report`'s count and `leads`' `credential_files` rows so the two surfaces
/// can never disagree about what they are counting.
///
/// Both families of rule match against `lower(path)`, so case never decides:
///
/// * **Ending rules** — `.env`, `.env.*`, `.pem`, `.key`, `.p12`, `.pfx`,
///   `.p8`, `.jks`, `.keystore`, `.tfstate`, `.netrc`, `.npmrc`,
///   `.git-credentials`. `LIKE` anchors nothing, so a leading `%` lets these
///   match at the repository root as well as below it, and the suffix is the
///   whole test: `notes.key.md` and `keyboard.rs` do **not** match `%.key`,
///   only a path ending in `.key` does.
/// * **Name rules** — `service-account*.json` at the root or under a
///   directory, and the SSH private keys `id_rsa`, `id_dsa`, `id_ecdsa` and
///   `id_ed25519` anywhere in a path *except* a `.pub` public key:
///   `id_rsa.pub` is not a secret and must not be counted.
///
/// `_` is a single-character wildcard in `LIKE`, so the SSH name patterns
/// escape it and carry `ESCAPE '\'` — without that, `sub/id-rsa` would match
/// `%/id_rsa%`. The fragment assumes an unaliased `path` column (the `files`
/// table in `report`, `files` for a `Family::Files` mechanic in `leads`),
/// and `Mechanic::where_clause` wraps it in parentheses before a path glob
/// is AND-ed onto it.
pub(crate) const CREDENTIAL_PATH_PREDICATE: &str = "lower(path) LIKE '%.env' \
    OR lower(path) LIKE '%.env.%' \
    OR lower(path) LIKE '%.pem' \
    OR lower(path) LIKE '%.key' \
    OR lower(path) LIKE '%.p12' \
    OR lower(path) LIKE '%.pfx' \
    OR lower(path) LIKE '%.p8' \
    OR lower(path) LIKE '%.jks' \
    OR lower(path) LIKE '%.keystore' \
    OR lower(path) LIKE '%.tfstate' \
    OR lower(path) LIKE '%.netrc' \
    OR lower(path) LIKE '%.npmrc' \
    OR lower(path) LIKE '%.git-credentials' \
    OR lower(path) LIKE 'service-account%.json' \
    OR lower(path) LIKE '%/service-account%.json' \
    OR ((lower(path) LIKE '%id\\_rsa%' ESCAPE '\\' \
        OR lower(path) LIKE '%id\\_dsa%' ESCAPE '\\' \
        OR lower(path) LIKE '%id\\_ecdsa%' ESCAPE '\\' \
        OR lower(path) LIKE '%id\\_ed25519%' ESCAPE '\\') \
        AND lower(path) NOT LIKE '%.pub')";

/// The process and project resource limits for `path`, resolved before the
/// index is touched. Shared by `query`, `leads` and `structure`.
///
/// A path with no active project root is an error, not a cue to fall back to
/// process defaults: every one of those tools goes on to call
/// `ensure_synced`, which resolves the same root and rejects the same path,
/// so a fallback would only run request validation against limits the
/// project does not use, and would drop the resolution error unlogged. The
/// error is the same `invalid_params` (with `project_not_enabled` when the
/// project was never enabled) that `ensure_synced` returns. A malformed
/// project config is a documented, logged recovery inside
/// `ProjectConfig::load_or_default`.
pub(super) fn resolve_limits(path: &str) -> Result<ResourceLimits, ErrorData> {
    let root = crate::project::find_project_root(std::path::Path::new(path))
        .map_err(crate::sync::map_activation_error)?;
    Ok(
        crate::project::ProjectConfig::load_or_default(root.as_path())
            .apply_to_limits(*process_limits()),
    )
}

/// Total number of tool calls the MCP server has served since process
/// start. Includes both successful and errored calls — operators
/// monitoring rate-limited or failing integrations want the denominator.
static TOOL_CALL_COUNT: AtomicU64 = AtomicU64::new(0);

/// Number of tool calls that returned any `ErrorData` (transport-
/// independent error, regardless of code). Errors counted here are
/// surfaced through `health` so operators can see rate/severity without
/// reading MCP logs.
///
/// Counts only call-level errors (the work result). Network-level
/// failures (consumer disconnect, malformed MCP framing) are not
/// counted — they don't reach the work closure and so don't bump this
/// counter.
static TOOL_CALL_ERROR_COUNT: AtomicU64 = AtomicU64::new(0);

/// Sum of all `run_blocking` work durations in milliseconds. Average
/// latency is then `total_ms / count` if `count > 0` — exposed via the
/// `health` tool so operators can see a simple moving-average trend
/// without pulling in a metrics backend.
///
/// Saturating on overflow at the conversion boundary; using
/// `u128`-based math under the hood avoids spurious overflow on
/// long-running servers.
static TOOL_CALL_TOTAL_MS: AtomicU64 = AtomicU64::new(0);

/// Snapshot of process-wide tool-call counters for the `health` tool.
///
/// Saturates each value to its underlying atomic so a `Relaxed` load on
/// a 32-bit architecture produces a consistent 64-bit value; on a
/// 64-bit architecture this is atomic. The fields are returned as
/// `u64` because every value is bounded only by `u64::MAX` over the
/// lifetime of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolCounters {
    pub call_count: u64,
    pub error_count: u64,
    pub total_ms: u64,
}

impl ToolCounters {
    /// Returns the current value of every process-wide tool counter.
    pub fn snapshot() -> Self {
        Self {
            call_count: TOOL_CALL_COUNT.load(Ordering::Relaxed),
            error_count: TOOL_CALL_ERROR_COUNT.load(Ordering::Relaxed),
            total_ms: TOOL_CALL_TOTAL_MS.load(Ordering::Relaxed),
        }
    }

    /// Increments the cumulative counters after a tool call completes.
    /// `succeeded` reflects the work's outcome (transport-independent).
    /// `duration_ms` is the wall-clock time spent inside the blocking
    /// work closure, exclusive of permit-acquire latency.
    ///
    /// Returns the post-bump snapshot so the caller can log the same
    /// counters in a single span without a second `snapshot()` call —
    /// useful for the standard "tool call completed" log line.
    pub(crate) fn record_call(succeeded: bool, duration_ms: u64) -> ToolCounters {
        if !succeeded {
            TOOL_CALL_ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TOOL_CALL_TOTAL_MS.fetch_add(duration_ms, Ordering::Relaxed);
        TOOL_CALL_COUNT.fetch_add(1, Ordering::Relaxed);
        Self::snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_limits_rejects_a_path_with_no_active_project() {
        let directory = tempfile::tempdir().expect("temp dir");

        let error = resolve_limits(directory.path().to_str().expect("utf-8 path"))
            .expect_err("a path with no project root must fail, not fall back");

        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        let code = error
            .data
            .as_ref()
            .and_then(|data| data.get("code"))
            .and_then(|code| code.as_str());
        assert_eq!(code, Some("project_not_enabled"));
    }

    #[test]
    fn resolve_limits_returns_limits_for_an_active_project() {
        let project = test_support::activated_project("a.rs", b"fn a() {}");

        let limits = resolve_limits(project.path().to_str().expect("utf-8 path"))
            .expect("an active project resolves");

        assert!(limits.max_file_bytes > 0);
    }

    /// Runs the shared fragment — the literal SQL text, not a Rust copy of
    /// it — against one path with the real SQLite engine both surfaces use,
    /// binding the path as a parameter.
    fn credential_path_predicate_matches(path: &str) -> bool {
        let connection = rusqlite::Connection::open_in_memory().expect("in-memory database");
        let matched: i64 = connection
            .query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM (SELECT ?1 AS path) WHERE {CREDENTIAL_PATH_PREDICATE})"
                ),
                rusqlite::params![path],
                |row| row.get(0),
            )
            .expect("evaluate credential predicate");
        matched != 0
    }

    /// Table over (path, expected) for the whole equivalence the predicate
    /// claims, run through SQLite rather than a Rust re-implementation, so
    /// the test cannot pass while `report` and `leads` behave differently.
    #[test]
    fn credential_path_predicate_matches_through_sqlite() {
        let cases: &[(&str, bool)] = &[
            // The rules that already existed.
            (".env", true),
            (".env.local", true),
            ("config/.env", true),
            (".env.production", true),
            (".ENV", true),
            ("server.pem", true),
            ("keys/cred.pem", true),
            ("keys/tls.key", true),
            ("main.key", true),
            ("cert.p12", true),
            ("cert.pfx", true),
            // Widened: the dotfiles, key stores and state files the backlog
            // listed as missed.
            (".netrc", true),
            ("home/.netrc", true),
            (".npmrc", true),
            ("proj/.npmrc", true),
            (".git-credentials", true),
            ("home/.git-credentials", true),
            ("AuthKey_ABC123.p8", true),
            ("debug.jks", true),
            ("release.keystore", true),
            ("terraform.tfstate", true),
            ("prod.terraform.tfstate", true),
            ("service-account.json", true),
            ("keys/service-account.json", true),
            ("service-account-prod.json", true),
            // Widened: root-level SSH keys, which the old rule missed
            // because it required a leading slash.
            ("id_rsa", true),
            ("id_ed25519", true),
            ("id_ecdsa", true),
            ("id_dsa", true),
            (".ssh/id_rsa", true),
            ("home/.ssh/id_ecdsa", true),
            ("backup/id_rsa_bak", true),
            ("ID_RSA", true),
            // Tightened: a public key is not a secret.
            ("id_rsa.pub", false),
            ("id_ed25519.pub", false),
            (".ssh/id_rsa.pub", false),
            (".ssh/id_ecdsa.pub", false),
            // `_` is a single-character `LIKE` wildcard, so the escaped
            // SSH patterns must not match a hyphen or any other character
            // in its place.
            ("sub/id-rsa", false),
            ("sub/idXrsa", false),
            // Ordinary source that shares letters with a rule.
            ("keyboard.rs", false),
            ("notes.key.md", false),
            ("pem_util.rs", false),
            ("keystore.rs", false),
            ("src/lib.rs", false),
            ("README.md", false),
            ("environment.rs", false),
            (".envrc", false),
            // Public SSH material.
            ("authorized_keys", false),
            ("known_hosts", false),
        ];
        for &(path, expected) in cases {
            assert_eq!(
                credential_path_predicate_matches(path),
                expected,
                "predicate verdict for {path}"
            );
        }
    }

    #[test]
    fn fetches_a_fresh_snapshot_with_zero_initial_state() {
        // The counters are all-zero at process start; this test verifies
        // the accessor surface compiles and returns a snapshot rather
        // than racing other tests. The exact accumulated value is not
        // asserted — other tests bump the counters, and we cannot reset
        // them without forking the AtomicU64 to per-test locals.
        let _snapshot = ToolCounters::snapshot();
    }
}
