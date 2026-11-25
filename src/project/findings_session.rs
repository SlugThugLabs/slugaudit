//! The per-process lifetime of a project's findings file.
//!
//! Findings are what the model concluded in this session, so each SlugAudit
//! process keeps them in its own database beside `project.db`,
//! `findings-<id>.db`, and never lets them outlive that process. The first
//! time this process touches a project it picks a fresh id, takes an
//! exclusive OS lock on `findings-<id>.lock`, and keeps that lock until the
//! process exits (or `off` runs). The lock is how another process tells a
//! live session's file from a dead one: see [`super::findings_sweep`]. Every
//! later call (and a rebuild of `project.db`) keeps using the same file.

use super::activation::{ActivationError, activation_dir};
use super::findings_sweep::{lock_error, session_error, sweep_dead_sessions};
use super::root::ProjectRoot;
use crate::util::LockPoisoned;
use std::collections::HashMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The findings file name from before per-session files; the sweep
/// deletes it.
pub(super) const LEGACY_FINDINGS_FILENAME: &str = "findings.db";

/// One project's session in this process: its findings file, its lock path,
/// and the lock handle that keeps the file marked live. The handle is never
/// read; holding it open is the point.
pub(crate) struct Session {
    findings: PathBuf,
    lock_path: PathBuf,
    lock: File,
}

/// Sessions this process has started, keyed by activation directory.
static SESSIONS: Mutex<Option<HashMap<PathBuf, Session>>> = Mutex::new(None);

pub(super) fn db_name(id: &str) -> String {
    format!("findings-{id}.db")
}

pub(super) fn lock_name(id: &str) -> String {
    format!("findings-{id}.lock")
}

/// Returns this process's findings database for the project whose activation
/// directory is `dir`: `<dir>/findings-<id>.db`. `None` before the process
/// first touched the project, or after `off` ended its session. The file
/// itself exists only once a finding was written.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the session table was poisoned.
pub fn session_findings_path(dir: &Path) -> Result<Option<PathBuf>, LockPoisoned> {
    let sessions = crate::util::lock(&SESSIONS, "findings_sessions")?;
    Ok(sessions
        .as_ref()
        .and_then(|sessions| sessions.get(dir))
        // KEEP clone: returns an owned PathBuf so the global SESSIONS MutexGuard can be released
        .map(|session| session.findings.clone()))
}

/// Starts this process's session for `root` on the first call: picks the
/// findings file, locks it as live, then deletes every dead session's file
/// (and an old build's `findings.db`) in the activation directory. Later
/// calls for the same root do nothing.
///
/// # Errors
///
/// Returns [`ActivationError::FindingsSession`] when the lock cannot be
/// taken or a dead session's file cannot be removed; the root then stays
/// untouched, so the next call retries. Returns [`ActivationError::Lock`]
/// when the session table was poisoned.
pub fn begin_process_session(root: &ProjectRoot) -> Result<(), ActivationError> {
    begin_process_session_with(root, |_| Ok(()))
}

/// Core of [`begin_process_session`]; `after_create` runs between creating the
/// own lock file and locking it (a no-op in production; tests inject a
/// concurrent sweep or brief lock holder).
fn begin_process_session_with(
    root: &ProjectRoot,
    after_create: impl FnMut(&Path) -> Result<(), ActivationError>,
) -> Result<(), ActivationError> {
    let dir = activation_dir(root);
    let mut sessions = crate::util::lock(&SESSIONS, "findings_sessions")?;
    let sessions = sessions.get_or_insert_with(HashMap::new);
    if let Some(existing) = sessions.get(&dir) {
        if session_is_live(existing)? {
            return Ok(());
        }
        sessions.remove(&dir);
    }
    let (id, lock_path, lock) = take_own_lock_with(&dir, after_create)?;
    // Own lock first, so a concurrent process's sweep sees this file as live.
    sweep_dead_sessions(&dir, &id)?;
    let findings = dir.join(db_name(&id));
    sessions.insert(
        dir,
        Session {
            findings,
            lock_path,
            lock,
        },
    );
    Ok(())
}

/// Checks whether a cached session's lock file is still the live file on disk.
/// An `off`/`on` cycle in another process or a concurrent `disable` can unlink
/// or replace the lock file while this process still holds its old FD.
fn session_is_live(session: &Session) -> Result<bool, ActivationError> {
    let path_meta = match std::fs::metadata(&session.lock_path) {
        Ok(meta) => meta,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(false);
        }
        Err(source) => return Err(session_error(&session.lock_path, source)),
    };
    same_lock_file(&session.lock, &path_meta, &session.lock_path)
}

#[cfg(unix)]
fn same_lock_file(
    lock: &File,
    path_meta: &std::fs::Metadata,
    lock_path: &Path,
) -> Result<bool, ActivationError> {
    use std::os::unix::fs::MetadataExt;
    let fd_meta = lock
        .metadata()
        .map_err(|source| session_error(lock_path, source))?;
    Ok(path_meta.dev() == fd_meta.dev() && path_meta.ino() == fd_meta.ino())
}

#[cfg(not(unix))]
fn same_lock_file(
    _lock: &File,
    _path_meta: &std::fs::Metadata,
    _lock_path: &Path,
) -> Result<bool, ActivationError> {
    Ok(true)
}

/// Attempts at taking the own lock file that a concurrent sweep deleted.
const LOCK_ATTEMPTS: usize = 3;

/// Creates a fresh own lock file and locks it. A sweep running in another
/// process can list the file after it is created and lock it before this
/// process does, then delete it as a dead session's. Locking the deleted file
/// would hold a lock nobody can see, so after locking, the path must still
/// exist; if not, start over with a fresh UUID so a deleted lock is never
/// recreated under the same ID.
fn take_own_lock_with(
    dir: &Path,
    mut after_create: impl FnMut(&Path) -> Result<(), ActivationError>,
) -> Result<(String, PathBuf, File), ActivationError> {
    let mut last_path = dir.to_path_buf();
    for _ in 0..LOCK_ATTEMPTS {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let lock_path = dir.join(lock_name(&id));
        // Owner decision (2026-10-31): the lock only tells a live session's findings file from a dead one; each agent session keeps its own findings and never wipes another's.
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|source| session_error(&lock_path, source))?;
        after_create(&lock_path)?;
        lock_when_free(&lock, &lock_path)?;
        if lock_path
            .try_exists()
            .map_err(|source| session_error(&lock_path, source))?
        {
            return Ok((id, lock_path, lock));
        }
        last_path = lock_path;
    }
    Err(session_error(
        &last_path,
        std::io::Error::other("the lock file was deleted repeatedly while locking it"),
    ))
}

/// How long the own lock may stay held by someone else, and how often to
/// look again.
const LOCK_WAIT_STEPS: u32 = 100;
const LOCK_WAIT_STEP: std::time::Duration = std::time::Duration::from_millis(10);

/// Locks the own, freshly named file. Nobody legitimately holds a lock on it,
/// but briefly someone may: a sweep in another process that listed the file
/// and is locking it to delete it, or a child process that was forked while a
/// sweep's handle was open and has not yet exec'd (a forked descriptor shares
/// the lock until the child closes it). Both let go within moments, so
/// "locked" waits a bounded time (one second) before it counts as a failure.
fn lock_when_free(lock: &File, lock_path: &Path) -> Result<(), ActivationError> {
    for _ in 0..LOCK_WAIT_STEPS {
        match lock.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => std::thread::sleep(LOCK_WAIT_STEP),
            Err(TryLockError::Error(source)) => return Err(session_error(lock_path, source)),
        }
    }
    Err(lock_error(lock_path))
}

/// Ends this process's session for `root`: closes the lock handle so the
/// activation directory can be deleted (a locked file cannot be deleted
/// through another handle on Windows). The next call starts a new session.
/// Called by `disable` before it removes the directory.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the session table was poisoned.
pub(super) fn end_process_session(root: &ProjectRoot) -> Result<(), LockPoisoned> {
    if let Some(sessions) = crate::util::lock(&SESSIONS, "findings_sessions")?.as_mut() {
        sessions.remove(&activation_dir(root));
    }
    Ok(())
}

/// Test seam: takes `root`'s session out of the process-wide set, so the next
/// call behaves like the first call of a fresh process. Dropping the returned
/// session releases its lock (the old process died); holding it keeps that
/// file live (a second process still running). Per root, because the set is
/// process-wide and other tests use their own roots.
#[cfg(test)]
pub(crate) fn take_session_for_test(root: &ProjectRoot) -> Result<Option<Session>, LockPoisoned> {
    Ok(crate::util::lock(&SESSIONS, "findings_sessions")?
        .as_mut()
        .and_then(|sessions| sessions.remove(&activation_dir(root))))
}

/// Test seam: makes `session` this process's session for `root` again,
/// returning the one it replaces.
#[cfg(test)]
pub(crate) fn put_session_for_test(
    root: &ProjectRoot,
    session: Session,
) -> Result<Option<Session>, LockPoisoned> {
    Ok(crate::util::lock(&SESSIONS, "findings_sessions")?
        .get_or_insert_with(HashMap::new)
        .insert(activation_dir(root), session))
}

/// Test seam: forgets `root`'s session, like [`take_session_for_test`] with
/// the process then exiting.
#[cfg(test)]
pub(crate) fn forget_root_for_test(root: &ProjectRoot) -> Result<(), LockPoisoned> {
    drop(take_session_for_test(root)?);
    Ok(())
}

#[cfg(test)]
#[path = "findings_session_tests.rs"]
mod tests;
