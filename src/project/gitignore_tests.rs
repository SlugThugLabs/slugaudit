//! Failure-arm tests for the `.gitignore` activation guard.
//!
//! The happy paths live in `activation_gitignore_tests.rs`; these tests reach the
//! writer's own error arms, which a fixture that fails at the read cannot.

use super::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::fs;

/// Counts temp files the atomic writer may have left behind in `directory`.
fn temp_leftovers(directory: &Path) -> TestResult<usize> {
    let mut count = 0usize;
    for entry in fs::read_dir(directory).check("read the directory")? {
        let entry = entry.check("read a directory entry")?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(".slugaudit-tmp-")
        {
            count = count.checked_add(1).check("entry count fits usize")?;
        }
    }
    Ok(count)
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
            ensure_eq(&reported, &path, "the unwritable path is named")?;
            ensure(!reason.is_empty(), "the write failure carries a reason")?;
        }
        other => {
            return Err(format!(
                "a failed write must report NotWritten, not Added, got {other:?}"
            ))
            .check("unexpected outcome");
        }
    }
    ensure(
        !path.exists(),
        "no file is created when the atomic write cannot start",
    )?;
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

    ensure(
        result.is_err(),
        format!("renaming a file over a directory fails: {result:?}"),
    )?;
    ensure_eq(
        &fs::read(destination.join("keep")).check("the marker survives")?,
        &b"x",
        "the destination directory is untouched",
    )?;
    let leftovers = temp_leftovers(outer.path())?;
    ensure_eq(&leftovers, &0, "a failed rename removes its temp file")?;
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

    ensure_eq(
        &fs::read(&destination).check("read back")?,
        &b"a\n.planning/slugaudit/\n",
        "fs::read(&destination).check(\"read back\")? == b\"a\\n.planning/slugaudit/\\n\"",
    )?;
    ensure_eq(
        &temp_leftovers(dir.path())?,
        &0,
        "temp_leftovers(dir.path())? == 0",
    )?;
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
    ensure_eq(
        &fs::read(&destination).check("read back")?,
        &b"a\n.planning/slugaudit/\n",
        "fs::read(&destination).check(\"read back\")? == b\"a\\n.planning/slugaudit/\\n\"",
    )?;
    ensure(logs.contains("injected fsync failure"), &logs)?;
    ensure_eq(
        &temp_leftovers(dir.path())?,
        &0,
        "temp_leftovers(dir.path())? == 0",
    )?;
    Ok(())
}

#[test]
fn a_negation_after_the_ignore_leaves_the_directory_unignored() -> TestResult {
    ensure(
        !ignores_activation_dir(b".planning/slugaudit/\n!.planning/slugaudit/\n"),
        "!ignores_activation_dir( b\".planning/slugaudit/\\n!.planning/slugaudit/\\n\" )",
    )?;
    ensure(
        !ignores_activation_dir(b"/.planning/slugaudit\n!/.planning/slugaudit\n"),
        "!ignores_activation_dir( b\"/.planning/slugaudit\\n!/.planning/slugaudit\\n\" )",
    )?;
    Ok(())
}

#[test]
fn an_ignore_after_the_negation_wins_again() -> TestResult {
    ensure(
        ignores_activation_dir(b"!.planning/slugaudit/\n.planning/slugaudit/\n"),
        "ignores_activation_dir( b\"!.planning/slugaudit/\\n.planning/slugaudit/\\n\" )",
    )?;
    ensure(
        ignores_activation_dir(
            b".planning/slugaudit/\n!.planning/slugaudit/\n.planning/slugaudit\n",
        ),
        "ignores_activation_dir( b\".planning/slugaudit/\\n!.planning/slugaudit/\\n.planning/slugaudit\\n\" )",
    )?;
    Ok(())
}

#[test]
fn a_negation_alone_or_a_comment_ignores_nothing() -> TestResult {
    ensure(
        !ignores_activation_dir(b"!.planning/slugaudit/\n"),
        "!ignores_activation_dir(b\"!.planning/slugaudit/\\n\")",
    )?;
    ensure(
        !ignores_activation_dir(b"# .planning/slugaudit/\n"),
        "!ignores_activation_dir(b\"# .planning/slugaudit/\\n\")",
    )?;
    ensure(
        ignores_activation_dir(b".planning/slugaudit/\n# note\n"),
        "ignores_activation_dir(b\".planning/slugaudit/\\n# note\\n\")",
    )?;
    Ok(())
}
