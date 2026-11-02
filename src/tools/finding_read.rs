//! `finding_read` MCP tool — returns this SlugAudit process's findings,
//! read from this process's findings file, each with the status it has against the freshly
//! synced index: `current` while the file still has the hash the finding was
//! written against, `stale` otherwise.
//!
//! A page is cut by serialized JSON bytes, not by a row count: a finding
//! carries up to 10,000 characters of agent-authored description, so a
//! session that wrote many findings would otherwise produce an unbounded
//! response. Rows are ordered by id and stored as one immutable result when
//! the first page is read; later pages are served from that stored result by
//! `cursor` under the shared [`Page`] contract, so a finding recorded between
//! pages never changes the answer being read. The first row of a page is always returned, with its
//! description clipped on a character boundary and flagged, so paging always
//! makes progress and nothing is silently dropped.

use super::page::{LegacyPaging, Page, PageLimit};
use super::page_size::{self, PageSize};
use super::paginate::{
    PageRequest, PageSpec, Produced, Served, decode_rows, page_request, paginate,
};
use super::resolve_limits;
use crate::store::StoredRow;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rows::{FindingExtra, PageCut, StoredFinding, clip_description, produce, serialized_len};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_FILE_BYTES: usize = 8 * 1024;
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
    /// Optional project-relative file path. When supplied, only findings
    /// for that file are returned. Omit to return all of this session's
    /// findings for the project.
    #[schemars(default)]
    pub file: Option<String>,
    /// The previous page's `page.cursor`. The call then serves the next page
    /// of the stored result and ignores `path` and `file`.
    #[schemars(
        description = "The previous page's `page.cursor`; continues that stored result and ignores `path` and `file`"
    )]
    #[serde(default)]
    pub cursor: Option<String>,
    /// The `offset` / `expected_revision` parameters, accepted only to refuse them.
    #[serde(flatten, default)]
    #[schemars(skip)]
    pub legacy_paging: LegacyPaging,
}

/// One finding row of this session. Every field is exactly what the
/// `finding` tool stored — SlugAudit never generates any of this text.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FindingReadEntry {
    /// Row id of the finding in this session's findings; a restated finding
    /// keeps its id.
    pub id: i64,
    /// Project-relative path of the file the finding is about.
    pub path: String,
    /// One-based, inclusive line where the finding starts.
    pub line_start: u32,
    /// One-based, inclusive line where the finding ends.
    pub line_end: u32,
    /// Severity text exactly as the calling agent supplied it; SlugAudit
    /// defines no scale and adds no judgment.
    pub severity: String,
    /// Category text exactly as the calling agent supplied it; SlugAudit
    /// defines no taxonomy.
    pub category: String,
    /// One-line title exactly as the calling agent supplied it.
    pub title: String,
    /// Description text exactly as the calling agent supplied it; possibly
    /// clipped — see `detail_clipped`.
    pub description: String,
    /// True when `description` was clipped to fit this page's byte budget:
    /// the returned text is then a character-boundary prefix of the stored
    /// description. False means the whole stored text is present.
    pub detail_clipped: bool,
    /// `current` while the file's content hash still equals `source_hash`;
    /// `stale` once the file has a different hash or is gone.
    pub status: String,
    /// Content hash of the file at the revision the finding was written
    /// against; a different hash now makes the finding `stale`.
    pub source_hash: String,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One page of this session's findings, read against the current revision.
pub struct FindingReadResponse {
    /// Paging for `findings`; `total` counts this session's findings under
    /// the `file` filter, as they were when the first page was read.
    /// `page_bytes` is `default_page_bytes` clamped to the ceiling (no request
    /// override). Page on by repeating the call with `page.cursor`.
    pub page: Page,
    /// This page's findings, ordered by id.
    pub findings: Vec<FindingReadEntry>,
}

/// # Errors
///
/// Returns an error if the request carries `offset` or
/// `expected_revision` (`paging_parameter_removed`), has neither `cursor`
/// nor `path`, `path` isn't inside an active project, the `cursor` is
/// unknown or expired, or syncing, storing or reading the result fails.
pub fn finding_read(
    request: &Parameters<FindingReadRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<FindingReadResponse>, ErrorData> {
    let request = &request.0;
    request.legacy_paging.refuse()?;
    validate_read_request(request)?;
    let page_request = page_request(request.cursor.as_deref(), request.path.as_deref(), sink)?;
    // Only a new request names a project, so only it reads limits; a
    // continuation uses the budget and ceiling flag stored with the result.
    let size = match &page_request {
        PageRequest::Fresh { path, .. } => Some(page_size::resolve(None, &resolve_limits(path)?)?),
        PageRequest::Cursor(_) => None,
    };
    let spec = PageSpec::typed::<StoredFinding>(TOOL, size.map_or(0, |size| size.bytes));
    let file_filter = request.file.as_deref();

    let mut cut = PageCut::new(0);
    let served = paginate::<FindingExtra, _, _>(
        manager,
        &spec,
        page_request,
        |tx, rows| {
            produce(tx, file_filter, rows)?;
            Ok(Produced {
                completeness: super::page::Completeness::Complete,
                extra: FindingExtra {
                    at_ceiling: size.is_some_and(|size| size.at_ceiling),
                },
            })
        },
        |ctx| {
            cut = PageCut::new(ctx.page_bytes);
            |row: &StoredRow| cut.accept(row)
        },
    )?;
    cut.finish()?;
    respond(served)
}

/// Builds the response from the served page: decodes the findings, clips the
/// first one when it alone exceeds the page budget, measures the bytes used,
/// and names what limited the page.
fn respond(served: Served<FindingExtra>) -> Result<Json<FindingReadResponse>, ErrorData> {
    let budget = served.page.page_bytes;
    let mut findings: Vec<FindingReadEntry> = decode_rows::<StoredFinding>(&served.rows)?
        .into_iter()
        .map(|stored| stored.entry)
        .collect();
    if let Some(first) = findings.first_mut()
        && serialized_len(first)? > budget
    {
        clip_description(first, budget)?;
    }
    let mut bytes_used = 0usize;
    for entry in &findings {
        bytes_used = bytes_used.saturating_add(serialized_len(entry)?);
    }
    let mut page = served.page;
    page.bytes_used = bytes_used;
    page.limited_by = if page.complete {
        PageLimit::None
    } else {
        PageSize {
            bytes: budget,
            at_ceiling: served.meta.at_ceiling,
        }
        .byte_cut()
    };
    tracing::info!(
        revision_id = page.revision_id,
        row_count = page.returned,
        total = page.total,
        complete = page.complete,
        "finding_read page built"
    );
    Ok(Json(FindingReadResponse { page, findings }))
}

/// Rejects an input that cannot be served before any database work: an
/// oversized `path` or `file`.
fn validate_read_request(request: &FindingReadRequest) -> Result<(), ErrorData> {
    if request
        .path
        .as_deref()
        .is_some_and(|value| crate::util::exceeds_input_bytes(value, MAX_PATH_BYTES))
        || request
            .file
            .as_deref()
            .is_some_and(|value| crate::util::exceeds_input_bytes(value, MAX_FILE_BYTES))
    {
        return Err(ErrorData::invalid_params(
            "path or file exceeds its input size limit",
            None,
        ));
    }
    Ok(())
}

#[path = "finding_read_rows.rs"]
mod rows;

#[cfg(test)]
#[path = "finding_read_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "finding_read_paged_tests.rs"]
mod paged_tests;
