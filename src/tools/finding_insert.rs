//! The `finding` write path: input validation, the indexed-file lookup, and
//! the session-scoped insert-or-refresh. Split from `finding.rs` so the tool
//! contract (request/response shapes) stays separate from the statements
//! that store an agent-authored conclusion.
//!
//! The insert restates a conclusion by refreshing the row it already owns
//! instead of appending a near-duplicate — the Python build's `identity_hash`
//! dedupe, restored inside the session. Two calls are *the same finding* when
//! they name the same file at the same content hash, the same line range, and
//! the same `category` and `title`; `severity` and `description` are body, so
//! the restatement replaces them. The match is scoped to the writing session:
//! a conclusion is only ever merged into its author's notebook.

use super::context::{internal_error, session_id};
use super::finding::{
    FindingRequest, FindingResponse, MAX_CATEGORY_CHARS, MAX_DESCRIPTION_CHARS, MAX_FILE_BYTES,
    MAX_PATH_BYTES, MAX_SEVERITY_CHARS, MAX_TITLE_CHARS,
};
use crate::error_codes::{self, ErrorCode};
use crate::util;
use rmcp::ErrorData;
use rusqlite::{OptionalExtension, Transaction, params};

/// Stores one validated finding and returns the persisted row identity.
///
/// # Errors
///
/// Returns an error when the file is not indexed, the line range does not
/// fit the file, or any statement fails.
pub(super) fn insert_finding(
    tx: &Transaction<'_>,
    request: &FindingRequest,
    revision_id: &str,
) -> Result<FindingResponse, ErrorData> {
    let (source_hash, content) = fetch_file(tx, &request.file)?;
    validate_line_range(request, &content)?;
    let id = persist_finding(tx, request, &source_hash, revision_id)?;

    // File, line range, and id are structural metadata about *where* a
    // finding landed — never the title/description/severity/category
    // text, which is exactly the AI-authored judgment content that must
    // stay out of logs.
    tracing::info!(
        target: "slugaudit_mcp_rust::tools::finding",
        revision_id,
        file_id = %util::redacted_log_id(request.file.as_bytes()),
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

/// Refreshes a restated finding's row, or inserts a new one.
fn persist_finding(
    tx: &Transaction<'_>,
    request: &FindingRequest,
    source_hash: &str,
    revision_id: &str,
) -> Result<i64, ErrorData> {
    match find_restated(tx, request, source_hash)? {
        Some(id) => {
            refresh_restated(tx, request, revision_id, id)?;
            Ok(id)
        }
        None => insert_new(tx, request, source_hash, revision_id),
    }
}

/// The row this session already owns for the same file, content hash, line
/// range, category, and title, if any.
fn find_restated(
    tx: &Transaction<'_>,
    request: &FindingRequest,
    source_hash: &str,
) -> Result<Option<i64>, ErrorData> {
    tx.query_row(
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
    .map_err(|error| ErrorData::internal_error(error.to_string(), None))
}

/// Restatement body: replaces the judgment-bearing fields and marks the row
/// current against the verified revision.
fn refresh_restated(
    tx: &Transaction<'_>,
    request: &FindingRequest,
    revision_id: &str,
    id: i64,
) -> Result<(), ErrorData> {
    tx.execute(
        "UPDATE findings SET severity = ?1, description = ?2, evidence_revision = ?3, \
         status = 'current' WHERE id = ?4",
        params![request.severity, request.description, revision_id, id],
    )
    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(())
}

/// Appends a new finding, timestamped at least as late as the newest
/// persisted row (`created_at_unix` is the ordering key in the notebook).
fn insert_new(
    tx: &Transaction<'_>,
    request: &FindingRequest,
    source_hash: &str,
    revision_id: &str,
) -> Result<i64, ErrorData> {
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
    Ok(tx.last_insert_rowid())
}

/// Bounds every caller-supplied text field before the index is touched.
pub(super) fn validate_text_fields(request: &FindingRequest) -> Result<(), ErrorData> {
    if util::exceeds_input_bytes(&request.path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params("path exceeds 32768 bytes", None));
    }
    if util::exceeds_input_bytes(&request.file, MAX_FILE_BYTES) {
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

/// The indexed file's `(content_hash, content)`.
///
/// Only "this file is not in the index" is the caller's input being invalid.
/// A failure to *read* the index — a lock, corruption, or schema drift — is
/// an operational error and must not be reported as a bad `file` argument:
/// that tells the caller to fix an input that was fine.
pub(super) fn fetch_file(tx: &Transaction<'_>, file: &str) -> Result<(String, String), ErrorData> {
    let row: Option<(String, Option<String>)> = tx
        .query_row(
            "SELECT content_hash, content FROM files WHERE path = ?1",
            [file],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| internal_error("reading the indexed file row", error))?;
    let Some((source_hash, content)) = row else {
        let payload = error_codes::payload(ErrorCode::FileNotIndexed);
        return Err(ErrorData::invalid_params(
            format!("{file} is not indexed in the current revision"),
            payload,
        ));
    };
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
