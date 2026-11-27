//! Which `slugaudit` binary `connect` registers: the installed path over the
//! running build, and the typed errors when either cannot be determined.

use super::*;
use crate::util::TEST_ENV_LOCK;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::fs;
use std::path::PathBuf;

#[test]
fn prefer_slugthug_binary_uses_the_installed_path_when_present() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let bin_dir = temp.path().join("slugaudit");
    fs::create_dir_all(&bin_dir).check("bin dir")?;
    fs::write(bin_dir.join("slugaudit"), b"#!fake").check("fake binary")?;

    let current = PathBuf::from("/build/artifacts/slugaudit");
    temp_env::with_var(
        "SLUGTHUG_HOME",
        Some(temp.path().as_os_str()),
        || -> TestResult {
            ensure_eq(
                &prefer_slugthug_binary(&current).check("stat succeeds")?,
                &bin_dir.join("slugaudit"),
                "the stable installed path wins over the current build artifact",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn prefer_slugthug_binary_keeps_current_when_not_installed() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", None::<&std::ffi::OsStr>),
            ("HOME", None::<&std::ffi::OsStr>),
        ],
        || -> TestResult {
            let current = PathBuf::from("/build/artifacts/slugaudit");
            ensure_eq(
                &prefer_slugthug_binary(&current).check("stat succeeds")?,
                &current,
                "returns running binary when no home directory is configured",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn a_failed_running_binary_lookup_is_the_binary_path_error() -> TestResult {
    let source = std::io::Error::new(std::io::ErrorKind::NotFound, "no executable path");

    let error = binary_path(Err(source)).check_err("a lookup failure must surface")?;

    let ConnectError::BinaryPath(inner) = error else {
        return Err(format!("expected BinaryPath, got {error:?}")).check("unexpected outcome");
    };
    ensure_eq(
        &inner.kind(),
        &std::io::ErrorKind::NotFound,
        "preserves NotFound I/O kind",
    )?;
    ensure_eq(
        &inner.to_string(),
        &"no executable path",
        "preserves original error message",
    )?;
    Ok(())
}

#[test]
fn a_stat_failure_for_the_installed_binary_is_an_io_error() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    // A file where the slugthug home should be makes the installed binary's
    // stat fail with ENOTDIR; the stat must not read as "not installed".
    let slugthug_home = temp.path().join("slugthug-home");
    fs::write(&slugthug_home, b"x").check("write file")?;
    let config_home = temp.path().join("config-home");
    fs::create_dir_all(&config_home).check("config home")?;
    let cursor = find_agent("cursor").check("cursor agent")?;

    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(slugthug_home.as_os_str())),
            ("SLUGTHUG_CONFIG_HOME", Some(config_home.as_os_str())),
        ],
        || -> TestResult {
            let error =
                connect_one(cursor, None, false).check_err("the installed-path stat must fail")?;

            ensure(
                matches!(error, ConnectError::Io(_)),
                format!("expected Io, got {error:?}"),
            )?;
            Ok(())
        },
    )?;
    Ok(())
}
