//! Input validation for the `finding` tool: the size and emptiness checks on
//! every caller-supplied field, which run before the index is touched, and the
//! line-range check against the indexed file's content.

use super::MAX_PATH_BYTES;
use super::finding::{
    FindingRequest, MAX_CATEGORY_CHARS, MAX_DESCRIPTION_CHARS, MAX_FILE_BYTES, MAX_SEVERITY_CHARS,
    MAX_TITLE_CHARS,
};
use crate::util;
use rmcp::ErrorData;

/// Bounds every caller-supplied text field before the index is touched.
pub(super) fn validate_text_fields(request: &FindingRequest) -> Result<(), ErrorData> {
    if util::exceeds_input_bytes(&request.path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params(
            format!("path exceeds {MAX_PATH_BYTES} bytes"),
            None,
        ));
    }
    if util::exceeds_input_bytes(&request.file, MAX_FILE_BYTES) {
        return Err(ErrorData::invalid_params(
            format!("file exceeds {MAX_FILE_BYTES} bytes"),
            None,
        ));
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

pub(super) fn validate_line_range(
    request: &FindingRequest,
    content: &str,
) -> Result<(), ErrorData> {
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
