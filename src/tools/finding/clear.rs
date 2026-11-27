//! `finding_clear` MCP tool: deletes every finding this SlugAudit process
//! holds for a project.

use super::store::with_findings_write;
use crate::sync;
use crate::tools::check_path_bytes;
use crate::tools::context::{ensure_synced, internal_error};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    /// Present only when this call discarded and rebuilt the project database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_discard: Option<crate::sync::DatabaseDiscard>,
}

/// Deletes every session finding for the active project containing `request.path`.
///
/// # Errors
///
/// Returns `invalid_params` if `path` exceeds `MAX_PATH_BYTES` or is not an
/// active project, or an internal error if syncing or database access fails.
pub fn finding_clear(
    request: &Parameters<FindingClearRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<FindingClearResponse>, ErrorData> {
    let request = &request.0;
    check_path_bytes(&request.path)?;
    let synced = ensure_synced(&request.path, sink, manager)?;
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
