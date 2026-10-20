//! The `project_control` tool's request/response contract: the action the
//! caller asks for, what enabling reports about the initial import, and
//! what it did about the project-root `.gitignore`.
//!
//! Split from `project_control.rs` so the schema types and the
//! side-effecting handler each stay under the source-size ceiling. Field
//! docs here become the MCP schema descriptions.

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
    /// Create the activation marker (`.planning/slugaudit`) and run the
    /// initial import, so the first evidence call finds a fresh index.
    #[serde(rename = "on")]
    On,
    /// Remove the activation marker and purge the project database,
    /// evidence, and findings. An already-disabled project is a no-op
    /// reported as `"already_disabled"`.
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
    /// The `rev-N` id of the revision this import committed. Pass it back
    /// as `expected_revision` when resuming a paged read.
    pub revision_id: String,
}
