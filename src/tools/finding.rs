use super::context::{ensure_synced, session_id, with_verified_write};
use crate::error_codes::{self, ErrorCode};
use crate::sync;
use crate::util;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::{OptionalExtension, Transaction, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
// slugaudit-line-exception: approved-by=slugthug; reason=one concern - the finding tool's write path: the request/response contract, the input caps that bound every field the agent supplies, and the insert that stores agent-authored text verbatim by design

const MAX_TITLE_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 10_000;
const MAX_SEVERITY_CHARS: usize = 100;
const MAX_CATEGORY_CHARS: usize = 100;
const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_FILE_BYTES: usize = 8 * 1024;

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

fn insert_finding(
    tx: &Transaction<'_>,
    request: &FindingRequest,
    revision_id: &str,
) -> Result<FindingResponse, ErrorData> {
    let (source_hash, content) = fetch_file(tx, &request.file)?;
    validate_line_range(request, &content)?;

    // Restating a conclusion refreshes the row it already owns instead of
    // appending a near-duplicate — the Python build's `identity_hash` dedupe,
    // restored inside the session. Two calls are *the same finding* when they
    // name the same file at the same content hash, the same line range, and
    // the same `category` and `title`; `severity` and `description` are body,
    // so the restatement replaces them. The match is scoped to the writing
    // session: a conclusion is only ever merged into its author's notebook.
    let restated: Option<i64> = tx
        .query_row(
            "SELECT id FROM findings \
             WHERE session_id = ?1 AND path = ?2 AND source_hash = ?3 AND line_start = ?4 \
               AND line_end = ?5 AND category = ?6 AND title = ?7 \
             ORDER BY id LIMIT 1",
            params![
                session_id().to_string(),
                request.file,
                source_hash,
                request.line_start,
                request.line_end,
                request.category,
                request.title,
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let id = if let Some(id) = restated {
        tx.execute(
            "UPDATE findings SET severity = ?1, description = ?2, evidence_revision = ?3, \
             status = 'current' WHERE id = ?4",
            params![request.severity, request.description, revision_id, id],
        )
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        id
    } else {
        let persisted_time: Option<i64> = tx
            .query_row("SELECT max(created_at_unix) FROM findings", [], |row| {
                row.get(0)
            })
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let created_at = util::at_least_timestamp(util::now_unix(), persisted_time);
        tx.execute(
            "INSERT INTO findings (\
                path, source_hash, line_start, line_end, severity, category, title, description, \
                created_at_unix, evidence_revision, status, session_id\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'current', ?11)",
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
                session_id().to_string(),
            ],
        )
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        tx.last_insert_rowid()
    };

    // File, line range, and id are structural metadata about *where* a
    // finding landed — never the title/description/severity/category
    // text, which is exactly the AI-authored judgment content that must
    // stay out of logs.
    tracing::info!(
        revision_id,
        file_id = %crate::util::redacted_log_id(request.file.as_bytes()),
        id,
        "finding recorded"
    );
    Ok(FindingResponse {
        id,
        revision_id: revision_id.to_owned(),
        source_hash,
        status: "current",
    })
}

fn validate_text_fields(request: &FindingRequest) -> Result<(), ErrorData> {
    if crate::util::exceeds_input_bytes(&request.path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params("path exceeds 32768 bytes", None));
    }
    if crate::util::exceeds_input_bytes(&request.file, MAX_FILE_BYTES) {
        return Err(ErrorData::invalid_params("file exceeds 8192 bytes", None));
    }
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
        let payload = error_codes::payload(ErrorCode::FileNotIndexed);
        ErrorData::invalid_params(format!("{file} has no indexed source content"), payload)
    })?;
    Ok((source_hash, content))
}

fn validate_line_range(request: &FindingRequest, content: &str) -> Result<(), ErrorData> {
    let line_count = u32::try_from(content.lines().count()).map_err(|error| {
        ErrorData::invalid_params(
            format!(
                "{} has too many lines to attach a finding to: {error}",
                request.file
            ),
            None,
        )
    })?;
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

#[cfg(test)]
#[path = "finding_session_tests.rs"]
mod session_tests;
