use super::context::{SyncRecencyCache, ensure_synced, with_verified_write};
use crate::util;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::{Transaction, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_TITLE_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 10_000;
const MAX_SEVERITY_CHARS: usize = 100;
const MAX_CATEGORY_CHARS: usize = 100;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindingRequest {
    /// Any path inside the active project — used to select which project's
    /// database to write to. Must be a path that `slugaudit enable` was run on.
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
    #[schemars(description = "Severity of the finding, e.g. critical/high/medium/low/informational")]
    pub severity: String,
    /// A short label for what kind of issue this is — e.g. "security",
    /// "performance", "correctness", "style", "maintainability". SlugAudit
    /// does not enforce a fixed taxonomy.
    #[schemars(description = "Category label for the finding, e.g. security/performance/correctness/style")]
    pub category: String,
    /// A concise, one-line title summarizing the finding. Max 200 characters.
    #[schemars(description = "Concise one-line title summarizing the finding (max 200 chars)")]
    pub title: String,
    /// A fuller explanation of what the issue is, why it matters, and (if
    /// relevant) how to fix it. Max 10,000 characters.
    #[schemars(description = "Detailed description of the finding, its impact, and any remediation (max 10000 chars)")]
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
    cache: &SyncRecencyCache,
) -> Result<Json<FindingResponse>, ErrorData> {
    let request = &request.0;
    validate_text_fields(request)?;

    let synced = ensure_synced(&request.path, cache)?;
    let revision_id = synced.revision_id.clone();
    let response = with_verified_write(&synced, |tx| insert_finding(tx, request, &revision_id))?;
    Ok(Json(response))
}

fn insert_finding(
    tx: &Transaction<'_>,
    request: &FindingRequest,
    revision_id: &str,
) -> Result<FindingResponse, ErrorData> {
    let (source_hash, content) = fetch_file(tx, &request.file)?;
    validate_line_range(request, &content)?;

    let created_at = util::now_unix();
    tx.execute(
        "INSERT INTO findings (\
            path, source_hash, line_start, line_end, severity, category, title, description, \
            created_at_unix, evidence_revision, status\
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'current')",
        params![
            request.file,
            source_hash,
            request.line_start,
            request.line_end,
            request.severity,
            request.category,
            request.title,
            request.description,
            created_at,
            revision_id,
        ],
    )
    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

    let id = tx.last_insert_rowid();
    // File, line range, and id are structural metadata about *where* a
    // finding landed — never the title/description/severity/category
    // text, which is exactly the AI-authored judgment content that must
    // stay out of logs.
    tracing::info!(revision_id, file = request.file, id, "finding recorded");
    Ok(FindingResponse {
        id,
        revision_id: revision_id.to_owned(),
        source_hash,
        status: "current",
    })
}

fn validate_text_fields(request: &FindingRequest) -> Result<(), ErrorData> {
    check_field("title", &request.title, MAX_TITLE_CHARS)?;
    check_field("description", &request.description, MAX_DESCRIPTION_CHARS)?;
    check_field("severity", &request.severity, MAX_SEVERITY_CHARS)?;
    check_field("category", &request.category, MAX_CATEGORY_CHARS)?;
    if request.line_start == 0 || request.line_end == 0 {
        return Err(ErrorData::invalid_params(
            "line_start and line_end are one-based; zero is not a valid line",
            None,
        ));
    }
    if request.line_start > request.line_end {
        return Err(ErrorData::invalid_params(
            "line_start must not exceed line_end",
            None,
        ));
    }
    Ok(())
}

fn check_field(name: &str, value: &str, max_chars: usize) -> Result<(), ErrorData> {
    if value.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            format!("{name} must not be empty"),
            None,
        ));
    }
    if value.chars().count() > max_chars {
        return Err(ErrorData::invalid_params(
            format!("{name} exceeds {max_chars} characters"),
            None,
        ));
    }
    Ok(())
}

fn fetch_file(tx: &Transaction<'_>, file: &str) -> Result<(String, String), ErrorData> {
    let (source_hash, content): (String, Option<String>) = tx
        .query_row(
            "SELECT content_hash, content FROM files WHERE path = ?1",
            [file],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| ErrorData::invalid_params(format!("{file}: {error}"), None))?;
    let content = content.ok_or_else(|| {
        ErrorData::invalid_params(format!("{file} has no indexed source content"), None)
    })?;
    Ok((source_hash, content))
}

fn validate_line_range(request: &FindingRequest, content: &str) -> Result<(), ErrorData> {
    let line_count = u32::try_from(content.lines().count()).unwrap_or(u32::MAX);
    // An empty file still has no addressable one-based lines.
    if line_count == 0 {
        return Err(ErrorData::invalid_params(
            format!("{} has no lines to attach a finding to", request.file),
            None,
        ));
    }
    if request.line_end > line_count {
        return Err(ErrorData::invalid_params(
            format!(
                "line_end {} exceeds {}'s length ({line_count} lines)",
                request.line_end, request.file
            ),
            None,
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "finding_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "finding_race_tests.rs"]
mod race_tests;
