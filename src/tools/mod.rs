// slugaudit-line-exception: approved-by=slugthug; reason=the tools module root: one declaration and re-export list for every tool plus the per-tool golden test modules
//! Tool handlers orchestrate: they resolve the active project, ensure it's
//! synced, and format a response. They own no parsing, SQL, or storage
//! logic themselves — that lives in `sync`, `store`, and `evidence`.
//!
//! This module also owns the **tool-level call counters** (defined in
//! `tool_counters`): process-wide `AtomicU64` counts of total calls, error
//! calls, and total elapsed wall-clock time across every tool. The `health`
//! tool exposes these counters for observability without changing anything
//! about how the other tools execute.

pub(crate) mod context;
mod finding;
mod finding_insert;
mod finding_read;
mod finding_sessions;
mod health;
mod health_phase;
mod health_result_store;
mod leads;
mod leads_compact;
mod leads_covers;
mod leads_custom;
mod leads_mechanics;
mod leads_page;
mod leads_query;
mod mechanic;
mod mechanic_catalog;
mod mechanic_register;
mod mechanic_scan;
mod mechanic_scan_cached;
mod page;
mod page_size;
mod paginate;
mod project_control;
mod query;
mod query_clip;
mod query_exec;
mod query_value;

mod report;
mod report_completeness;
mod report_sections;
mod structure;
mod structure_exec;
mod structure_page;
mod structure_source;
mod tool_counters;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "discard_surface_tests.rs"]
mod discard_surface_tests;

#[cfg(test)]
#[path = "redaction_tests.rs"]
mod redaction_tests;

#[cfg(test)]
mod golden_phase9_support;

#[cfg(test)]
#[path = "golden_phase9_leads_tests.rs"]
mod golden_phase9_leads_tests;

#[cfg(test)]
#[path = "golden_phase9_mechanic_tests.rs"]
mod golden_phase9_mechanic_tests;

#[cfg(test)]
#[path = "golden_phase9_query_tests.rs"]
mod golden_phase9_query_tests;

#[cfg(test)]
#[path = "golden_phase9_structure_tests.rs"]
mod golden_phase9_structure_tests;

#[cfg(test)]
#[path = "golden_phase9_finding_read_tests.rs"]
mod golden_phase9_finding_read_tests;

pub use context::{
    PageContext, PageRequest, PageSpec, Produced, RowSink, Served, decode_rows, ensure_synced,
    page_request, paginate,
};
pub use finding::{FindingRequest, FindingResponse, finding};
pub use finding_read::{FindingReadEntry, FindingReadRequest, FindingReadResponse, finding_read};
pub use finding_sessions::{
    FindingSessionsAction, FindingSessionsRequest, FindingSessionsResponse, PriorSession,
    finding_sessions,
};
pub use health::{HealthRequest, HealthResponse, health};
pub use leads::{
    CompactLeadRow, FileCount, LeadMode, LeadRow, LeadRows, LeadsRequest, LeadsResponse,
    MechanicCount, leads,
};
pub use mechanic::{
    InvalidMechanic, MechanicAction, MechanicListing, MechanicRequest, MechanicResponse,
    MechanicSource, mechanic,
};
pub use page::{Completeness, LegacyPaging, Page, PageLimit};
pub use project_control::{
    GitignoreReport, GitignoreStatus, ImportReport, ProjectControlAction, ProjectControlRequest,
    ProjectControlResponse, project_control,
};
pub use query::{QueryRequest, QueryResponse, query};
pub use query_clip::ClippedCell;
pub use query_value::NonFiniteCell;
pub use report::DependencyEdgeTotals;
pub use report::{ReportMechanics, ReportRequest, ReportResponse, report};
pub use report_completeness::{
    CompletenessCount, CompletenessEnvelope, FileClassCount, FileKindCount, SkipReasonCount,
};
pub use structure::{StructureRequest, StructureResponse, structure};
pub use tool_counters::ToolCounters;

use crate::model::{ResourceLimits, process_limits};
use rmcp::ErrorData;

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

/// The `files` predicate for a parser-failure row, shared by every `report`
/// count and list so they cannot disagree: an indexed file whose parser was
/// unavailable or whose parse failed. Prose (`language = 'prose'`, see
/// `parse::PROSE_LANGUAGE`) is excluded: it is deliberately never
/// parsed, so it is not a failure. A file with no detected language
/// (`language IS NULL`) still counts. The literal is checked against the
/// constant by a unit test.
pub(crate) const PARSER_FAILURE_PREDICATE: &str = "file_kind = 'indexed' \
    AND language IS NOT 'prose' \
    AND (parser_availability != 'Available' OR parse_outcome = 'Failed')";

/// The process and project resource limits for `path`, resolved before the
/// index is touched. Shared by `query`, `leads` and `structure`.
///
/// A path with no active project root is an error, not a cue to fall back to
/// process defaults: every one of those tools goes on to call
/// `ensure_synced`, which resolves the same root and rejects the same path,
/// so a fallback would only run request validation against limits the
/// project does not use, and would drop the resolution error unlogged. The
/// error is the same `invalid_params` (with `project_not_enabled` when the
/// project was never enabled) that `ensure_synced` returns. A present but
/// unreadable or malformed project config fails the call with an
/// `invalid_params` naming the file.
pub(super) fn resolve_limits(path: &str) -> Result<ResourceLimits, ErrorData> {
    limits_for_root(&project_root(path)?)
}

/// The active project root `path` lies in, with the same error
/// `ensure_synced` returns for a path outside every enabled project.
pub(super) fn project_root(path: &str) -> Result<std::path::PathBuf, ErrorData> {
    crate::project::find_project_root(std::path::Path::new(path))
        .map(|root| root.as_path().to_path_buf())
        .map_err(crate::sync::map_activation_error)
}

/// The process-wide limits, or the tool error for an environment that is
/// invalid and was never validated at startup (a library embedder; the shipped
/// binary validates before serving).
pub(super) fn process_limits_for_tool() -> Result<ResourceLimits, ErrorData> {
    process_limits()
        .copied()
        .map_err(crate::sync::map_process_limits_error)
}

/// The process limits with `root`'s project config applied.
pub(super) fn limits_for_root(root: &std::path::Path) -> Result<ResourceLimits, ErrorData> {
    crate::project::ProjectConfig::limits_for(root, process_limits_for_tool()?).map_err(|error| {
        tracing::error!(%error, "project limits could not be derived");
        ErrorData::invalid_params(
            format!("project config is unusable: {error}; fix or remove the file and retry"),
            None,
        )
    })
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

    #[test]
    fn fail_closed_a_malformed_project_config_fails_the_call_naming_the_file() {
        let project = test_support::activated_project("a.rs", b"fn a() {}");
        let config = project.path().join(".planning/slugaudit/config.json");
        std::fs::create_dir_all(config.parent().expect("config dir")).expect("create dir");
        std::fs::write(&config, "{ not json").expect("write config");

        let error = resolve_limits(project.path().to_str().expect("utf-8 path"))
            .expect_err("a malformed config must fail the call, not fall back to defaults");

        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            error.message.contains(config.to_str().expect("utf-8 path")),
            "{}",
            error.message
        );
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

    /// Asserts each `(path, expected)` verdict through the real SQLite engine,
    /// so the test cannot pass while `report` and `leads` behave differently.
    fn assert_predicate_cases(cases: &[(&str, bool)]) {
        for &(path, expected) in cases {
            assert_eq!(
                credential_path_predicate_matches(path),
                expected,
                "predicate verdict for {path}"
            );
        }
    }

    #[test]
    fn credential_path_predicate_matches_credential_shaped_paths() {
        assert_predicate_cases(&[
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
        ]);
    }

    #[test]
    fn credential_path_predicate_matches_ssh_private_keys() {
        assert_predicate_cases(&[
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
        ]);
    }

    #[test]
    fn credential_path_predicate_rejects_public_keys_and_lookalikes() {
        assert_predicate_cases(&[
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
        ]);
    }

    // `ToolCounters` is deliberately not tested here. Its three counters are
    // process-wide `AtomicU64`s that any test in this binary may bump and
    // there is no reset (adding one would be a production test hook), so a
    // zero-initial-state assertion cannot be pinned in the shared test
    // process. The snapshot/record delta contract is covered by
    // `tool_counters_record_bumps_call_and_total_counts`,
    // `tool_counters_record_bumps_error_count_on_error` and
    // `tool_counters_always_return_non_decreasing_after_a_bump` in
    // `src/tools/health_tests.rs`.
}
