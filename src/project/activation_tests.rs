use super::*;
use std::fs;

fn create_activation(root: &Path) {
    fs::create_dir_all(root.join(PLANNING_DIR).join(ACTIVATION_DIR))
        .expect("create activation dir");
}

#[test]
fn finds_an_active_project_at_its_own_root() {
    let directory = tempfile::tempdir().expect("temp dir");
    create_activation(directory.path());
    let root = find_project_root(directory.path()).expect("active project");
    assert_eq!(root.as_path(), directory.path().canonicalize().unwrap());
}

#[test]
fn finds_an_active_project_from_a_nested_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    create_activation(directory.path());
    let nested = directory.path().join("src").join("nested");
    fs::create_dir_all(&nested).expect("create nested dir");
    let file = nested.join("main.rs");
    fs::write(&file, b"fn main() {}").expect("write fixture file");

    let root = find_project_root(&file).expect("active project");
    assert_eq!(root.as_path(), directory.path().canonicalize().unwrap());
}

#[test]
fn rejects_a_project_with_no_activation_marker() {
    let directory = tempfile::tempdir().expect("temp dir");
    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::NotActive)
    ));
}

#[cfg(unix)]
#[test]
fn rejects_a_symlinked_activation_directory() {
    let directory = tempfile::tempdir().expect("temp dir");
    let real_target = directory.path().join("elsewhere");
    fs::create_dir_all(&real_target).expect("create link target");
    fs::create_dir_all(directory.path().join(PLANNING_DIR)).expect("create planning dir");
    std::os::unix::fs::symlink(
        &real_target,
        directory.path().join(PLANNING_DIR).join(ACTIVATION_DIR),
    )
    .expect("create symlink");

    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::SymlinkedActivationPath)
    ));
}

#[test]
fn enable_creates_the_marker_and_find_project_root_then_sees_it() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");

    let created = enable(&root).expect("enable succeeds");
    assert!(created.is_dir());
    assert_eq!(
        find_project_root(directory.path())
            .expect("now active")
            .as_path(),
        root.as_path()
    );
}

#[test]
fn enable_is_idempotent() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");
    enable(&root).expect("first enable");
    assert!(
        enable(&root).is_ok(),
        "enabling an already-active project must not error"
    );
}

#[test]
fn disable_removes_the_marker_and_find_project_root_stops_seeing_it() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");
    enable(&root).expect("enable");

    let removed = disable(&root).expect("disable succeeds");
    assert!(
        removed,
        "an active project's disable must report it actually removed something"
    );
    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::NotActive)
    ));
}

#[test]
fn disabling_an_already_inactive_project_is_a_no_op_not_an_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");
    let removed = disable(&root).expect("disable succeeds even when never enabled");
    assert!(!removed);
}

#[cfg(unix)]
#[test]
fn enable_refuses_to_create_through_a_symlinked_planning_dir() {
    let directory = tempfile::tempdir().expect("temp dir");
    let real_target = directory.path().join("elsewhere");
    fs::create_dir_all(&real_target).expect("create link target");
    std::os::unix::fs::symlink(&real_target, directory.path().join(PLANNING_DIR))
        .expect("create symlink");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");

    assert!(matches!(
        enable(&root),
        Err(ActivationError::SymlinkedActivationPath)
    ));
}

/// `find_project_root` is a read-only lookup that can race a concurrent
/// toggle of `.planning/slugaudit` — whether that's the `enable`/
/// `disable` CLI commands (`src/cli.rs`) or a host application doing
/// the same thing directly, the marker can be created or removed
/// mid-lookup. `find_project_root` is a single synchronous ancestor
/// walk with no hook to pause it deterministically mid-loop, so this
/// drives the race with a real second thread hammering create/remove
/// on the marker directory while many real lookups run concurrently,
/// asserting the only two acceptable outcomes (a clean success
/// matching the real root, or a clean `NotActive`) and never a panic
/// or any other error variant.
#[cfg(unix)]
#[test]
fn a_marker_toggled_concurrently_never_panics_or_returns_a_partial_state() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path().to_path_buf();
    create_activation(&root);
    let nested = root.join("src").join("nested");
    fs::create_dir_all(&nested).expect("create nested dir");
    let canonical_root = root.canonicalize().expect("canonicalize root");

    let activation_dir = root.join(PLANNING_DIR).join(ACTIVATION_DIR);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_toggler = Arc::clone(&stop);
    let toggler = std::thread::spawn(move || {
        while !stop_toggler.load(Ordering::Relaxed) {
            let _ = fs::remove_dir_all(&activation_dir);
            let _ = fs::create_dir_all(&activation_dir);
        }
    });

    for _ in 0..2_000 {
        match find_project_root(&nested) {
            Ok(found) => assert_eq!(found.as_path(), canonical_root),
            Err(ActivationError::NotActive) => {}
            Err(other) => {
                panic!("a benign concurrent marker toggle must never produce {other:?}")
            }
        }
    }

    stop.store(true, Ordering::Relaxed);
    toggler.join().expect("toggler thread joins");
}
