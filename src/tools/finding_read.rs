// slugaudit-line-exception: approved-by=slugthug; reason=the finding_read contract: request/response shapes, validation, the session-scoped page read with its count, and the byte cut that clips the first row share one session filter
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
//! response. Rows are ordered by id and resumed with `next_offset` under the
//! shared [`Page`] contract, pinned by `expected_revision` like every other
//! paging tool. The first row of a page is always returned, with its
//! description clipped on a character boundary and flagged, so paging always
//! makes progress and nothing is silently dropped.

use super::context::{
    ensure_synced, guard_expected_revision, internal_error, session_id, with_verified_read,
};
use super::page::{Page, PageLimit};
use super::page_size::{self, PageSize};
use super::resolve_limits;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_FILE_BYTES: usize = 8 * 1024;
const MAX_REVISION_BYTES: usize = 256;

#[derive(Debug, Deserialize, JsonSchema)]
/// Read request for the calling session's findings: the project, and
/// optionally one file within it.
pub struct FindingReadRequest {
    /// Any path inside the active project — used to select the database.
    #[schemars(description = "Any path inside the active project")]
    pub path: String,
    /// Optional project-relative file path. When supplied, only findings
    /// for that file are returned. Omit to return all current-session
    /// findings for the project.
    #[schemars(default)]
    pub file: Option<String>,
    /// Number of findings to skip when paging. Pass the previous page's
    /// `page.next_offset` here; a response carries it only while rows remain.
    #[schemars(description = "Findings to skip when paging; use the response's page.next_offset")]
    #[serde(default)]
    pub offset: usize,
    /// Set to the previous page's `page.revision_id` when resuming. If the
    /// index changed since that page, the call is refused with the
    /// `stale_revision` code (whose remedy restarts from offset 0), with the
    /// same semantics as `leads`, `query` and `structure`.
    #[schemars(description = "Pin the revision a resumed page expects; a mismatch is refused")]
    #[serde(default)]
    pub expected_revision: Option<String>,
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
    /// The shared paging contract for `findings`. `page_bytes` is the
    /// configured `default_page_bytes` (64 KiB unless configured otherwise),
    /// clamped to the hard ceiling `max_query_response_bytes`; there is no
    /// request override. A single row larger than the budget is still
    /// returned, with its description clipped and `detail_clipped` set (or
    /// whole when even an empty description cannot fit), so paging always
    /// advances. Page on with the same `file` filter.
    pub page: Page,
    /// This page's findings, ordered by id.
    pub findings: Vec<FindingReadEntry>,
}

/// # Errors
///
/// Returns an error if `request.path` isn't inside an active project,
/// `offset` exceeds SQLite's supported range, `expected_revision` no longer
/// matches the current revision (`stale_revision`), or syncing or reading
/// the database fails.
pub fn finding_read(
    request: &Parameters<FindingReadRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<FindingReadResponse>, ErrorData> {
    let request = &request.0;
    validate_read_request(request)?;
    let limits = resolve_limits(&request.path)?;
    let size = page_size::resolve(None, &limits)?;

    let synced = ensure_synced(&request.path, sink, manager)?;
    guard_expected_revision(&synced.revision_id, request.expected_revision.as_deref())?;
    let current_session = session_id().to_string();
    let (read, total) = with_verified_read(&synced, |tx| {
        let scope = (current_session.as_str(), request.file.as_deref());
        let read = read_session_findings(tx, scope, request.offset, size.bytes)?;
        Ok((read, count_session_findings(tx, scope)?))
    })?;
    let page = page_of(
        &read,
        total,
        request.offset,
        size,
        synced.revision_id.clone(),
    );
    tracing::info!(
        revision_id = page.revision_id,
        row_count = page.returned,
        total,
        complete = page.complete,
        "finding_read page built"
    );
    Ok(Json(FindingReadResponse {
        page,
        findings: read.findings,
    }))
}

/// The page envelope for one read: `next_offset` and a byte cut only when
/// rows remain past it.
fn page_of(read: &Read, total: i64, offset: usize, size: PageSize, revision_id: String) -> Page {
    let returned = read.findings.len();
    Page {
        returned,
        total: Some(total),
        complete: !read.more,
        next_offset: read.more.then_some(offset.saturating_add(returned)),
        page_bytes: size.bytes,
        bytes_used: read.bytes_used,
        limited_by: if read.more {
            size.byte_cut()
        } else {
            PageLimit::None
        },
        revision_id,
    }
}

/// One byte-budgeted page of findings, its serialized size, and whether rows
/// remain after it.
#[derive(Debug)]
struct Read {
    findings: Vec<FindingReadEntry>,
    bytes_used: usize,
    more: bool,
}

/// Rejects an input that cannot be served before any database work: an
/// oversized `path`/`file`, or an `offset` outside SQLite's integer range.
fn validate_read_request(request: &FindingReadRequest) -> Result<(), ErrorData> {
    if crate::util::exceeds_input_bytes(&request.path, MAX_PATH_BYTES)
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
    if request
        .expected_revision
        .as_deref()
        .is_some_and(|value| crate::util::exceeds_input_bytes(value, MAX_REVISION_BYTES))
    {
        return Err(ErrorData::invalid_params(
            "expected_revision exceeds 256 bytes",
            None,
        ));
    }
    // SQLite treats an integer literal too large for i64 as a REAL, which
    // would page from an unpredictable position instead of failing. This is
    // a contract error in the caller's own arithmetic, so it carries no code.
    if request.offset as u64 > i64::MAX as u64 {
        return Err(ErrorData::invalid_params(
            format!(
                "offset {} exceeds the maximum supported page offset ({}); \
                 page from the start or from a returned next_offset",
                request.offset,
                i64::MAX
            ),
            None,
        ));
    }
    Ok(())
}

/// Which findings a read selects: the calling session's, optionally for one
/// file.
type SessionScope<'a> = (&'a str, Option<&'a str>);

/// Counts every finding the read selects, ignoring the offset and the page
/// cut, in the same transaction as the page.
fn count_session_findings(
    tx: &rusqlite::Transaction<'_>,
    (current_session, file_filter): SessionScope<'_>,
) -> Result<i64, ErrorData> {
    let (filter, query_params) = session_filter(current_session, file_filter);
    tx.query_row(
        &format!("SELECT count(*) FROM findings WHERE {filter}"),
        rusqlite::params_from_iter(query_params.iter()),
        |row| row.get(0),
    )
    .map_err(|error| internal_error("counting the session findings", error))
}

/// Reads at most one byte-budgeted page of the session's findings.
fn read_session_findings(
    tx: &rusqlite::Transaction<'_>,
    (current_session, file_filter): SessionScope<'_>,
    offset: usize,
    page_bytes: usize,
) -> Result<Read, ErrorData> {
    let (filter, query_params) = session_filter(current_session, file_filter);
    // `offset` was validated against i64::MAX above; LIMIT -1 is "no limit"
    // (SQLite requires a LIMIT before OFFSET) and the byte budget cuts the
    // page instead.
    let sql = format!(
        "SELECT id, path, line_start, line_end, severity, category, \
             title, description, status, source_hash \
         FROM findings WHERE {filter} ORDER BY id LIMIT -1 OFFSET {offset}"
    );
    let mut statement = tx
        .prepare(&sql)
        .map_err(|error| internal_error("preparing the session findings read", error))?;
    let rows = statement
        .query_map(
            rusqlite::params_from_iter(query_params.iter()),
            finding_entry,
        )
        .map_err(|error| internal_error("executing the session findings read", error))?;
    collect_page(rows, page_bytes)
}

/// The WHERE clause and bound parameters that select one session's
/// findings, shared by the page read and its count so the two cannot select
/// different rows.
fn session_filter(
    current_session: &str,
    file_filter: Option<&str>,
) -> (&'static str, Vec<rusqlite::types::Value>) {
    let mut query_params = vec![rusqlite::types::Value::Text(current_session.to_owned())];
    let Some(file) = file_filter else {
        return ("session_id = ?1", query_params);
    };
    query_params.push(rusqlite::types::Value::Text(file.to_owned()));
    ("session_id = ?1 AND path = ?2", query_params)
}

/// Maps one findings row to its response entry.
fn finding_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<FindingReadEntry> {
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
        status: row.get(8)?,
        source_hash: row.get(9)?,
    })
}

/// Cuts the byte-budgeted page from the row stream, always keeping the
/// first row so paging makes progress.
fn collect_page(
    rows: impl Iterator<Item = rusqlite::Result<FindingReadEntry>>,
    page_bytes: usize,
) -> Result<Read, ErrorData> {
    let mut findings = Vec::new();
    let mut bytes_used = 0usize;
    let mut more = false;
    for row in rows {
        let mut entry = row.map_err(|error| internal_error("reading a session finding", error))?;
        let mut row_bytes = serialized_len(&entry)?;
        if bytes_used.saturating_add(row_bytes) > page_bytes {
            if !findings.is_empty() {
                more = true;
                break;
            }
            // The first row is always kept so paging makes progress.
            clip_description(&mut entry, page_bytes)?;
            row_bytes = serialized_len(&entry)?;
        }
        bytes_used = bytes_used.saturating_add(row_bytes);
        findings.push(entry);
    }
    Ok(Read {
        findings,
        bytes_used,
        more,
    })
}

/// Clips `description` to the longest character prefix whose serialized row
/// fits `budget`, setting `detail_clipped` only when characters were
/// removed. Restores the whole text when not even an empty description
/// fits, so an oversized row is returned whole rather than silently
/// emptied (its `page.bytes_used` then exceeds `page.page_bytes`, which the
/// response reports as-is).
fn clip_description(entry: &mut FindingReadEntry, budget: usize) -> Result<(), ErrorData> {
    let chars: Vec<char> = entry.description.chars().collect();
    let original = entry.description.clone();
    // The flag is part of the serialized row, so the search must measure the
    // row as it will actually be returned (flag set), never one byte short.
    entry.detail_clipped = true;
    let mut low = 0usize;
    let mut high = chars.len();
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        entry.description = chars[..mid].iter().collect();
        if serialized_len(entry)? <= budget {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    entry.description = chars[..low].iter().collect();
    if low == chars.len() || (low == 0 && serialized_len(entry)? > budget) {
        entry.description = original;
        entry.detail_clipped = false;
    }
    Ok(())
}

fn serialized_len(entry: &FindingReadEntry) -> Result<usize, ErrorData> {
    crate::util::serialized_len(entry)
        .map_err(|error| internal_error("measuring a session finding's response size", error))
}

#[cfg(test)]
#[path = "finding_read_tests.rs"]
mod tests;
