//! Best-effort detection probe for the `connect` surface: is a client CLI on
//! `PATH`. Only a definite "not found" is a silent negative; any other failure
//! is a deliberate recovery to the negative that is logged with the source
//! error, so a detection miss always has a cause. Config-path existence and
//! config reads are not probed here: `AgentDef::is_installed` and
//! `AgentDef::is_connected` return those failures to their callers.

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

    /// Runs `f` with `PATH` set to an empty directory, so no client CLI
    /// resolves, and returns what it logged at warn level or above.
    fn warns_with_empty_path<T>(f: impl FnOnce() -> T) -> TestResult<(T, String)> {
        let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
        let empty = tempfile::tempdir().check("temp dir")?;
        temp_env::with_var("PATH", Some(empty.path().as_os_str()), || {
            crate::store::test_capture::capture_warns(f)
        })
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
        })?;

        assert!(!found);
        assert!(logs.contains("could not search PATH"), "{logs}");
        assert!(logs.contains("error="), "the source error is kept:\n{logs}");
        Ok(())
    }
}
