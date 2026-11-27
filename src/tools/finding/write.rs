//! The `finding` write path: input validation, indexed-file lookup, line-range
//! verification, and insert-or-refresh persistence in the session findings DB.

use super::{
    FindingRequest, FindingResponse, MAX_CATEGORY_CHARS, MAX_DESCRIPTION_CHARS, MAX_FILE_BYTES,
    MAX_SEVERITY_CHARS, MAX_TITLE_CHARS,
};
use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::tools::context::internal_error;
use crate::tools::{check_bytes_limit, check_path_bytes};
use crate::util;
use rmcp::ErrorData;
use rusqlite::{OptionalExtension, Transaction, params};

/// Bounds every caller-supplied text field and basic line ordering before sync.
pub(super) fn validate_text_fields(request: &FindingRequest) -> Result<(), ErrorData> {
    check_path_bytes(&request.path)?;
    check_bytes_limit(&request.file, "file", MAX_FILE_BYTES)?;
    check_field("title", &request.title, MAX_TITLE_CHARS)?;
    check_field("description", &request.description, MAX_DESCRIPTION_CHARS)?;
    check_field("severity", &request.severity, MAX_SEVERITY_CHARS)?;
    check_field("category", &request.category, MAX_CATEGORY_CHARS)?;
    if request.line_start == 0 || request.line_end == 0 {
        return Err(invalid_params_with_code(
            "line_start and line_end are one-based; zero is not a valid line",
            ErrorCode::InvalidInput,
        ));
    }
    if request.line_start > request.line_end {
        return Err(invalid_params_with_code(
            "line_start must not exceed line_end",
            ErrorCode::InvalidInput,
        ));
    }
    Ok(())
}

fn check_field(name: &str, value: &str, max_chars: usize) -> Result<(), ErrorData> {
    if value.trim().is_empty() {
        return Err(invalid_params_with_code(
            format!("{name} must not be empty"),
            ErrorCode::InvalidInput,
        ));
    }
    if value.chars().count() > max_chars {
        return Err(invalid_params_with_code(
            format!("{name} exceeds {max_chars} characters"),
            ErrorCode::InvalidInput,
        ));
    }
    Ok(())
}

pub(super) fn validate_line_range(
    request: &FindingRequest,
    content: &str,
) -> Result<(), ErrorData> {
    let line_count = u32::try_from(content.lines().count()).map_err(|error| {
        invalid_params_with_code(
            format!(
                "{} has too many lines to attach a finding to: {error}",
                request.file
            ),
            ErrorCode::InvalidInput,
        )
    })?;
    if line_count == 0 {
        return Err(invalid_params_with_code(
            format!("{} has no lines to attach a finding to", request.file),
            ErrorCode::InvalidInput,
        ));
    }
    if request.line_end > line_count {
        return Err(invalid_params_with_code(
            format!(
                "line_end {} exceeds {}'s length ({line_count} lines)",
                request.line_end, request.file
            ),
            ErrorCode::InvalidInput,
        ));
    }
    Ok(())
}

/// Reads the indexed file's `(content_hash, content)`.
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
        return Err(invalid_params_with_code(
            format!("{file} is not indexed in the current revision"),
            ErrorCode::FileNotIndexed,
        ));
    };
    let content = content.ok_or_else(|| {
        invalid_params_with_code(
            format!("{file} has no indexed source content"),
            ErrorCode::FileNotIndexed,
        )
    })?;
    Ok((source_hash, content))
}

/// Stores or refreshes one validated finding in the session findings DB.
pub(super) fn insert_finding(
    tx: &Transaction<'_>,
    request: &FindingRequest,
    source_hash: String,
    revision_id: &str,
) -> Result<FindingResponse, ErrorData> {
    let id = match find_restated(tx, request, &source_hash)? {
        Some(existing) => {
            refresh_restated(tx, request, revision_id, existing)?;
            existing
        }
        None => insert_new(tx, request, &source_hash, revision_id)?,
    };

    tracing::info!(
        target: "slugaudit_mcp_rust::tools::finding",
        revision_id,
        file = ?request.file,
        id,
        "finding recorded"
    );
    Ok(FindingResponse {
        database_discard: None,
        id,
        revision_id: revision_id.to_owned(),
        source_hash,
        status: "current",
    })
}

fn find_restated(
    tx: &Transaction<'_>,
    request: &FindingRequest,
    source_hash: &str,
) -> Result<Option<i64>, ErrorData> {
    tx.query_row(
        "SELECT id FROM findings \
         WHERE path = ?1 AND source_hash = ?2 AND line_start = ?3 \
           AND line_end = ?4 AND category = ?5 AND title = ?6 \
         ORDER BY id LIMIT 1",
        params![
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

fn refresh_restated(
    tx: &Transaction<'_>,
    request: &FindingRequest,
    revision_id: &str,
    id: i64,
) -> Result<(), ErrorData> {
    tx.execute(
        "UPDATE findings SET severity = ?1, description = ?2, evidence_revision = ?3 \
         WHERE id = ?4",
        params![request.severity, request.description, revision_id, id],
    )
    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(())
}

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
    let created_at = util::at_least_timestamp(util::now_unix()?, persisted_time);
    tx.execute(
        "INSERT INTO findings (\
            path, source_hash, line_start, line_end, severity, category, title, description, \
            created_at_unix, evidence_revision\
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
    Ok(tx.last_insert_rowid())
}
