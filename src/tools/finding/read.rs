//! `finding_read` MCP tool: streams this session's findings with live
//! `current` / `stale` hash status, byte-cut and cursor-paged via `paginate_rows`.

use super::MAX_FILE_BYTES;
use super::store::{HashLookup, findings_path_of};
use super::stored::{StoredFinding, cut_finding_row};
use crate::store::StoredRow;
use crate::sync;
use crate::tools::context::internal_error;
use crate::tools::page::{Completeness, LegacyPaging, Page, PageSize};
use crate::tools::paginate::{
    PageRequest, PageSpec, Produced, RowSink, ServedRows, page_request, paginate_rows,
};
use crate::tools::{check_bytes_limit, check_optional_path_bytes, resolve_limits};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const TOOL: &str = "finding_read";

#[derive(Debug, Deserialize, JsonSchema)]
/// Read request for this session's findings: the project, and optionally one
/// file within it; or the `cursor` of a page already read.
pub struct FindingReadRequest {
    /// Any path inside the active project — used to select the database.
    /// Required unless `cursor` is given.
    #[schemars(description = "Any path inside the active project; required without a cursor")]
    #[serde(default)]
    pub path: Option<String>,
    /// Optional project-relative file path filter.
    #[schemars(default)]
    pub file: Option<String>,
    /// The previous page's `page.cursor`.
    #[schemars(
        description = "The previous page's `page.cursor`; continues that stored result and ignores `path` and `file`"
    )]
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size in serialized JSON bytes of `findings`.
    #[schemars(default)]
    #[serde(default)]
    pub page_bytes: Option<usize>,
    /// Removed `offset` / `expected_revision` parameters, accepted only to refuse them.
    #[serde(flatten, default)]
    #[schemars(skip)]
    pub legacy_paging: LegacyPaging,
}

/// One finding row of this session.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FindingReadEntry {
    /// Row id of the finding in this session's findings.
    pub id: i64,
    /// Project-relative path of the file the finding is about.
    pub path: String,
    /// One-based, inclusive line where the finding starts.
    pub line_start: u32,
    /// One-based, inclusive line where the finding ends.
    pub line_end: u32,
    /// Severity text as supplied by the calling agent.
    pub severity: String,
    /// Category text as supplied by the calling agent.
    pub category: String,
    /// One-line title as supplied by the calling agent.
    pub title: String,
    /// Description text as supplied by the calling agent; possibly clipped.
    pub description: String,
    /// True when `description` was clipped on row 0 to fit `page_bytes`.
    pub detail_clipped: bool,
    /// `current` while the file's hash matches `source_hash`; `stale` otherwise.
    pub status: String,
    /// Content hash of the file at the revision the finding was written against.
    pub source_hash: String,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One page of this session's findings, read against the current revision.
pub struct FindingReadResponse {
    /// Shared paging metadata for `findings`.
    pub page: Page,
    /// This page's findings, ordered by id.
    pub findings: Vec<FindingReadEntry>,
}

/// Reads one byte-budgeted page of session findings.
///
/// # Errors
///
/// Returns `invalid_params` on removed paging fields, invalid `page_bytes`,
/// oversized `path`/`file`/`cursor`, or unknown/expired cursors, or an internal
/// error on database failures.
pub fn finding_read(
    request: &Parameters<FindingReadRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<FindingReadResponse>, ErrorData> {
    let request = &request.0;
    request.legacy_paging.refuse()?;
    check_optional_path_bytes(request.path.as_deref())?;
    if let Some(file) = request.file.as_deref() {
        check_bytes_limit(file, "file", MAX_FILE_BYTES)?;
    }
    let page_request = page_request(request.cursor.as_deref(), request.path.as_deref(), sink)?;
    let mut spec = PageSpec::typed::<StoredFinding>(TOOL, 0);
    spec.requested_page_bytes = request.page_bytes;
    match &page_request {
        PageRequest::Fresh { path, .. } => {
            let limits = resolve_limits(path)?;
            spec.page_bytes = PageSize::resolve(request.page_bytes, &limits)?.bytes;
            spec.ceiling = limits.max_query_response_bytes;
            spec.store_limits = Some(crate::store::ResultStoreLimits::from(&limits));
        }
        PageRequest::Cursor(_) => {
            PageSize::refuse_zero(request.page_bytes)?;
        }
    }
    let file_filter = request.file.as_deref();

    let served = paginate_rows::<(), FindingReadEntry, _, _>(
        manager,
        &spec,
        page_request,
        |tx, rows| {
            produce(tx, file_filter, rows)?;
            Ok(Produced {
                completeness: Completeness::Complete,
                extra: (),
            })
        },
        |ctx| {
            let budget = ctx.page_bytes;
            Ok(move |row: &StoredRow, taken: usize, bytes_so_far: usize| {
                cut_finding_row(row, taken, bytes_so_far, budget)
            })
        },
    )?;
    Ok(respond(served))
}

fn respond(served: ServedRows<FindingReadEntry, ()>) -> Json<FindingReadResponse> {
    let findings = served.rows;
    let page = served.page;
    tracing::info!(
        revision_id = page.revision_id,
        row_count = page.returned,
        total = page.total,
        complete = page.complete,
        "finding_read page built"
    );
    Json(FindingReadResponse { page, findings })
}

fn produce(
    project_tx: &rusqlite::Transaction<'_>,
    file_filter: Option<&str>,
    rows: &mut RowSink<'_, '_>,
) -> Result<(), ErrorData> {
    let Some(findings) = crate::store::open_findings_existing(&findings_path_of(project_tx)?)
        .map_err(|error| internal_error("opening the findings database", error))?
    else {
        return Ok(());
    };
    let mut lookup = HashLookup::new(project_tx)?;
    let (filter, query_params) = match file_filter {
        None => ("", Vec::new()),
        Some(file) => (
            " WHERE path = ?1",
            vec![rusqlite::types::Value::Text(file.to_owned())],
        ),
    };
    let sql = format!(
        "SELECT id, path, line_start, line_end, severity, category, \
             title, description, source_hash \
         FROM findings{filter} ORDER BY id"
    );
    let mut statement = findings
        .prepare(&sql)
        .map_err(|error| internal_error("preparing the session findings read", error))?;
    let mapped = statement
        .query_map(rusqlite::params_from_iter(query_params.iter()), |row| {
            Ok(FindingReadEntry {
                id: row.get(0)?,
                path: row.get(1)?,
                line_start: row.get(2)?,
                line_end: row.get(3)?,
                severity: row.get(4)?,
                category: row.get(5)?,
                title: row.get(6)?,
                description: row.get(7)?,
                detail_clipped: false,
                status: String::new(),
                source_hash: row.get(8)?,
            })
        })
        .map_err(|error| internal_error("executing the session findings read", error))?;
    for item in mapped {
        let mut entry = item.map_err(|error| internal_error("reading a session finding", error))?;
        let status = if lookup.is_current(&entry.path, &entry.source_hash)? {
            "current"
        } else {
            "stale"
        };
        status.clone_into(&mut entry.status);
        rows.push(&StoredFinding { entry })?;
    }
    Ok(())
}
