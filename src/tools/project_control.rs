//! Project enable/disable control. Exposed as an MCP tool so the TUI can
//! drive activation via `/slugaudit on` / `/slugaudit off`.
//!
//! `on` creates the activation directory and runs the initial import.
//! `off` removes the activation directory and purges the project database.

use crate::parse;
use crate::project::{self, ProjectRoot};
use crate::{store, sync};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Request for the `project_control` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectControlRequest {
    /// Project root path. Defaults to the current directory when omitted.
    #[schemars(default)]
    pub path: Option<String>,
    /// Action to perform: `"on"` enables the project, `"off"` disables it.
    pub action: ProjectControlAction,
}

/// The action to take — on enables, off disables.
#[derive(Debug, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum ProjectControlAction {
    #[serde(rename = "on")]
    On,
    #[serde(rename = "off")]
    Off,
}

/// Response from the `project_control` tool.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectControlResponse {
    /// One of `"enabled"`, `"disabled"`, or `"already_disabled"` (no-op).
    pub status: String,
    /// Absolute path of the project root that was affected.
    pub path: String,
    /// Present when action was `"on"` and an initial import ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import: Option<ImportReport>,
}

/// Summary of an initial import that ran on enable.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportReport {
    pub added: usize,
    pub unchanged: usize,
    pub revision_id: String,
}

/// Wraps any error into a user-friendly `ErrorData` message.
fn err(msg: impl Into<String>) -> ErrorData {
    ErrorData::internal_error(msg.into(), None)
}

/// Enables or disables SlugAudit for a project.
///
/// When `action` is `"on"`, creates the activation directory and immediately
/// runs the initial import so the AI's first tool call finds fresh evidence.
///
/// When `action` is `"off"`, removes the activation directory and purges the
/// project's database, evidence, and findings. Returns `already_disabled` if
/// the project was already off.
pub fn project_control(
    request: &Parameters<ProjectControlRequest>,
    sink: &dyn crate::progress::ProgressSink,
) -> Result<Json<ProjectControlResponse>, ErrorData> {
    let inner = &request.0;
    let path_str = inner.path.as_deref().unwrap_or(".").to_string();
    let path = std::path::PathBuf::from(&path_str);
    let root = ProjectRoot::resolve(&path).map_err(|e| err(format!("resolve: {e}")))?;

    match inner.action {
        ProjectControlAction::On => enable(&root, sink),
        ProjectControlAction::Off => disable(&root),
    }
}

fn enable(
    root: &ProjectRoot,
    sink: &dyn crate::progress::ProgressSink,
) -> Result<Json<ProjectControlResponse>, ErrorData> {
    project::enable(root).map_err(|e| err(format!("enable: {e}")))?;
    let db_path = project::database_path(root);
    let mut connection =
        store::open_read_write(&db_path).map_err(|e| err(format!("database: {e}")))?;
    let report = sync::publish(&mut connection, root.as_path(), parse::PACK_VERSION, sink)
        .map_err(|e| err(format!("import: {e}")))?;
    Ok(Json(ProjectControlResponse {
        status: "enabled".into(),
        path: root.as_path().to_string_lossy().to_string(),
        import: Some(ImportReport {
            added: report.added,
            unchanged: report.unchanged,
            revision_id: report.revision_id,
        }),
    }))
}

fn disable(root: &ProjectRoot) -> Result<Json<ProjectControlResponse>, ErrorData> {
    let activation = project::activation_dir(root);
    if !activation.exists() {
        return Ok(Json(ProjectControlResponse {
            status: "already_disabled".into(),
            path: root.as_path().to_string_lossy().to_string(),
            import: None,
        }));
    }
    project::disable(root).map_err(|e| err(format!("disable: {e}")))?;
    Ok(Json(ProjectControlResponse {
        status: "disabled".into(),
        path: root.as_path().to_string_lossy().to_string(),
        import: None,
    }))
}

#[cfg(test)]
#[path = "project_control_tests.rs"]
mod tests;
