//! `finding_read` MCP tool — returns findings scoped to the current
//! agent session. Unlike raw `query` access to the `findings` table
//! (which returns rows from every session), `finding_read` gates on the
//! current `session_id` — a new agent session sees only its own
//! conclusions. This is the safe, curated view; `query` remains the
//! unconstrained "I know what I'm doing" access path.

use super::context::{ensure_synced, session_id, with_verified_read};
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindingReadRequest {
    /// Any path inside the active project — used to select the database.
    #[schemars(description = "Any path inside the active project")]
    pub path: String,
    /// Optional project-relative file path. When supplied, only findings
    /// for that file are returned. Omit to return all current-session
    /// findings for the project.
    #[schemars(default)]
    pub file: Option<String>,
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
    pub status: String,
    pub source_hash: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FindingReadResponse {
    pub revision_id: String,
    pub findings: Vec<FindingReadEntry>,
}

/// # Errors
///
/// Returns an error if `request.path` isn't inside an active project,
/// or if syncing or reading the database fails.
pub fn finding_read(
    request: &Parameters<FindingReadRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<FindingReadResponse>, ErrorData> {
    let request = &request.0;
    let synced = ensure_synced(&request.path, sink, manager)?;
    let revision_id = synced.revision_id.clone();
    let current_session = session_id().to_string();

    let findings = with_verified_read(&synced, |tx| {
        read_session_findings(tx, &current_session, request.file.as_deref())
    })?;

    Ok(Json(FindingReadResponse {
        revision_id,
        findings,
    }))
}

fn read_session_findings(
    tx: &rusqlite::Transaction<'_>,
    current_session: &str,
    file_filter: Option<&str>,
) -> Result<Vec<FindingReadEntry>, ErrorData> {
    let mut sql = String::from(
        "SELECT id, path, line_start, line_end, severity, category, \
             title, description, status, source_hash \
         FROM findings WHERE session_id = ?1",
    );
    let mut query_params: Vec<String> = vec![current_session.to_owned()];
    if let Some(file) = file_filter {
        sql.push_str(" AND path = ?2");
        query_params.push(file.to_owned());
    }
    sql.push_str(" ORDER BY id");

    let params = rusqlite::params_from_iter(query_params.iter());

    let mut statement = tx
        .prepare(&sql)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let rows = statement
        .query_map(params, |row| {
            Ok(FindingReadEntry {
                id: row.get(0)?,
                path: row.get(1)?,
                line_start: row.get(2)?,
                line_end: row.get(3)?,
                severity: row.get(4)?,
                category: row.get(5)?,
                title: row.get(6)?,
                description: row.get(7)?,
                status: row.get(8)?,
                source_hash: row.get(9)?,
            })
        })
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

    let mut findings = Vec::new();
    for row in rows {
        findings.push(row.map_err(|error| ErrorData::internal_error(error.to_string(), None))?);
    }
    Ok(findings)
}

#[cfg(test)]
#[path = "finding_read_tests.rs"]
mod tests;
