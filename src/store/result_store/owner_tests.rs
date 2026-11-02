//! The sweep's lock check: only `NotFound` means no lock, and a file is never
//! deleted on an unknown answer.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult};

fn orphan_file() -> TestResult<(tempfile::TempDir, PathBuf)> {
    let dir = tempfile::tempdir().check("temp dir")?;
    let path = dir.path().join("orphan.db");
    std::fs::write(&path, b"x").check("write the orphan file")?;
    Ok((dir, path))
}

#[test]
fn lock_presence_reads_not_found_as_absent_and_other_errors_as_errors() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let present = lock_presence(std::fs::symlink_metadata(dir.path())).check("a present path")?;
    let absent = lock_presence(std::fs::symlink_metadata(dir.path().join("missing")))
        .check("a missing path is absent, not an error")?;
    let denied = lock_presence(Err(io::Error::from(io::ErrorKind::PermissionDenied)))
        .check_err("EACCES is not absence")?;
    let broken =
        lock_presence(Err(io::Error::other("EIO"))).check_err("an I/O error is not absence")?;

    assert!(present);
    assert!(!absent);
    assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(broken.to_string(), "EIO");
    Ok(())
}

#[test]
fn sweep_orphan_keeps_the_file_and_records_the_error_when_the_lock_check_failed() -> TestResult {
    let (_dir, path) = orphan_file()?;
    let mut outcome = SweepOutcome::default();

    sweep_orphan(
        &path,
        Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        &mut outcome,
    );

    assert!(
        path.exists(),
        "the file must not be deleted on a failed check"
    );
    assert_eq!(outcome.removed_files, 0);
    assert_eq!(outcome.errors, 1);
    assert_eq!(
        outcome.first_error.as_deref(),
        Some(
            "check for a lock before removing a result file: \
             permission denied"
        )
    );
    Ok(())
}

#[test]
fn sweep_orphan_keeps_the_file_when_a_lock_exists() -> TestResult {
    let (_dir, path) = orphan_file()?;
    let mut outcome = SweepOutcome::default();

    sweep_orphan(&path, Ok(true), &mut outcome);

    assert!(path.exists());
    assert_eq!(outcome, SweepOutcome::default());
    Ok(())
}

#[test]
fn sweep_orphan_removes_the_file_only_when_the_lock_is_known_absent() -> TestResult {
    let (_dir, path) = orphan_file()?;
    let mut outcome = SweepOutcome::default();

    sweep_orphan(&path, Ok(false), &mut outcome);

    assert!(!path.exists());
    assert_eq!(outcome.removed_files, 1);
    assert_eq!(outcome.errors, 0);
    Ok(())
}
