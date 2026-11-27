//! Tests for the shell PATH line written by `install`: which config file is
//! chosen, how the install directory is quoted for each shell, newline
//! rejection, dotfile symlink preservation, and idempotency.

use super::*;
use crate::install::InstallError;
use crate::util::TEST_ENV_LOCK;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
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
            ensure_eq(
                &config,
                &temp.path().join(".bashrc"),
                "bash selects ~/.bashrc",
            )?;
            ensure_eq(
                &line,
                &"export PATH=\"/x/slugthug/slugaudit:$PATH\"",
                "bash formats double-quoted export PATH line",
            )?;
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
            ensure_eq(
                &config,
                &temp.path().join(".config/fish/config.fish"),
                "fish selects ~/.config/fish/config.fish",
            )?;
            ensure_eq(
                &line,
                &"fish_add_path /x/slugthug/slugaudit",
                "fish formats fish_add_path line",
            )?;
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
            ensure(
                matches!(error, InstallError::UnknownShell { .. }),
                format!("{error}"),
            )?;
            ensure(
                error
                    .to_string()
                    .contains("export PATH=\"/x/slugaudit:$PATH\""),
                format!("{error}"),
            )?;
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
            ensure(
                matches!(error, InstallError::UnknownShell { .. }),
                format!("{error}"),
            )?;
            let entries = std::fs::read_dir(temp.path()).check("read home")?.count();
            ensure_eq(&entries, &0, "no rc file or temp file may be created")?;
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
    ensure_eq(&mode, &0o640, "existing rc file keeps 0640 permissions")?;
    let contents = std::fs::read_to_string(&config).check("read rc")?;
    ensure(contents.starts_with("alias a=b\nexport PATH="), &contents)?;
    ensure_eq(
        &std::fs::read_dir(temp.path()).check("read home")?.count(),
        &1,
        "append leaves no stray temporary files in home",
    )?;
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
            ensure_eq(
                &config,
                &home.join(".bashrc"),
                "bash selects .bashrc under special home",
            )?;
            let escaped = bin_dir
                .display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$");
            ensure_eq(
                &line,
                &format!("export PATH=\"{escaped}:$PATH\""),
                "escapes backslashes, quotes, and dollar signs",
            )?;
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
            ensure_eq(
                &config,
                &home.join(".config/fish/config.fish"),
                "fish selects config.fish under special home",
            )?;
            let escaped = bin_dir
                .display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('\'', "\\'");
            ensure_eq(
                &line,
                &format!("fish_add_path '{escaped}'"),
                "single-quotes and escapes fish path",
            )?;
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
            ensure(
                contents.contains("export PATH=\""),
                "the export line should have been appended",
            )?;

            add_to_path(&bin_dir).check("second add")?;
            let again = std::fs::read_to_string(&config).check("read config")?;
            ensure_eq(
                &contents,
                &again,
                "adding twice must not duplicate the export line",
            )?;
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

            ensure_eq(
                &std::fs::read_to_string(&config).check("read config again")?,
                &contents,
                "the escaped line must match itself on the next run",
            )?;
            ensure_eq(
                &contents.lines().count(),
                &1,
                format!("one PATH line: {contents}"),
            )?;
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
            ensure(
                matches!(error, InstallError::Io(_)),
                "directory at .bashrc surfaces InstallError::Io",
            )?;
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

            ensure(
                matches!(error, InstallError::ShellNotUnicode),
                format!("{error}"),
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_non_utf8_bin_dir_is_refused_instead_of_writing_a_lossy_path_line() -> TestResult {
    use std::os::unix::ffi::OsStrExt as _;
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let bin_dir = Path::new(std::ffi::OsStr::from_bytes(b"/x/\xff/slugaudit"));

    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || -> TestResult {
            let error = path_config(bin_dir).check_err("non-UTF-8 bin dir fails")?;

            ensure(
                matches!(error, InstallError::BinDirNotUnicode { .. }),
                format!("{error}"),
            )?;
            Ok(())
        },
    )?;
    ensure(
        !temp.path().join(".bashrc").exists(),
        "no shell config may be written",
    )?;
    Ok(())
}

#[test]
fn path_config_rejects_bin_dir_containing_newlines_across_shells() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/fish", "/usr/bin/nushell"] {
        for bad_dir in ["/opt/slug\necho injected", "/opt/slug\recho injected"] {
            temp_env::with_vars(
                [
                    ("HOME", Some(temp.path().as_os_str())),
                    ("SHELL", Some(std::ffi::OsStr::new(shell))),
                ],
                || -> TestResult {
                    let error = add_to_path(Path::new(bad_dir))
                        .check_err("bin_dir with LF or CR must be rejected")?;
                    ensure(
                        matches!(error, InstallError::BinDirContainsNewline { .. }),
                        format!("expected BinDirContainsNewline for {shell}, got {error:?}"),
                    )?;
                    Ok(())
                },
            )?;
        }
    }
    ensure_eq(
        &std::fs::read_dir(temp.path()).check("read home")?.count(),
        &0,
        "newline rejection must not create any shell startup file",
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn add_to_path_preserves_valid_symlink_to_dotfiles() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let target = temp.path().join("dotfiles-bashrc");
    std::fs::write(&target, "# dotfiles\n").check("create valid target")?;
    let bashrc = temp.path().join(".bashrc");
    std::os::unix::fs::symlink(&target, &bashrc).check("create dotfiles symlink")?;

    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || -> TestResult {
            add_to_path(&temp.path().join("slugaudit"))
                .check("valid symlink target accepts PATH append")?;
            ensure(
                std::fs::symlink_metadata(&bashrc)
                    .check("stat symlink")?
                    .file_type()
                    .is_symlink(),
                ".bashrc symlink survives append",
            )?;
            let updated = std::fs::read_to_string(&target).check("read target")?;
            ensure(
                updated.contains("export PATH="),
                format!("target received PATH export: {updated}"),
            )?;
            Ok(())
        },
    )
}

#[test]
fn add_to_path_creates_missing_fish_config_directory() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let bin_dir = temp.path().join(".slugthug/slugaudit");
    temp_env::with_vars(
        [
            ("HOME", Some(temp.path().as_os_str())),
            ("SHELL", Some(std::ffi::OsStr::new("/usr/bin/fish"))),
        ],
        || -> TestResult {
            add_to_path(&bin_dir).check("fish add_to_path succeeds without pre-existing dir")?;
            let config = temp.path().join(".config/fish/config.fish");
            let contents = std::fs::read_to_string(&config).check("read fish config")?;
            ensure(
                contents.contains(&format!("fish_add_path {}", bin_dir.display())),
                format!("expected fish_add_path entry, got: {contents}"),
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn path_config_reports_shell_config_home_missing_when_home_is_unset() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(temp.path().as_os_str())),
            ("HOME", None::<&std::ffi::OsStr>),
            ("SHELL", Some(std::ffi::OsStr::new("/bin/bash"))),
        ],
        || -> TestResult {
            let error = path_config(&temp.path().join("slugaudit"))
                .check_err("unset HOME must fail shell config lookup")?;
            ensure(
                matches!(error, InstallError::ShellConfigHomeMissing),
                format!("expected ShellConfigHomeMissing, got {error:?}"),
            )?;
            let message = error.to_string();
            ensure(
                message.contains("HOME") && !message.contains("SLUGTHUG_HOME"),
                format!("message should name HOME without SLUGTHUG_HOME: {message}"),
            )?;
            Ok(())
        },
    )?;
    Ok(())
}
