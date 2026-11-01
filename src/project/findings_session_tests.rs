use super::*;
use crate::project::enable;
use std::fs::OpenOptions;

fn project() -> (tempfile::TempDir, ProjectRoot) {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("project root");
    enable(&root).expect("enable");
    (directory, root)
}

/// Another process's sweep lists the new lock file and takes its lock before
/// the owner does, then deletes it as a dead session's. The owner must notice
/// and end up holding a lock on a file that exists.
#[test]
fn a_sweep_deleting_the_lock_file_before_it_is_locked_does_not_hide_the_session() {
    let (_directory, root) = project();
    let dir = activation_dir(&root);
    set_window_hook_for_test(|lock_path| {
        let dir = lock_path.parent().expect("lock dir");
        sweep_dead_sessions(dir, "other")?;
        assert!(!lock_path.exists(), "the sweep deleted the unlocked file");
        Ok(())
    });

    begin_process_session(&root).expect("the session starts anyway");

    let findings = session_findings_path(&dir).expect("session started");
    let lock_path = findings.with_extension("lock");
    assert!(lock_path.exists(), "the lock file was recreated");
    let probe = OpenOptions::new()
        .write(true)
        .open(&lock_path)
        .expect("open the lock file");
    assert!(
        matches!(probe.try_lock(), Err(TryLockError::WouldBlock)),
        "the recreated file is locked by this process"
    );
}

/// A sweep in another process holds the new lock file for a moment while it
/// decides about it; the owner waits for it instead of failing the call.
#[test]
fn a_lock_held_for_a_moment_by_another_process_delays_the_session_without_failing_it() {
    let (_directory, root) = project();
    let dir = activation_dir(&root);
    set_window_hook_for_test(|lock_path| {
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
    });

    begin_process_session(&root).expect("the session starts once the other holder lets go");

    assert!(session_findings_path(&dir).is_some());
}
