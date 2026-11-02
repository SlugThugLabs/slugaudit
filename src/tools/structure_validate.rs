//! Input validation for the `structure` tool: the byte caps that need no
//! limits, and the checks on an empty, contradictory or over-long query.

use super::{MAX_FILTER_BYTES, MAX_PATH_BYTES, StructureRequest};
use crate::model::ResourceLimits;
use crate::tools::MAX_CURSOR_BYTES;
use rmcp::ErrorData;

/// The checks that need no limits, so they run before the project's limits
/// are resolved: a continuation's cursor is bounded, a new request's
/// free-form fields are.
pub(super) fn entry(request: &StructureRequest) -> Result<(), ErrorData> {
    match request.cursor.as_deref() {
        Some(cursor) => validate_cursor(cursor),
        None => validate_request_sizes(request),
    }
}

/// Bounds the cursor like every other string input.
fn validate_cursor(cursor: &str) -> Result<(), ErrorData> {
    if crate::util::exceeds_input_bytes(cursor, MAX_CURSOR_BYTES) {
        return Err(ErrorData::invalid_params(
            format!("cursor exceeds {MAX_CURSOR_BYTES} bytes"),
            None,
        ));
    }
    Ok(())
}

/// Rejects over-long free-form fields. Needs no limits, so it runs before
/// the project's limits are resolved.
fn validate_request_sizes(request: &StructureRequest) -> Result<(), ErrorData> {
    if crate::util::exceeds_input_bytes(&request.path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params(
            format!("path exceeds {MAX_PATH_BYTES} bytes"),
            None,
        ));
    }
    for (name, value, max_bytes) in [
        ("file", request.file.as_deref(), MAX_FILTER_BYTES),
        ("language", request.language.as_deref(), MAX_FILTER_BYTES),
        ("pattern", request.pattern.as_deref(), MAX_FILTER_BYTES),
        (
            "start_after_file",
            request.start_after_file.as_deref(),
            MAX_PATH_BYTES,
        ),
    ] {
        if value.is_some_and(|value| crate::util::exceeds_input_bytes(value, max_bytes)) {
            return Err(ErrorData::invalid_params(
                format!("{name} exceeds {max_bytes} bytes"),
                None,
            ));
        }
    }
    Ok(())
}

/// Rejects empty or contradictory inputs, and a query over the project's
/// limit, before any sync or parse work.
pub(super) fn content(
    request: &StructureRequest,
    limits: &ResourceLimits,
) -> Result<(), ErrorData> {
    if request
        .language
        .as_deref()
        .is_some_and(crate::parse::is_prose_language_name)
    {
        return Err(crate::tools::structure_source::prose_not_parsed_error());
    }
    if request.query.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            "structure query must not be empty",
            None,
        ));
    }
    if request.query.len() > limits.max_structure_query_bytes {
        return Err(ErrorData::invalid_params(
            format!(
                "structure query is {} bytes; the limit is {} (max_structure_query_bytes)",
                request.query.len(),
                limits.max_structure_query_bytes
            ),
            None,
        ));
    }
    if request.file.is_some() && request.start_after_file.is_some() {
        return Err(ErrorData::invalid_params(
            "start_after_file applies only to multi-file queries; omit it with `file`",
            None,
        ));
    }
    Ok(())
}
