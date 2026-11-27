//! The resource limits a tool call runs under: the project's effective
//! limits for a request path, the call deadline helper, and the tool error
//! mapping when resolving limits fails.

use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::model::ResourceLimits;
use rmcp::ErrorData;
use std::time::{Duration, Instant};

/// Resolves the effective resource limits for the active project containing `path`.
pub(crate) fn resolve_limits(path: &str) -> Result<ResourceLimits, ErrorData> {
    limits_for_root(&project_root(path)?)
}

/// Resolves the canonical active project root `path` lies in, returning the
/// same `project_not_enabled` error `ensure_synced` returns when inactive.
pub(crate) fn project_root(path: &str) -> Result<std::path::PathBuf, ErrorData> {
    crate::project::find_project_root(std::path::Path::new(path))
        .map(|root| root.as_path().to_path_buf())
        .map_err(|error| crate::sync::map_activation_error(&error))
}

/// Resolves the effective limits for the project at `root`.
pub(crate) fn limits_for_root(root: &std::path::Path) -> Result<ResourceLimits, ErrorData> {
    crate::project::effective_limits(root).map_err(map_limits_error)
}

/// `now + budget`, or `None` when that instant cannot be represented (an
/// effectively unbounded duration). Shared by `query`, `structure`, and
/// custom-mechanic scans.
pub(in crate::tools) fn deadline_after(budget: Duration) -> Option<Instant> {
    Instant::now().checked_add(budget)
}

/// Maps a failed limits derivation to its wire [`ErrorData`].
fn map_limits_error(error: crate::project::ProjectLimitsError) -> ErrorData {
    use crate::project::ProjectLimitsError;
    match error {
        ProjectLimitsError::Env(error) => crate::sync::map_process_limits_error(&error),
        ProjectLimitsError::Lock(error) => error.into(),
        ProjectLimitsError::Memory(error) => invalid_params_with_code(
            format!(
                "limits could not be resolved: {error}; select a fixed session profile \
                 (`lean` or `audit`) via project_control"
            ),
            ErrorCode::InvalidInput,
        ),
        error @ ProjectLimitsError::Config(_) => invalid_params_with_code(
            format!("limits could not be resolved: {error}"),
            ErrorCode::InvalidInput,
        ),
    }
}

#[cfg(test)]
#[path = "tool_limits_tests.rs"]
mod tests;
