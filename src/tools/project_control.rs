// slugaudit-line-exception: approved-by=slugthug; reason=Recombined project_control tool request/response contract and execution handlers into cohesive module.
//! Project enable/disable control. Exposed as an MCP tool so the calling
//! AI can drive activation: `project_control` with `action = "on"` / `"off"`.
//!
//! A project is active iff `.planning/slugaudit/project.db` exists. `on`
//! creates the activation directory, then the database, then runs the initial
//! import. `off` removes the database, its sidecars, the `results` directory
//! and the config lock, keeps `config.json` and any other file, and removes
//! the directory only when it is empty afterwards.

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
    /// Optional audit profile to configure: `"audit"`, `"lean"`, or `"adaptive"`.
    #[schemars(default)]
    pub profile: Option<String>,
}

/// The action to take — on enables, off disables.
#[derive(Debug, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum ProjectControlAction {
    /// Create `.planning/slugaudit`, then the project database (the
    /// activation marker), and run the initial import, so the first evidence
    /// call finds a fresh index. Persisted settings in `config.json` apply.
    #[serde(rename = "on")]
    On,
    /// Remove the project database (the activation marker), its SQLite
    /// sidecars, the `results` directory and the config lock. `config.json`
    /// and any other file in `.planning/slugaudit` are kept, and the
    /// directory is removed only when nothing is left in it. Tool calls on
    /// the path then return not-enabled until `on` runs again. A project
    /// with nothing to remove is a no-op reported as `"already_disabled"`.
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
    /// True when `"off"` found a `config.json` and kept it.
    #[schemars(default)]
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub config_kept: bool,
    /// Present when action was `"on"` and an initial import ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import: Option<ImportReport>,
    /// What `"on"` did about the project-root `.gitignore`. Absent for
    /// `"off"`, which never touches that file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gitignore: Option<GitignoreReport>,
    /// Present only when `"on"` found a corrupt or outdated project database
    /// and discarded it: how many findings were lost (null = unknown, the
    /// database was unreadable) and why.
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

/// Maps a failed profile-config update to the tool error. A file the caller
/// must fix — a symlinked or non-regular `config.json`, one that is not a
/// JSON object, or one that would outgrow the read cap — is `invalid_params`;
/// an I/O failure, a lock held past the bounded wait, or a directory sync
/// that failed after the replace is operational (`internal_error`).
fn config_write_error(error: project::ConfigWriteError) -> ErrorData {
    match &error {
        project::ConfigWriteError::NotRegularFile { .. }
        | project::ConfigWriteError::Malformed { .. }
        | project::ConfigWriteError::NotAnObject { .. }
        | project::ConfigWriteError::TooLarge { .. } => {
            ErrorData::invalid_params(format!("write profile config: {error}"), None)
        }
        project::ConfigWriteError::Locked { .. }
        | project::ConfigWriteError::Io { .. }
        | project::ConfigWriteError::NotDurable { .. } => {
            err(format!("write profile config: {error}"))
        }
    }
}

/// The name `config.json` stores for a profile — the serde name
/// `ProjectConfig` reads back.
fn profile_name(profile: crate::model::AuditProfile) -> &'static str {
    match profile {
        crate::model::AuditProfile::Lean => "lean",
        crate::model::AuditProfile::Audit => "audit",
        crate::model::AuditProfile::Adaptive => "adaptive",
    }
}

/// Enables or disables SlugAudit for a project.
///
/// When `action` is `"on"`, creates the activation directory and immediately
/// runs the initial import so the AI's first tool call finds fresh evidence.
///
/// When `action` is `"off"`, removes the project database, its sidecars, the
/// `results` directory and the config lock, keeping `config.json` and any
/// other user file. Returns `already_disabled` if there was nothing to remove.
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
    if let Some(profile) = profile_override {
        // Sets `profile` only: every other key (limits, persisted mechanics)
        // is kept by the shared locked, atomic read-modify-write.
        project::ProjectConfig::update_raw(root.as_path(), |config| {
            config.insert(
                "profile".to_owned(),
                serde_json::Value::from(profile_name(profile)),
            );
            Ok::<(), project::ConfigWriteError>(())
        })
        .map_err(config_write_error)?;
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
            let (notice, count_error) = sync::DatabaseDiscard::assess_file(
                &db_path,
                format!("the project database is corrupt or outdated: {error}"),
            );
            tracing::warn!(
                operation = "enable_project",
                database_id = %crate::util::redacted_path(&db_path),
                error = %sync::scrub_path(&error.to_string(), &db_path),
                findings_discarded = ?notice.findings_discarded,
                count_error = count_error
                    .as_ref()
                    .map(|error| sync::scrub_path(&error.to_string(), &db_path)),
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
        config_kept: false,
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
/// SlugAudit knows nothing about, and the backlog item only asks for `on`.
/// Every response from here therefore omits the `gitignore` field.
fn disable(
    root: &ProjectRoot,
    manager: Option<&sync::SourceSyncManager>,
) -> Result<Json<ProjectControlResponse>, ErrorData> {
    // Unwatch only once the project is actually off: a failed (busy) disable
    // leaves the project active, and it must keep its watcher.
    let report = project::disable(root).map_err(|e| err(format!("disable: {e}")))?;
    if let Some(m) = manager {
        m.unwatch(root.as_path());
    }
    // Nothing removed means the project was already off (no database, no
    // results directory, no lock), whether or not `config.json` is still there.
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
        config_kept: report.config_kept,
        import: None,
        gitignore: None,
        database_discard: None,
    }))
}

#[cfg(test)]
#[path = "project_control_config_tests.rs"]
mod config_write_tests;

#[cfg(test)]
#[path = "project_control_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "project_control_symlink_tests.rs"]
mod symlink_tests;
