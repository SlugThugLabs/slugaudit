//! `disable` (`project_control off`): what it removes, what it keeps, and
//! that the project is inactive afterwards.

use super::*;
use std::fs;

/// An enabled project whose `names` exist in the activation directory.
/// `project.db` is a real database because `disable` opens it for its lock.
fn enabled_with_files(names: &[&str]) -> (tempfile::TempDir, ProjectRoot, PathBuf) {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");
    let activation = enable(&root).expect("enable");
    for name in names {
        if *name == DATABASE_FILENAME {
            drop(crate::store::open_read_write(&activation.join(name)).expect("create database"));
        } else {
            fs::write(activation.join(name), b"x").expect("write fixture file");
        }
    }
    (directory, root, activation)
}

#[test]
fn project_off_removes_an_otherwise_empty_activation_directory_and_deactivates() {
    let (directory, root, activation) = enabled_with_files(&[DATABASE_FILENAME]);

    let report = disable(&root).expect("disable succeeds");

    assert_eq!(
        report.removed,
        vec![
            activation.join(DATABASE_FILENAME),
            activation.join("project.db-wal"),
            activation.join("project.db-shm"),
            activation.clone()
        ]
    );
    assert!(!report.config_kept);
    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::NotActive)
    ));
}

#[test]
fn project_off_on_a_never_enabled_project_removes_nothing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");

    let report = disable(&root).expect("disable succeeds even when never enabled");

    assert_eq!(report, DisableReport::default());
}

#[test]
fn project_off_keeps_config_json_and_unrelated_user_files_but_deactivates() {
    let (directory, root, activation) =
        enabled_with_files(&[DATABASE_FILENAME, "config.json", "notes.txt"]);

    let report = disable(&root).expect("disable succeeds");

    assert_eq!(
        report.removed,
        vec![
            activation.join(DATABASE_FILENAME),
            activation.join("project.db-wal"),
            activation.join("project.db-shm"),
        ]
    );
    assert!(report.config_kept);
    assert_eq!(
        fs::read(activation.join("config.json")).expect("config kept"),
        b"x"
    );
    assert!(activation.join("notes.txt").is_file());
    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::NotActive)
    ));
}

#[test]
fn project_off_removes_sidecars_results_directory_and_config_lock() {
    let (_directory, root, activation) = enabled_with_files(&[
        "project.db-wal",
        "project.db-shm",
        "project.db-journal",
        "config.lock",
        "keep.txt",
    ]);
    fs::create_dir_all(activation.join("results")).expect("create results dir");
    fs::write(activation.join("results").join("a.lock"), b"x").expect("write result lock");

    let report = disable(&root).expect("disable succeeds");

    assert_eq!(
        report.removed,
        vec![
            activation.join("project.db-wal"),
            activation.join("project.db-shm"),
            activation.join("project.db-journal"),
            activation.join("config.lock"),
            activation.join("results"),
        ]
    );
    assert!(!report.config_kept);
    assert!(!activation.join("results").exists());
    assert!(activation.join("keep.txt").is_file());
}

#[test]
fn project_off_twice_on_a_clean_directory_removes_nothing_the_second_time() {
    let (_dir, root, _activation) = enabled_with_files(&["config.json"]);
    disable(&root).expect("first disable");

    let report = disable(&root).expect("second disable");

    assert!(report.removed.is_empty(), "{report:?}");
    assert!(report.config_kept);
}

#[cfg(unix)]
#[test]
fn project_off_unlinks_a_symlinked_results_entry_without_touching_its_target() {
    let (directory, root, activation) = enabled_with_files(&[]);
    let outside = directory.path().join("outside");
    fs::create_dir_all(&outside).expect("create outside dir");
    fs::write(outside.join("victim.txt"), b"keep").expect("write victim");
    std::os::unix::fs::symlink(&outside, activation.join("results")).expect("symlink results");

    let report = disable(&root).expect("disable succeeds");

    assert_eq!(report.removed, vec![activation.join("results"), activation]);
    assert_eq!(
        fs::read(outside.join("victim.txt")).expect("target untouched"),
        b"keep"
    );
}
