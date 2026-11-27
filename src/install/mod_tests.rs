//! Tests for `install`: home-directory discovery and the atomic
//! self-install flow. Env-sensitive tests (`SLUGTHUG_HOME` / `HOME`) are
//! scoped with `temp_env` (which restores the previous value afterwards)
//! and serialized behind `crate::util::TEST_ENV_LOCK` so parallel test
//! threads can't observe each other's mutation windows.

use super::place::retain_previous_binary;
use super::*;
use crate::util::TEST_ENV_LOCK;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};
use std::path::PathBuf;

/// `run_install` with the PATH question declined, so no test reads a terminal.
fn run_install_declined() -> Result<(), InstallError> {
    run_install_with(|_| false)
}

fn install_to(temp_dir: &tempfile::TempDir) -> TestResult<PathBuf> {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_var(
        "SLUGTHUG_HOME",
        Some(temp_dir.path().as_os_str()),
        || -> TestResult {
            run_install_declined().check("install succeeds")?;
            Ok(())
        },
    )?;
    Ok(temp_dir.path().join("slugaudit").join("slugaudit"))
}

/// An install that displaces an existing binary keeps it beside the new one.
#[test]
fn installing_over_an_existing_binary_retains_the_previous_one() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let installed = install_to(&directory)?;
    std::fs::write(&installed, b"the previous build").check("plant a previous binary")?;

    install_to(&directory)?;

    let previous = installed.with_file_name(PREVIOUS_BINARY_NAME);
    ensure_eq(
        &std::fs::read(&previous).check("the previous binary is retained")?,
        &b"the previous build",
        "the retained file must be the binary that was replaced, not the new one",
    )?;
    Ok(())
}

/// The retained copy keeps the source's permission bits, so restoring it needs
/// no separate `chmod`.
#[cfg(unix)]
#[test]
fn a_retained_binary_keeps_the_permissions_of_the_binary_it_replaced() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().check("temp dir")?;
    let target = directory.path().join("slugaudit");
    std::fs::write(&target, b"the previous build").check("write the binary to replace")?;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).check("chmod")?;

    let previous = retain_previous_binary(&target)
        .check("retaining succeeds")?
        .check("there was a binary to retain")?;

    ensure_eq(
        &previous.file_name().map(std::ffi::OsStr::to_owned),
        &Some(std::ffi::OsString::from(PREVIOUS_BINARY_NAME)),
        "the retained copy sits beside the binary it replaces",
    )?;
    let mode = std::fs::metadata(&previous)
        .check("stat the retained copy")?
        .permissions()
        .mode()
        & 0o777;
    ensure_eq(&mode, &0o755, "restoring it must not need a chmod")?;
    Ok(())
}

/// A copy that fails part-way must leave the last good `.prev` untouched and
/// no temporary file behind.
#[test]
fn retain_previous_failure_keeps_the_old_copy_and_leaves_no_temp() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let target = directory.path().join("slugaudit");
    std::fs::create_dir(&target).check("a directory cannot be copied as a file")?;
    let previous = directory.path().join(PREVIOUS_BINARY_NAME);
    std::fs::write(&previous, b"last good").check("seed prev")?;

    retain_previous_binary(&target).check_err("copying a directory fails")?;

    ensure_eq(
        &std::fs::read(&previous).check("read prev")?,
        &b"last good",
        "existing previous binary remains intact when staging fails",
    )?;
    let names: Vec<_> = std::fs::read_dir(directory.path())
        .check("list")?
        .map(|e| e.check("entry").map(|e| e.file_name()))
        .collect::<TestResult<_>>()?;
    ensure_eq(
        &names.len(),
        &2,
        format!("only target and prev remain: {names:?}"),
    )?;
    Ok(())
}

/// Staging uses `create_new(true)`, refusing an existing file or symlink rather
/// than overwriting through it.
#[test]
fn stage_copy_refuses_an_existing_staging_path_without_deleting_it() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let source = directory.path().join("source-bin");
    let occupied = directory.path().join("occupied.tmp");
    std::fs::write(&source, b"binary-bytes").check("write source")?;
    std::fs::write(&occupied, b"keep-existing").check("write occupied")?;

    let error = stage_copy(&source, &occupied)
        .check_err("create_new must refuse an already-existing staging path")?;

    ensure_eq(
        &error.kind(),
        &std::io::ErrorKind::AlreadyExists,
        "collision fails with AlreadyExists",
    )?;
    ensure_eq(
        &std::fs::read(&occupied).check("read occupied")?,
        &b"keep-existing",
        "pre-existing file at staging path must not be deleted or overwritten",
    )?;
    Ok(())
}

#[test]
fn a_first_install_has_nothing_to_retain() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let target = directory.path().join("slugaudit");

    ensure(
        retain_previous_binary(&target)
            .check("retaining succeeds")?
            .is_none(),
        "no previous binary existed, so none is created",
    )?;
    ensure(!target.exists(), "and the target is not created either")?;
    Ok(())
}

#[test]
fn run_install_copies_the_running_binary_to_a_stable_path() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let target = install_to(&temp)?;

    ensure(
        target.exists(),
        "the binary must land at $SLUGTHUG_HOME/slugaudit/slugaudit",
    )?;
    let source = running_binary().check("running binary")?;
    ensure_ne(
        &target,
        &source,
        "the install target differs from the source",
    )?;
    ensure_eq(
        &std::fs::read(&source).check("read source")?,
        &std::fs::read(&target).check("read target")?,
        "the installed binary must match the running one byte-for-byte",
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn installed_binary_is_executable() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().check("temp dir")?;
    let target = install_to(&temp)?;
    let mode = std::fs::metadata(&target)
        .check("metadata")?
        .permissions()
        .mode();
    ensure_eq(
        &(mode & 0o111),
        &0o111,
        "the installed binary must be executable",
    )?;
    Ok(())
}

#[test]
fn an_install_without_any_home_variable_reports_no_home() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;

    let result = temp_env::with_vars_unset(["SLUGTHUG_HOME", "HOME"], run_install_declined);

    ensure(
        matches!(result, Err(InstallError::NoHome)),
        format!("expected NoHome, got {result:?}"),
    )?;
    Ok(())
}

#[test]
fn an_install_home_that_is_a_file_reports_the_directory_it_could_not_create() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let home = directory.path().join("not-a-directory");
    std::fs::write(&home, b"a file where the home should be").check("plant file")?;
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;

    let result = temp_env::with_var(
        "SLUGTHUG_HOME",
        Some(home.as_os_str()),
        run_install_declined,
    );

    let Err(InstallError::Mkdir { path, .. }) = result else {
        return Err(format!("expected Mkdir, got {result:?}")).check("unexpected outcome");
    };
    ensure(
        path.ends_with("slugaudit"),
        format!("the path names the install dir: {path}"),
    )?;
    Ok(())
}

#[test]
fn an_install_target_that_is_a_directory_reports_the_failed_step_and_cleans_up() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let target = directory.path().join("slugaudit").join("slugaudit");
    std::fs::create_dir_all(&target).check("plant a directory at the target")?;
    std::fs::write(target.join("keep"), b"kept").check("make it non-empty")?;
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;

    let result = temp_env::with_var(
        "SLUGTHUG_HOME",
        Some(directory.path().as_os_str()),
        run_install_declined,
    );

    let Err(InstallError::Place(error)) = result else {
        return Err(format!("expected Place, got {result:?}")).check("unexpected outcome");
    };
    ensure_eq(
        &error.action,
        &"retain the previous binary at",
        "placement failure names the retain step",
    )?;
    ensure(
        target.join("keep").exists(),
        "the target must be left in place",
    )?;
    let leftovers: Vec<_> = std::fs::read_dir(directory.path().join("slugaudit"))
        .check("list install dir")?
        .collect::<Result<Vec<_>, _>>()
        .check("read install dir entries")?
        .into_iter()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    ensure(
        leftovers.is_empty(),
        format!("the staged copy must not be left behind: {leftovers:?}"),
    )?;
    Ok(())
}

#[test]
fn retaining_the_previous_binary_reports_a_stat_failure() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let file = directory.path().join("file");
    std::fs::write(&file, b"x").check("write file")?;

    let error = retain_previous_binary(&file.join("slugaudit"))
        .check_err("a target whose stat fails must not read as a first install")?;

    ensure_ne(
        &error.kind(),
        &std::io::ErrorKind::NotFound,
        "stat failure preserves non-NotFound error kind",
    )?;
    Ok(())
}

/// Builds a nested directory path of exactly `target_len` bytes, with every
/// component within `NAME_MAX` (255) so the path itself is valid.
#[cfg(target_os = "linux")]
fn directory_path_of_length(base: &Path, target_len: usize) -> TestResult<PathBuf> {
    let base_len = base.as_os_str().len();
    let minimum = base_len
        .checked_add(2)
        .check("base length plus a component overflowed")?;
    ensure(
        target_len > minimum,
        "target length must leave room for a component",
    )?;
    let available = target_len
        .checked_sub(base_len)
        .check("target length is shorter than the base")?;
    let components = available.div_ceil(256);
    let names_total = available
        .checked_sub(components)
        .check("component count exceeds the available length")?;
    let mut path = base.to_path_buf();
    for index in 0..components {
        let remainder = names_total
            .checked_rem(components)
            .check("component count is zero")?;
        let extra = usize::from(index < remainder);
        let name_len = names_total
            .checked_div(components)
            .check("component count is zero")?
            .checked_add(extra)
            .check("component length overflowed")?;
        path.push("d".repeat(name_len));
    }
    ensure_eq(
        &path.as_os_str().len(),
        &target_len,
        "the helper must build a path of the requested length",
    )?;
    Ok(path)
}

#[cfg(target_os = "linux")]
#[test]
fn an_install_that_cannot_stage_the_binary_reports_the_copy_step() -> TestResult {
    const INSTALL_DIR_LEN: usize = 4090;
    let directory = tempfile::tempdir().check("temp dir")?;
    let home_len = INSTALL_DIR_LEN
        .checked_sub("slugaudit".len())
        .and_then(|n| n.checked_sub(1))
        .check("home length fits")?;
    let home = directory_path_of_length(directory.path(), home_len)?;
    std::fs::create_dir_all(&home).check("create the deep home")?;
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;

    let result = temp_env::with_var(
        "SLUGTHUG_HOME",
        Some(home.as_os_str()),
        run_install_declined,
    );

    let Err(InstallError::Copy { path, .. }) = result else {
        return Err(format!("expected Copy, got {result:?}")).check("unexpected outcome");
    };
    ensure(
        Path::new(&path).starts_with(home.join("slugaudit")),
        format!("the reported path names the staged copy: {path}"),
    )?;
    Ok(())
}

#[test]
fn a_failed_running_binary_lookup_is_the_current_exe_error() -> TestResult {
    let source = std::io::Error::new(std::io::ErrorKind::NotFound, "no executable path");

    let error = running_binary_path(Err(source)).check_err("a lookup failure must surface")?;

    let InstallError::CurrentExe(inner) = error else {
        return Err(format!("expected CurrentExe, got {error:?}")).check("unexpected outcome");
    };
    ensure_eq(
        &inner.kind(),
        &std::io::ErrorKind::NotFound,
        "preserves NotFound I/O kind",
    )?;
    ensure_eq(
        &inner.to_string(),
        &"no executable path",
        "preserves lookup failure message",
    )?;
    Ok(())
}

#[test]
fn a_pre_epoch_clock_is_the_typed_clock_error() -> TestResult {
    let before_epoch = std::time::UNIX_EPOCH
        .checked_sub(std::time::Duration::from_secs(1))
        .check("pre-epoch timestamp")?;

    let error =
        elapsed_since_epoch(before_epoch).check_err("a pre-epoch time cannot be stamped")?;

    ensure(
        matches!(error, InstallError::Clock(_)),
        format!("expected Clock, got {error:?}"),
    )?;
    ensure(
        error.to_string().contains("system clock"),
        format!("the message must name the clock: {error}"),
    )?;
    Ok(())
}

#[test]
fn an_uncanonicalizable_path_entry_is_compared_as_written_and_logged() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let file = temp.path().join("file");
    std::fs::write(&file, b"x").check("write file")?;
    let broken_entry = file.join("child");

    let (found, logs) = crate::store::test_capture::capture_warns(|| {
        temp_env::with_var("PATH", Some(broken_entry.as_os_str()), || {
            on_path(temp.path())
        })
    })?;

    ensure(
        !found.check("the install directory resolves")?,
        "an entry that is not the directory does not match",
    )?;
    ensure(logs.contains("could not canonicalize a PATH entry"), &logs)?;
    Ok(())
}

#[test]
fn an_install_directory_that_cannot_be_resolved_is_an_error() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let file = temp.path().join("file");
    std::fs::write(&file, b"x").check("write file")?;
    let broken = file.join("child");

    let error = on_path(&broken).check_err("an unresolvable directory is an error")?;

    ensure(
        matches!(&error, InstallError::Canonicalize { path, .. } if *path == broken.display().to_string()),
        format!("{error:?}"),
    )?;
    Ok(())
}

#[path = "home_tests.rs"]
mod home_tests;
