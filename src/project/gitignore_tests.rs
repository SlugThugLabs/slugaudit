//! Failure-arm and file-type tests for the `.gitignore` activation guard.
//!
//! The happy paths live in `activation_tests.rs`; these tests reach the
//! writer's own error arms, which a fixture that fails at the read cannot.

use super::*;
use std::fs;
use std::sync::mpsc;
use std::time::Duration;

/// Counts temp files the atomic writer may have left behind in `directory`.
fn temp_leftovers(directory: &Path) -> usize {
    fs::read_dir(directory)
        .expect("read the directory")
        .map(|entry| entry.expect("read a directory entry").file_name())
        .filter(|name| name.to_string_lossy().starts_with(".gitignore.slugaudit."))
        .count()
}

/// Runs the guard on a thread so a regression to the blocking open fails the
/// test instead of hanging the whole suite.
fn outcome_without_blocking(root: ProjectRoot) -> GitignoreOutcome {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        sender
            .send(ensure_gitignore(&root))
            .expect("test receives the outcome");
    });
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("the .gitignore guard must not block on a non-regular file")
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

/// A write that cannot even create its temp file must report `NotWritten`,
/// never `Added`, and must not create the destination.
#[test]
fn a_gitignore_that_cannot_be_written_is_reported_not_added() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("valid root");
    let path = root.as_path().join(GITIGNORE_FILE);
    drop(directory);

    let outcome = ensure_gitignore(&root);

    match outcome {
        GitignoreOutcome::NotWritten {
            path: reported,
            reason,
        } => {
            assert_eq!(reported, path, "the unwritable path is named");
            assert!(!reason.is_empty(), "the write failure carries a reason");
        }
        other => panic!("a failed write must report NotWritten, not Added, got {other:?}"),
    }
    assert!(
        !path.exists(),
        "no file is created when the atomic write cannot start"
    );
}

/// The rename arm fails after the write; the temp file is cleaned up and the
/// destination is untouched.
#[test]
fn a_writer_whose_rename_fails_removes_the_temp_file() {
    let outer = tempfile::tempdir().expect("temp dir");
    let destination = outer.path().join("occupied");
    fs::create_dir(&destination).expect("directory in the destination's place");
    fs::write(destination.join("keep"), b"x").expect("write a marker");

    let result = write_gitignore_atomically(&destination, b"", b".planning/slugaudit/\n");

    assert!(
        result.is_err(),
        "renaming a file over a directory fails: {result:?}"
    );
    assert_eq!(
        fs::read(destination.join("keep")).expect("the marker survives"),
        b"x",
        "the destination directory is untouched"
    );
    let leftovers = temp_leftovers(outer.path());
    assert_eq!(leftovers, 0, "a failed rename removes its temp file");
}

/// A FIFO would block `File::open` forever, so the guard must refuse it from
/// metadata alone and leave it untouched.
#[cfg(unix)]
#[test]
fn a_fifo_gitignore_is_refused_without_opening_it() {
    use std::os::unix::fs::FileTypeExt;
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("valid root");
    let path = root.as_path().join(GITIGNORE_FILE);
    create_fifo(&path);

    let outcome = outcome_without_blocking(root);

    match outcome {
        GitignoreOutcome::NotWritten {
            path: reported,
            reason,
        } => {
            assert_eq!(reported, path, "the refused path is named");
            assert!(
                reason.contains("not a regular file"),
                "the reason names the refusal: {reason}"
            );
        }
        other => panic!("a FIFO .gitignore must be refused, got {other:?}"),
    }
    assert!(
        path.symlink_metadata()
            .expect("stat the FIFO")
            .file_type()
            .is_fifo(),
        "the FIFO is left untouched"
    );
}

/// Following the symlink must stop at the FIFO's metadata, not at its open.
#[cfg(unix)]
#[test]
fn a_symlink_to_a_fifo_gitignore_is_refused_without_opening_it() {
    use std::os::unix::fs::FileTypeExt;
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("valid root");
    let fifo = directory.path().join("fifo-target");
    create_fifo(&fifo);
    let path = root.as_path().join(GITIGNORE_FILE);
    std::os::unix::fs::symlink(&fifo, &path).expect("create the symlink");

    let outcome = outcome_without_blocking(root);

    match outcome {
        GitignoreOutcome::NotWritten {
            path: reported,
            reason,
        } => {
            assert_eq!(reported, path, "the refused path is named");
            assert!(
                reason.contains("symbolic link"),
                "the link is refused, not opened: {reason}"
            );
        }
        other => panic!("a symlink to a FIFO must be refused, got {other:?}"),
    }
    assert!(
        fifo.symlink_metadata()
            .expect("stat the FIFO")
            .file_type()
            .is_fifo(),
        "the FIFO target is left untouched"
    );
    assert!(
        path.symlink_metadata()
            .expect("stat the link")
            .file_type()
            .is_symlink(),
        "the user's link survives"
    );
}

/// A symlink is refused even when its target is an in-project regular file:
/// the write must never follow a link, so the target keeps its bytes.
#[cfg(unix)]
#[test]
fn a_symlink_to_an_in_project_gitignore_is_refused_and_target_untouched() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("valid root");
    let target = directory.path().join("shared-ignore");
    fs::write(&target, b"/target\n").expect("write the link target");
    let path = root.as_path().join(GITIGNORE_FILE);
    std::os::unix::fs::symlink(&target, &path).expect("create the symlink");

    let outcome = ensure_gitignore(&root);

    match outcome {
        GitignoreOutcome::NotWritten { path: reported, .. } => {
            assert_eq!(reported, path, "the refused path is named");
        }
        other => panic!("a symlinked .gitignore must be refused, got {other:?}"),
    }
    assert_eq!(
        fs::read(&target).expect("read the link target"),
        b"/target\n".to_vec(),
        "the link target is unchanged"
    );
}

/// A `.gitignore` symlinked outside the project must not be modified or
/// replaced: content and inode of the victim stay as they were.
#[cfg(unix)]
#[test]
fn a_gitignore_symlinked_outside_the_project_leaves_the_victim_untouched() {
    use std::os::unix::fs::MetadataExt;
    let project = tempfile::tempdir().expect("project dir");
    let outside = tempfile::tempdir().expect("outside dir");
    let root = ProjectRoot::resolve(project.path()).expect("valid root");
    let victim = outside.path().join("victim_rc");
    fs::write(&victim, b"export SECRET=1\n").expect("write victim");
    let inode = fs::metadata(&victim).expect("stat victim").ino();
    let path = root.as_path().join(GITIGNORE_FILE);
    std::os::unix::fs::symlink(&victim, &path).expect("create the symlink");

    let outcome = ensure_gitignore(&root);

    match outcome {
        GitignoreOutcome::NotWritten {
            path: reported,
            reason,
        } => {
            assert_eq!(reported, path, "the .gitignore path is named");
            assert!(
                reason.contains("symbolic link"),
                "the reason names the link: {reason}"
            );
        }
        other => panic!("an escaping symlink must be refused, got {other:?}"),
    }
    assert_eq!(
        fs::read(&victim).expect("read victim"),
        b"export SECRET=1\n".to_vec(),
        "victim content is unchanged"
    );
    assert_eq!(
        fs::metadata(&victim).expect("stat victim").ino(),
        inode,
        "victim inode is unchanged"
    );
    assert_eq!(temp_leftovers(project.path()), 0, "no temp file is left");
}

/// Replacing a dangling link with a regular file would destroy the user's
/// link, so the guard reports instead.
#[cfg(unix)]
#[test]
fn a_dangling_gitignore_symlink_is_reported_rather_than_replaced() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("valid root");
    let absent = directory.path().join("missing-target");
    let path = root.as_path().join(GITIGNORE_FILE);
    std::os::unix::fs::symlink(&absent, &path).expect("create the dangling symlink");

    let outcome = ensure_gitignore(&root);

    match outcome {
        GitignoreOutcome::NotWritten {
            path: reported,
            reason,
        } => {
            assert_eq!(reported, path, "the refused path is named");
            assert!(
                reason.contains("symbolic link"),
                "the reason names the dangling link: {reason}"
            );
        }
        other => panic!("a dangling symlink must not be replaced by a regular file, got {other:?}"),
    }
    assert!(
        path.symlink_metadata()
            .expect("stat the link")
            .file_type()
            .is_symlink(),
        "the user's link survives"
    );
    assert!(
        !absent.exists(),
        "the write does not create the missing target"
    );
}

#[test]
fn gitignore_write_syncs_the_parent_and_lands_the_content() {
    let dir = tempfile::tempdir().expect("temp dir");
    let destination = dir.path().join(".gitignore");

    write_gitignore_atomically(&destination, b"a\n", b".planning/slugaudit/\n")
        .expect("write and sync the directory");

    assert_eq!(
        fs::read(&destination).expect("read back"),
        b"a\n.planning/slugaudit/\n"
    );
    assert_eq!(temp_leftovers(dir.path()), 0);
}
