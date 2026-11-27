//! Best-effort detection probe for the `connect` surface: is a client CLI on
//! `PATH`. Only a definite "not found" is a silent negative; any other failure
//! is a deliberate recovery to the negative that is logged with the source
//! error, so a detection miss always has a cause. Config-path existence and
//! config reads are not probed here: `AgentDef::is_installed` and
//! `AgentDef::is_connected` return those failures to their callers.

use std::time::Duration;

/// Maximum wall-clock time spent verifying a colliding executable name (`q`).
const Q_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// True when `cli` resolves on `PATH`. Only a definite "not found" is a silent
/// `false`; any other lookup failure (empty `PATH`, an uncanonicalizable hit)
/// is a deliberate recovery to `false` that is logged with the source error.
fn cli_on_path(cli: &str) -> bool {
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

/// True when `cli` resolves on `PATH` and, for colliding binary names (`q`),
/// identifies itself as the expected MCP client rather than an unrelated OS
/// package (such as Debian's `python3-q-text-as-data` at `/usr/bin/q`).
pub(crate) fn cli_installed(cli: &str) -> bool {
    if !cli_on_path(cli) {
        return false;
    }
    if cli == "q" {
        return verify_amazon_q(cli);
    }
    true
}

/// Runs `q mcp --help` under [`Q_PROBE_TIMEOUT`] to distinguish Amazon Q
/// Developer CLI from unrelated `q` tools (such as `python3-q-text-as-data`).
fn verify_amazon_q(cli: &str) -> bool {
    let args = ["mcp".to_owned(), "--help".to_owned()];
    match crate::connect::run::run_bounded(cli, &args, Q_PROBE_TIMEOUT) {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
            !combined.contains("text-as-data")
                && !combined.contains("harelba")
                && (combined.contains("amazon q")
                    || (combined.contains("mcp") && combined.contains("add")))
        }
        Ok(_) => false,
        Err(error) => {
            tracing::debug!(
                cli,
                error = %error,
                "bounded probe of `q mcp --help` failed; treating Amazon Q as not installed",
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slugaudit_test_check::{Check, TestResult, ensure};

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

        ensure(!found, "missing CLI must report false")?;
        ensure(!logs.contains("could not search PATH"), &logs)?;
        Ok(())
    }

    #[test]
    fn a_path_lookup_failure_is_absent_but_logged_with_the_cause() -> TestResult {
        let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;

        let (found, logs) = temp_env::with_var_unset("PATH", || {
            crate::store::test_capture::capture_warns(|| cli_on_path("slugaudit-no-such-cli"))
        })?;

        ensure(!found, "unset PATH must report false")?;
        ensure(logs.contains("could not search PATH"), &logs)?;
        ensure(
            logs.contains("error="),
            format!("the source error is kept:\n{logs}"),
        )?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn debian_text_as_data_q_binary_is_not_mistaken_for_amazon_q() -> TestResult {
        use std::os::unix::fs::PermissionsExt as _;
        let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
        let bin_dir = tempfile::tempdir().check("temp dir")?;
        let q_script = bin_dir.path().join("q");
        std::fs::write(
            &q_script,
            "#!/bin/sh\nprintf 'q allows performing SQL-like statements on tabular text data (https://harelba.github.io/q/)\\n'\nexit 0\n",
        )
        .check("write fake debian q")?;
        std::fs::set_permissions(&q_script, std::fs::Permissions::from_mode(0o755))
            .check("chmod fake debian q")?;

        let installed = temp_env::with_var("PATH", Some(bin_dir.path().as_os_str()), || {
            cli_installed("q")
        });
        ensure(
            !installed,
            "Debian python3-q-text-as-data binary must not be detected as Amazon Q Developer",
        )?;

        std::fs::write(
            &q_script,
            "#!/bin/sh\nprintf 'Usage: q mcp <COMMAND>\\nCommands:\\n  add   Add a server\\n'\nexit 0\n",
        )
        .check("write fake amazon q")?;

        let installed_amazon_q =
            temp_env::with_var("PATH", Some(bin_dir.path().as_os_str()), || {
                cli_installed("q")
            });
        ensure(
            installed_amazon_q,
            "Amazon Q Developer CLI exposing `q mcp` subcommands must be detected as installed",
        )?;
        Ok(())
    }
}
