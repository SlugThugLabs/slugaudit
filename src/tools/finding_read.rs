//! `finding_read` MCP tool — returns findings scoped to the current
//! agent session. Unlike raw `query` access to the `findings` table
//! (which returns rows from every session), `finding_read` gates on the
//! current `session_id` — a new agent session sees only its own
//! conclusions. This is the safe, curated view; `query` remains the
//! unconstrained "I know what I'm doing" access path.
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

use super::context::session_id;
use super::page::{LegacyPaging, Page, PageLimit};
use super::page_size;
use super::paginate::{PageRequest, PageSpec, Produced, decode_rows, page_request, paginate_with};
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
/// Read request for the calling session's findings: the project, and
/// optionally one file within it; or the `cursor` of a page already read.
pub struct FindingReadRequest {
    /// Any path inside the active project — used to select the database.
    /// Required unless `cursor` is given.
    #[schemars(description = "Any path inside the active project; required without a cursor")]
    #[serde(default)]
    pub path: Option<String>,
    /// Optional project-relative file path. When supplied, only findings
    /// for that file are returned. Omit to return all current-session
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
    /// The removed `offset` / `expected_revision`, kept only to refuse them.
    #[serde(flatten, default)]
    #[schemars(skip)]
    pub legacy_paging: LegacyPaging,
}

/// One finding row, scoped to the current agent session. Every field is
/// exactly what the `finding` tool stored — SlugAudit never generates
/// any of this text.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FindingReadEntry {
    /// Row id of the finding in this project's `findings` table; a restated
    /// finding keeps its id.
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
    /// `stale` once a sync sees a different hash.
    pub status: String,
    /// Content hash of the file at the revision the finding was written
    /// against; a sync that sees a different hash marks the finding `stale`.
    pub source_hash: String,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One page of the current session's findings, read against the current
/// revision.
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
/// Returns an error if the request carries the removed `offset` or
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
    let mut spec = PageSpec::typed::<StoredFinding>(TOOL, size.map_or(0, |size| size.bytes));
    spec.offer_first_row = true;
    let current_session = session_id().to_string();
    let scope = (current_session.as_str(), request.file.as_deref());

    let mut cut = PageCut::new(0);
    let served = paginate_with::<FindingExtra, _, _>(
        manager,
        &spec,
        page_request,
        |tx, rows| {
            produce(tx, scope, rows)?;
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
    } else if served.meta.at_ceiling {
        PageLimit::Ceiling
    } else {
        PageLimit::PageBytes
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
