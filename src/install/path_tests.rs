//! Tests for the shell PATH line written by `install`: which config file is
//! chosen, how the install directory is quoted for each shell, and that the
//! append is idempotent.

use super::*;
use crate::install::InstallError;
use crate::util::TEST_ENV_LOCK;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::path::Path;

#[test]
fn path_config_maps_bash_shell_to_bashrc_export() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || -> TestResult {
            let (config, line) =
                path_config(Path::new("/x/slugthug/slugaudit")).check("path config")?;
            assert_eq!(config, temp.path().join(".bashrc"));
            assert_eq!(line, "export PATH=\"/x/slugthug/slugaudit:$PATH\"");
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn path_config_maps_fish_shell_to_config_fish() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/usr/bin/fish"))),
        ],
        || -> TestResult {
            let (config, line) =
                path_config(Path::new("/x/slugthug/slugaudit")).check("path config")?;
            assert_eq!(config, temp.path().join(".config/fish/config.fish"));
            assert_eq!(line, "fish_add_path /x/slugthug/slugaudit");
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn install_path_unset_shell_is_refused_with_the_line_to_add() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", None::<&std::ffi::OsStr>),
        ],
        || -> TestResult {
            let error = path_config(Path::new("/x/slugaudit")).check_err("unset SHELL refused")?;
            assert!(
                matches!(error, InstallError::UnknownShell { .. }),
                "{error}"
            );
            assert!(
                error
                    .to_string()
                    .contains("export PATH=\"/x/slugaudit:$PATH\""),
                "{error}"
            );
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn install_path_unknown_shell_writes_no_rc_file() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/usr/bin/nushell"))),
        ],
        || -> TestResult {
            let error = add_to_path(&temp.path().join("slugaudit")).check_err("refused")?;
            assert!(
                matches!(error, InstallError::UnknownShell { .. }),
                "{error}"
            );
            let entries = std::fs::read_dir(temp.path()).check("read home")?.count();
            assert_eq!(entries, 0, "no rc file or temp file may be created");
            Ok(())
        },
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn install_path_append_keeps_mode_and_leaves_no_temp_file() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let config = temp.path().join(".bashrc");
    std::fs::write(&config, "alias a=b").check("seed rc")?;
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o640)).check("chmod")?;
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || add_to_path(&temp.path().join("slugaudit")).check("add"),
    )?;
    let mode = std::fs::metadata(&config)
        .check("stat")?
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o640);
    let contents = std::fs::read_to_string(&config).check("read rc")?;
    assert!(
        contents.starts_with("alias a=b\nexport PATH="),
        "{contents}"
    );
    assert_eq!(
        std::fs::read_dir(temp.path()).check("read home")?.count(),
        1
    );
    Ok(())
}

#[test]
fn path_config_escapes_a_bash_bin_dir_with_spaces_quotes_and_dollars() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home with a \"quote\" $USER");
    temp_env::with_vars(
        [
            ("HOME", Some(home.as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || -> TestResult {
            let bin_dir = home.join("slugthug").join("slugaudit");
            let (config, line) = path_config(&bin_dir).check("path config")?;
            assert_eq!(config, home.join(".bashrc"));
            let escaped = bin_dir
                .display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$");
            assert_eq!(line, format!("export PATH=\"{escaped}:$PATH\""));
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn path_config_quotes_a_fish_bin_dir_with_spaces_and_quotes() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home with a 'quote'");
    temp_env::with_vars(
        [
            ("HOME", Some(home.as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/usr/bin/fish"))),
        ],
        || -> TestResult {
            let bin_dir = home.join("slugaudit");
            let (config, line) = path_config(&bin_dir).check("path config")?;
            assert_eq!(config, home.join(".config/fish/config.fish"));
            let escaped = bin_dir
                .display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('\'', "\\'");
            assert_eq!(line, format!("fish_add_path '{escaped}'"));
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn add_to_path_is_idempotent() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let bin_dir = temp.path().join("slugaudit");
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || -> TestResult {
            add_to_path(&bin_dir).check("first add")?;
            let config = temp.path().join(".bashrc");
            let contents = std::fs::read_to_string(&config).check("read config")?;
            assert!(
                contents.contains("export PATH=\""),
                "the export line should have been appended"
            );

            add_to_path(&bin_dir).check("second add")?;
            let again = std::fs::read_to_string(&config).check("read config")?;
            assert_eq!(
                contents, again,
                "adding twice must not duplicate the export line"
            );
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn add_to_path_is_idempotent_for_a_home_with_spaces_and_quotes() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home with a \"quote\"");
    std::fs::create_dir_all(&home).check("create the special home")?;
    let bin_dir = home.join("slugaudit");
    temp_env::with_vars(
        [
            ("HOME", Some(home.as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || -> TestResult {
            add_to_path(&bin_dir).check("first add")?;
            let config = home.join(".bashrc");
            let contents = std::fs::read_to_string(&config).check("read config")?;

            add_to_path(&bin_dir).check("second add")?;

            assert_eq!(
                std::fs::read_to_string(&config).check("read config again")?,
                contents,
                "the escaped line must match itself on the next run"
            );
            assert_eq!(contents.lines().count(), 1, "one PATH line: {contents}");
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn add_to_path_reports_unreadable_shell_config() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    std::fs::create_dir(temp.path().join(".bashrc")).check("create config directory")?;
    let bin_dir = temp.path().join("slugaudit");
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || -> TestResult {
            let error = add_to_path(&bin_dir).check_err("a directory cannot be read as config")?;
            assert!(matches!(error, InstallError::Io(_)));
            Ok(())
        },
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_non_utf8_shell_variable_is_refused_instead_of_choosing_bashrc() -> TestResult {
    use std::os::unix::ffi::OsStringExt as _;
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let shell = std::ffi::OsString::from_vec(b"/bin/\xff".to_vec());

    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(shell.as_os_str())),
        ],
        || -> TestResult {
            let error =
                path_config(Path::new("/x/slugaudit")).check_err("non-UTF-8 SHELL fails")?;

            assert!(matches!(error, InstallError::ShellNotUnicode), "{error}");
            Ok(())
        },
    )?;
    Ok(())
}
