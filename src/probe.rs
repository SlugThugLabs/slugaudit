//! Best-effort detection probes for the `connect` surface (is a client CLI on
//! `PATH`, is its config readable). Only a definite "not found" is a silent
//! negative; any other failure is a deliberate recovery to the negative that
//! is logged with the source error, so a detection miss always has a cause.
//! Config-path existence is not probed here: `AgentDef::is_installed` returns
//! that failure to its caller.

use crate::util::{MAX_FOREIGN_TEXT_BYTES, read_file_capped};
use std::path::Path;

/// Reads a client's config. A missing file is a silent `None`; an unreadable,
/// non-UTF-8, or over-cap file is `None` with the cause logged. The write path
/// re-reads the same file and fails explicitly.
pub(crate) fn read_config_or_warn(path: &Path) -> Option<String> {
    match read_file_capped(path, MAX_FOREIGN_TEXT_BYTES) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "could not read client config; treating it as not registered",
            );
            None
        }
    }
}

/// True when `cli` resolves on `PATH`. Only a definite "not found" is a silent
/// `false`; any other lookup failure (empty `PATH`, an uncanonicalizable hit)
/// is a deliberate recovery to `false` that is logged with the source error.
pub(crate) fn cli_on_path(cli: &str) -> bool {
    match which::which(cli) {
        Ok(_) => true,
        Err(which::Error::CannotFindBinaryPath) => false,
        Err(error) => {
            tracing::warn!(
                cli,
                error = %error,
                "could not search PATH for a client CLI; treating it as not installed",
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slugaudit_test_check::{Check, TestResult};

    #[test]
    fn a_missing_config_reads_as_none_without_a_warning() -> TestResult {
        let directory = tempfile::tempdir().check("temp dir")?;

        let (contents, logs) = crate::store::test_capture::capture_warns(|| {
            tracing::warn!("control-line-probe-config-missing");
            read_config_or_warn(&directory.path().join("absent.json"))
        });

        assert_eq!(contents, None);
        assert!(logs.contains("control-line-probe-config-missing"));
        assert!(!logs.contains("could not read client config"), "{logs}");
        Ok(())
    }

    #[test]
    fn an_over_cap_config_reads_as_none_and_warns_with_the_cause() -> TestResult {
        let directory = tempfile::tempdir().check("temp dir")?;
        let big = directory.path().join("mcp.json");
        let over_cap = usize::try_from(MAX_FOREIGN_TEXT_BYTES).check("cap fits usize")? + 1;
        std::fs::write(&big, vec![b' '; over_cap]).check("write oversized config")?;

        let (contents, logs) =
            crate::store::test_capture::capture_warns(|| read_config_or_warn(&big));

        assert_eq!(contents, None);
        assert!(logs.contains("could not read client config"), "{logs}");
        assert!(logs.contains("-byte cap"), "the cause is kept:\n{logs}");
        Ok(())
    }

    /// Runs `f` with `PATH` set to an empty directory, so no client CLI
    /// resolves, and returns what it logged at warn level or above.
    fn warns_with_empty_path<T>(f: impl FnOnce() -> T) -> TestResult<(T, String)> {
        let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
        let empty = tempfile::tempdir().check("temp dir")?;
        Ok(temp_env::with_var(
            "PATH",
            Some(empty.path().as_os_str()),
            || crate::store::test_capture::capture_warns(f),
        ))
    }

    #[test]
    fn a_cli_that_is_not_on_path_is_absent_without_a_warning() -> TestResult {
        let (found, logs) = warns_with_empty_path(|| cli_on_path("slugaudit-no-such-cli"))?;

        assert!(!found);
        assert!(!logs.contains("could not search PATH"), "{logs}");
        Ok(())
    }

    #[test]
    fn a_path_lookup_failure_is_absent_but_logged_with_the_cause() -> TestResult {
        let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;

        let (found, logs) = temp_env::with_var_unset("PATH", || {
            crate::store::test_capture::capture_warns(|| cli_on_path("slugaudit-no-such-cli"))
        });

        assert!(!found);
        assert!(logs.contains("could not search PATH"), "{logs}");
        assert!(logs.contains("error="), "the source error is kept:\n{logs}");
        Ok(())
    }
}
