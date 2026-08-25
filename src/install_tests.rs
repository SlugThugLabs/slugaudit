//! Tests for `install`: home-directory discovery and the atomic
//! self-install flow. Env-sensitive tests (`SLUGTHUG_HOME` / `HOME`) are
//! scoped with `temp_env` (which restores the previous value afterwards)
//! and serialized behind `crate::util::TEST_ENV_LOCK` so parallel test
//! threads can't observe each other's mutation windows.

use super::*;
use crate::util::TEST_ENV_LOCK;
use std::path::PathBuf;

fn install_to(temp_dir: &tempfile::TempDir) -> PathBuf {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("SLUGTHUG_HOME", Some(temp_dir.path().as_os_str()), || {
        run_install().expect("install succeeds");
    });
    temp_dir.path().join("bin").join("slugaudit-mcp")
}

#[test]
fn slugthug_home_prefers_slugthug_home_over_home() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(temp.path().as_os_str())),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || assert_eq!(slugthug_home().expect("home resolves"), temp.path()),
    );
}

#[test]
fn slugthug_home_falls_back_to_home_slugthug() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", None),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || {
            assert_eq!(
                slugthug_home().expect("home resolves"),
                PathBuf::from("/somewhere/else/.slugthug")
            );
        },
    );
}

#[test]
fn slugthug_home_is_none_without_either_var() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", None::<&std::ffi::OsStr>),
            ("HOME", None::<&std::ffi::OsStr>),
        ],
        || assert_eq!(slugthug_home(), None),
    );
}

#[test]
fn run_install_copies_the_running_binary_to_a_stable_path() {
    let temp = tempfile::tempdir().expect("temp dir");
    let target = install_to(&temp);

    assert!(
        target.exists(),
        "the binary must land at $SLUGTHUG_HOME/bin/slugaudit-mcp"
    );
    let source = running_binary().expect("running binary");
    assert_ne!(target, source, "the install target differs from the source");
    assert_eq!(
        std::fs::read(&source).expect("read source"),
        std::fs::read(&target).expect("read target"),
        "the installed binary must match the running one byte-for-byte"
    );
}

#[cfg(unix)]
#[test]
fn installed_binary_is_executable() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("temp dir");
    let target = install_to(&temp);
    let mode = std::fs::metadata(&target)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o111,
        0o111,
        "the installed binary must be executable"
    );
}

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
            let (config, line) = path_config(Path::new("/x/slugthug/bin"))
                .expect("path config");
            assert_eq!(config, temp.path().join(".bashrc"));
            assert_eq!(line, "export PATH=\"/x/slugthug/bin:$PATH\"");
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
            let (config, line) = path_config(Path::new("/x/slugthug/bin"))
                .expect("path config");
            assert_eq!(config, temp.path().join(".config/fish/config.fish"));
            assert_eq!(line, "fish_add_path /x/slugthug/bin");
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
            let (config, line) = path_config(Path::new("/x/slugthug/bin"))
                .expect("path config");
            assert_eq!(config, temp.path().join(".bashrc"));
            assert!(line.contains("/x/slugthug/bin"));
        },
    );
}

#[test]
fn add_to_path_is_idempotent() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    let bin_dir = temp.path().join("bin");
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
