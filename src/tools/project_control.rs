//! Project enable/disable control. Exposed as an MCP tool so the calling
//! AI can drive activation: `project_control` with `action = "on"` / `"off"`.
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

const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_PROFILE_BYTES: usize = 64;

/// Request for the `project_control` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectControlRequest {
    /// Project root path. Defaults to the current directory when omitted.
    #[schemars(default)]
    pub path: Option<String>,
    /// Action to perform: `"on"` enables the project, `"off"` disables it.
    pub action: ProjectControlAction,
    /// Optional audit profile to configure: `"audit"`, `"lean"`, or `"adaptive"`.
    #[schemars(default)]
    pub profile: Option<String>,
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
    /// What `"on"` did about the project-root `.gitignore`. Absent for
    /// `"off"`, which never touches that file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gitignore: Option<GitignoreReport>,
}

/// What enabling a project did about its root `.gitignore`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GitignoreReport {
    /// One of `"added"` (the ignore rule was appended), `"already_ignored"`
    /// (an equivalent rule was present; file untouched), or `"not_written"`
    /// (the file was left untouched and the project is enabled anyway).
    pub status: GitignoreStatus,
    /// Absolute path of the project-root `.gitignore` that was inspected.
    pub path: String,
    /// Why nothing was written. Present only for `"not_written"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Outcome of the `.gitignore` guard, as a closed set.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitignoreStatus {
    Added,
    AlreadyIgnored,
    NotWritten,
}

/// Renders an activation outcome as the response's machine-readable field.
fn gitignore_report(outcome: project::GitignoreOutcome) -> GitignoreReport {
    let (status, path, error) = match outcome {
        project::GitignoreOutcome::Added(path) => (GitignoreStatus::Added, path, None),
        project::GitignoreOutcome::AlreadyIgnored(path) => {
            (GitignoreStatus::AlreadyIgnored, path, None)
        }
        project::GitignoreOutcome::NotWritten { path, reason } => {
            (GitignoreStatus::NotWritten, path, Some(reason))
        }
    };
    GitignoreReport {
        status,
        path: path.to_string_lossy().into_owned(),
        error,
    }
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
    manager: Option<&sync::SourceSyncManager>,
) -> Result<Json<ProjectControlResponse>, ErrorData> {
    let inner = &request.0;
    let path_str = inner.path.as_deref().unwrap_or(".").to_string();
    if crate::util::exceeds_input_bytes(&path_str, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params("path exceeds 32768 bytes", None));
    }
    if inner
        .profile
        .as_deref()
        .is_some_and(|value| crate::util::exceeds_input_bytes(value, MAX_PROFILE_BYTES))
    {
        return Err(ErrorData::invalid_params("profile exceeds 64 bytes", None));
    }
    // Refused before any side effect: an unknown profile must not enable the
    // project on the default profile while the response says it applied.
    let profile = inner
        .profile
        .as_deref()
        .map(|value| {
            crate::model::AuditProfile::parse_str(value).ok_or_else(|| {
                ErrorData::invalid_params(
                    format!("unknown profile `{value}`; use adaptive, audit or lean"),
                    None,
                )
            })
        })
        .transpose()?;
    let path = std::path::PathBuf::from(&path_str);
    // `ProjectRoot::resolve`, not `project::resolve_project`: enabling a
    // project must work *before* the activation marker exists, so there
    // is no active project to find yet.
    let root = resolve_root(&path_str, &path)?;

    match inner.action {
        ProjectControlAction::On => enable(&root, profile, sink),
        ProjectControlAction::Off => disable(&root, manager),
    }
}

/// Resolves the caller's project path, separating a bad argument from a
/// server-side failure.
///
/// A missing path or a path that is not a directory will never resolve by
/// retrying, so those are `invalid_params` naming the caller's path and the
/// fix. Any other resolution failure (an unreadable parent, an I/O error)
/// stays an internal error: the argument may be fine and the environment is
/// the problem.
fn resolve_root(path_str: &str, path: &std::path::Path) -> Result<ProjectRoot, ErrorData> {
    ProjectRoot::resolve(path).map_err(|error| match error {
        project::RootError::NotADirectory => ErrorData::invalid_params(
            format!("project path `{path_str}` is not a directory; pass a project directory"),
            None,
        ),
        project::RootError::Unresolvable(source)
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            ErrorData::invalid_params(
                format!("project path `{path_str}` does not exist; pass an existing directory"),
                None,
            )
        }
        project::RootError::Unresolvable(source) => {
            err(format!("resolve: failed to resolve project root: {source}"))
        }
    })
}

fn enable(
    root: &ProjectRoot,
    profile_override: Option<crate::model::AuditProfile>,
    sink: &dyn crate::progress::ProgressSink,
) -> Result<Json<ProjectControlResponse>, ErrorData> {
    project::enable(root).map_err(|e| err(format!("enable: {e}")))?;
    // Right after the directory itself: the guard must be in place even if
    // the initial import below fails, and it never fails this call.
    let gitignore = gitignore_report(project::ensure_gitignore(root));
    if let Some(profile) = profile_override {
        let conf_path = root
            .as_path()
            .join(".planning")
            .join("slugaudit")
            .join("config.json");
        let cfg = serde_json::json!({ "profile": profile });
        // `{:#}` renders a `Value` through `Display` with the same pretty
        // formatter `to_string_pretty` uses, but without a fallible call:
        // serializing a `Value` cannot fail, so there is no error arm to
        // leave untested.
        let encoded = format!("{cfg:#}");
        std::fs::write(&conf_path, encoded)
            .map_err(|e| err(format!("write profile config {}: {e}", conf_path.display())))?;
    }
    let db_path = project::database_path(root);
    let mut connection = match store::open_read_write(&db_path) {
        Ok(conn) => conn,
        Err(error) if error.is_corruption() => recover_corrupt_database(&db_path)?,
        Err(e) => return Err(err(format!("database: {e}"))),
    };
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
        gitignore: Some(gitignore),
    }))
}

/// Discards a database SQLite reported as corrupt or outdated, then recreates
/// it. Kept separate from the open that detects the corruption so both
/// failure arms — the discard and the reopen — can be driven with real
/// filesystem fixtures rather than a production-only hook.
fn recover_corrupt_database(path: &std::path::Path) -> Result<rusqlite::Connection, ErrorData> {
    store::discard_corrupt_database(path)
        .map_err(|e| err(format!("discarding outdated database: {e}")))?;
    store::open_read_write(path).map_err(|e| err(format!("database: {e}")))
}

/// `off` never touches `.gitignore` — removing a rule can unhide paths
/// SlugAudit knows nothing about, and the backlog item only asks for `on`.
/// Every response from here therefore omits the `gitignore` field.
fn disable(
    root: &ProjectRoot,
    manager: Option<&sync::SourceSyncManager>,
) -> Result<Json<ProjectControlResponse>, ErrorData> {
    let activation = project::activation_dir(root);
    // `try_exists`: a stat error (permissions, I/O) is not "already disabled".
    let present = activation
        .try_exists()
        .map_err(|e| err(format!("inspect {}: {e}", activation.display())))?;
    if !present {
        return Ok(Json(ProjectControlResponse {
            status: "already_disabled".into(),
            path: root.as_path().to_string_lossy().to_string(),
            import: None,
            gitignore: None,
        }));
    }
    if let Some(m) = manager {
        m.unwatch(root.as_path());
    }
    project::disable(root).map_err(|e| err(format!("disable: {e}")))?;
    Ok(Json(ProjectControlResponse {
        status: "disabled".into(),
        path: root.as_path().to_string_lossy().to_string(),
        import: None,
        gitignore: None,
    }))
}

#[cfg(test)]
#[path = "project_control_tests.rs"]
mod tests;
