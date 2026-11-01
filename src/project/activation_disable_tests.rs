//! `disable` (`project_control off`): the directory it deletes, what it
//! leaves alone, and that the project is inactive afterwards.

use super::*;
use std::fs;

/// An enabled project whose activation directory holds `names`.
fn enabled_with_files(names: &[&str]) -> (tempfile::TempDir, ProjectRoot, PathBuf) {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");
    let activation = enable(&root).expect("enable");
    for name in names {
        fs::write(activation.join(name), b"x").expect("write fixture file");
    }
    (directory, root, activation)
}

#[test]
fn project_off_deletes_the_directory_with_everything_in_it_and_then_an_empty_planning_dir() {
    let (directory, root, activation) =
        enabled_with_files(&["project.db", "config.json", "notes.txt"]);
    fs::create_dir_all(activation.join("results")).expect("create results dir");
    fs::write(activation.join("results").join("a.lock"), b"x").expect("write result lock");

    let report = disable(&root).expect("disable succeeds");

    assert_eq!(
        report.removed,
        vec![
            activation.clone(),
            activation.parent().expect("planning").to_path_buf()
        ]
    );
    assert!(!directory.path().join(PLANNING_DIR).exists());
    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::NotActive)
    ));
}

#[test]
fn project_off_keeps_a_planning_dir_that_holds_other_files() {
    let (directory, root, activation) = enabled_with_files(&["config.json"]);
    let planning = directory.path().join(PLANNING_DIR);
    fs::write(planning.join("PLAN.md"), b"mine").expect("write user file");

    let report = disable(&root).expect("disable succeeds");

    assert_eq!(report.removed, vec![activation.clone()]);
    assert!(!activation.exists());
    assert_eq!(fs::read(planning.join("PLAN.md")).expect("kept"), b"mine");
}

#[test]
fn project_off_on_a_never_enabled_project_removes_nothing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");

    let report = disable(&root).expect("disable succeeds even when never enabled");

    assert_eq!(report, DisableReport::default());
}

#[test]
fn project_off_twice_removes_nothing_the_second_time() {
    let (_dir, root, _activation) = enabled_with_files(&["config.json"]);
    disable(&root).expect("first disable");

    let report = disable(&root).expect("second disable");

    assert!(report.removed.is_empty(), "{report:?}");
}

#[cfg(unix)]
#[test]
fn project_off_does_not_follow_a_symlink_inside_the_directory() {
    let (directory, root, activation) = enabled_with_files(&[]);
    let outside = directory.path().join("outside");
    fs::create_dir_all(&outside).expect("create outside dir");
    fs::write(outside.join("victim.txt"), b"keep").expect("write victim");
    std::os::unix::fs::symlink(&outside, activation.join("results")).expect("symlink results");

    disable(&root).expect("disable succeeds");

    assert!(!activation.exists());
    assert_eq!(
        fs::read(outside.join("victim.txt")).expect("target untouched"),
        b"keep"
    );
}

#[cfg(unix)]
#[test]
fn project_off_on_a_symlinked_activation_directory_removes_only_the_link() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("elsewhere");
    fs::create_dir_all(&target).expect("create target");
    fs::write(target.join("keep.txt"), b"keep").expect("write target file");
    let planning = directory.path().join(PLANNING_DIR);
    fs::create_dir_all(&planning).expect("create planning dir");
    let link = planning.join(ACTIVATION_DIR);
    std::os::unix::fs::symlink(&target, &link).expect("symlink activation");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");

    let report = disable(&root).expect("disable succeeds");

    assert_eq!(report.removed, vec![link.clone(), planning.clone()]);
    assert!(!link.exists() && link.symlink_metadata().is_err());
    assert_eq!(
        fs::read(target.join("keep.txt")).expect("target untouched"),
        b"keep"
    );
}
