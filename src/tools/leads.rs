//! `leads` — unranked mechanical observations about an indexed project.
//!
//! This tool surfaces facts the index already holds — call sites by callee
//! name, diagnostics, comment markers, credential-shaped file paths — and
//! deliberately ranks, scores, orders, and interprets none of them. Which of
//! these is worth pursuing is the calling agent's judgment; see
//! `INSTRUCTIONS` in `src/server.rs` for that contract in the agent's own
//! words.
//!
//! Every row is derived at read time from `evidence` / `files`; nothing is
//! persisted, so there is no risk table and no second source of truth.

use super::context::{ensure_synced, with_verified_read};
use super::leads_query::{count_all, resolve_mechanic, rows_for};
use crate::progress::ProgressSink;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Rows returned when the caller supplies no `limit`.
pub(crate) const DEFAULT_LEADS_ROWS: usize = 100;
/// Hard ceiling on rows per response, mirroring `query`'s `MAX_ROWS`.
pub(crate) const MAX_LEADS_ROWS: usize = 500;
/// Longest `detail` string returned per row. Callee text and comment lines
/// can be arbitrarily long; the caller can always read the file itself.
const MAX_DETAIL_CHARS: usize = 200;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LeadsRequest {
    /// Any path inside the active project — selects which project's database
    /// to read. Must be a path inside an enabled project.
    #[schemars(description = "Any path inside the active project, used to select the database")]
    pub path: String,
    /// Omit to get `mechanic_counts` only. Pass one of the ids from that
    /// response to also get rows for that mechanic.
    #[schemars(description = "Optional mechanic id; omit for counts only")]
    #[serde(default)]
    pub mechanic: Option<String>,
    /// Rows per page, capped at `MAX_LEADS_ROWS`.
    #[schemars(description = "Rows per page (default 100, max 500)")]
    #[serde(default)]
    pub limit: Option<usize>,
    /// Rows to skip, combined with the response's `next_offset`.
    #[schemars(description = "Rows to skip when paging")]
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MechanicCount {
    /// Stable id — pass it back as `mechanic` to get this mechanic's rows.
    pub mechanic: String,
    /// How many observations this mechanic has in the current revision.
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LeadRow {
    /// Project-relative path of the file the observation is in.
    pub file: String,
    /// One-based start line, or `0` when the evidence carries no span.
    pub line: i64,
    /// The mechanic's own text: callee text, diagnostic message, or comment
    /// text. Empty for path-shaped mechanics.
    pub detail: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LeadsResponse {
    /// Revision the counts and rows were read from.
    pub revision_id: String,
    /// Every mechanic this revision can answer, with its count. Counts are
    /// complete regardless of `limit`/`offset`, and carry no ordering by
    /// importance — the order is the fixed declaration order.
    pub mechanic_counts: Vec<MechanicCount>,
    /// Empty when the request supplied no `mechanic`.
    pub rows: Vec<LeadRow>,
    /// True when more rows exist beyond this page for the requested mechanic.
    pub truncated: bool,
    /// Pass as `offset` on the next call while `revision_id` is unchanged.
    pub next_offset: Option<usize>,
}

/// Truncates a detail string to `MAX_DETAIL_CHARS` characters on a char
/// boundary. No shared truncation helper exists in `crate::util`.
fn truncate_detail(value: String) -> String {
    if value.chars().count() <= MAX_DETAIL_CHARS {
        return value;
    }
    value.chars().take(MAX_DETAIL_CHARS).collect()
}

/// Unranked mechanical observations from the project's current revision.
///
/// # Errors
///
/// Returns an error if `request.path` is not inside an active project, if
/// syncing or reading the database fails, if the requested `mechanic` id is
/// not one this tool defines, or if a read races a concurrent publish (the
/// caller retries).
pub fn leads(
    request: &Parameters<LeadsRequest>,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<LeadsResponse>, ErrorData> {
    let synced = ensure_synced(&request.0.path, sink, manager)?;
    let selected = resolve_mechanic(request.0.mechanic.as_deref())?;
    let limit = request
        .0
        .limit
        .unwrap_or(DEFAULT_LEADS_ROWS)
        .min(MAX_LEADS_ROWS);
    let offset = request.0.offset.unwrap_or(0);

    let response = with_verified_read(&synced, |tx| {
        let mechanic_counts = count_all(tx)?;
        let Some(mechanic) = selected else {
            return Ok(Json(LeadsResponse {
                revision_id: synced.revision_id.clone(),
                mechanic_counts,
                rows: Vec::new(),
                truncated: false,
                next_offset: None,
            }));
        };
        let total = mechanic_counts
            .iter()
            .find(|entry| entry.mechanic == mechanic.id)
            .map_or(0, |entry| entry.count);
        let rows: Vec<LeadRow> = rows_for(tx, mechanic, limit, offset)?
            .into_iter()
            .map(|row| LeadRow {
                detail: truncate_detail(row.detail),
                ..row
            })
            .collect();
        let seen = i64::try_from(offset).unwrap_or(i64::MAX)
            + i64::try_from(rows.len()).unwrap_or(i64::MAX);
        let truncated = seen < total;
        Ok(Json(LeadsResponse {
            revision_id: synced.revision_id.clone(),
            mechanic_counts,
            next_offset: truncated.then_some(offset + rows.len()),
            rows,
            truncated,
        }))
    })?;

    tracing::info!(
        revision_id = response.0.revision_id,
        mechanic = ?request.0.mechanic,
        row_count = response.0.rows.len(),
        "leads built"
    );
    Ok(response)
}

#[cfg(test)]
#[path = "leads_tests.rs"]
mod tests;
