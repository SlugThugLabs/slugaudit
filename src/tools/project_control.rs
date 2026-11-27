// slugaudit-line-exception: approved-by=split-judge; reason=Single-concern project_control MCP tool schema, input validation, and paired on/off project lifecycle coordination.
//! Project enable/disable control (`project_control` with `action = "on"` / `"off"`).
//!
//! A project is on iff `.planning/slugaudit/` exists. `on` creates it and runs
//! the initial import through [`sync::SourceSyncManager`]; `off` releases
//! in-memory session state and deletes `.planning/slugaudit/` (plus `.planning`
//! when empty).

use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::project::{self, ProjectRoot};
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PROFILE_BYTES: usize = 64;

/// Request for the `project_control` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectControlRequest {
    /// Project root path. Defaults to the current directory when omitted.
    #[schemars(default)]
    pub path: Option<String>,
    /// Action to perform: `"on"` enables the project, `"off"` disables it.
    pub action: ProjectControlAction,
    /// Optional audit profile for this session: `"audit"`, `"lean"`, or
    /// `"adaptive"`. Held in memory until the server exits.
    #[schemars(default)]
    pub profile: Option<String>,
}

/// The action to take — `on` enables, `off` disables.
#[derive(Debug, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum ProjectControlAction {
    /// Create `.planning/slugaudit`, build the database, and run the initial import.
    #[serde(rename = "on")]
    On,
    /// Delete `.planning/slugaudit` (and `.planning` when empty), clearing session state.
    #[serde(rename = "off")]
    Off,
}

/// Response from the `project_control` tool.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectControlResponse {
    /// One of `"enabled"`, `"disabled"`, or `"already_disabled"` (no-op).
    pub status: ProjectControlStatus,
    /// Absolute path of the project root that was affected.
    pub path: String,
    /// Absolute paths `"off"` removed. Empty when nothing existed to remove.
    #[schemars(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// Present when action was `"on"` and an initial import ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import: Option<ImportReport>,
    /// What `"on"` did about the project-root `.gitignore`. Absent for `"off"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gitignore: Option<GitignoreReport>,
    /// Present only when `"on"` found a corrupt or outdated project database
    /// and discarded it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_discard: Option<sync::DatabaseDiscard>,
}

/// What enabling a project did about its root `.gitignore`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GitignoreReport {
    /// One of `"added"`, `"already_ignored"`, or `"not_written"`.
    pub status: GitignoreStatus,
    /// Absolute path of the project-root `.gitignore` that was inspected.
    pub path: String,
    /// Why nothing was written (`"not_written"` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Outcome of the `.gitignore` guard, as a closed set.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitignoreStatus {
    /// The ignore rule was appended to the project-root `.gitignore`.
    Added,
    /// An equivalent rule was already present; the file was left untouched.
    AlreadyIgnored,
    /// Nothing was written and the project is enabled anyway.
    NotWritten,
}

/// Outcome of an enable/disable call, as a closed set.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectControlStatus {
    /// The project was enabled by this call.
    Enabled,
    /// The project was disabled by this call.
    Disabled,
    /// The project was already disabled; nothing was removed.
    AlreadyDisabled,
}

/// Summary of an initial import that ran on enable.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportReport {
    /// Files discovered on disk that had no row in the previous revision.
    pub added: usize,
    /// Files whose content hash differs from the previous revision.
    pub modified: usize,
    /// Files the previous revision indexed that are no longer on disk.
    pub deleted: usize,
    /// Files whose content hash matches the previous revision.
    pub unchanged: usize,
    /// Files discovery found but could not include (oversize, unreadable, non-UTF-8).
    pub skipped: usize,
    /// The `rev-N` id of the revision this import committed.
    pub revision_id: String,
}

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

fn err(msg: impl Into<String>) -> ErrorData {
    ErrorData::internal_error(msg.into(), None)
}

/// Enables or disables SlugAudit for a project through `manager`.
///
/// # Errors
///
/// `invalid_params` when `path` or `profile` exceeds its byte cap, `profile`
/// is unknown, or `path` is not an existing directory; an internal error when
/// enabling, importing, or disabling fails.
pub fn project_control(
    request: &Parameters<ProjectControlRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<ProjectControlResponse>, ErrorData> {
    let inner = &request.0;
    let mut path_str = ".";
    if let Some(path) = inner.path.as_deref() {
        path_str = path;
    }
    super::check_path_bytes(path_str)?;
    if let Some(value) = inner.profile.as_deref() {
        super::check_bytes_limit(value, "profile", MAX_PROFILE_BYTES)?;
    }
    let profile = inner
        .profile
        .as_deref()
        .map(|value| {
            crate::model::AuditProfile::parse_str(value).ok_or_else(|| {
                invalid_params_with_code(
                    format!("unknown profile `{value}`; use adaptive, audit or lean"),
                    ErrorCode::InvalidInput,
                )
            })
        })
        .transpose()?;
    let root = resolve_root(path_str, std::path::Path::new(path_str))?;

    match inner.action {
        ProjectControlAction::On => enable(&root, profile, sink, manager),
        ProjectControlAction::Off => disable(&root, manager),
    }
}

fn resolve_root(path_str: &str, path: &std::path::Path) -> Result<ProjectRoot, ErrorData> {
    ProjectRoot::resolve(path).map_err(|error| match error {
        project::RootError::NotADirectory => invalid_params_with_code(
            format!("project path `{path_str}` is not a directory; pass a project directory"),
            ErrorCode::InvalidInput,
        ),
        project::RootError::Unresolvable(source)
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            invalid_params_with_code(
                format!("project path `{path_str}` does not exist; pass an existing directory"),
                ErrorCode::InvalidInput,
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
    manager: &sync::SourceSyncManager,
) -> Result<Json<ProjectControlResponse>, ErrorData> {
    project::enable(root).map_err(|e| err(format!("enable: {e}")))?;
    let gitignore = gitignore_report(project::ensure_gitignore(root));
    project::begin_process_session(root).map_err(|e| err(format!("enable: {e}")))?;
    if let Some(profile) = profile_override {
        project::set_session_profile(root.as_path(), profile)?;
    }
    let (report, discard) = manager.sync_enabled_project(root, sink)?;
    Ok(Json(ProjectControlResponse {
        status: ProjectControlStatus::Enabled,
        path: root.as_path().to_string_lossy().into_owned(),
        removed: Vec::new(),
        import: Some(ImportReport {
            added: report.added,
            modified: report.modified,
            deleted: report.deleted,
            unchanged: report.unchanged,
            skipped: report.skipped.len(),
            revision_id: report.revision_id,
        }),
        gitignore: Some(gitignore),
        database_discard: discard,
    }))
}

fn disable(
    root: &ProjectRoot,
    manager: &sync::SourceSyncManager,
) -> Result<Json<ProjectControlResponse>, ErrorData> {
    manager.release_disabled_project(root)?;
    manager.session_mechanics().clear_project(root.as_path())?;
    project::clear_session_profile(root.as_path())?;
    let report = project::disable(root).map_err(|e| err(format!("disable: {e}")))?;
    let status = if report.removed.is_empty() {
        ProjectControlStatus::AlreadyDisabled
    } else {
        ProjectControlStatus::Disabled
    };
    Ok(Json(ProjectControlResponse {
        status,
        path: root.as_path().to_string_lossy().into_owned(),
        removed: report
            .removed
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        import: None,
        gitignore: None,
        database_discard: None,
    }))
}

#[cfg(test)]
#[path = "project_control_session_tests.rs"]
mod session_tests;

#[cfg(test)]
#[path = "project_control_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "project_control_gitignore_tests.rs"]
mod gitignore_tests;

#[cfg(test)]
#[path = "project_control_activation_tests.rs"]
mod activation_tests;
