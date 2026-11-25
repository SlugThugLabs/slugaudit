use super::*;
use crate::project::enable;
use slugaudit_test_check::{Check, TestResult, ensure};
use std::fs::OpenOptions;

fn project() -> TestResult<(tempfile::TempDir, ProjectRoot)> {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = ProjectRoot::resolve(directory.path()).check("project root")?;
    enable(&root).check("enable")?;
    Ok((directory, root))
}

fn once(
    mut action: impl FnMut(&Path) -> Result<(), ActivationError>,
) -> impl FnMut(&Path) -> Result<(), ActivationError> {
    let mut fired = false;
    move |lock_path| {
        if fired {
            return Ok(());
        }
        fired = true;
        action(lock_path)
    }
}

/// Another process's sweep lists the new lock file and takes its lock before
/// the owner does, then deletes it as a dead session's. The owner must notice
/// and end up holding a lock on a file that exists.
#[test]
fn a_sweep_deleting_the_lock_file_before_it_is_locked_does_not_hide_the_session() -> TestResult {
    let (_directory, root) = project()?;
    let dir = activation_dir(&root);

    begin_process_session_with(
        &root,
        once(|lock_path| {
            let Some(dir) = lock_path.parent() else {
                return Err(ActivationError::FindingsSession {
                    path: lock_path.to_path_buf(),
                    source: std::io::Error::other("the lock path has no parent directory"),
                });
            };
            sweep_dead_sessions(dir, "other")?;
            if lock_path.exists() {
                return Err(ActivationError::FindingsSession {
                    path: lock_path.to_path_buf(),
                    source: std::io::Error::other("the sweep did not delete the unlocked file"),
                });
            }
            Ok(())
        }),
    )
    .check("the session starts anyway")?;

    let findings = session_findings_path(&dir)
        .check("the session table")?
        .check("session started")?;
    let lock_path = findings.with_extension("lock");
    ensure(lock_path.exists(), "the lock file was recreated")?;
    let probe = OpenOptions::new()
        .write(true)
        .open(&lock_path)
        .check("open the lock file")?;
    ensure(
        matches!(probe.try_lock(), Err(TryLockError::WouldBlock)),
        "the recreated file is locked by this process",
    )?;
    Ok(())
}

/// A sweep in another process holds the new lock file for a moment while it
/// decides about it; the owner waits for it instead of failing the call.
#[test]
fn a_lock_held_for_a_moment_by_another_process_delays_the_session_without_failing_it() -> TestResult
{
    let (_directory, root) = project()?;
    let dir = activation_dir(&root);

    begin_process_session_with(
        &root,
        once(|lock_path| {
            let other = OpenOptions::new()
                .write(true)
                .open(lock_path)
                .map_err(|source| session_error(lock_path, source))?;
            other
                .lock()
                .map_err(|source| session_error(lock_path, source))?;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                drop(other);
            });
            Ok(())
        }),
    )
    .check("the session starts once the other holder lets go")?;

    session_findings_path(&dir)
        .check("the session table")?
        .check("the session findings table exists")?;
    Ok(())
}

#[test]
fn unlinking_the_lock_file_causes_begin_process_session_to_start_a_fresh_session() -> TestResult {
    let (_directory, root) = project()?;
    let dir = activation_dir(&root);
    begin_process_session(&root).check("initial session")?;
    let first_findings = session_findings_path(&dir)
        .check("read session table")?
        .check("first session present")?;
    let first_lock = first_findings.with_extension("lock");
    std::fs::remove_file(&first_lock).check("unlink first lock file")?;

    begin_process_session(&root).check("recreate session after unlinked lock")?;

    let second_findings = session_findings_path(&dir)
        .check("read session table after refresh")?
        .check("second session present")?;
    ensure(
        second_findings != first_findings,
        "stale session must be replaced with a new session id",
    )?;
    let second_lock = second_findings.with_extension("lock");
    ensure(second_lock.exists(), "new lock file exists on disk")?;
    let probe = OpenOptions::new()
        .write(true)
        .open(&second_lock)
        .check("open new lock file")?;
    ensure(
        matches!(probe.try_lock(), Err(TryLockError::WouldBlock)),
        "new lock file is locked by this process",
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn replacing_the_lock_file_inode_causes_begin_process_session_to_refresh() -> TestResult {
    let (_directory, root) = project()?;
    let dir = activation_dir(&root);
    begin_process_session(&root).check("initial session")?;
    let first_findings = session_findings_path(&dir)
        .check("read session table")?
        .check("first session present")?;
    let first_lock = first_findings.with_extension("lock");
    std::fs::remove_file(&first_lock).check("unlink first lock file")?;
    std::fs::write(&first_lock, b"").check("create replacement file at same path")?;

    begin_process_session(&root).check("recreate session after inode replacement")?;

    let second_findings = session_findings_path(&dir)
        .check("read session table after refresh")?
        .check("second session present")?;
    ensure(
        second_findings != first_findings,
        "replaced lock inode must trigger a fresh session",
    )?;
    Ok(())
}

#[test]
fn retrying_a_deleted_lock_uses_a_fresh_session_id() -> TestResult {
    let (_directory, root) = project()?;
    let dir = activation_dir(&root);
    let mut deleted_lock = None;

    begin_process_session_with(
        &root,
        once(|lock_path| {
            deleted_lock = Some(lock_path.to_path_buf());
            std::fs::remove_file(lock_path).map_err(|source| session_error(lock_path, source))
        }),
    )
    .check("retry succeeds with a fresh session id")?;

    let first_lock = deleted_lock.check("window hook recorded the deleted lock path")?;
    let findings = session_findings_path(&dir)
        .check("read session table")?
        .check("session started")?;
    let final_lock = findings.with_extension("lock");
    ensure(
        final_lock != first_lock,
        "retry must mint a new session UUID rather than reusing the deleted lock path",
    )?;
    ensure(!first_lock.exists(), "deleted lock path was not recreated")?;
    ensure(final_lock.exists(), "new lock path exists on disk")?;
    Ok(())
}
