//! Tests for the shell PATH line written by `install`: which config file is
//! chosen, how the install directory is quoted for each shell, and that the
//! append is idempotent.

use super::*;
use crate::install::InstallError;
use crate::util::TEST_ENV_LOCK;
use std::path::Path;

#[test]
fn path_config_maps_bash_shell_to_bashrc_export() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || {
            let (config, line) =
                path_config(Path::new("/x/slugthug/slugaudit")).expect("path config");
            assert_eq!(config, temp.path().join(".bashrc"));
            assert_eq!(line, "export PATH=\"/x/slugthug/slugaudit:$PATH\"");
        },
    );
}

#[test]
fn path_config_maps_fish_shell_to_config_fish() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/usr/bin/fish"))),
        ],
        || {
            let (config, line) =
                path_config(Path::new("/x/slugthug/slugaudit")).expect("path config");
            assert_eq!(config, temp.path().join(".config/fish/config.fish"));
            assert_eq!(line, "fish_add_path /x/slugthug/slugaudit");
        },
    );
}

#[test]
fn path_config_defaults_to_bashrc_without_shell() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", None::<&std::ffi::OsStr>),
        ],
        || {
            let (config, line) =
                path_config(Path::new("/x/slugthug/slugaudit")).expect("path config");
            assert_eq!(config, temp.path().join(".bashrc"));
            assert!(line.contains("/x/slugthug/slugaudit"));
        },
    );
}

#[test]
fn path_config_escapes_a_bash_bin_dir_with_spaces_quotes_and_dollars() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home with a \"quote\" $USER");
    temp_env::with_vars(
        [
            ("HOME", Some(home.as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || {
            let bin_dir = home.join("slugthug").join("slugaudit");
            let (config, line) = path_config(&bin_dir).expect("path config");
            assert_eq!(config, home.join(".bashrc"));
            let escaped = bin_dir
                .display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$");
            assert_eq!(line, format!("export PATH=\"{escaped}:$PATH\""));
        },
    );
}

#[test]
fn path_config_quotes_a_fish_bin_dir_with_spaces_and_quotes() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home with a 'quote'");
    temp_env::with_vars(
        [
            ("HOME", Some(home.as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/usr/bin/fish"))),
        ],
        || {
            let bin_dir = home.join("slugaudit");
            let (config, line) = path_config(&bin_dir).expect("path config");
            assert_eq!(config, home.join(".config/fish/config.fish"));
            let escaped = bin_dir
                .display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('\'', "\\'");
            assert_eq!(line, format!("fish_add_path '{escaped}'"));
        },
    );
}

#[test]
fn add_to_path_is_idempotent() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    let bin_dir = temp.path().join("slugaudit");
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || {
            add_to_path(&bin_dir).expect("first add");
            let config = temp.path().join(".bashrc");
            let contents = std::fs::read_to_string(&config).expect("read config");
            assert!(
                contents.contains("export PATH=\""),
                "the export line should have been appended"
            );

            add_to_path(&bin_dir).expect("second add");
            let again = std::fs::read_to_string(&config).expect("read config");
            assert_eq!(
                contents, again,
                "adding twice must not duplicate the export line"
            );
        },
    );
}

#[test]
fn add_to_path_is_idempotent_for_a_home_with_spaces_and_quotes() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home with a \"quote\"");
    std::fs::create_dir_all(&home).expect("create the special home");
    let bin_dir = home.join("slugaudit");
    temp_env::with_vars(
        [
            ("HOME", Some(home.as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || {
            add_to_path(&bin_dir).expect("first add");
            let config = home.join(".bashrc");
            let contents = std::fs::read_to_string(&config).expect("read config");

            add_to_path(&bin_dir).expect("second add");

            assert_eq!(
                std::fs::read_to_string(&config).expect("read config again"),
                contents,
                "the escaped line must match itself on the next run"
            );
            assert_eq!(contents.lines().count(), 1, "one PATH line: {contents}");
        },
    );
}

#[test]
fn add_to_path_reports_unreadable_shell_config() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir(temp.path().join(".bashrc")).expect("create config directory");
    let bin_dir = temp.path().join("slugaudit");
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || {
            let error = add_to_path(&bin_dir).expect_err("a directory cannot be read as config");
            assert!(matches!(error, InstallError::Io(_)));
        },
    );
}

#[cfg(unix)]
#[test]
fn a_non_utf8_shell_variable_is_refused_instead_of_choosing_bashrc() {
    use std::os::unix::ffi::OsStringExt as _;
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    let shell = std::ffi::OsString::from_vec(b"/bin/\xff".to_vec());

    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(shell.as_os_str())),
        ],
        || {
            let error = path_config(Path::new("/x/slugaudit")).expect_err("non-UTF-8 SHELL fails");

            assert!(matches!(error, InstallError::ShellNotUnicode), "{error}");
        },
    );
}
