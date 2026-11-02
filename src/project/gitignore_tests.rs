//! Failure-arm and file-type tests for the `.gitignore` activation guard.
//!
//! The happy paths live in `activation_gitignore_tests.rs`; these tests reach the
//! writer's own error arms, which a fixture that fails at the read cannot.

use super::*;
use slugaudit_test_check::{Check, TestResult};
use std::fs;
use std::sync::mpsc;
use std::time::Duration;

/// Counts temp files the atomic writer may have left behind in `directory`.
fn temp_leftovers(directory: &Path) -> TestResult<usize> {
    let mut count = 0;
    for entry in fs::read_dir(directory).check("read the directory")? {
        let entry = entry.check("read a directory entry")?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(".gitignore.slugaudit.")
        {
            count += 1;
        }
    }
    Ok(count)
}

/// Runs the guard on a thread so a regression to the blocking open fails the
/// test instead of hanging the whole suite.
fn outcome_without_blocking(root: ProjectRoot) -> TestResult<GitignoreOutcome> {
    let (sender, receiver) = mpsc::channel();
    // A failed send means the receiver already timed out and failed the test.
    std::thread::spawn(move || sender.send(ensure_gitignore(&root)));
    receiver
        .recv_timeout(Duration::from_secs(10))
        .check("the .gitignore guard must not block on a non-regular file")
}

/// `mkfifo(1)` creates the fixture because std exposes no safe FIFO
/// constructor and `unsafe` is forbidden; it exists on both supported targets.
#[cfg(unix)]
fn create_fifo(path: &Path) -> TestResult {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .check("run mkfifo")?;
    assert!(status.success(), "mkfifo creates the fixture: {status}");
    Ok(())
}

/// A write that cannot even create its temp file must report `NotWritten`,
/// never `Added`, and must not create the destination.
#[test]
fn a_gitignore_that_cannot_be_written_is_reported_not_added() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = ProjectRoot::resolve(directory.path()).check("valid root")?;
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
        other => {
            return Err(format!(
                "a failed write must report NotWritten, not Added, got {other:?}"
            ))
            .check("unexpected outcome");
        }
    }
    assert!(
        !path.exists(),
        "no file is created when the atomic write cannot start"
    );
    Ok(())
}

/// The rename arm fails after the write; the temp file is cleaned up and the
/// destination is untouched.
#[test]
fn a_writer_whose_rename_fails_removes_the_temp_file() -> TestResult {
    let outer = tempfile::tempdir().check("temp dir")?;
    let destination = outer.path().join("occupied");
    fs::create_dir(&destination).check("directory in the destination's place")?;
    fs::write(destination.join("keep"), b"x").check("write a marker")?;

    let result =
        write_gitignore_atomically(&destination, b"", b".planning/slugaudit/\n", sync_directory);

    assert!(
        result.is_err(),
        "renaming a file over a directory fails: {result:?}"
    );
    assert_eq!(
        fs::read(destination.join("keep")).check("the marker survives")?,
        b"x",
        "the destination directory is untouched"
    );
    let leftovers = temp_leftovers(outer.path())?;
    assert_eq!(leftovers, 0, "a failed rename removes its temp file");
    Ok(())
}

/// A FIFO would block `File::open` forever, so the guard must refuse it from
/// metadata alone and leave it untouched.
#[cfg(unix)]
#[test]
fn a_fifo_gitignore_is_refused_without_opening_it() -> TestResult {
    use std::os::unix::fs::FileTypeExt;
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = ProjectRoot::resolve(directory.path()).check("valid root")?;
    let path = root.as_path().join(GITIGNORE_FILE);
    create_fifo(&path)?;

    let outcome = outcome_without_blocking(root)?;

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
        other => {
            return Err(format!("a FIFO .gitignore must be refused, got {other:?}"))
                .check("unexpected outcome");
        }
    }
    assert!(
        path.symlink_metadata()
            .check("stat the FIFO")?
            .file_type()
            .is_fifo(),
        "the FIFO is left untouched"
    );
    Ok(())
}

/// Following the symlink must stop at the FIFO's metadata, not at its open.
#[cfg(unix)]
#[test]
fn a_symlink_to_a_fifo_gitignore_is_refused_without_opening_it() -> TestResult {
    use std::os::unix::fs::FileTypeExt;
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = ProjectRoot::resolve(directory.path()).check("valid root")?;
    let fifo = directory.path().join("fifo-target");
    create_fifo(&fifo)?;
    let path = root.as_path().join(GITIGNORE_FILE);
    std::os::unix::fs::symlink(&fifo, &path).check("create the symlink")?;

    let outcome = outcome_without_blocking(root)?;

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
        other => {
            return Err(format!(
                "a symlink to a FIFO must be refused, got {other:?}"
            ))
            .check("unexpected outcome");
        }
    }
    assert!(
        fifo.symlink_metadata()
            .check("stat the FIFO")?
            .file_type()
            .is_fifo(),
        "the FIFO target is left untouched"
    );
    assert!(
        path.symlink_metadata()
            .check("stat the link")?
            .file_type()
            .is_symlink(),
        "the user's link survives"
    );
    Ok(())
}

/// A symlink is refused even when its target is an in-project regular file:
/// the write must never follow a link, so the target keeps its bytes.
#[cfg(unix)]
#[test]
fn a_symlink_to_an_in_project_gitignore_is_refused_and_target_untouched() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = ProjectRoot::resolve(directory.path()).check("valid root")?;
    let target = directory.path().join("shared-ignore");
    fs::write(&target, b"/target\n").check("write the link target")?;
    let path = root.as_path().join(GITIGNORE_FILE);
    std::os::unix::fs::symlink(&target, &path).check("create the symlink")?;

    let outcome = ensure_gitignore(&root);

    match outcome {
        GitignoreOutcome::NotWritten { path: reported, .. } => {
            assert_eq!(reported, path, "the refused path is named");
        }
        other => {
            return Err(format!(
                "a symlinked .gitignore must be refused, got {other:?}"
            ))
            .check("unexpected outcome");
        }
    }
    assert_eq!(
        fs::read(&target).check("read the link target")?,
        b"/target\n".to_vec(),
        "the link target is unchanged"
    );
    Ok(())
}

/// A `.gitignore` symlinked outside the project must not be modified or
/// replaced: content and inode of the victim stay as they were.
#[cfg(unix)]
#[test]
fn a_gitignore_symlinked_outside_the_project_leaves_the_victim_untouched() -> TestResult {
    use std::os::unix::fs::MetadataExt;
    let project = tempfile::tempdir().check("project dir")?;
    let outside = tempfile::tempdir().check("outside dir")?;
    let root = ProjectRoot::resolve(project.path()).check("valid root")?;
    let victim = outside.path().join("victim_rc");
    fs::write(&victim, b"export SECRET=1\n").check("write victim")?;
    let inode = fs::metadata(&victim).check("stat victim")?.ino();
    let path = root.as_path().join(GITIGNORE_FILE);
    std::os::unix::fs::symlink(&victim, &path).check("create the symlink")?;

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
        other => {
            return Err(format!(
                "an escaping symlink must be refused, got {other:?}"
            ))
            .check("unexpected outcome");
        }
    }
    assert_eq!(
        fs::read(&victim).check("read victim")?,
        b"export SECRET=1\n".to_vec(),
        "victim content is unchanged"
    );
    assert_eq!(
        fs::metadata(&victim).check("stat victim")?.ino(),
        inode,
        "victim inode is unchanged"
    );
    assert_eq!(temp_leftovers(project.path())?, 0, "no temp file is left");
    Ok(())
}

/// Replacing a dangling link with a regular file would destroy the user's
/// link, so the guard reports instead.
#[cfg(unix)]
#[test]
fn a_dangling_gitignore_symlink_is_reported_rather_than_replaced() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = ProjectRoot::resolve(directory.path()).check("valid root")?;
    let absent = directory.path().join("missing-target");
    let path = root.as_path().join(GITIGNORE_FILE);
    std::os::unix::fs::symlink(&absent, &path).check("create the dangling symlink")?;

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
        other => {
            return Err(format!(
                "a dangling symlink must not be replaced by a regular file, got {other:?}"
            ))
            .check("unexpected outcome");
        }
    }
    assert!(
        path.symlink_metadata()
            .check("stat the link")?
            .file_type()
            .is_symlink(),
        "the user's link survives"
    );
    assert!(
        !absent.exists(),
        "the write does not create the missing target"
    );
    Ok(())
}

#[test]
fn gitignore_write_syncs_the_parent_and_lands_the_content() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let destination = dir.path().join(".gitignore");

    write_gitignore_atomically(
        &destination,
        b"a\n",
        b".planning/slugaudit/\n",
        sync_directory,
    )
    .check("write and sync the directory")?;

    assert_eq!(
        fs::read(&destination).check("read back")?,
        b"a\n.planning/slugaudit/\n"
    );
    assert_eq!(temp_leftovers(dir.path())?, 0);
    Ok(())
}

#[test]
fn a_failed_directory_sync_after_the_rename_still_reports_a_written_file() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let destination = dir.path().join(".gitignore");

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        write_gitignore_atomically(&destination, b"a\n", b".planning/slugaudit/\n", |_| {
            Err(std::io::Error::other("injected fsync failure"))
        })
    })?;

    result.check("the rename happened, so the write succeeded")?;
    assert_eq!(
        fs::read(&destination).check("read back")?,
        b"a\n.planning/slugaudit/\n"
    );
    assert!(logs.contains("injected fsync failure"), "{logs}");
    assert_eq!(temp_leftovers(dir.path())?, 0);
    Ok(())
}

#[test]
fn a_negation_after_the_ignore_leaves_the_directory_unignored() {
    assert!(!ignores_activation_dir(
        b".planning/slugaudit/\n!.planning/slugaudit/\n"
    ));
    assert!(!ignores_activation_dir(
        b"/.planning/slugaudit\n!/.planning/slugaudit\n"
    ));
}

#[test]
fn an_ignore_after_the_negation_wins_again() {
    assert!(ignores_activation_dir(
        b"!.planning/slugaudit/\n.planning/slugaudit/\n"
    ));
    assert!(ignores_activation_dir(
        b".planning/slugaudit/\n!.planning/slugaudit/\n.planning/slugaudit\n"
    ));
}

#[test]
fn a_negation_alone_or_a_comment_ignores_nothing() {
    assert!(!ignores_activation_dir(b"!.planning/slugaudit/\n"));
    assert!(!ignores_activation_dir(b"# .planning/slugaudit/\n"));
    assert!(ignores_activation_dir(b".planning/slugaudit/\n# note\n"));
}
