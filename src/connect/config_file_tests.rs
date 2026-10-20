//! Behaviour tests for the client-config access layer: removal truthfulness,
//! owner-only atomic replacement, symlink preservation, and the typed I/O
//! failures `connect_agent` surfaces.

use super::*;
use crate::connect_agents::find_agent;
use crate::connect_exec::connect_agent;
use std::path::{Path, PathBuf};

fn cursor() -> crate::connect_agents::AgentDef {
    find_agent("cursor").expect("cursor is registered")
}

/// Writes an executable shell script named `name`: it appends its argv (one
/// line per call) to the file named by `FAKE_ARGV_LOG`, prints `FAKE_STDOUT`
/// when that is set, and exits with `exit_code`. Tests run it with `PATH`
/// replaced, never prepended, so no real client can be resolved.
#[cfg(unix)]
fn fake_cli(bin_dir: &Path, name: &str, exit_code: i32) {
    use std::os::unix::fs::PermissionsExt as _;
    let script = bin_dir.join(name);
    let body = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$FAKE_ARGV_LOG\"\n\
         if [ -n \"$FAKE_STDOUT\" ]; then printf '%s' \"$FAKE_STDOUT\"; fi\n\
         exit {exit_code}\n"
    );
    std::fs::write(&script, body).expect("write fake cli");
    let mut permissions = std::fs::metadata(&script)
        .expect("stat fake cli")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).expect("chmod fake cli");
}

/// Runs `f` with `PATH` pointing at exactly `bin_dir` and the fake CLI's argv
/// log at `log`; the inherited entries are dropped so a real client CLI can
/// neither be found nor executed.
#[cfg(unix)]
fn with_fake_cli<T>(bin_dir: &Path, log: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("PATH", Some(bin_dir.as_os_str())),
            ("FAKE_ARGV_LOG", Some(log.as_os_str())),
            ("FAKE_STDOUT", None::<&std::ffi::OsStr>),
        ],
        f,
    )
}

/// Counts the staging files an atomic write may have left behind.
fn temp_leftovers(directory: &Path) -> usize {
    std::fs::read_dir(directory)
        .expect("list the directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".slugaudit-tmp")
        })
        .count()
}

/// `mkfifo(1)` creates the fixture because std exposes no safe FIFO
/// constructor and `unsafe` is forbidden; it exists on both supported targets.
#[cfg(unix)]
fn create_fifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo creates the fixture: {status}");
}

/// Runs `f` on a thread so a regression to a blocking open fails the test
/// instead of hanging the whole suite.
#[cfg(unix)]
fn within_ten_seconds<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        sender.send(f()).expect("the test receives the result");
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the config probe must not block on a non-regular file")
}

#[test]
fn removing_an_existing_entry_reports_that_it_removed_one() {
    let directory = tempfile::tempdir().expect("temp dir");
    let config = directory.path().join("mcp.json");
    std::fs::write(
        &config,
        r#"{"mcpServers":{"slugaudit":{"command":"/bin/slugaudit"},"other":{}}}"#,
    )
    .expect("write config");

    let removed = remove_json_slugaudit(&config, &["mcpServers"]).expect("remove the entry");

    assert!(removed, "a real removal must be reported as one");
    let content = std::fs::read_to_string(&config).expect("read config");
    assert!(!content.contains("slugaudit"), "{content}");
    assert!(
        content.contains("other"),
        "other entries must survive: {content}"
    );
}

#[test]
fn a_config_without_the_entry_reports_nothing_removed_and_is_not_rewritten() {
    let directory = tempfile::tempdir().expect("temp dir");
    let config = directory.path().join("mcp.json");
    let original = "{\n  \"mcpServers\": {\"other\": {\"command\": \"/bin/other\"}}\n}";
    std::fs::write(&config, original).expect("write config");

    let removed = remove_json_slugaudit(&config, &["mcpServers", "mcp"]).expect("inspect config");

    assert!(
        !removed,
        "a file with no slugaudit entry must report no removal"
    );
    assert_eq!(
        std::fs::read_to_string(&config).expect("read config"),
        original,
        "a no-op must not rewrite (or reformat) the user's config"
    );
}

#[cfg(unix)]
#[test]
fn rewriting_a_0600_config_keeps_it_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let home = tempfile::tempdir().expect("temp dir");
    let config = home.path().join(".cursor/mcp.json");
    std::fs::create_dir_all(config.parent().expect("parent")).expect("mkdir");
    std::fs::write(&config, "{}").expect("write config");
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).expect("chmod");

    connect_agent(&cursor(), Path::new("/bin/slugaudit"), Some(home.path())).expect("connect");

    let mode = std::fs::metadata(&config)
        .expect("stat config")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "a credential-bearing config must stay owner-only"
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_config_is_written_through_and_the_link_survives() {
    use std::os::unix::fs::PermissionsExt as _;
    let home = tempfile::tempdir().expect("temp dir");
    let dotfiles = home.path().join("dotfiles");
    std::fs::create_dir_all(&dotfiles).expect("mkdir dotfiles");
    let target = dotfiles.join("mcp.json");
    std::fs::write(&target, "{}").expect("write target");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let config = home.path().join(".cursor/mcp.json");
    std::fs::create_dir_all(config.parent().expect("parent")).expect("mkdir");
    std::os::unix::fs::symlink(&target, &config).expect("symlink the dotfiles config");

    connect_agent(&cursor(), Path::new("/bin/slugaudit"), Some(home.path())).expect("connect");

    let metadata = std::fs::symlink_metadata(&config).expect("stat the link");
    assert!(
        metadata.file_type().is_symlink(),
        "the dotfiles link must survive the write"
    );
    let content = std::fs::read_to_string(&target).expect("read the link target");
    assert!(content.contains("slugaudit"), "{content}");
    let mode = std::fs::metadata(&target)
        .expect("stat target")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "the resolved target keeps the owner-only mode");
}

#[cfg(unix)]
#[test]
fn a_non_utf8_binary_path_is_refused_before_the_config_is_written() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    let home = tempfile::tempdir().expect("temp dir");
    let binary = PathBuf::from(OsString::from_vec(b"/bin/slug\xffaudit".to_vec()));

    let result = connect_agent(&cursor(), &binary, Some(home.path()));

    let Err(ConnectError::BinaryPath(error)) = result else {
        panic!("expected BinaryPath, got {result:?}");
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
}

#[test]
fn a_config_path_that_is_a_directory_is_refused_before_reading() {
    let home = tempfile::tempdir().expect("temp dir");
    let config = home.path().join(".cursor/mcp.json");
    std::fs::create_dir_all(&config).expect("plant a directory at the config path");

    let result = connect_agent(&cursor(), Path::new("/bin/slugaudit"), Some(home.path()));

    let Err(ConnectError::Io(error)) = result else {
        panic!("expected Io, got {result:?}");
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
}

#[test]
fn a_config_parent_that_is_a_file_reports_the_create_directory_error() {
    let home = tempfile::tempdir().expect("temp dir");
    std::fs::write(home.path().join(".cursor"), b"x").expect("plant a file at the parent");

    let result = connect_agent(&cursor(), Path::new("/bin/slugaudit"), Some(home.path()));

    let Err(ConnectError::Io(error)) = result else {
        panic!("expected Io, got {result:?}");
    };
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::AlreadyExists,
        "the mkdir failure's cause must reach the caller: {error}"
    );
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
fn a_non_utf8_binary_path_is_refused_before_the_client_cli_runs() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    let temp = tempfile::tempdir().expect("temp dir");
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    let log = temp.path().join("pi-argv.log");
    fake_cli(&bin_dir, "pi", 0);
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let binary = PathBuf::from(OsString::from_vec(b"/bin/slug\xffaudit".to_vec()));
    let pi = find_agent("pi").expect("pi is registered");

    let result = with_fake_cli(&bin_dir, &log, || connect_agent(&pi, &binary, Some(&home)));

    let Err(ConnectError::BinaryPath(error)) = result else {
        panic!("expected BinaryPath, got {result:?}");
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
}

/// A lossy rendering would register a command with U+FFFD, which no client can
/// launch; guidance must carry the refusal text instead.
#[cfg(unix)]
#[test]
fn a_non_utf8_binary_path_is_not_rendered_lossily_in_guidance() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    let binary = PathBuf::from(OsString::from_vec(b"/bin/slug\xffaudit".to_vec()));

    for id in ["goose", "cursor"] {
        let agent = find_agent(id).expect(id);

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
}

/// A client that explains its rejection on stdout still leaves output to look
/// at above the error, so it must not be reported as silent.
#[cfg(unix)]
#[test]
fn an_add_failure_printed_only_to_stdout_is_not_reported_as_silent() {
    let temp = tempfile::tempdir().expect("temp dir");
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    let log = temp.path().join("pi-argv.log");
    fake_cli(&bin_dir, "pi", 1);
    let args = ["mcp", "add", "slugaudit", "--", "/bin/slugaudit"].map(str::to_string);

    let result = with_fake_cli(&bin_dir, &log, || {
        temp_env::with_var("FAKE_STDOUT", Some("the client rejected it"), || {
            crate::connect_cli::run_add("pi", &args)
        })
    });

    let Err(ConnectError::AddFailed { cli, .. }) = result else {
        panic!("stdout-only output must be reported as visible: {result:?}");
    };
    assert_eq!(cli, "pi");
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

/// A file sync failure must fail the write before the rename, leaving the
/// destination bytes untouched and no staging file behind.
#[test]
fn a_config_write_whose_file_sync_fails_leaves_the_destination_untouched() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("mcp.json");
    std::fs::write(&path, b"original").expect("pre-write the destination");

    let error = write_atomic_with(
        &path,
        b"replacement",
        |_| Err(std::io::Error::other("injected file sync failure")),
        |_| Ok(()),
    )
    .expect_err("a failed file sync must fail the write");

    let ConnectError::Io(error) = error else {
        panic!("expected Io, got {error:?}");
    };
    assert_eq!(error.to_string(), "injected file sync failure");
    assert_eq!(
        std::fs::read(&path).expect("read destination"),
        b"original",
        "a failed sync must leave the destination untouched"
    );
    assert_eq!(temp_leftovers(directory.path()), 0, "no temp may remain");
}

/// The rename is already committed when the directory sync runs, so its
/// failure is logged once and the visible write still succeeds.
#[test]
fn a_directory_sync_failure_after_the_rename_is_logged_and_kept() {
    const SENTENCE: &str = "client config renamed but its directory was not synced; the file is on disk, but the rename may not survive a crash";
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("mcp.json");

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        write_atomic_with(
            &path,
            b"replacement",
            |file| file.sync_all(),
            |_| Err(std::io::Error::other("injected directory failure")),
        )
    });

    result.expect("the committed write must still be reported as successful");
    assert_eq!(
        std::fs::read(&path).expect("read destination"),
        b"replacement",
        "the renamed file must remain on disk"
    );
    assert_eq!(temp_leftovers(directory.path()), 0, "no temp may remain");
    assert_eq!(
        logs.lines().filter(|line| line.contains(SENTENCE)).count(),
        1,
        "the post-commit failure must be logged exactly once: {logs}"
    );
}

/// Opening a FIFO blocks until a writer appears, so a connect must refuse it
/// from metadata alone and leave it untouched.
#[cfg(unix)]
#[test]
fn a_fifo_config_is_refused_by_a_connect_without_blocking() {
    use std::os::unix::fs::FileTypeExt as _;
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home");
    std::fs::create_dir_all(home.join(".cursor")).expect("cursor dir");
    let config = home.join(".cursor/mcp.json");
    create_fifo(&config);
    let cursor = find_agent("cursor").expect("cursor is registered");
    let probe_home = home.clone();

    let result = within_ten_seconds(move || {
        connect_agent(&cursor, Path::new("/bin/slugaudit"), Some(&probe_home))
    });

    let Err(ConnectError::Io(error)) = result else {
        panic!("expected Io, got {result:?}");
    };
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::InvalidInput,
        "the FIFO must be refused before the read: {error}"
    );
    assert!(
        error.to_string().contains("not a regular file"),
        "the refusal must name the file type: {error}"
    );
    assert!(
        config
            .symlink_metadata()
            .expect("stat the FIFO")
            .file_type()
            .is_fifo(),
        "the FIFO is left untouched"
    );
}

/// Detection probes the config path too, so cursor and goose must both judge a
/// FIFO from metadata instead of opening it.
#[cfg(unix)]
#[test]
fn a_fifo_config_is_not_opened_by_connection_detection() {
    use std::os::unix::fs::FileTypeExt as _;
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home");
    std::fs::create_dir_all(home.join(".cursor")).expect("cursor dir");
    std::fs::create_dir_all(home.join(".config/goose")).expect("goose dir");
    let cursor_fifo = home.join(".cursor/mcp.json");
    let goose_fifo = home.join(".config/goose/config.yaml");
    create_fifo(&cursor_fifo);
    create_fifo(&goose_fifo);
    let empty = tempfile::tempdir().expect("empty bin dir");
    let cursor = find_agent("cursor").expect("cursor is registered");
    let goose = find_agent("goose").expect("goose is registered");
    let cursor_home = home.clone();
    let goose_home = home.clone();

    let (cursor_connected, goose_connected) =
        temp_env::with_var("PATH", Some(empty.path().as_os_str()), || {
            (
                within_ten_seconds(move || cursor.is_connected(Some(&cursor_home))),
                within_ten_seconds(move || goose.is_connected(Some(&goose_home))),
            )
        });

    assert!(!cursor_connected, "a FIFO must not read as connected");
    assert!(!goose_connected, "a FIFO must not read as connected");
    for fifo in [&cursor_fifo, &goose_fifo] {
        assert!(
            fifo.symlink_metadata()
                .expect("stat the FIFO")
                .file_type()
                .is_fifo(),
            "the FIFO is left untouched"
        );
    }
    let (readable, logs) =
        crate::store::test_capture::capture_warns(|| readable_config_or_warn(&cursor_fifo));
    assert_eq!(readable, None, "a non-regular config has no contents");
    assert!(
        logs.contains("could not inspect client config"),
        "the refusal must be logged once: {logs}"
    );
}

/// The config path itself is valid, but the temp name appended beside it
/// crosses Linux's `PATH_MAX`, so creating the staging file fails. The
/// failure is reported as the typed I/O error and no temp is left behind.
#[cfg(target_os = "linux")]
#[test]
fn a_config_write_whose_temp_path_exceeds_the_limit_fails_without_a_stray_temp() {
    // `home/.cursor/mcp.json` adds 17 bytes to the home; keeping the home at
    // 4075 puts the target at 4092 (valid) while every temp name
    // (`.slugaudit-tmp-<pid>-<count>`) pushes the path past 4095.
    const HOME_LEN: usize = 4075;
    let directory = tempfile::tempdir().expect("temp dir");
    let home = directory_path_of_length(directory.path(), HOME_LEN);
    let config = home.join(".cursor/mcp.json");
    assert!(
        config.as_os_str().len() < 4096,
        "the target path must be valid"
    );

    let result = connect_agent(&cursor(), Path::new("/bin/slugaudit"), Some(&home));

    let Err(ConnectError::Io(error)) = result else {
        panic!("expected Io, got {result:?}");
    };
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::InvalidFilename,
        "the staging failure's cause must reach the caller: {error}"
    );
    let leftovers: Vec<_> = std::fs::read_dir(config.parent().expect("parent"))
        .expect("list the config directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".slugaudit-tmp")
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "a failed write must not leave a temp behind: {leftovers:?}"
    );
}
