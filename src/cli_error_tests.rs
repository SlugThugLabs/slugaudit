//! Tests for `cli` error conversions and how output write failures are
//! classified (broken pipe versus a real failure).

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn a_serde_failure_converts_to_the_json_error_variant() -> TestResult {
    let source = serde_json::from_str::<serde_json::Value>("{").check_err("invalid JSON")?;

    let error = ConnectError::from(source);

    match error {
        ConnectError::Json(inner) => {
            ensure(
                inner.is_eof(),
                format!("the parse error must be kept: {inner}"),
            )?;
        }
        other => return Err(format!("expected Json, got {other:?}")).check("unexpected outcome"),
    }
    Ok(())
}

#[test]
fn a_config_write_without_a_home_reports_the_home_unavailable() -> TestResult {
    let cursor = crate::connect::find_agent("cursor").check("cursor is registered")?;

    let error =
        crate::connect::connect_agent(&cursor, std::path::Path::new("/bin/slugaudit"), None)
            .check_err("a config write has nowhere to go without a home")?;

    ensure(
        matches!(error, ConnectError::HomeUnavailable),
        format!("expected HomeUnavailable, got {error:?}"),
    )?;
    Ok(())
}

#[test]
fn a_config_parent_that_cannot_be_created_is_an_io_error() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    std::fs::write(temp.path().join(".cursor"), b"not a directory").check("blocking file")?;
    let cursor = crate::connect::find_agent("cursor").check("cursor is registered")?;

    let error = crate::connect::connect_agent(
        &cursor,
        std::path::Path::new("/bin/slugaudit"),
        Some(temp.path()),
    )
    .check_err("create_dir_all must fail on a file")?;

    ensure(
        matches!(error, ConnectError::Io(_)),
        format!("expected Io, got {error:?}"),
    )?;
    Ok(())
}

#[test]
fn a_config_path_that_is_a_directory_is_an_io_error() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    std::fs::create_dir_all(temp.path().join(".cursor/mcp.json")).check("config as directory")?;
    let cursor = crate::connect::find_agent("cursor").check("cursor is registered")?;

    let error = crate::connect::connect_agent(
        &cursor,
        std::path::Path::new("/bin/slugaudit"),
        Some(temp.path()),
    )
    .check_err("reading a directory as a config must fail")?;

    ensure(
        matches!(error, ConnectError::Io(_)),
        format!("expected Io, got {error:?}"),
    )?;
    Ok(())
}

/// A writer that fails every write with `kind`; the CLI's output helper must
/// return that failure rather than panic.
struct FailingWriter(std::io::ErrorKind);

impl std::io::Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(self.0, "injected write failure"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_closed_pipe_write_is_reported_as_broken_pipe_not_a_panic() -> TestResult {
    let mut writer = FailingWriter(std::io::ErrorKind::BrokenPipe);

    let error = write_to(&mut writer, format_args!("partial output"))
        .check_err("a closed pipe must surface as an error")?;

    ensure_eq(
        &error.kind(),
        &std::io::ErrorKind::BrokenPipe,
        "preserves BrokenPipe error kind",
    )?;
    ensure(
        is_broken_pipe(&error),
        "recognizes BrokenPipe as clean pipe close",
    )?;
    Ok(())
}

#[test]
fn a_non_pipe_write_failure_is_reported_and_not_classified_as_a_clean_exit() -> TestResult {
    let mut writer = FailingWriter(std::io::ErrorKind::StorageFull);

    let error = write_to(&mut writer, format_args!("partial output"))
        .check_err("a write failure must surface as an error")?;

    ensure_eq(
        &error.kind(),
        &std::io::ErrorKind::StorageFull,
        "preserves StorageFull error kind",
    )?;
    ensure_eq(
        &error.to_string(),
        &"injected write failure",
        "preserves injected failure message",
    )?;
    ensure(
        !is_broken_pipe(&error),
        "StorageFull is not classified as broken pipe",
    )?;
    Ok(())
}

#[test]
fn a_successful_write_keeps_the_exact_output_text() -> TestResult {
    let mut writer = Vec::new();

    write_to(&mut writer, format_args!("slugaudit {}\n", "1.0.10")).check("the write succeeds")?;

    ensure_eq(
        &writer,
        &b"slugaudit 1.0.10\n",
        "writes exact formatted output bytes",
    )?;
    Ok(())
}

#[test]
fn a_closed_pipe_is_found_through_every_cli_error_wrapper() -> TestResult {
    let broken = || std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed pipe");

    ensure(
        is_broken_pipe(&ConnectError::Io(broken())),
        "ConnectError::Io wraps BrokenPipe",
    )?;
    ensure(
        is_broken_pipe(&crate::install::InstallError::Io(broken())),
        "InstallError::Io wraps BrokenPipe",
    )?;
    ensure(
        is_broken_pipe(&crate::update::UpdateError::Io(broken())),
        "UpdateError::Io wraps BrokenPipe",
    )?;
    ensure(
        is_broken_pipe(&crate::grammar_cli::GrammarCliError::Io(broken())),
        "GrammarCliError::Io wraps BrokenPipe",
    )?;
    Ok(())
}

#[test]
fn a_non_pipe_io_failure_in_a_wrapper_is_not_a_clean_exit() -> TestResult {
    let refused = || std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");

    ensure(
        !is_broken_pipe(&ConnectError::Io(refused())),
        "ConnectError::Io PermissionDenied is not BrokenPipe",
    )?;
    ensure(
        !is_broken_pipe(&crate::install::InstallError::Io(refused())),
        "InstallError::Io PermissionDenied is not BrokenPipe",
    )?;
    ensure(
        !is_broken_pipe(&crate::update::UpdateError::Io(refused())),
        "UpdateError::Io PermissionDenied is not BrokenPipe",
    )?;
    ensure(
        !is_broken_pipe(&crate::grammar_cli::GrammarCliError::Io(refused())),
        "GrammarCliError::Io PermissionDenied is not BrokenPipe",
    )?;
    Ok(())
}

#[test]
fn a_rejected_add_with_output_keeps_the_pointer_to_it() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("tempdir")?;
    let bin_dir = tempfile::tempdir().check("bin dir")?;
    let script = bin_dir.path().join("claude");
    std::fs::write(
        &script,
        "#!/bin/sh\necho 'client rejected it' >&2\nexit 1\n",
    )
    .check("write fake cli")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script).check("metadata")?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).check("chmod fake cli")?;
    }

    let result = temp_env::with_vars(
        [
            ("PATH", Some(bin_dir.path().as_os_str())),
            ("SLUGTHUG_CONFIG_HOME", Some(temp.path().as_os_str())),
            ("SLUGTHUG_HOME", Some(temp.path().as_os_str())),
        ],
        || crate::connect::run_connect(Some("claude")),
    );

    let error = result.check_err("expected AddFailed, got Ok")?;
    match &error {
        ConnectError::AddFailed { cli, .. } => {
            ensure_eq(&cli, &"claude", "AddFailed names claude executable")?;
            ensure(
                error.to_string().contains("see the output above"),
                format!("captured client output makes the pointer true: {error}"),
            )?;
        }
        other => {
            return Err(format!("expected AddFailed, got {other:?}")).check("unexpected outcome");
        }
    }
    Ok(())
}
