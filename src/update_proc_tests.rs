//! Unit tests for `update_proc::run_bounded`: wall-clock deadlines and silent
//! non-zero exit diagnostics.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure};
use std::os::unix::fs::PermissionsExt as _;
use std::time::Instant;

fn write_executable_script(
    dir: &std::path::Path,
    name: &str,
    body: &str,
) -> TestResult<std::path::PathBuf> {
    let path = dir.join(name);
    std::fs::write(&path, body).check("write helper script")?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .check("chmod helper script")?;
    Ok(path)
}

#[test]
fn run_bounded_terminates_hung_child_at_deadline() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let dir = tempfile::tempdir().check("temp dir")?;
    let pid_file = dir.path().join("helper.pid");
    let helper = write_executable_script(
        dir.path(),
        "hung-helper",
        &format!(
            "#!/bin/sh\necho $$ > '{}'\nexec /bin/sleep 30\n",
            pid_file.display()
        ),
    )?;

    let mut command = Command::new(&helper);
    let started = Instant::now();
    let error = run_bounded(
        &mut command,
        "sha256sum",
        Some(1024),
        Duration::from_millis(300),
    )
    .check_err("hung helper must time out")?;

    ensure(
        started.elapsed() < Duration::from_secs(10),
        "wall-clock timeout stops child well before 30s sleep",
    )?;
    ensure(
        error.to_string().contains("did not finish within"),
        format!("timeout error mentions deadline: {error}"),
    )?;
    let pid = std::fs::read_to_string(&pid_file).check("read helper pid")?;
    ensure(
        !std::path::Path::new("/proc").join(pid.trim()).exists(),
        "hung update helper is killed and reaped",
    )?;
    Ok(())
}

#[test]
fn run_bounded_reports_exit_status_when_stderr_is_empty() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let helper = write_executable_script(dir.path(), "silent-fail", "#!/bin/sh\nexit 7\n")?;

    let mut command = Command::new(&helper);
    let error = run_bounded(&mut command, "curl", Some(1024), Duration::from_secs(5))
        .check_err("non-zero exit must fail")?;

    ensure(
        error.to_string().contains("curl exited with"),
        format!("silent non-zero exit reports tool and status: {error}"),
    )?;
    Ok(())
}
