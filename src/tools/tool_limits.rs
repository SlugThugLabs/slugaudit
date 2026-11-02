//! The resource limits a tool call runs under: the project's effective
//! limits for a request path, or the process limits when no project is named,
//! and the tool error for each way resolving them fails.

use crate::model::{ResourceLimits, process_limits};
use rmcp::ErrorData;

/// The process and project resource limits for `path`, resolved before the
/// index is touched. Shared by `query`, `leads` and `structure`.
///
/// A path with no active project root is an error, not a cue to fall back to
/// process defaults: every one of those tools goes on to call
/// `ensure_synced`, which resolves the same root and rejects the same path,
/// so a fallback would only run request validation against limits the
/// project does not use, and would drop the resolution error unlogged. The
/// error is the same `invalid_params` (with `project_not_enabled` when the
/// project was never enabled) that `ensure_synced` returns. A present but
/// unreadable or malformed global config fails the call with an
/// `invalid_params` naming the file.
pub(crate) fn resolve_limits(path: &str) -> Result<ResourceLimits, ErrorData> {
    limits_for_root(&project_root(path)?)
}

/// The active project root `path` lies in, with the same error
/// `ensure_synced` returns for a path outside every enabled project.
pub(crate) fn project_root(path: &str) -> Result<std::path::PathBuf, ErrorData> {
    crate::project::find_project_root(std::path::Path::new(path))
        .map(|root| root.as_path().to_path_buf())
        .map_err(|error| crate::sync::map_activation_error(&error))
}

/// The process-wide limits, or the tool error for an environment that is
/// invalid and was never validated at startup (a library embedder; the shipped
/// binary validates before serving).
pub(crate) fn process_limits_for_tool() -> Result<ResourceLimits, ErrorData> {
    process_limits()
        .copied()
        .map_err(|error| crate::sync::map_process_limits_error(&error))
}

/// The limits for the project at `root`: defaults, the global file, the
/// `SLUGAUDIT_*` environment and the session profile, in the order
/// `project::effective_limits` documents. The error is returned to the tool
/// call unlogged; `server_runner` logs it once.
pub(crate) fn limits_for_root(root: &std::path::Path) -> Result<ResourceLimits, ErrorData> {
    crate::project::effective_limits(root).map_err(map_limits_error)
}

/// The tool error for a failed limits derivation. Each cause gets its own
/// remedy: an invalid environment variable, a poisoned lock (restart), an
/// undetectable host memory (choose a fixed profile) and an unreadable or
/// malformed global config (fix the file).
fn map_limits_error(error: crate::project::ProjectLimitsError) -> ErrorData {
    use crate::project::ProjectLimitsError;
    match error {
        ProjectLimitsError::Env(error) => crate::sync::map_process_limits_error(&error),
        ProjectLimitsError::Lock(error) => error.into(),
        ProjectLimitsError::Memory(error) => {
            ErrorData::invalid_params(format!("limits could not be resolved: {error}"), None)
        }
        error @ ProjectLimitsError::Config(_) => ErrorData::invalid_params(
            format!("limits could not be resolved: {error}; fix the file and retry"),
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support;
    use slugaudit_test_check::{Check, CheckErr, TestResult};

    #[test]
    fn a_poisoned_lock_maps_to_a_restart_error_without_the_file_hint() -> TestResult {
        let mutex = std::sync::Mutex::new(0_u8);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TestResult {
            let _guard = mutex.lock().check("lock the fresh mutex")?;
            std::panic::resume_unwind(Box::new("simulated panic inside the critical section"));
        }));
        assert!(unwound.is_err(), "the holder must have unwound");
        let poisoned = crate::util::lock(&mutex, "limits_test_mutex")
            .check_err("a poisoned mutex is an error")?;

        let error = map_limits_error(crate::project::ProjectLimitsError::Lock(poisoned));

        assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert_eq!(
            error.message,
            "mutex limits_test_mutex was poisoned by a panic in an earlier holder; \
             restart the SlugAudit server"
        );
        Ok(())
    }

    #[test]
    fn resolve_limits_rejects_a_path_with_no_active_project() -> TestResult {
        let directory = tempfile::tempdir().check("temp dir")?;

        let error = resolve_limits(directory.path().to_str().check("utf-8 path")?)
            .check_err("a path with no project root must fail, not fall back")?;

        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        let code = error
            .data
            .as_ref()
            .and_then(|data| data.get("code"))
            .and_then(|code| code.as_str());
        assert_eq!(code, Some("project_not_enabled"));
        Ok(())
    }

    #[test]
    fn resolve_limits_returns_limits_for_an_active_project() -> TestResult {
        let project = test_support::activated_project("a.rs", b"fn a() {}")?;

        let limits = resolve_limits(project.path().to_str().check("utf-8 path")?)
            .check("an active project resolves")?;

        assert!(limits.max_file_bytes > 0);
        Ok(())
    }

    #[test]
    fn fail_closed_a_malformed_global_config_fails_the_call_naming_the_file() -> TestResult {
        let project = test_support::activated_project("a.rs", b"fn a() {}")?;
        let home = tempfile::tempdir().check("temp home")?;
        let config = home.path().join("config.json");
        std::fs::write(&config, "{ not json").check("write config")?;
        crate::project::set_global_config_path_for_test(Some(config.clone()));

        let error = resolve_limits(project.path().to_str().check("utf-8 path")?)
            .check_err("a malformed config must fail the call, not fall back to defaults")?;

        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            error.message.contains(config.to_str().check("utf-8 path")?),
            "{}",
            error.message
        );
        Ok(())
    }
}
