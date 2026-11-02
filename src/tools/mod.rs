//! Tool handlers orchestrate: they resolve the active project, ensure it's
//! synced, and format a response. They own no parsing, SQL, or storage
//! logic themselves — that lives in `sync`, `store`, and `evidence`.
//!
//! This module also owns the **tool-level call counters** (defined in
//! `tool_counters`): process-wide `AtomicU64` counts of total calls, error
//! calls, and total elapsed wall-clock time across every tool. The `health`
//! tool exposes these counters for observability without changing anything
//! about how the other tools execute.

pub(crate) mod context;
mod finding;
mod finding_clear;
mod finding_insert;
mod finding_read;
mod finding_store;
mod finding_validate;
mod health;
mod health_phase;
mod health_result_store;
mod input_caps;
mod language_notices;
mod leads;
mod leads_compact;
mod leads_covers;
mod leads_custom;
mod leads_mechanics;
mod leads_page;
mod leads_query;
mod mechanic;
mod mechanic_catalog;
mod mechanic_register;
mod mechanic_scan;
mod mechanic_scan_cached;
mod page;
mod page_size;
mod paginate;
mod project_control;
mod query;
mod query_clip;
mod query_exec;
mod query_value;

mod report;
mod report_completeness;
mod report_sections;
mod sql_predicates;
mod structure;
mod structure_exec;
mod structure_page;
mod structure_source;
mod tool_counters;
mod tool_limits;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "discard_surface_tests.rs"]
mod discard_surface_tests;

#[cfg(test)]
#[path = "redaction_tests.rs"]
mod redaction_tests;

#[cfg(test)]
mod golden_phase9_support;

#[cfg(test)]
#[path = "golden_phase9_leads_tests.rs"]
mod golden_phase9_leads_tests;

#[cfg(test)]
#[path = "golden_phase9_mechanic_tests.rs"]
mod golden_phase9_mechanic_tests;

#[cfg(test)]
#[path = "golden_phase9_query_tests.rs"]
mod golden_phase9_query_tests;

#[cfg(test)]
#[path = "golden_phase9_structure_tests.rs"]
mod golden_phase9_structure_tests;

#[cfg(test)]
#[path = "golden_phase9_finding_read_tests.rs"]
mod golden_phase9_finding_read_tests;

pub use context::{
    PageContext, PageRequest, PageSpec, Produced, RowSink, Served, decode_rows, ensure_synced,
    page_request, paginate,
};
pub use finding::{FindingRequest, FindingResponse, finding};
pub use finding_clear::{FindingClearRequest, FindingClearResponse, finding_clear};
pub use finding_read::{FindingReadEntry, FindingReadRequest, FindingReadResponse, finding_read};
pub use health::{HealthRequest, HealthResponse, health};
pub use leads::{
    CompactLeadRow, FileCount, LeadMode, LeadRow, LeadRows, LeadsRequest, LeadsResponse,
    MechanicCount, leads,
};
pub use mechanic::{
    InvalidMechanic, MechanicAction, MechanicListing, MechanicRequest, MechanicResponse,
    MechanicSource, mechanic,
};
pub use page::{Completeness, LegacyPaging, Page, PageLimit};
pub use project_control::{
    GitignoreReport, GitignoreStatus, ImportReport, ProjectControlAction, ProjectControlRequest,
    ProjectControlResponse, project_control,
};
pub use query::{QueryRequest, QueryResponse, query};
pub use query_clip::ClippedCell;
pub use query_value::NonFiniteCell;
pub use report::DependencyEdgeTotals;
pub use report::{ReportMechanics, ReportRequest, ReportResponse, report};
pub use report_completeness::{
    CompletenessCount, CompletenessEnvelope, FileClassCount, FileKindCount, SkipReasonCount,
};
pub use structure::{StructureRequest, StructureResponse, structure};
pub use tool_counters::ToolCounters;

pub(crate) use input_caps::{MAX_CURSOR_BYTES, MAX_PATH_BYTES};
pub(crate) use sql_predicates::{CREDENTIAL_PATH_PREDICATE, PARSER_FAILURE_PREDICATE};
pub(crate) use tool_limits::{
    limits_for_root, process_limits_for_tool, project_root, resolve_limits,
};

// `ToolCounters` is deliberately not tested here. Its three counters are
// process-wide `AtomicU64`s that any test in this binary may bump and
// there is no reset (adding one would be a production test hook), so a
// zero-initial-state assertion cannot be pinned in the shared test
// process. The snapshot/record delta contract is covered by
// `tool_counters_record_bumps_call_and_total_counts`,
// `tool_counters_record_bumps_error_count_on_error` and
// `tool_counters_always_return_non_decreasing_after_a_bump` in
// `src/tools/health_tests.rs`.
