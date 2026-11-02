//! Behaviour tests for the client-config access layer: removal truthfulness,
//! owner-only atomic replacement, symlink preservation, and the typed I/O
//! failures `connect_agent` surfaces.

use super::*;
use crate::connect_agents::find_agent;
use crate::connect_exec::{DisconnectOutcome, connect_agent, disconnect_agent};
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::path::{Path, PathBuf};

fn cursor() -> TestResult<crate::connect_agents::AgentDef> {
    find_agent("cursor").check("cursor is registered")
}

/// Writes an executable shell script named `name`: it appends its argv (one
/// line per call) to the file named by `FAKE_ARGV_LOG`, prints `FAKE_STDOUT`
/// when that is set, and exits with `exit_code`. Tests run it with `PATH`
/// replaced, never prepended, so no real client can be resolved.
#[cfg(unix)]
fn fake_cli(bin_dir: &Path, name: &str, exit_code: i32) -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let script = bin_dir.join(name);
    let body = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$FAKE_ARGV_LOG\"\n\
         if [ -n \"$FAKE_STDOUT\" ]; then printf '%s' \"$FAKE_STDOUT\"; fi\n\
         exit {exit_code}\n"
    );
    std::fs::write(&script, body).check("write fake cli")?;
    let mut permissions = std::fs::metadata(&script)
        .check("stat fake cli")?
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).check("chmod fake cli")?;
    Ok(())
}

/// Runs `f` with `PATH` pointing at exactly `bin_dir` and the fake CLI's argv
/// log at `log`; the inherited entries are dropped so a real client CLI can
/// neither be found nor executed.
#[cfg(unix)]
fn with_fake_cli<T>(bin_dir: &Path, log: &Path, f: impl FnOnce() -> T) -> TestResult<T> {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    Ok(temp_env::with_vars(
        [
            ("PATH", Some(bin_dir.as_os_str())),
            ("FAKE_ARGV_LOG", Some(log.as_os_str())),
            ("FAKE_STDOUT", None::<&std::ffi::OsStr>),
        ],
        f,
    ))
}

/// Counts the staging files an atomic write may have left behind.
fn temp_leftovers(directory: &Path) -> TestResult<usize> {
    let mut count = 0;
    for entry in std::fs::read_dir(directory).check("list the directory")? {
        let entry = entry.check("read a directory entry")?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(".slugaudit-tmp")
        {
            count += 1;
        }
    }
    Ok(count)
}

#[test]
fn removing_an_existing_entry_reports_that_it_removed_one() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let config = directory.path().join("mcp.json");
    std::fs::write(
        &config,
        r#"{"mcpServers":{"slugaudit":{"command":"/bin/slugaudit"},"other":{}}}"#,
    )
    .check("write config")?;

    let removed = remove_json_slugaudit(&config, &["mcpServers"]).check("remove the entry")?;

    assert!(removed, "a real removal must be reported as one");
    let content = std::fs::read_to_string(&config).check("read config")?;
    assert!(!content.contains("slugaudit"), "{content}");
    assert!(
        content.contains("other"),
        "other entries must survive: {content}"
    );
    Ok(())
}

#[test]
fn a_config_without_the_entry_reports_nothing_removed_and_is_not_rewritten() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let config = directory.path().join("mcp.json");
    let original = "{\n  \"mcpServers\": {\"other\": {\"command\": \"/bin/other\"}}\n}";
    std::fs::write(&config, original).check("write config")?;

    let removed = remove_json_slugaudit(&config, &["mcpServers", "mcp"]).check("inspect config")?;

    assert!(
        !removed,
        "a file with no slugaudit entry must report no removal"
    );
    assert_eq!(
        std::fs::read_to_string(&config).check("read config")?,
        original,
        "a no-op must not rewrite (or reformat) the user's config"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn rewriting_a_0600_config_keeps_it_owner_only() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let home = tempfile::tempdir().check("temp dir")?;
    let config = home.path().join(".cursor/mcp.json");
    std::fs::create_dir_all(config.parent().check("parent")?).check("mkdir")?;
    std::fs::write(&config, "{}").check("write config")?;
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).check("chmod")?;

    connect_agent(&cursor()?, Path::new("/bin/slugaudit"), Some(home.path())).check("connect")?;

    let mode = std::fs::metadata(&config)
        .check("stat config")?
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "a credential-bearing config must stay owner-only"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_symlinked_config_is_written_through_and_the_link_survives() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let home = tempfile::tempdir().check("temp dir")?;
    let dotfiles = home.path().join("dotfiles");
    std::fs::create_dir_all(&dotfiles).check("mkdir dotfiles")?;
    let target = dotfiles.join("mcp.json");
    std::fs::write(&target, "{}").check("write target")?;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).check("chmod")?;
    let config = home.path().join(".cursor/mcp.json");
    std::fs::create_dir_all(config.parent().check("parent")?).check("mkdir")?;
    std::os::unix::fs::symlink(&target, &config).check("symlink the dotfiles config")?;

    connect_agent(&cursor()?, Path::new("/bin/slugaudit"), Some(home.path())).check("connect")?;

    let metadata = std::fs::symlink_metadata(&config).check("stat the link")?;
    assert!(
        metadata.file_type().is_symlink(),
        "the dotfiles link must survive the write"
    );
    let content = std::fs::read_to_string(&target).check("read the link target")?;
    assert!(content.contains("slugaudit"), "{content}");
    let mode = std::fs::metadata(&target)
        .check("stat target")?
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "the resolved target keeps the owner-only mode");
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_non_utf8_binary_path_is_refused_before_the_config_is_written() -> TestResult {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    let home = tempfile::tempdir().check("temp dir")?;
    let binary = PathBuf::from(OsString::from_vec(b"/bin/slug\xffaudit".to_vec()));

    let result = connect_agent(&cursor()?, &binary, Some(home.path()));

    let Err(ConnectError::BinaryPath(error)) = result else {
        return Err(format!("expected BinaryPath, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::InvalidInput,
        "the refusal must say the path is unusable: {error}"
    );
    assert!(
        !home.path().join(".cursor/mcp.json").exists(),
        "the refusal must happen before any write"
    );
    Ok(())
}

#[test]
fn a_config_path_that_is_a_directory_is_refused_before_reading() -> TestResult {
    let home = tempfile::tempdir().check("temp dir")?;
    let config = home.path().join(".cursor/mcp.json");
    std::fs::create_dir_all(&config).check("plant a directory at the config path")?;

    let result = connect_agent(&cursor()?, Path::new("/bin/slugaudit"), Some(home.path()));

    let Err(ConnectError::Io(error)) = result else {
        return Err(format!("expected Io, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::InvalidInput,
        "a non-regular config must be refused before the read: {error}"
    );
    assert!(
        error.to_string().contains("not a regular file"),
        "the refusal must name the file type: {error}"
    );
    Ok(())
}

#[test]
fn a_config_parent_that_is_a_file_reports_the_create_directory_error() -> TestResult {
    let home = tempfile::tempdir().check("temp dir")?;
    std::fs::write(home.path().join(".cursor"), b"x").check("plant a file at the parent")?;

    let result = connect_agent(&cursor()?, Path::new("/bin/slugaudit"), Some(home.path()));

    let Err(ConnectError::Io(error)) = result else {
        return Err(format!("expected Io, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::AlreadyExists,
        "the mkdir failure's cause must reach the caller: {error}"
    );
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

/// The client's CLI rejects an argv containing a raw non-UTF-8 byte, so a path
/// no client can represent must be refused before the client is executed or a
/// config file is written.
#[cfg(unix)]
#[test]
fn a_non_utf8_binary_path_is_refused_before_the_client_cli_runs() -> TestResult {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    let temp = tempfile::tempdir().check("temp dir")?;
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).check("bin dir")?;
    let log = temp.path().join("pi-argv.log");
    fake_cli(&bin_dir, "pi", 0)?;
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).check("home")?;
    let binary = PathBuf::from(OsString::from_vec(b"/bin/slug\xffaudit".to_vec()));
    let pi = find_agent("pi").check("pi is registered")?;

    let result = with_fake_cli(&bin_dir, &log, || connect_agent(&pi, &binary, Some(&home)))?;

    let Err(ConnectError::BinaryPath(error)) = result else {
        return Err(format!("expected BinaryPath, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::InvalidInput,
        "the refusal must name the unusable path: {error}"
    );
    assert!(
        error.to_string().contains("not valid UTF-8"),
        "the refusal must say why: {error}"
    );
    assert!(
        !log.exists(),
        "the client CLI must not run before the path is refused"
    );
    assert!(
        !home.join(".pi/agent/mcp.json").exists(),
        "the refusal must happen before any config write"
    );
    Ok(())
}

/// A lossy rendering would register a command with U+FFFD, which no client can
/// launch; guidance must carry the refusal text instead.
#[cfg(unix)]
#[test]
fn a_non_utf8_binary_path_is_not_rendered_lossily_in_guidance() -> TestResult {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    let binary = PathBuf::from(OsString::from_vec(b"/bin/slug\xffaudit".to_vec()));

    for id in ["goose", "cursor"] {
        let agent = find_agent(id).check(id)?;

        let text = crate::connect_guidance::refusal(&agent, &binary, None).to_string();

        assert!(
            text.contains("not valid UTF-8"),
            "{id} must report the unusable path: {text}"
        );
        assert!(
            !text.contains('\u{FFFD}'),
            "{id} must not render a lossy replacement character: {text}"
        );
    }
    Ok(())
}

/// A client that explains its rejection on stdout still leaves output to look
/// at above the error, so it must not be reported as silent.
#[cfg(unix)]
#[test]
fn an_add_failure_printed_only_to_stdout_is_not_reported_as_silent() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).check("bin dir")?;
    let log = temp.path().join("pi-argv.log");
    fake_cli(&bin_dir, "pi", 1)?;
    let args = ["mcp", "add", "slugaudit", "--", "/bin/slugaudit"].map(str::to_string);

    let result = with_fake_cli(&bin_dir, &log, || {
        temp_env::with_var("FAKE_STDOUT", Some("the client rejected it"), || {
            crate::connect_cli::run_add("pi", &args)
        })
    })?;

    let Err(ConnectError::AddFailed { cli, .. }) = result else {
        return Err(format!(
            "stdout-only output must be reported as visible: {result:?}"
        ))
        .check("unexpected outcome");
    };
    assert_eq!(cli, "pi");
    Ok(())
}

/// A closed pipe while relaying the client's output must not replace the
/// failure that produced it.
#[test]
fn a_failed_failure_output_print_keeps_the_source_error() {
    let source = ConnectError::AddFailed {
        cli: "claude".into(),
        status: "exit status: 1".into(),
    };
    let print_failure = Err(ConnectError::Io(std::io::ErrorKind::BrokenPipe.into()));

    let (kept, logs) = crate::store::test_capture::capture_warns(|| {
        crate::connect_guidance::keep_source_error(source, print_failure)
    });

    assert!(
        kept.to_string().contains("see the output above"),
        "the source error must survive the failed print: {kept}"
    );
    assert!(
        logs.contains("could not print the failure output"),
        "the lost output must be logged: {logs}"
    );
}

/// The CLI remove route never needs a home, so a missing home must not suppress
/// the installed client's own `mcp remove`.
#[cfg(unix)]
#[test]
fn disconnecting_without_a_home_still_runs_the_installed_client_remove() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).check("bin dir")?;
    let log = temp.path().join("claude-argv.log");
    fake_cli(&bin_dir, "claude", 0)?;
    let claude = find_agent("claude").check("claude is registered")?;

    let outcome = with_fake_cli(&bin_dir, &log, || disconnect_agent(&claude, None))?
        .check("disconnect without a home")?;

    assert_eq!(outcome, DisconnectOutcome::Disconnected);
    assert_eq!(
        std::fs::read_to_string(&log).check("read argv log")?,
        "mcp remove -s user slugaudit\n",
        "the installed client must run its own remove even without a home"
    );
    Ok(())
}

/// A file sync failure must fail the write before the rename, leaving the
/// destination bytes untouched and no staging file behind.
#[test]
fn a_config_write_whose_file_sync_fails_leaves_the_destination_untouched() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("mcp.json");
    std::fs::write(&path, b"original").check("pre-write the destination")?;

    let error = write_atomic_with(
        &path,
        b"replacement",
        |_| Err(std::io::Error::other("injected file sync failure")),
        |_| Ok(()),
    )
    .check_err("a failed file sync must fail the write")?;

    let ConnectError::Io(error) = error else {
        return Err(format!("expected Io, got {error:?}")).check("unexpected outcome");
    };
    assert_eq!(error.to_string(), "injected file sync failure");
    assert_eq!(
        std::fs::read(&path).check("read destination")?,
        b"original",
        "a failed sync must leave the destination untouched"
    );
    assert_eq!(temp_leftovers(directory.path())?, 0, "no temp may remain");
    Ok(())
}

/// The rename is already committed when the directory sync runs, so its
/// failure is logged once and the visible write still succeeds.
#[test]
fn a_directory_sync_failure_after_the_rename_is_logged_and_kept() -> TestResult {
    const SENTENCE: &str = "client config renamed but its directory was not synced; the file is on disk, but the rename may not survive a crash";
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("mcp.json");

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        write_atomic_with(&path, b"replacement", std::fs::File::sync_all, |_| {
            Err(std::io::Error::other("injected directory failure"))
        })
    });

    result.check("the committed write must still be reported as successful")?;
    assert_eq!(
        std::fs::read(&path).check("read destination")?,
        b"replacement",
        "the renamed file must remain on disk"
    );
    assert_eq!(temp_leftovers(directory.path())?, 0, "no temp may remain");
    assert_eq!(
        logs.lines().filter(|line| line.contains(SENTENCE)).count(),
        1,
        "the post-commit failure must be logged exactly once: {logs}"
    );
    Ok(())
}

/// The config path itself is valid, but the temp name appended beside it
/// crosses Linux's `PATH_MAX`, so creating the staging file fails. The
/// failure is reported as the typed I/O error and no temp is left behind.
#[cfg(target_os = "linux")]
#[test]
fn a_config_write_whose_temp_path_exceeds_the_limit_fails_without_a_stray_temp() -> TestResult {
    // `home/.cursor/mcp.json` adds 17 bytes to the home; keeping the home at
    // 4075 puts the target at 4092 (valid) while every temp name
    // (`.slugaudit-tmp-<pid>-<count>`) pushes the path past 4095.
    const HOME_LEN: usize = 4075;
    let directory = tempfile::tempdir().check("temp dir")?;
    let home = directory_path_of_length(directory.path(), HOME_LEN);
    let config = home.join(".cursor/mcp.json");
    assert!(
        config.as_os_str().len() < 4096,
        "the target path must be valid"
    );

    let result = connect_agent(&cursor()?, Path::new("/bin/slugaudit"), Some(&home));

    let Err(ConnectError::Io(error)) = result else {
        return Err(format!("expected Io, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::InvalidFilename,
        "the staging failure's cause must reach the caller: {error}"
    );
    assert_eq!(
        temp_leftovers(config.parent().check("parent")?)?,
        0,
        "a failed write must not leave a temp behind"
    );
    Ok(())
}

#[path = "config_file_fifo_tests.rs"]
mod fifo_tests;
