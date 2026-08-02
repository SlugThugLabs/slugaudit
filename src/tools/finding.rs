use super::context::ensure_synced;
use crate::store;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::params;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

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
/// isn't indexed, required text fields are empty, `line_start` exceeds
/// `line_end`, or the write itself fails.
pub fn finding(request: &Parameters<FindingRequest>) -> Result<Json<FindingResponse>, ErrorData> {
    let request = &request.0;
    validate(request)?;

    let synced = ensure_synced(&request.path)?;
    let mut connection = store::open_read_write(&synced.database_path)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

    let source_hash: String = connection
        .query_row(
            "SELECT content_hash FROM files WHERE path = ?1",
            [&request.file],
            |row| row.get(0),
        )
        .map_err(|error| ErrorData::invalid_params(format!("{}: {error}", request.file), None))?;

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

fn validate(request: &FindingRequest) -> Result<(), ErrorData> {
    if request.title.trim().is_empty() {
        return Err(ErrorData::invalid_params("title must not be empty", None));
    }
    if request.description.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            "description must not be empty",
            None,
        ));
    }
    if request.severity.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            "severity must not be empty",
            None,
        ));
    }
    if request.category.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            "category must not be empty",
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
