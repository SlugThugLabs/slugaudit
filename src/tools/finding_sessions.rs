//! `finding_sessions` MCP tool: lists the earlier sessions whose findings
//! are still in the project database, and adopts one of them into the
//! calling session.
//!
//! A restart is a new session, and a new session starts with a clean
//! `finding_read` view: the first sync of each process marks every other
//! session's findings `stale` and deletes nothing. The server cannot tell a
//! continuing audit from a new one, so the agent resumes deliberately here:
//! `list` shows what earlier sessions left (id, first-finding time, count)
//! and `adopt` re-tags one session's findings to the current session.

use super::context::{ensure_synced, internal_error, session_id};
use super::context::{with_verified_read, with_verified_write};
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::{Transaction, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_SESSION_ID_BYTES: usize = 64;
/// Most sessions one `list` returns, newest first; `total_sessions` is exact.
const MAX_SESSIONS: usize = 100;

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
/// What to do with earlier sessions' findings.
pub enum FindingSessionsAction {
    /// List earlier sessions that still have findings in the database.
    List,
    /// Re-tag one earlier session's findings to the current session.
    Adopt,
}

#[derive(Debug, Deserialize, JsonSchema)]
/// Request to list or adopt earlier sessions' findings.
pub struct FindingSessionsRequest {
    /// Any path inside the active project — used to select the database.
    #[schemars(description = "Any path inside the active project, used to select the database")]
    pub path: String,
    /// `list` earlier sessions, or `adopt` one of them.
    #[schemars(description = "`list` earlier sessions, or `adopt` one by `session_id`")]
    pub action: FindingSessionsAction,
    /// The session to adopt, as returned by `list`; required for `adopt`,
    /// refused for `list`.
    #[schemars(description = "Session id from `list`; required for `adopt`, refused for `list`")]
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One earlier session that still has findings in the database.
pub struct PriorSession {
    /// The session's id, exactly as stored on its findings.
    pub session_id: String,
    /// Unix seconds of the session's earliest finding; the findings table
    /// stores no separate session start.
    pub started_at_unix: i64,
    /// How many findings the session still has, current or stale.
    pub findings: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Result of a `finding_sessions` call.
pub struct FindingSessionsResponse {
    /// For `list`: earlier sessions, newest first, at most 100; empty for
    /// `adopt`.
    pub sessions: Vec<PriorSession>,
    /// For `list`: the exact number of earlier sessions, which may exceed
    /// `sessions.len()`; 0 for `adopt`.
    pub total_sessions: u64,
    /// For `adopt`: how many findings now belong to the current session; null
    /// for `list`. Adopted findings read `current` when their file still
    /// has the content hash they were written against, else `stale`.
    pub adopted: Option<u64>,
}

/// # Errors
///
/// Returns an error if `path` is oversized or not an active project,
/// `session_id` is missing for `adopt`, given for `list`, oversized or the
/// current session's own id, or syncing or the database access fails.
pub fn finding_sessions(
    request: &Parameters<FindingSessionsRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<FindingSessionsResponse>, ErrorData> {
    let request = &request.0;
    let target = validate(request)?;
    let synced = ensure_synced(&request.path, sink, manager)?;
    let current = session_id().to_string();
    let response = match target {
        None => with_verified_read(&synced, |tx| list_prior_sessions(tx, &current))?,
        Some(target) => with_verified_write(&synced, |tx| adopt_session(tx, &current, target))?,
    };
    tracing::info!(
        action = ?request.action,
        listed = response.sessions.len(),
        total_sessions = response.total_sessions,
        adopted = response.adopted,
        "finding_sessions completed"
    );
    Ok(Json(response))
}

/// Checks the request shape and returns the session to adopt, if any.
fn validate(request: &FindingSessionsRequest) -> Result<Option<&str>, ErrorData> {
    if crate::util::exceeds_input_bytes(&request.path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params(
            "path exceeds its input size limit",
            None,
        ));
    }
    match (request.action, request.session_id.as_deref()) {
        (FindingSessionsAction::List, None) => Ok(None),
        (FindingSessionsAction::List, Some(_)) => Err(ErrorData::invalid_params(
            "session_id is only valid with action `adopt`",
            None,
        )),
        (FindingSessionsAction::Adopt, None) => Err(ErrorData::invalid_params(
            "action `adopt` requires session_id; call `list` first",
            None,
        )),
        (FindingSessionsAction::Adopt, Some(target)) => {
            if target.is_empty() || crate::util::exceeds_input_bytes(target, MAX_SESSION_ID_BYTES) {
                return Err(ErrorData::invalid_params(
                    "session_id must be 1 to 64 bytes",
                    None,
                ));
            }
            Ok(Some(target))
        }
    }
}

fn list_prior_sessions(
    tx: &Transaction<'_>,
    current: &str,
) -> Result<FindingSessionsResponse, ErrorData> {
    let mut statement = tx
        .prepare(
            "SELECT session_id, min(created_at_unix), count(*) FROM findings \
             WHERE session_id != ?1 GROUP BY session_id \
             ORDER BY min(created_at_unix) DESC, session_id LIMIT ?2",
        )
        .map_err(|error| internal_error("preparing the prior-session list", error))?;
    let sessions = statement
        .query_map(params![current, MAX_SESSIONS as i64], |row| {
            Ok(PriorSession {
                session_id: row.get(0)?,
                started_at_unix: row.get(1)?,
                findings: row.get(2)?,
            })
        })
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|error| internal_error("reading the prior sessions", error))?;
    let total_sessions: u64 = tx
        .query_row(
            "SELECT count(DISTINCT session_id) FROM findings WHERE session_id != ?1",
            [current],
            |row| row.get(0),
        )
        .map_err(|error| internal_error("counting the prior sessions", error))?;
    Ok(FindingSessionsResponse {
        sessions,
        total_sessions,
        adopted: None,
    })
}

fn adopt_session(
    tx: &Transaction<'_>,
    current: &str,
    target: &str,
) -> Result<FindingSessionsResponse, ErrorData> {
    if target == current {
        return Err(ErrorData::invalid_params(
            "session_id is the current session; its findings are already yours",
            None,
        ));
    }
    let adopted = tx
        .execute(
            "UPDATE findings SET session_id = ?1, status = CASE WHEN EXISTS ( \
                 SELECT 1 FROM files \
                 WHERE files.path = findings.path AND files.content_hash = findings.source_hash \
             ) THEN 'current' ELSE 'stale' END \
             WHERE session_id = ?2",
            params![current, target],
        )
        .map_err(|error| internal_error("adopting the session's findings", error))?;
    Ok(FindingSessionsResponse {
        sessions: Vec::new(),
        total_sessions: 0,
        adopted: Some(adopted as u64),
    })
}

#[cfg(test)]
#[path = "finding_sessions_tests.rs"]
mod tests;
