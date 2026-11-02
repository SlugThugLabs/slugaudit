// slugaudit-line-exception: approved-by=slugthug; reason=Recombined project_control tool request/response contract and execution handlers into cohesive module.
//! Project enable/disable control. Exposed as an MCP tool so the calling
//! AI can drive activation: `project_control` with `action = "on"` / `"off"`.
//!
//! A project is on iff the directory `.planning/slugaudit/` exists. `on`
//! creates it, then the database, then runs the initial import. `off` deletes
//! the whole directory (databases and everything else in it), then
//! `.planning` if that left it empty. If the database is missing or
//! broken while the directory exists, the next tool call rebuilds it.

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
    /// Optional audit profile for this session: `"audit"`, `"lean"`, or
    /// `"adaptive"`. Held in memory until the server exits.
    #[schemars(default)]
    pub profile: Option<String>,
}

/// The action to take — on enables, off disables.
#[derive(Debug, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum ProjectControlAction {
    /// Create the directory `.planning/slugaudit` (the on switch), then the
    /// project database, and run the initial import, so the first evidence
    /// call finds a fresh index.
    #[serde(rename = "on")]
    On,
    /// Delete the whole `.planning/slugaudit` directory, then `.planning` if
    /// that left it empty. Tool calls on the
    /// path then return not-enabled until `on` runs again. A project with no
    /// such directory is a no-op reported as `"already_disabled"`.
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
    /// Absolute paths `"off"` removed. Empty when nothing existed to remove.
    #[schemars(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// Present when action was `"on"` and an initial import ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import: Option<ImportReport>,
    /// What `"on"` did about the project-root `.gitignore`. Absent for
    /// `"off"`, which never touches that file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gitignore: Option<GitignoreReport>,
    /// Present only when `"on"` found a corrupt or outdated project database
    /// and discarded it, and why. Session findings live in their own database
    /// and are not affected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_discard: Option<sync::DatabaseDiscard>,
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
    /// The ignore rule was appended to the project-root `.gitignore`.
    Added,
    /// An equivalent rule was already present; the file was left untouched.
    AlreadyIgnored,
    /// Nothing was written and the project is enabled anyway; the
    /// report's `error` field says why.
    NotWritten,
}

/// Summary of an initial import that ran on enable.
///
/// The counts describe the project's discovered file set, not only the
/// files that produced indexed content: a file skipped as oversize or
/// unreadable still counts in `added`, and `skipped` reports how many such
/// files the import could not include.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportReport {
    /// Files discovered on disk that had no row in the previous revision —
    /// including files the import then skipped (they are counted again in
    /// `skipped`). `added + modified + unchanged` is the size of the
    /// current discovered file set.
    pub added: usize,
    /// Files whose content hash differs from the previous revision.
    pub modified: usize,
    /// Files the previous revision indexed but that are no longer on disk.
    pub deleted: usize,
    /// Files whose content hash matches the previous revision.
    pub unchanged: usize,
    /// Files discovery found but could not include — oversize, unreadable,
    /// or a path that is not valid UTF-8 — so this import holds no content
    /// for them. A non-zero value means the import is incomplete; `report`'s
    /// `completeness` carries the per-reason breakdown after the import.
    pub skipped: usize,
    /// The `rev-N` id of the revision this import committed.
    pub revision_id: String,
}

const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_PROFILE_BYTES: usize = 64;

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

/// Wraps any error into a user-friendly `ErrorData` message.
fn err(msg: impl Into<String>) -> ErrorData {
    ErrorData::internal_error(msg.into(), None)
}

/// Enables or disables SlugAudit for a project.
///
/// When `action` is `"on"`, creates the activation directory and immediately
/// runs the initial import so the AI's first tool call finds fresh evidence.
///
/// When `action` is `"off"`, deletes the whole `.planning/slugaudit`
/// directory (then `.planning` if empty). Returns `already_disabled` if the
/// directory did not exist.
///
/// # Errors
///
/// `invalid_params` when the path exceeds 32768 bytes, the profile exceeds 64
/// bytes or is unknown, or the path cannot be resolved to a directory. An
/// internal error when the project database cannot be opened, when enabling
/// or disabling the project fails, or when the initial import fails.
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
    // project must work *before* the activation directory exists, so there
    // is no active project to find yet.
    let root = resolve_root(&path_str, &path)?;

    match inner.action {
        ProjectControlAction::On => enable(&root, profile, sink, manager),
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
    manager: Option<&sync::SourceSyncManager>,
) -> Result<Json<ProjectControlResponse>, ErrorData> {
    project::enable(root).map_err(|e| err(format!("enable: {e}")))?;
    // Right after the directory itself: the guard must be in place even if
    // the initial import below fails, and it never fails this call.
    let gitignore = gitignore_report(project::ensure_gitignore(root));
    project::begin_process_session(root).map_err(|e| err(format!("enable: {e}")))?;
    if let Some(profile) = profile_override {
        // A session setting: held in memory for this project until the
        // process exits, and applied by every limit derivation below.
        project::set_session_profile(root.as_path(), profile);
    }
    let db_path = project::database_path(root);
    // A previous `off` removed `results/` under any store this process still
    // holds for the directory; drop it so the next result recreates it. Not
    // done on `off`: a result opened before it must stay readable.
    if let (Some(m), Some(runtime_dir)) = (manager, db_path.parent()) {
        m.result_stores().release_dir(runtime_dir);
    }
    let mut discard = None;
    let mut connection = match store::open_read_write(&db_path) {
        Ok(conn) => conn,
        Err(error) if error.is_corruption() => {
            let notice = sync::DatabaseDiscard {
                cause: format!("the project database is corrupt or outdated: {error}"),
            };
            tracing::warn!(
                operation = "enable_project",
                path = %db_path.display(),
                error = %error,
                "database is corrupt or outdated; discarding and re-importing from scratch",
            );
            discard = Some(notice);
            recover_corrupt_database(&db_path)?
        }
        Err(e) => return Err(err(format!("database: {e}"))),
    };
    let report = sync::publish(&mut connection, root.as_path(), parse::PACK_VERSION, sink)
        .map_err(|e| err(format!("import: {e}")))?;
    Ok(Json(ProjectControlResponse {
        status: "enabled".into(),
        path: root.as_path().to_string_lossy().to_string(),
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
/// SlugAudit knows nothing about, and only `on` has a reason to add one.
/// Every response from here therefore omits the `gitignore` field.
fn disable(
    root: &ProjectRoot,
    _manager: Option<&sync::SourceSyncManager>,
) -> Result<Json<ProjectControlResponse>, ErrorData> {
    let report = project::disable(root).map_err(|e| err(format!("disable: {e}")))?;
    // Nothing removed means there was no activation directory: already off.
    let status = if report.removed.is_empty() {
        "already_disabled"
    } else {
        "disabled"
    };
    Ok(Json(ProjectControlResponse {
        status: status.into(),
        path: root.as_path().to_string_lossy().to_string(),
        removed: report
            .removed
            .iter()
            .map(|path| path.to_string_lossy().to_string())
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
#[path = "project_control_symlink_tests.rs"]
mod symlink_tests;

#[cfg(test)]
#[path = "project_control_activation_tests.rs"]
mod activation_tests;

#[cfg(test)]
#[path = "project_control_log_tests.rs"]
mod log_tests;
