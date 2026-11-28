//! Input validation for `structure` requests before execution or paging.

use super::super::grammar::check_non_alias_non_prose_language;
use super::super::{MAX_CURSOR_BYTES, MAX_PATH_BYTES, check_bytes_limit, check_path_bytes};
use super::StructureRequest;
use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::model::ResourceLimits;
use rmcp::ErrorData;
use std::borrow::Cow;

const MAX_FILTER_BYTES: usize = 8 * 1024;

fn invalid_input(reason: impl Into<Cow<'static, str>>) -> ErrorData {
    invalid_params_with_code(reason, ErrorCode::InvalidInput)
}

pub(super) fn validate_entry(request: &StructureRequest) -> Result<(), ErrorData> {
    request.legacy_paging.refuse()?;
    if let Some(cursor) = request.cursor.as_deref() {
        return check_bytes_limit(cursor, "cursor", MAX_CURSOR_BYTES);
    }
    check_path_bytes(&request.path)?;
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
        if let Some(text) = value {
            check_bytes_limit(text, name, max_bytes)?;
        }
    }
    Ok(())
}

pub(super) fn validate_content(
    request: &StructureRequest,
    limits: &ResourceLimits,
) -> Result<(), ErrorData> {
    if let Some(language) = request.language.as_deref()
        && let Err(reason) = check_non_alias_non_prose_language(language)
    {
        return Err(invalid_input(reason));
    }
    if request.query.trim().is_empty() {
        return Err(invalid_input("structure query must not be empty"));
    }
    if request.query.len() > limits.max_structure_query_bytes {
        return Err(invalid_input(format!(
            "structure query is {} bytes; the limit is {} (max_structure_query_bytes)",
            request.query.len(),
            limits.max_structure_query_bytes
        )));
    }
    if request.file.is_some() && request.pattern.is_some() {
        return Err(invalid_input(
            "pattern applies only to multi-file queries; omit it with `file`",
        ));
    }
    if request.file.is_some() && request.start_after_file.is_some() {
        return Err(invalid_input(
            "start_after_file applies only to multi-file queries; omit it with `file`",
        ));
    }
    Ok(())
}
