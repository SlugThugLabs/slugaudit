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
//! response. Rows are ordered by id and resumed with `next_offset`. The
//! first row of a page is always returned, with its description clipped on
//! a character boundary and flagged, so paging always makes progress and
//! nothing is silently dropped.

use super::context::{ensure_synced, internal_error, session_id, with_verified_read};
use super::resolve_limits;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_FILE_BYTES: usize = 8 * 1024;

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
    /// `next_offset` here; a response carries `next_offset` only while rows
    /// remain.
    #[schemars(description = "Findings to skip when paging; use the response's next_offset")]
    #[serde(default)]
    pub offset: usize,
}

/// One finding row, scoped to the current agent session. Every field is
/// exactly what the `finding` tool stored — SlugAudit never generates
/// any of this text.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FindingReadEntry {
    pub id: i64,
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub severity: String,
    pub category: String,
    pub title: String,
    pub description: String,
    /// True when `description` was clipped to fit this page's byte budget:
    /// the returned text is then a character-boundary prefix of the stored
    /// description. False means the whole stored text is present.
    pub detail_clipped: bool,
    pub status: String,
    pub source_hash: String,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One page of the current session's findings, read against the current
/// revision.
pub struct FindingReadResponse {
    pub revision_id: String,
    /// This page's findings, ordered by id.
    pub findings: Vec<FindingReadEntry>,
    /// Serialized-JSON bytes of `findings`, measured per row exactly as
    /// `query` measures its rows.
    pub findings_bytes: usize,
    /// The byte budget applied to `findings` this call: the configured
    /// `default_page_bytes` (64 KiB unless configured otherwise), clamped to
    /// the hard ceiling `max_query_response_bytes`. A single row larger than
    /// the budget is still returned, with its description clipped and
    /// `detail_clipped` set (or whole when even an empty description cannot
    /// fit), so paging always advances.
    pub page_bytes: usize,
    /// Pass as `offset` on the next call, with the same `file` filter, while
    /// this is present. Absent when this page reaches the end of the scope.
    pub next_offset: Option<usize>,
}

/// # Errors
///
/// Returns an error if `request.path` isn't inside an active project,
/// `offset` exceeds SQLite's supported range, or syncing or reading the
/// database fails.
pub fn finding_read(
    request: &Parameters<FindingReadRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<FindingReadResponse>, ErrorData> {
    let request = &request.0;
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
    let limits = resolve_limits(&request.path)?;
    let page_bytes = limits
        .default_page_bytes
        .min(limits.max_query_response_bytes);

    let synced = ensure_synced(&request.path, sink, manager)?;
    let revision_id = synced.revision_id.clone();
    let current_session = session_id().to_string();
    let (findings, findings_bytes, more) = with_verified_read(&synced, |tx| {
        read_session_findings(
            tx,
            &current_session,
            request.file.as_deref(),
            request.offset,
            page_bytes,
        )
    })?;
    let next_offset = more.then_some(request.offset.saturating_add(findings.len()));

    Ok(Json(FindingReadResponse {
        revision_id,
        findings,
        findings_bytes,
        page_bytes,
        next_offset,
    }))
}

/// Reads at most one byte-budgeted page of the session's findings. Returns
/// the page, its serialized-JSON size, and whether rows remain after it.
fn read_session_findings(
    tx: &rusqlite::Transaction<'_>,
    current_session: &str,
    file_filter: Option<&str>,
    offset: usize,
    page_bytes: usize,
) -> Result<(Vec<FindingReadEntry>, usize, bool), ErrorData> {
    let mut sql = String::from(
        "SELECT id, path, line_start, line_end, severity, category, \
             title, description, status, source_hash \
         FROM findings WHERE session_id = ?1",
    );
    let mut query_params = vec![rusqlite::types::Value::Text(current_session.to_owned())];
    if let Some(file) = file_filter {
        sql.push_str(" AND path = ?2");
        query_params.push(rusqlite::types::Value::Text(file.to_owned()));
    }
    // `offset` was validated against i64::MAX above; LIMIT -1 is "no limit"
    // (SQLite requires a LIMIT before OFFSET) and the byte budget cuts the
    // page instead.
    sql.push_str(&format!(" ORDER BY id LIMIT -1 OFFSET {offset}"));

    let mut statement = tx
        .prepare(&sql)
        .map_err(|error| internal_error("preparing the session findings read", error))?;
    let rows = statement
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
                status: row.get(8)?,
                source_hash: row.get(9)?,
            })
        })
        .map_err(|error| internal_error("executing the session findings read", error))?;

    let mut findings = Vec::new();
    let mut findings_bytes = 0usize;
    let mut more = false;
    for row in rows {
        let mut entry = row.map_err(|error| internal_error("reading a session finding", error))?;
        let mut row_bytes = serialized_len(&entry)?;
        if findings_bytes.saturating_add(row_bytes) > page_bytes {
            if !findings.is_empty() {
                more = true;
                break;
            }
            // The first row is always kept so paging makes progress.
            clip_description(&mut entry, page_bytes)?;
            row_bytes = serialized_len(&entry)?;
        }
        findings_bytes = findings_bytes.saturating_add(row_bytes);
        findings.push(entry);
    }
    Ok((findings, findings_bytes, more))
}

/// Clips `description` to the longest character prefix whose serialized row
/// fits `budget`, setting `detail_clipped` only when characters were
/// removed. Restores the whole text when not even an empty description
/// fits, so an oversized row is returned whole rather than silently
/// emptied (its `findings_bytes` then exceeds `page_bytes`, which the
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
    serde_json::to_vec(entry)
        .map(|bytes| bytes.len())
        .map_err(|error| internal_error("measuring a session finding's response size", error))
}

#[cfg(test)]
#[path = "finding_read_tests.rs"]
mod tests;
