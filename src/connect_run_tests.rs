use super::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::os::unix::fs::PermissionsExt as _;

/// Writes an executable script. Callers hold `TEST_ENV_LOCK` so no other
/// test has swapped `PATH` away while the script runs.
fn script(dir: &std::path::Path, body: &str) -> TestResult<String> {
    let path = dir.join("fake-client");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).check("write script")?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).check("chmod")?;
    Ok(path.display().to_string())
}

#[test]
fn a_blocking_client_times_out_and_the_child_is_killed_and_reaped() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let dir = tempfile::tempdir().check("dir")?;
    let pid_file = dir.path().join("pid");
    let cli = script(
        dir.path(),
        &format!("echo $$ > '{}'\nexec /bin/sleep 30", pid_file.display()),
    )?;

    let started = Instant::now();
    let result = run_bounded(&cli, &[], Duration::from_millis(400));

    let Err(ConnectError::AgentTimeout {
        cli: named,
        timeout_secs,
    }) = result
    else {
        return Err(format!("expected AgentTimeout, got {result:?}")).check("unexpected outcome");
    };
    ensure_eq(&named, &cli, "named == cli")?;
    ensure((timeout_secs - 0.4).abs() < 0.01, format!("{timeout_secs}"))?;
    ensure(
        started.elapsed() < Duration::from_secs(10),
        "started.elapsed() < Duration::from_secs(10)",
    )?;
    let pid = std::fs::read_to_string(&pid_file).check("pid file")?;
    // A reaped child has no /proc entry; a killed-but-unreaped one would
    // linger as a zombie.
    ensure(
        !std::path::Path::new("/proc").join(pid.trim()).exists(),
        format!("child {} must be gone", pid.trim()),
    )?;
    Ok(())
}

#[test]
fn a_background_process_holding_the_output_pipe_times_out_instead_of_hanging() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let dir = tempfile::tempdir().check("dir")?;
    // The client exits at once but leaves `sleep` holding its stdout.
    let cli = script(dir.path(), "/bin/sleep 20 &\nexit 0")?;

    let started = Instant::now();
    let result = run_bounded(&cli, &[], Duration::from_millis(400));

    let Err(ConnectError::AgentTimeout { cli: named, .. }) = result else {
        return Err(format!("expected AgentTimeout, got {result:?}")).check("unexpected outcome");
    };
    ensure_eq(&named, &cli, "named == cli")?;
    ensure(
        started.elapsed() < Duration::from_secs(10),
        "must not wait for the sleeper",
    )?;
    Ok(())
}

#[test]
fn a_blocking_client_timeout_kills_grandchild_processes_too() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let dir = tempfile::tempdir().check("dir")?;
    let grandchild_pid_file = dir.path().join("grandchild.pid");
    let cli = script(
        dir.path(),
        &format!(
            "/bin/sleep 30 &\necho $! > '{}'\nwait",
            grandchild_pid_file.display()
        ),
    )?;

    let result = run_bounded(&cli, &[], Duration::from_millis(400));
    ensure(
        matches!(result, Err(ConnectError::AgentTimeout { .. })),
        format!("expected AgentTimeout, got {result:?}"),
    )?;

    let pid = std::fs::read_to_string(&grandchild_pid_file).check("grandchild pid file")?;
    let proc_path = std::path::Path::new("/proc").join(pid.trim());
    let wait_until = Instant::now()
        .checked_add(Duration::from_millis(500))
        .check("deadline")?;
    while proc_path.exists() && Instant::now() < wait_until {
        thread::sleep(Duration::from_millis(20));
    }
    ensure(
        !proc_path.exists(),
        format!(
            "grandchild {} must be terminated with its process group",
            pid.trim()
        ),
    )?;
    Ok(())
}

#[test]
fn a_client_that_finishes_returns_its_status_and_output() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let dir = tempfile::tempdir().check("dir")?;
    let cli = script(dir.path(), "echo out; echo err >&2; exit 3")?;

    let output = run_bounded(&cli, &[], CLIENT_CLI_TIMEOUT).check("runs")?;

    ensure_eq(
        &output.status.code(),
        &Some(3),
        "output.status.code() == Some(3)",
    )?;
    ensure_eq(&output.stdout, &b"out\n", "output.stdout == b\"out\\n\"")?;
    ensure_eq(&output.stderr, &b"err\n", "output.stderr == b\"err\\n\"")?;
    Ok(())
}

#[test]
fn output_beyond_the_cap_is_drained_without_blocking_the_client() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let dir = tempfile::tempdir().check("dir")?;
    let cli = script(dir.path(), "/usr/bin/head -c 3000000 /dev/zero")?;

    let output = run_bounded(&cli, &[], CLIENT_CLI_TIMEOUT).check("runs")?;

    ensure(output.status.success(), "output.status.success()")?;
    ensure_eq(
        &(output.stdout.len() as u64),
        &MAX_CAPTURED_BYTES,
        "output.stdout.len() as u64 == MAX_CAPTURED_BYTES",
    )?;
    Ok(())
}
