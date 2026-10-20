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
    temp_dir.path().join("slugaudit").join("slugaudit")
}

/// An install that displaces an existing binary keeps it beside the new one.
/// This is the whole reason an update is reversible without a download.
#[test]
fn installing_over_an_existing_binary_retains_the_previous_one() {
    let directory = tempfile::tempdir().expect("temp dir");
    let installed = install_to(&directory);
    std::fs::write(&installed, b"the previous build").expect("plant a previous binary");

    install_to(&directory);

    let previous = installed.with_file_name(PREVIOUS_BINARY_NAME);
    assert_eq!(
        std::fs::read(&previous).expect("the previous binary is retained"),
        b"the previous build",
        "the retained file must be the binary that was replaced, not the new one"
    );
}

/// The retained copy keeps the source's permission bits, so restoring it needs
/// no separate `chmod` — a revert path that yielded a non-executable binary
/// would be worse than no revert path.
#[cfg(unix)]
#[test]
fn a_retained_binary_keeps_the_permissions_of_the_binary_it_replaced() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("slugaudit");
    std::fs::write(&target, b"the previous build").expect("write the binary to replace");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let previous = retain_previous_binary(&target)
        .expect("retaining succeeds")
        .expect("there was a binary to retain");

    assert_eq!(
        previous.file_name().map(std::ffi::OsStr::to_owned),
        Some(std::ffi::OsString::from(PREVIOUS_BINARY_NAME)),
        "the retained copy sits beside the binary it replaces"
    );
    let mode = std::fs::metadata(&previous)
        .expect("stat the retained copy")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o755, "restoring it must not need a chmod");
}

/// A first install displaces nothing, so it retains nothing and creates no
/// stray file.
#[test]
fn a_first_install_has_nothing_to_retain() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("slugaudit");

    assert!(
        retain_previous_binary(&target)
            .expect("retaining succeeds")
            .is_none(),
        "no previous binary existed, so none is created"
    );
    assert!(!target.exists(), "and the target is not created either");
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
fn slugaudit_dir_resolves_under_slugthug_home() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    temp_env::with_var("SLUGTHUG_HOME", Some(temp.path().as_os_str()), || {
        assert_eq!(
            slugaudit_dir().expect("slugaudit dir resolves"),
            temp.path().join("slugaudit")
        );
    });
}

#[test]
fn config_home_is_the_user_home_not_the_slugthug_directory() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(std::ffi::OsStr::new("/slug/home"))),
            ("SLUGTHUG_CONFIG_HOME", None),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || {
            assert_eq!(
                config_home().expect("config home resolves"),
                PathBuf::from("/somewhere/else"),
                "client config must resolve against the user's home, never ~/.slugthug"
            );
        },
    );
}

#[test]
fn config_home_override_wins_over_home() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            (
                "SLUGTHUG_CONFIG_HOME",
                Some(std::ffi::OsStr::new("/agents/home")),
            ),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || {
            assert_eq!(
                config_home().expect("config home resolves"),
                PathBuf::from("/agents/home")
            );
        },
    );
}

#[test]
fn a_client_config_path_resolves_under_the_user_home() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(std::ffi::OsStr::new("/slug/home"))),
            ("SLUGTHUG_CONFIG_HOME", None),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || {
            assert_eq!(
                config_home()
                    .expect("config home resolves")
                    .join(".cursor/mcp.json"),
                PathBuf::from("/somewhere/else/.cursor/mcp.json"),
                "guards the regression where a client config landed in ~/.slugthug/ and the client never read it"
            );
        },
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
        "the binary must land at $SLUGTHUG_HOME/slugaudit/slugaudit"
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

/// With neither `SLUGTHUG_HOME` nor `HOME` there is no install location to
/// guess; the install names both variables instead of writing somewhere odd.
#[test]
fn an_install_without_any_home_variable_reports_no_home() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");

    let result = temp_env::with_vars_unset(["SLUGTHUG_HOME", "HOME"], run_install);

    assert!(
        matches!(result, Err(InstallError::NoHome)),
        "expected NoHome, got {result:?}"
    );
}

/// An install directory that cannot be created is reported with its path.
#[test]
fn an_install_home_that_is_a_file_reports_the_directory_it_could_not_create() {
    let directory = tempfile::tempdir().expect("temp dir");
    let home = directory.path().join("not-a-directory");
    std::fs::write(&home, b"a file where the home should be").expect("plant file");
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");

    let result = temp_env::with_var("SLUGTHUG_HOME", Some(home.as_os_str()), run_install);

    let Err(InstallError::Mkdir { path, .. }) = result else {
        panic!("expected Mkdir, got {result:?}");
    };
    assert!(
        path.ends_with("slugaudit"),
        "the path names the install dir: {path}"
    );
}

/// A target that cannot be replaced fails the placement step — naming it —
/// and leaves what was there untouched, with no staged copy left behind.
#[test]
fn an_install_target_that_is_a_directory_reports_the_failed_step_and_cleans_up() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("slugaudit").join("slugaudit");
    std::fs::create_dir_all(&target).expect("plant a directory at the target");
    std::fs::write(target.join("keep"), b"kept").expect("make it non-empty");
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");

    let result = temp_env::with_var(
        "SLUGTHUG_HOME",
        Some(directory.path().as_os_str()),
        run_install,
    );

    let Err(InstallError::Place(error)) = result else {
        panic!("expected Place, got {result:?}");
    };
    assert_eq!(error.action, "retain the previous binary at");
    assert!(
        target.join("keep").exists(),
        "the target must be left in place"
    );
    let leftovers: Vec<_> = std::fs::read_dir(directory.path().join("slugaudit"))
        .expect("list install dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "the staged copy must not be left behind: {leftovers:?}"
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

#[test]
fn retaining_the_previous_binary_reports_a_stat_failure() {
    let directory = tempfile::tempdir().expect("temp dir");
    let file = directory.path().join("file");
    std::fs::write(&file, b"x").expect("write file");

    let error = retain_previous_binary(&file.join("slugaudit"))
        .expect_err("a target whose stat fails must not read as a first install");

    assert_ne!(error.kind(), std::io::ErrorKind::NotFound);
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
fn an_install_that_cannot_stage_the_binary_reports_the_copy_step() {
    // The install directory is `$SLUGTHUG_HOME/slugaudit`; keep it at 4090
    // bytes so its creation succeeds but the staged name (fixed suffix plus
    // pid and timestamp) exceeds the 4095-byte maximum.
    const INSTALL_DIR_LEN: usize = 4090;
    let directory = tempfile::tempdir().expect("temp dir");
    let home_len = INSTALL_DIR_LEN - "slugaudit".len() - 1;
    let home = directory_path_of_length(directory.path(), home_len);
    std::fs::create_dir_all(&home).expect("create the deep home");
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");

    let result = temp_env::with_var("SLUGTHUG_HOME", Some(home.as_os_str()), run_install);

    let Err(InstallError::Copy { path, .. }) = result else {
        panic!("expected Copy, got {result:?}");
    };
    assert!(
        Path::new(&path).starts_with(home.join("slugaudit")),
        "the reported path names the staged copy: {path}"
    );
}

#[test]
fn an_uncanonicalizable_path_entry_is_compared_as_written_and_logged() {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    let file = temp.path().join("file");
    std::fs::write(&file, b"x").expect("write file");
    let broken_entry = file.join("child");

    let (found, logs) = crate::store::test_capture::capture_warns(|| {
        temp_env::with_var("PATH", Some(broken_entry.as_os_str()), || {
            on_path(&broken_entry)
        })
    });

    assert!(
        found,
        "an entry equal to the directory still matches as written"
    );
    assert!(
        logs.contains("could not canonicalize a PATH entry"),
        "{logs}"
    );
}
