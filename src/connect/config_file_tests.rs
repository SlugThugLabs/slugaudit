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
fn a_config_path_that_is_a_directory_reports_the_read_error() {
    let home = tempfile::tempdir().expect("temp dir");
    let config = home.path().join(".cursor/mcp.json");
    std::fs::create_dir_all(&config).expect("plant a directory at the config path");

    let result = connect_agent(&cursor(), Path::new("/bin/slugaudit"), Some(home.path()));

    let Err(ConnectError::Io(error)) = result else {
        panic!("expected Io, got {result:?}");
    };
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::IsADirectory,
        "the read failure's cause must reach the caller: {error}"
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
