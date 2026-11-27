//! MCP tool contracts and their tool-level execution, pagination, and session
//! state.

pub(crate) mod context;
mod finding;
mod grammar;
mod health;
mod leads;
mod mechanic;
mod page;
mod paginate;
mod paginate_cursor;
mod paginate_cut;
mod paginate_fresh;
mod paginate_sink;
mod paginate_store;
mod project_control;
mod query;
mod report;
mod sql_predicates;
mod structure;
mod tool_counters;
mod tool_limits;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod discard_surface_tests;

#[cfg(test)]
mod redaction_tests;

#[cfg(test)]
mod golden_phase9_support;

#[cfg(test)]
mod golden_phase9_leads_tests;

#[cfg(test)]
mod golden_phase9_mechanic_tests;

#[cfg(test)]
mod golden_phase9_query_tests;

#[cfg(test)]
mod golden_phase9_structure_tests;

#[cfg(test)]
mod golden_phase9_finding_read_tests;

pub use context::ensure_synced;
pub use finding::{
    FindingClearRequest, FindingClearResponse, FindingReadEntry, FindingReadRequest,
    FindingReadResponse, FindingRequest, FindingResponse, finding, finding_clear, finding_read,
};
pub use health::{HealthRequest, HealthResponse, health};
pub use leads::{
    CompactLeadRow, FileCount, LeadMode, LeadRow, LeadRows, LeadsRequest, LeadsResponse,
    MechanicCount, leads,
};
pub use mechanic::{
    MechanicAction, MechanicListing, MechanicRequest, MechanicResponse, MechanicSource, mechanic,
};
pub use page::{Completeness, LegacyPaging, Page, PageLimit};
#[cfg(test)]
pub(crate) use paginate::{CutStep, ServedRows, paginate_rows};
pub use paginate::{PageContext, PageRequest, PageSpec, Produced, RowSink, page_request};
pub use project_control::{
    GitignoreReport, GitignoreStatus, ImportReport, ProjectControlAction, ProjectControlRequest,
    ProjectControlResponse, ProjectControlStatus, project_control,
};
pub use query::{ClippedCell, NonFiniteCell, QueryRequest, QueryResponse, query};
pub use report::{
    CompletenessCount, CompletenessEnvelope, DependencyEdgeTotals, FileClassCount, FileKindCount,
    ReportMechanics, ReportRequest, ReportResponse, SkipReasonCount, report,
};
pub use structure::{StructureRequest, StructureResponse, structure};
pub use tool_counters::ToolCounters;

pub(crate) use paginate::MAX_CURSOR_BYTES;
pub(crate) use sql_predicates::{CREDENTIAL_PATH_PREDICATE, PARSER_FAILURE_PREDICATE};
pub(crate) use tool_limits::{limits_for_root, project_root, resolve_limits};

/// Longest `path` (or path-shaped field, such as `start_after_file`) a tool
/// accepts, checked before the value is used.
pub(crate) const MAX_PATH_BYTES: usize = 32 * 1024;

/// Rejects `value` when its UTF-8 byte length exceeds `max_bytes`.
pub(in crate::tools) fn check_bytes_limit(
    value: &str,
    field_name: &str,
    max_bytes: usize,
) -> Result<(), rmcp::ErrorData> {
    if crate::util::exceeds_input_bytes(value, max_bytes) {
        return Err(crate::error_codes::invalid_params_with_code(
            format!("{field_name} exceeds {max_bytes} bytes"),
            crate::error_codes::ErrorCode::InvalidInput,
        ));
    }
    Ok(())
}

/// Rejects `path` when its UTF-8 byte length exceeds [`MAX_PATH_BYTES`].
pub(in crate::tools) fn check_path_bytes(path: &str) -> Result<(), rmcp::ErrorData> {
    check_bytes_limit(path, "path", MAX_PATH_BYTES)
}

/// Rejects `Some(path)` when its UTF-8 byte length exceeds [`MAX_PATH_BYTES`].
pub(in crate::tools) fn check_optional_path_bytes(
    path: Option<&str>,
) -> Result<(), rmcp::ErrorData> {
    if let Some(value) = path {
        check_path_bytes(value)?;
    }
    Ok(())
}

/// Builds one notice per untested code language in `languages` (prose and
/// tested languages are skipped). Shared by `report`, `structure`, and `leads`.
pub(in crate::tools) fn untested_language_notices<'a>(
    languages: impl IntoIterator<Item = (&'a str, Option<i64>)>,
) -> Vec<String> {
    use crate::parse::{PROSE_LANGUAGE, is_tested_language};

    languages
        .into_iter()
        .filter(|(language, _)| *language != PROSE_LANGUAGE && !is_tested_language(language))
        .map(|(language, files)| {
            let subject = match files {
                Some(1) => "1 file uses".to_owned(),
                Some(files) => format!("{files} files use"),
                None => "uses".to_owned(),
            };
            format!(
                "{language}: {subject} an untested grammar (downloaded on first use). \
                 Only Rust is tested; treat {language} results as best-effort."
            )
        })
        .collect()
}
