use super::context::{ensure_synced, open_verified_read_write};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::{Connection, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_TITLE_LENGTH: usize = 200;
const MAX_DESCRIPTION_LENGTH: usize = 10_000;
const MAX_SEVERITY_LENGTH: usize = 100;
const MAX_CATEGORY_LENGTH: usize = 100;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindingRequest {
    /// Any path inside the active project.
    pub path: String,
    /// Project-relative path the finding is about.
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    /// AI-supplied; SlugAudit never generates this.
    pub severity: String,
    /// AI-supplied; SlugAudit never generates this.
    pub category: String,
    pub title: String,
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
/// exceed their length limit, `line_start` exceeds `line_end`, `line_end`
/// exceeds the file's actual line count, or the write itself fails.
pub fn finding(request: &Parameters<FindingRequest>) -> Result<Json<FindingResponse>, ErrorData> {
    let request = &request.0;
    validate_text_fields(request)?;

    let synced = ensure_synced(&request.path)?;
    let mut connection = open_verified_read_write(&synced)?;

    let (source_hash, content) = fetch_file(&connection, &request.file)?;
    validate_line_range(request, &content)?;

    let created_at = now_unix();
    let tx = connection
        .transaction()
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
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
            synced.revision_id,
        ],
    )
    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let id = tx.last_insert_rowid();
    tx.commit()
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

    Ok(Json(FindingResponse {
        id,
        revision_id: synced.revision_id,
        source_hash,
        status: "current",
    }))
}

fn validate_text_fields(request: &FindingRequest) -> Result<(), ErrorData> {
    check_field("title", &request.title, MAX_TITLE_LENGTH)?;
    check_field("description", &request.description, MAX_DESCRIPTION_LENGTH)?;
    check_field("severity", &request.severity, MAX_SEVERITY_LENGTH)?;
    check_field("category", &request.category, MAX_CATEGORY_LENGTH)?;
    if request.line_start > request.line_end {
        return Err(ErrorData::invalid_params(
            "line_start must not exceed line_end",
            None,
        ));
    }
    Ok(())
}

fn check_field(name: &str, value: &str, max_length: usize) -> Result<(), ErrorData> {
    if value.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            format!("{name} must not be empty"),
            None,
        ));
    }
    if value.len() > max_length {
        return Err(ErrorData::invalid_params(
            format!("{name} exceeds {max_length} characters"),
            None,
        ));
    }
    Ok(())
}

fn fetch_file(connection: &Connection, file: &str) -> Result<(String, String), ErrorData> {
    let (source_hash, content): (String, Option<String>) = connection
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

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
#[path = "finding_tests.rs"]
mod tests;
