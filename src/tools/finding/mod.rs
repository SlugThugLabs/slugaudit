//! Session findings tools: `finding`, `finding_read`, and `finding_clear`.

pub(in crate::tools) mod clear;
pub(in crate::tools) mod read;
pub(in crate::tools) mod store;
mod stored;
mod write;

pub use clear::{FindingClearRequest, FindingClearResponse, finding_clear};
pub use read::{FindingReadEntry, FindingReadRequest, FindingReadResponse, finding_read};

use super::context::{ensure_synced, with_verified_read};
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use store::with_findings_write;
use write::{fetch_file, insert_finding, validate_line_range, validate_text_fields};

pub(super) const MAX_TITLE_CHARS: usize = 200;
pub(super) const MAX_DESCRIPTION_CHARS: usize = 10_000;
pub(super) const MAX_SEVERITY_CHARS: usize = 100;
pub(super) const MAX_CATEGORY_CHARS: usize = 100;
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
    /// One-based line number where the finding ends (inclusive). Must be >= `line_start`.
    #[schemars(description = "One-based inclusive end line of the finding")]
    pub line_end: u32,
    /// How serious the finding is, in your own words — e.g. "critical",
    /// "high", "medium", "low", "informational".
    #[schemars(
        description = "Severity of the finding, e.g. critical/high/medium/low/informational"
    )]
    pub severity: String,
    /// A short label for what kind of issue this is — e.g. "security",
    /// "performance", "correctness", "style", "maintainability".
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

/// Persists exactly the conclusion the AI supplied — nothing here is generated.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FindingResponse {
    /// Row id of the stored finding in this session's findings.
    pub id: i64,
    /// Revision id the finding was recorded against (`evidence_revision`).
    pub revision_id: String,
    /// Content hash of the indexed file the finding was attached to.
    pub source_hash: String,
    /// `current` for every successful write.
    pub status: &'static str,
    /// Present only when this call discarded and rebuilt the project database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_discard: Option<crate::sync::DatabaseDiscard>,
}

/// Records or refreshes one reviewed session finding.
///
/// # Errors
///
/// Returns an error if `request.path` isn't an active project, `request.file`
/// isn't indexed with real content, text or line bounds fail, or writing fails.
pub fn finding(
    request: &Parameters<FindingRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<FindingResponse>, ErrorData> {
    let request = &request.0;
    validate_text_fields(request)?;

    let synced = ensure_synced(&request.path, sink, manager)?;
    let (source_hash, content) = with_verified_read(&synced, |tx| fetch_file(tx, &request.file))?;
    validate_line_range(request, &content)?;
    let mut response = with_findings_write(&synced, |tx| {
        insert_finding(tx, request, source_hash, &synced.revision_id)
    })?;
    response.database_discard = synced.discard;
    Ok(Json(response))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod failure_tests;

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod read_tests;

#[cfg(test)]
mod read_paged_tests;

#[cfg(test)]
mod clear_tests;
