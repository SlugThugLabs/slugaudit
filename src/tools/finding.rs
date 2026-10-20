#[cfg(test)]
use super::context::session_id;
use super::context::{ensure_synced, with_verified_write};
#[cfg(test)]
use super::finding_insert::fetch_file;
use super::finding_insert::{insert_finding, validate_text_fields};
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(super) const MAX_TITLE_CHARS: usize = 200;
pub(super) const MAX_DESCRIPTION_CHARS: usize = 10_000;
pub(super) const MAX_SEVERITY_CHARS: usize = 100;
pub(super) const MAX_CATEGORY_CHARS: usize = 100;
pub(super) const MAX_PATH_BYTES: usize = 32 * 1024;
pub(super) const MAX_FILE_BYTES: usize = 8 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
/// One AI-authored finding to store verbatim. SlugAudit validates shape and
/// bounds only; every judgment-bearing field is the caller's text.
pub struct FindingRequest {
    /// Any path inside the active project — used to select which project's
    /// database to write to. Must be a path inside an enabled project.
    #[schemars(description = "Any path inside the active project, used to select the database")]
    pub path: String,
    /// Project-relative path of the file the finding is about. Must be a file
    /// that SlugAudit has already indexed (i.e. it exists in the project at
    /// the time of the last `report`/`publish`).
    #[schemars(description = "Project-relative path of the file this finding is about")]
    pub file: String,
    /// One-based line number where the finding starts (inclusive).
    #[schemars(description = "One-based inclusive start line of the finding")]
    pub line_start: u32,
    /// One-based line number where the finding ends (inclusive). Must be >= line_start.
    #[schemars(description = "One-based inclusive end line of the finding")]
    pub line_end: u32,
    /// How serious the finding is, in your own words — e.g. "critical",
    /// "high", "medium", "low", "informational". SlugAudit does not enforce a
    /// fixed set of values; use whatever scale is meaningful for your review.
    #[schemars(
        description = "Severity of the finding, e.g. critical/high/medium/low/informational"
    )]
    pub severity: String,
    /// A short label for what kind of issue this is — e.g. "security",
    /// "performance", "correctness", "style", "maintainability". SlugAudit
    /// does not enforce a fixed taxonomy.
    #[schemars(
        description = "Category label for the finding, e.g. security/performance/correctness/style"
    )]
    pub category: String,
    /// A concise, one-line title summarizing the finding. Max 200 characters.
    #[schemars(description = "Concise one-line title summarizing the finding (max 200 chars)")]
    pub title: String,
    /// A fuller explanation of what the issue is, why it matters, and (if
    /// relevant) how to fix it. Max 10,000 characters.
    #[schemars(
        description = "Detailed description of the finding, its impact, and any remediation (max 10000 chars)"
    )]
    pub description: String,
}

/// Persists exactly the conclusion the AI supplied — nothing here is
/// generated. Tied to the file's real current hash so it auto-invalidates
/// (`sync::revision::invalidate_stale_findings`) the moment that hash
/// changes, without anyone having to remember to check.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FindingResponse {
    pub id: i64,
    pub revision_id: String,
    pub source_hash: String,
    pub status: &'static str,
}

/// # Errors
///
/// Returns an error if `request.path` isn't an active project, `request.file`
/// isn't indexed with real content, required text fields are empty or
/// exceed their length limit, line numbers are zero or reversed,
/// `line_end` exceeds the file's actual line count, or the write fails.
pub fn finding(
    request: &Parameters<FindingRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<FindingResponse>, ErrorData> {
    let request = &request.0;
    validate_text_fields(request)?;

    let synced = ensure_synced(&request.path, sink, manager)?;
    let revision_id = synced.revision_id.clone();
    let response = with_verified_write(&synced, |tx| insert_finding(tx, request, &revision_id))?;
    Ok(Json(response))
}

#[cfg(test)]
#[path = "finding_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "finding_race_tests.rs"]
mod race_tests;

#[cfg(test)]
#[path = "finding_session_tests.rs"]
mod session_tests;
