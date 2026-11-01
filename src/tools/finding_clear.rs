//! `finding_clear` MCP tool: deletes every finding this SlugAudit process
//! holds for a project. It exists so a human can tell the agent to start
//! clean after a model switch inside one session; a restart of the server
//! already starts clean.

use super::context::{ensure_synced, internal_error};
use super::finding_store::with_findings_write;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PATH_BYTES: usize = 32 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
/// Request to delete this session's findings for a project.
pub struct FindingClearRequest {
    /// Any path inside the active project — used to select the database.
    #[schemars(description = "Any path inside the active project, used to select the database")]
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Result of a `finding_clear` call.
pub struct FindingClearResponse {
    /// How many findings were deleted; 0 when the session had none.
    pub removed: u64,
    /// Present only when this call discarded and rebuilt the project
    /// database (corruption, version bump, root mismatch), and why. Session
    /// findings live in their own database and are not affected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_discard: Option<crate::sync::DatabaseDiscard>,
}

/// # Errors
///
/// Returns an error if `path` is oversized or not an active project, or if
/// syncing or the findings database access fails.
pub fn finding_clear(
    request: &Parameters<FindingClearRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<FindingClearResponse>, ErrorData> {
    let request = &request.0;
    if crate::util::exceeds_input_bytes(&request.path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params(
            "path exceeds its input size limit",
            None,
        ));
    }
    let synced = ensure_synced(&request.path, sink, manager)?;
    // No findings file means nothing was written this session: nothing to
    // delete, and nothing to create.
    let removed = if synced
        .findings_path()?
        .try_exists()
        .map_err(|error| internal_error("looking for the findings database", error))?
    {
        with_findings_write(&synced, |tx| {
            tx.execute("DELETE FROM findings", [])
                .map_err(|error| internal_error("deleting the session findings", error))
        })?
    } else {
        0
    };
    tracing::info!(removed, "finding_clear completed");
    Ok(Json(FindingClearResponse {
        removed: removed as u64,
        database_discard: synced.discard,
    }))
}

#[cfg(test)]
#[path = "finding_clear_tests.rs"]
mod tests;
