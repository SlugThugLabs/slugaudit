//! Tests for `install`: home-directory discovery and the atomic
//! self-install flow. Env-sensitive tests (`SLUGTHUG_HOME` / `HOME`) are
//! scoped with `temp_env` (which restores the previous value afterwards)
//! and serialized behind `crate::util::TEST_ENV_LOCK` so parallel test
//! threads can't observe each other's mutation windows.

use super::*;
use crate::util::TEST_ENV_LOCK;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::path::PathBuf;

fn install_to(temp_dir: &tempfile::TempDir) -> TestResult<PathBuf> {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_var(
        "SLUGTHUG_HOME",
        Some(temp_dir.path().as_os_str()),
        || -> TestResult {
            run_install().check("install succeeds")?;
            Ok(())
        },
    )?;
    Ok(temp_dir.path().join("slugaudit").join("slugaudit"))
}

/// An install that displaces an existing binary keeps it beside the new one.
/// This is the whole reason an update is reversible without a download.
#[test]
fn installing_over_an_existing_binary_retains_the_previous_one() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let installed = install_to(&directory)?;
    std::fs::write(&installed, b"the previous build").check("plant a previous binary")?;

    install_to(&directory)?;

    let previous = installed.with_file_name(PREVIOUS_BINARY_NAME);
    assert_eq!(
        std::fs::read(&previous).check("the previous binary is retained")?,
        b"the previous build",
        "the retained file must be the binary that was replaced, not the new one"
    );
    Ok(())
}

/// The retained copy keeps the source's permission bits, so restoring it needs
/// no separate `chmod` — a revert path that yielded a non-executable binary
/// would be worse than no revert path.
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

    assert_eq!(
        previous.file_name().map(std::ffi::OsStr::to_owned),
        Some(std::ffi::OsString::from(PREVIOUS_BINARY_NAME)),
        "the retained copy sits beside the binary it replaces"
    );
    let mode = std::fs::metadata(&previous)
        .check("stat the retained copy")?
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o755, "restoring it must not need a chmod");
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

    assert_eq!(std::fs::read(&previous).check("read prev")?, b"last good");
    let names: Vec<_> = std::fs::read_dir(directory.path())
        .check("list")?
        .map(|e| e.check("entry").map(|e| e.file_name()))
        .collect::<TestResult<_>>()?;
    assert_eq!(names.len(), 2, "only target and prev remain: {names:?}");
    Ok(())
}

/// A first install displaces nothing, so it retains nothing and creates no
/// stray file.
#[test]
fn a_first_install_has_nothing_to_retain() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let target = directory.path().join("slugaudit");

    assert!(
        retain_previous_binary(&target)
            .check("retaining succeeds")?
            .is_none(),
        "no previous binary existed, so none is created"
    );
    assert!(!target.exists(), "and the target is not created either");
    Ok(())
}

#[test]
fn slugthug_home_prefers_slugthug_home_over_home() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(temp.path().as_os_str())),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || -> TestResult {
            assert_eq!(slugthug_home().check("home resolves")?, temp.path());
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn slugaudit_dir_resolves_under_slugthug_home() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    temp_env::with_var(
        "SLUGTHUG_HOME",
        Some(temp.path().as_os_str()),
        || -> TestResult {
            assert_eq!(
                slugaudit_dir().check("slugaudit dir resolves")?,
                temp.path().join("slugaudit")
            );
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn config_home_is_the_user_home_not_the_slugthug_directory() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(std::ffi::OsStr::new("/slug/home"))),
            ("SLUGTHUG_CONFIG_HOME", None),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || -> TestResult {
            assert_eq!(
                config_home().check("config home resolves")?,
                PathBuf::from("/somewhere/else"),
                "client config must resolve against the user's home, never ~/.slugthug"
            );
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn config_home_override_wins_over_home() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            (
                "SLUGTHUG_CONFIG_HOME",
                Some(std::ffi::OsStr::new("/agents/home")),
            ),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || -> TestResult {
            assert_eq!(
                config_home().check("config home resolves")?,
                PathBuf::from("/agents/home")
            );
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn a_client_config_path_resolves_under_the_user_home() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(std::ffi::OsStr::new("/slug/home"))),
            ("SLUGTHUG_CONFIG_HOME", None),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || -> TestResult {
            assert_eq!(
                config_home()
                    .check("config home resolves")?
                    .join(".cursor/mcp.json"),
                PathBuf::from("/somewhere/else/.cursor/mcp.json"),
                "guards the regression where a client config landed in ~/.slugthug/ and the client never read it"
            );
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn slugthug_home_falls_back_to_home_slugthug() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", None),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || -> TestResult {
            assert_eq!(
                slugthug_home().check("home resolves")?,
                PathBuf::from("/somewhere/else/.slugthug")
            );
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn slugthug_home_is_none_without_either_var() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", None::<&std::ffi::OsStr>),
            ("HOME", None::<&std::ffi::OsStr>),
        ],
        || assert_eq!(slugthug_home(), None),
    );
    Ok(())
}

#[test]
fn run_install_copies_the_running_binary_to_a_stable_path() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let target = install_to(&temp)?;

    assert!(
        target.exists(),
        "the binary must land at $SLUGTHUG_HOME/slugaudit/slugaudit"
    );
    let source = running_binary().check("running binary")?;
    assert_ne!(target, source, "the install target differs from the source");
    assert_eq!(
        std::fs::read(&source).check("read source")?,
        std::fs::read(&target).check("read target")?,
        "the installed binary must match the running one byte-for-byte"
    );
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
    assert_eq!(
        mode & 0o111,
        0o111,
        "the installed binary must be executable"
    );
    Ok(())
}

/// With neither `SLUGTHUG_HOME` nor `HOME` there is no install location to
/// guess; the install names both variables instead of writing somewhere odd.
#[test]
fn an_install_without_any_home_variable_reports_no_home() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;

    let result = temp_env::with_vars_unset(["SLUGTHUG_HOME", "HOME"], run_install);

    assert!(
        matches!(result, Err(InstallError::NoHome)),
        "expected NoHome, got {result:?}"
    );
    Ok(())
}

/// An install directory that cannot be created is reported with its path.
#[test]
fn an_install_home_that_is_a_file_reports_the_directory_it_could_not_create() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let home = directory.path().join("not-a-directory");
    std::fs::write(&home, b"a file where the home should be").check("plant file")?;
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;

    let result = temp_env::with_var("SLUGTHUG_HOME", Some(home.as_os_str()), run_install);

    let Err(InstallError::Mkdir { path, .. }) = result else {
        return Err(format!("expected Mkdir, got {result:?}")).check("unexpected outcome");
    };
    assert!(
        path.ends_with("slugaudit"),
        "the path names the install dir: {path}"
    );
    Ok(())
}

/// A target that cannot be replaced fails the placement step — naming it —
/// and leaves what was there untouched, with no staged copy left behind.
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
        run_install,
    );

    let Err(InstallError::Place(error)) = result else {
        return Err(format!("expected Place, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(error.action, "retain the previous binary at");
    assert!(
        target.join("keep").exists(),
        "the target must be left in place"
    );
    let leftovers: Vec<_> = std::fs::read_dir(directory.path().join("slugaudit"))
        .check("list install dir")?
        .collect::<Result<Vec<_>, _>>()
        .check("read install dir entries")?
        .into_iter()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "the staged copy must not be left behind: {leftovers:?}"
    );
    Ok(())
}

#[test]
fn retaining_the_previous_binary_reports_a_stat_failure() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let file = directory.path().join("file");
    std::fs::write(&file, b"x").check("write file")?;

    let error = retain_previous_binary(&file.join("slugaudit"))
        .check_err("a target whose stat fails must not read as a first install")?;

    assert_ne!(error.kind(), std::io::ErrorKind::NotFound);
    Ok(())
}

/// Builds a nested directory path of exactly `target_len` bytes, with every
/// component within `NAME_MAX` (255) so the path itself is valid.
#[cfg(target_os = "linux")]
fn directory_path_of_length(base: &Path, target_len: usize) -> PathBuf {
    let base_len = base.as_os_str().len();
    assert!(
        target_len > base_len + 2,
        "target length must leave room for a component"
    );
    let available = target_len - base_len;
    // Each component contributes its bytes plus one separator; 256 is the
    // largest possible contribution (255-byte name + separator).
    let components = available.div_ceil(256);
    let names_total = available - components;
    let mut path = base.to_path_buf();
    for index in 0..components {
        let extra = usize::from(index < names_total % components);
        let name_len = names_total / components + extra;
        path.push("d".repeat(name_len));
    }
    assert_eq!(
        path.as_os_str().len(),
        target_len,
        "the helper must build a path of the requested length"
    );
    path
}

/// The staged copy is `<install dir>/.slugaudit.<pid>.<stamp>.tmp`. Linux's
/// `PATH_MAX` is 4096 bytes including the NUL, so an install directory just
/// under that limit is created successfully while the longer staged name
/// fails: the failure is reported as `Copy` naming the staged path.
#[cfg(target_os = "linux")]
#[test]
fn an_install_that_cannot_stage_the_binary_reports_the_copy_step() -> TestResult {
    // The install directory is `$SLUGTHUG_HOME/slugaudit`; keep it at 4090
    // bytes so its creation succeeds but the staged name (fixed suffix plus
    // pid and timestamp) exceeds the 4095-byte maximum.
    const INSTALL_DIR_LEN: usize = 4090;
    let directory = tempfile::tempdir().check("temp dir")?;
    let home_len = INSTALL_DIR_LEN - "slugaudit".len() - 1;
    let home = directory_path_of_length(directory.path(), home_len);
    std::fs::create_dir_all(&home).check("create the deep home")?;
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;

    let result = temp_env::with_var("SLUGTHUG_HOME", Some(home.as_os_str()), run_install);

    let Err(InstallError::Copy { path, .. }) = result else {
        return Err(format!("expected Copy, got {result:?}")).check("unexpected outcome");
    };
    assert!(
        Path::new(&path).starts_with(home.join("slugaudit")),
        "the reported path names the staged copy: {path}"
    );
    Ok(())
}

#[test]
fn a_failed_running_binary_lookup_is_the_current_exe_error() -> TestResult {
    let source = std::io::Error::new(std::io::ErrorKind::NotFound, "no executable path");

    let error = running_binary_path(Err(source)).check_err("a lookup failure must surface")?;

    let InstallError::CurrentExe(inner) = error else {
        return Err(format!("expected CurrentExe, got {error:?}")).check("unexpected outcome");
    };
    assert_eq!(inner.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(inner.to_string(), "no executable path");
    Ok(())
}

#[test]
fn a_pre_epoch_clock_is_the_typed_clock_error() -> TestResult {
    let before_epoch = std::time::UNIX_EPOCH - std::time::Duration::from_secs(1);

    let error =
        elapsed_since_epoch(before_epoch).check_err("a pre-epoch time cannot be stamped")?;

    assert!(
        matches!(error, InstallError::Clock(_)),
        "expected Clock, got {error:?}"
    );
    assert!(
        error.to_string().contains("system clock"),
        "the message must name the clock: {error}"
    );
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
    });

    assert!(
        !found.check("the install directory resolves")?,
        "an entry that is not the directory does not match"
    );
    assert!(
        logs.contains("could not canonicalize a PATH entry"),
        "{logs}"
    );
    Ok(())
}

#[test]
fn an_install_directory_that_cannot_be_resolved_is_an_error() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let file = temp.path().join("file");
    std::fs::write(&file, b"x").check("write file")?;
    let broken = file.join("child");

    let error = on_path(&broken).check_err("an unresolvable directory is an error")?;

    assert!(
        matches!(&error, InstallError::Canonicalize { path, .. } if *path == broken.display().to_string()),
        "{error:?}"
    );
    Ok(())
}
