//! The one write path of `config.json`: key-preserving read-modify-write,
//! refusal of anything it cannot read back, the atomic replace (a failure
//! before the rename leaves the old file), the symlink re-check right before
//! the rename, and the cross-process lock (bounded wait, no lost update).

use super::*;
use std::fs;
use std::sync::Arc;

fn project(contents: Option<&str>) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning/slugaudit")).expect("activation dir");
    if let Some(contents) = contents {
        fs::write(config_path(project.path()), contents).expect("write config");
    }
    project
}

fn read_config(project: &tempfile::TempDir) -> String {
    fs::read_to_string(config_path(project.path())).expect("read config")
}

fn set(
    key: &'static str,
    value: Value,
) -> impl FnOnce(&mut Map<String, Value>) -> Result<(), ConfigWriteError> {
    move |config| {
        config.insert(key.to_owned(), value);
        Ok(())
    }
}

/// Every name in the activation directory that is not `config.json` or
/// `config.lock` — a leftover temp file shows up here.
fn strays(project: &tempfile::TempDir) -> Vec<String> {
    fs::read_dir(project.path().join(".planning/slugaudit"))
        .expect("list activation dir")
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name != "config.json" && name != "config.lock")
        .collect()
}

#[test]
fn update_raw_creates_a_missing_config_holding_only_the_changed_key() {
    let project = project(None);

    ProjectConfig::update_raw(project.path(), set("profile", Value::from("lean"))).expect("update");

    assert_eq!(read_config(&project), "{\n  \"profile\": \"lean\"\n}\n");
    assert!(strays(&project).is_empty(), "{:?}", strays(&project));
}

#[test]
fn update_raw_keeps_every_key_the_change_does_not_touch() {
    let project = project(Some(
        r#"{"profile": "audit", "default_page_bytes": 4096, "mechanics": {"version": 1, "entries": []}}"#,
    ));

    ProjectConfig::update_raw(project.path(), set("profile", Value::from("lean"))).expect("update");

    let written: Value = serde_json::from_str(&read_config(&project)).expect("still JSON");
    assert_eq!(written["profile"], "lean");
    assert_eq!(written["default_page_bytes"], 4096);
    assert_eq!(written["mechanics"]["version"], 1);
}

#[test]
fn update_raw_refuses_malformed_json_and_leaves_the_file_unchanged() {
    let project = project(Some("{ not json"));

    let error = ProjectConfig::update_raw(project.path(), set("profile", Value::from("lean")))
        .expect_err("a file it cannot parse is never overwritten");

    assert!(
        matches!(error, ConfigWriteError::Malformed { .. }),
        "{error:?}"
    );
    assert_eq!(read_config(&project), "{ not json");
}

#[test]
fn update_raw_refuses_json_that_is_not_an_object_and_leaves_the_file_unchanged() {
    let project = project(Some("[1, 2]"));

    let error = ProjectConfig::update_raw(project.path(), set("profile", Value::from("lean")))
        .expect_err("an array is not a config");

    assert!(
        matches!(error, ConfigWriteError::NotAnObject { .. }),
        "{error:?}"
    );
    assert_eq!(read_config(&project), "[1, 2]");
}

#[test]
fn update_raw_refuses_a_directory_at_config_json() {
    let project = project(None);
    let target = config_path(project.path());
    fs::create_dir_all(&target).expect("directory in the file's place");

    let error = ProjectConfig::update_raw(project.path(), set("profile", Value::from("lean")))
        .expect_err("a directory must be refused");

    assert!(
        matches!(&error, ConfigWriteError::NotRegularFile { path } if *path == target),
        "{error:?}"
    );
    assert!(target.is_dir(), "the directory is left in place");
}

#[test]
fn a_failing_change_writes_nothing() {
    let project = project(Some(r#"{"profile": "audit"}"#));

    let error = ProjectConfig::update_raw(project.path(), |config| {
        config.insert("profile".to_owned(), Value::from("lean"));
        Err(ConfigWriteError::NotAnObject {
            path: PathBuf::from("refused by the change"),
        })
    })
    .expect_err("the change refused");

    assert!(
        matches!(error, ConfigWriteError::NotAnObject { .. }),
        "{error:?}"
    );
    assert_eq!(read_config(&project), r#"{"profile": "audit"}"#);
}

#[test]
fn an_update_over_the_read_cap_is_refused_and_the_file_unchanged() {
    let project = project(Some(r#"{"profile": "audit"}"#));
    let huge = "x".repeat(usize::try_from(crate::util::MAX_FOREIGN_TEXT_BYTES).expect("cap fits"));

    let error = ProjectConfig::update_raw(project.path(), set("padding", Value::from(huge)))
        .expect_err("the server could not read it back");

    assert!(
        matches!(error, ConfigWriteError::TooLarge { .. }),
        "{error:?}"
    );
    assert_eq!(read_config(&project), r#"{"profile": "audit"}"#);
    assert!(strays(&project).is_empty(), "{:?}", strays(&project));
}

#[test]
fn a_failure_between_the_temp_write_and_the_rename_leaves_the_old_file_and_no_temp() {
    let project = project(Some(r#"{"profile": "audit"}"#));
    let mut staged = String::new();
    let mut fail = |temp: &Path| -> std::io::Result<()> {
        staged = fs::read_to_string(temp)?;
        Err(std::io::Error::other("simulated crash before the rename"))
    };

    let error = update_with(
        project.path(),
        LOCK_WAIT,
        &mut fail,
        set("profile", Value::from("lean")),
    )
    .expect_err("the rename never happened");

    assert_eq!(
        staged, "{\n  \"profile\": \"lean\"\n}\n",
        "the temp file was complete"
    );
    assert!(
        matches!(&error, ConfigWriteError::Io { source, .. }
            if source.to_string() == "simulated crash before the rename"),
        "the source error is kept: {error:?}"
    );
    assert_eq!(read_config(&project), r#"{"profile": "audit"}"#);
    assert!(
        strays(&project).is_empty(),
        "temp removed: {:?}",
        strays(&project)
    );
}

#[cfg(unix)]
#[test]
fn a_config_swapped_for_a_symlink_before_the_rename_is_refused_and_the_link_target_untouched() {
    let project = project(Some(r#"{"profile": "audit"}"#));
    let outside = tempfile::tempdir().expect("outside dir");
    let victim = outside.path().join("victim.json");
    fs::write(&victim, "outside").expect("write victim");
    let target = config_path(project.path());
    let mut swap = |_: &Path| -> std::io::Result<()> {
        fs::remove_file(&target)?;
        std::os::unix::fs::symlink(&victim, &target)
    };

    let error = update_with(
        project.path(),
        LOCK_WAIT,
        &mut swap,
        set("profile", Value::from("lean")),
    )
    .expect_err("the re-check refuses the link");

    assert!(
        matches!(&error, ConfigWriteError::NotRegularFile { path } if *path == target),
        "{error:?}"
    );
    assert_eq!(fs::read_to_string(&victim).expect("read victim"), "outside");
    assert!(
        fs::symlink_metadata(&target)
            .expect("lstat")
            .file_type()
            .is_symlink(),
        "the link is left as found"
    );
    assert!(
        strays(&project).is_empty(),
        "temp removed: {:?}",
        strays(&project)
    );
}

#[cfg(unix)]
#[test]
fn update_raw_refuses_a_symlinked_config_and_leaves_the_target_untouched() {
    let project = project(None);
    let outside = tempfile::tempdir().expect("outside dir");
    let victim = outside.path().join("victim.json");
    fs::write(&victim, r#"{"outside": true}"#).expect("write victim");
    std::os::unix::fs::symlink(&victim, config_path(project.path())).expect("symlink");

    let error = ProjectConfig::update_raw(project.path(), set("profile", Value::from("lean")))
        .expect_err("a symlinked config is refused");

    assert!(
        matches!(error, ConfigWriteError::NotRegularFile { .. }),
        "{error:?}"
    );
    assert_eq!(
        fs::read_to_string(&victim).expect("read victim"),
        r#"{"outside": true}"#
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_lock_file_is_refused_and_nothing_is_written() {
    let project = project(Some(r#"{"profile": "audit"}"#));
    let outside = tempfile::tempdir().expect("outside dir");
    let victim = outside.path().join("victim.lock");
    fs::write(&victim, "outside").expect("write victim");
    let lock = project.path().join(".planning/slugaudit/config.lock");
    std::os::unix::fs::symlink(&victim, &lock).expect("symlink");

    let error = ProjectConfig::update_raw(project.path(), set("profile", Value::from("lean")))
        .expect_err("a symlinked lock is refused");

    assert!(
        matches!(&error, ConfigWriteError::NotRegularFile { path } if *path == lock),
        "{error:?}"
    );
    assert_eq!(fs::read_to_string(&victim).expect("read victim"), "outside");
    assert_eq!(read_config(&project), r#"{"profile": "audit"}"#);
}

#[cfg(unix)]
#[test]
fn a_dangling_symlinked_lock_file_is_refused_without_creating_its_target() {
    let project = project(None);
    let outside = tempfile::tempdir().expect("outside dir");
    let victim = outside.path().join("created.lock");
    let lock = project.path().join(".planning/slugaudit/config.lock");
    std::os::unix::fs::symlink(&victim, &lock).expect("symlink");

    let error = ProjectConfig::update_raw(project.path(), set("profile", Value::from("lean")))
        .expect_err("a dangling symlinked lock is refused");

    assert!(
        matches!(error, ConfigWriteError::NotRegularFile { .. }),
        "{error:?}"
    );
    assert!(!victim.exists(), "the link target was never created");
    assert!(!config_path(project.path()).exists(), "nothing was written");
}

#[test]
fn an_update_gives_up_with_locked_while_another_holder_keeps_the_lock() {
    let project = project(Some(r#"{"profile": "audit"}"#));
    let lock = project.path().join(".planning/slugaudit/config.lock");
    let holder = File::create(&lock).expect("create lock");
    holder.lock().expect("hold the lock");

    let error = update_with(
        project.path(),
        Duration::from_millis(100),
        &mut |_| Ok(()),
        set("profile", Value::from("lean")),
    )
    .expect_err("the lock is held throughout");

    assert!(
        matches!(&error, ConfigWriteError::Locked { path, waited_ms } if *path == lock && *waited_ms >= 100),
        "{error:?}"
    );
    assert_eq!(read_config(&project), r#"{"profile": "audit"}"#);
}

#[test]
fn an_update_waits_for_a_lock_that_is_released_within_the_wait() {
    let project = project(None);
    let lock = project.path().join(".planning/slugaudit/config.lock");
    let holder = File::create(&lock).expect("create lock");
    holder.lock().expect("hold the lock");
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        holder.unlock().expect("release the lock");
    });

    let started = Instant::now();
    ProjectConfig::update_raw(project.path(), set("profile", Value::from("lean")))
        .expect("the update runs once the lock is free");

    releaser.join().expect("releaser thread");
    assert!(
        started.elapsed() >= Duration::from_millis(150),
        "it waited for the holder"
    );
    assert_eq!(read_config(&project), "{\n  \"profile\": \"lean\"\n}\n");
}

#[test]
fn concurrent_updates_from_many_threads_lose_no_key() {
    const WRITERS: usize = 16;
    let project = Arc::new(project(None));
    let start = Arc::new(std::sync::Barrier::new(WRITERS));

    let handles: Vec<_> = (0..WRITERS)
        .map(|writer| {
            let project = Arc::clone(&project);
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                ProjectConfig::update_raw(project.path(), |config| {
                    config.insert(format!("key_{writer}"), Value::from(writer));
                    Ok::<(), ConfigWriteError>(())
                })
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("writer thread").expect("update");
    }

    let written: Value = serde_json::from_str(&read_config(&project)).expect("still JSON");
    let keys = written.as_object().expect("an object").len();
    assert_eq!(keys, WRITERS, "every writer's key survived: {written}");
    assert!(strays(&project).is_empty(), "{:?}", strays(&project));
}

#[cfg(unix)]
#[test]
fn the_replace_keeps_the_config_file_mode() {
    use std::os::unix::fs::PermissionsExt as _;
    let project = project(Some("{}"));
    let target = config_path(project.path());
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).expect("chmod");

    ProjectConfig::update_raw(project.path(), set("profile", Value::from("lean"))).expect("update");

    let mode = fs::metadata(&target).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}
