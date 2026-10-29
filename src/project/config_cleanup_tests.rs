//! Stale `config.json` temp-file cleanup: removed once and named in one
//! warning, kept while a writer holds `config.lock`, never followed through a
//! symlink, and only the exact temp-file pattern is touched. Each test uses a
//! temp name no other test uses and counts only the lines carrying it.

use super::*;
use crate::model::AuditProfile;
use crate::project::ProjectConfig;
use crate::store::test_capture::capture_warns;
use std::fs;
use std::path::PathBuf;

fn project() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning/slugaudit")).expect("activation dir");
    project
}

fn activation_dir(project: &tempfile::TempDir) -> PathBuf {
    project.path().join(".planning/slugaudit")
}

fn count_lines_with(logs: &str, needle: &str) -> usize {
    logs.lines().filter(|line| line.contains(needle)).count()
}

#[test]
fn a_stale_config_temp_file_is_removed_with_one_warning_naming_it() {
    let project = project();
    let stale = activation_dir(&project).join("config.json.900001.1.tmp");
    fs::write(&stale, "{}").expect("write stale temp");

    let ((), logs) = capture_warns(|| {
        remove_stale_temps(project.path());
        remove_stale_temps(project.path());
    });

    assert!(!stale.exists(), "the stale temp file is removed");
    assert_eq!(
        count_lines_with(&logs, "config.json.900001.1.tmp"),
        1,
        "{logs}"
    );
    assert!(
        !logs.contains(project.path().to_str().expect("utf-8 path")),
        "no raw path is logged:\n{logs}"
    );
}

#[test]
fn a_temp_file_is_kept_while_another_holder_has_the_config_lock_and_removed_after() {
    let project = project();
    let temp = activation_dir(&project).join("config.json.900002.1.tmp");
    fs::write(&temp, "{}").expect("write temp");
    let holder = File::create(activation_dir(&project).join("config.lock")).expect("create lock");
    holder.lock().expect("hold the lock");

    remove_stale_temps(project.path());

    assert!(temp.exists(), "a write in progress keeps its temp file");

    holder.unlock().expect("release the lock");
    remove_stale_temps(project.path());

    assert!(!temp.exists(), "with no writer left the orphan is removed");
}

#[cfg(unix)]
#[test]
fn a_symlink_named_like_a_temp_file_is_not_followed_or_removed() {
    let project = project();
    let outside = tempfile::tempdir().expect("outside dir");
    let victim = outside.path().join("victim.txt");
    fs::write(&victim, "keep").expect("write victim");
    let link = activation_dir(&project).join("config.json.900003.1.tmp");
    std::os::unix::fs::symlink(&victim, &link).expect("symlink");

    let ((), logs) = capture_warns(|| {
        remove_stale_temps(project.path());
        remove_stale_temps(project.path());
    });

    assert_eq!(fs::read_to_string(&victim).expect("read victim"), "keep");
    assert!(
        fs::symlink_metadata(&link).is_ok(),
        "the link itself is left in place"
    );
    assert_eq!(
        count_lines_with(&logs, "config.json.900003.1.tmp"),
        1,
        "one warning for the refused entry, however often the config loads:\n{logs}"
    );
    assert!(logs.contains("symlink"), "{logs}");
}

#[test]
fn only_the_exact_temp_file_pattern_is_removed() {
    let project = project();
    let directory = activation_dir(&project);
    let kept = [
        "config.json",
        "config.json.tmp",
        "config.json.abc.1.tmp",
        "config.json.1.tmp",
        "config.json.1.2.tmp.bak",
        "config.json.1.2.3.tmp",
        "other.1.2.tmp",
    ];
    for name in kept {
        fs::write(directory.join(name), "x").expect("write bystander");
    }
    let stale = directory.join("config.json.900004.1.tmp");
    fs::write(&stale, "{}").expect("write stale temp");

    remove_stale_temps(project.path());

    assert!(!stale.exists(), "the exact pattern is removed");
    for name in kept {
        assert!(directory.join(name).exists(), "{name} must be left alone");
    }
}

#[test]
fn a_clean_activation_directory_gains_no_lock_file() {
    let project = project();
    fs::write(config_path(project.path()), "{}").expect("write config");

    remove_stale_temps(project.path());

    assert!(
        !activation_dir(&project).join("config.lock").exists(),
        "the lock is only tried when a temp file exists"
    );
}

#[test]
fn a_project_without_an_activation_directory_is_left_untouched() {
    let project = tempfile::tempdir().expect("project dir");

    remove_stale_temps(project.path());

    assert!(!project.path().join(".planning").exists());
}

#[test]
fn loading_the_project_config_removes_a_stale_temp_file() {
    let project = project();
    fs::write(config_path(project.path()), r#"{"profile": "lean"}"#).expect("write config");
    let stale = activation_dir(&project).join("config.json.900005.1.tmp");
    fs::write(&stale, "{}").expect("write stale temp");

    let config = ProjectConfig::load_or_default(project.path()).expect("config loads");

    assert_eq!(config.profile, Some(AuditProfile::Lean));
    assert!(!stale.exists(), "the load cleans up after a crashed update");
}
