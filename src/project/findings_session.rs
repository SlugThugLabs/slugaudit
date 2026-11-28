// slugaudit-line-exception: approved-by=split-judge; reason=single per-process findings-<id>.db lifecycle: session lock acquisition, liveness check, and dead-session lock sweep share the same file naming and lock protocol
//! Per-process lifetime and dead-session sweep for `findings-<id>.db`.
//!
//! Each SlugAudit server process holds an exclusive advisory lock on
//! `findings-<id>.lock` beside its `findings-<id>.db` and deletes unlocked
//! `findings-*.db` files left behind by finished processes.

use super::activation::{ActivationError, ProjectRoot, activation_dir};
use crate::store::remove_database_files;
use crate::util::LockPoisoned;
use std::collections::{BTreeSet, HashMap};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const LEGACY_FINDINGS_FILENAME: &str = "findings.db";
const LOCK_ATTEMPTS: usize = 3;
const LOCK_WAIT_STEPS: u32 = 100;
const LOCK_WAIT_STEP: std::time::Duration = std::time::Duration::from_millis(10);

pub(crate) struct Session {
    findings: PathBuf,
    lock_path: PathBuf,
    lock: File,
}

static SESSIONS: Mutex<Option<HashMap<PathBuf, Session>>> = Mutex::new(None);

fn db_name(id: &str) -> String {
    format!("findings-{id}.db")
}

fn lock_name(id: &str) -> String {
    format!("findings-{id}.lock")
}

fn session_error(path: &Path, source: std::io::Error) -> ActivationError {
    tracing::error!(path = %path.display(), error = %source, "could not set up the session findings files");
    ActivationError::FindingsSession {
        path: path.to_path_buf(),
        source,
    }
}

fn lock_error(path: &Path) -> ActivationError {
    session_error(
        path,
        std::io::Error::other("the new session's lock file stayed locked by another holder"),
    )
}

/// Returns this process's findings database path (`<dir>/findings-<id>.db`) if
/// a session has been started for activation directory `dir`.
///
/// # Errors
///
/// Returns `LockPoisoned` when the process findings-session table was poisoned.
pub fn session_findings_path(dir: &Path) -> Result<Option<PathBuf>, LockPoisoned> {
    let sessions = crate::util::lock(&SESSIONS, "findings_sessions")?;
    Ok(sessions
        .as_ref()
        .and_then(|sessions| sessions.get(dir))
        .map(|session| session.findings.clone()))
}

/// Starts this process's findings session for `root` on first call, acquiring
/// its lock and sweeping dead sessions' files.
///
/// # Errors
///
/// Returns [`ActivationError`] if acquiring the session lock, sweeping dead
/// session files, or locking the process session table fails.
pub fn begin_process_session(root: &ProjectRoot) -> Result<(), ActivationError> {
    begin_process_session_with(root, |_| Ok(()))
}

fn begin_process_session_with(
    root: &ProjectRoot,
    after_create: impl FnMut(&Path) -> Result<(), ActivationError>,
) -> Result<(), ActivationError> {
    let dir = activation_dir(root);
    let mut sessions = crate::util::lock(&SESSIONS, "findings_sessions")?;
    let sessions = sessions.get_or_insert_with(HashMap::new);
    if let Some(existing) = sessions.get(&dir) {
        let live = crate::util::same_open_file(&existing.lock, &existing.lock_path)
            .map_err(|source| session_error(&existing.lock_path, source))?;
        if live {
            return Ok(());
        }
        sessions.remove(&dir);
    }
    let (id, lock_path, lock) = take_own_lock_with(&dir, after_create)?;
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

fn take_own_lock_with(
    dir: &Path,
    mut after_create: impl FnMut(&Path) -> Result<(), ActivationError>,
) -> Result<(String, PathBuf, File), ActivationError> {
    let mut last_path = dir.to_path_buf();
    for _ in 0..LOCK_ATTEMPTS {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let lock_path = dir.join(lock_name(&id));
        // Owner decision (2026-10-31): the lock only tells a live session's findings file from a dead one; each agent session keeps its own findings and never wipes another's.
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            // FILE_SHARE_READ | FILE_SHARE_WRITE (disallow delete while held).
            options.share_mode(0x0000_0001 | 0x0000_0002);
        }
        let lock = options
            .open(&lock_path)
            .map_err(|source| session_error(&lock_path, source))?;
        after_create(&lock_path)?;
        lock_when_free(&lock, &lock_path)?;
        if crate::util::same_open_file(&lock, &lock_path)
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

fn session_id_of(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("findings-")?;
    let id = rest
        .strip_suffix(".db")
        .or_else(|| rest.strip_suffix(".lock"))?;
    (!id.is_empty()).then_some(id)
}

fn sweep_dead_sessions(dir: &Path, own_id: &str) -> Result<(), ActivationError> {
    let mut others = BTreeSet::new();
    for entry in std::fs::read_dir(dir).map_err(|source| session_error(dir, source))? {
        let entry = entry.map_err(|source| session_error(dir, source))?;
        if let Some(id) = entry.file_name().to_str().and_then(session_id_of)
            && id != own_id
        {
            others.insert(id.to_owned());
        }
    }
    let legacy = dir.join(LEGACY_FINDINGS_FILENAME);
    let legacy_existed = legacy
        .try_exists()
        .map_err(|source| session_error(&legacy, source))?;
    remove_database_files(&legacy).map_err(|source| session_error(&legacy, source))?;
    if legacy_existed {
        tracing::info!("deleted the findings database an older build left behind");
    }
    for id in &others {
        reap_if_dead(dir, id)?;
    }
    Ok(())
}

fn reap_if_dead(dir: &Path, id: &str) -> Result<(), ActivationError> {
    let lock_path = dir.join(lock_name(id));
    let lock: Option<File> = match OpenOptions::new().write(true).open(&lock_path) {
        Ok(file) => Some(file),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => return Err(session_error(&lock_path, source)),
    };
    if let Some(lock) = &lock {
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(()),
            Err(TryLockError::Error(source)) => return Err(session_error(&lock_path, source)),
        }
    }
    let findings = dir.join(db_name(id));
    remove_database_files(&findings).map_err(|source| session_error(&findings, source))?;
    #[cfg(not(unix))]
    drop(lock);
    match std::fs::remove_file(&lock_path) {
        Ok(()) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(session_error(&lock_path, source)),
    }
    tracing::info!("deleted a finished session's findings database");
    Ok(())
}

pub(super) fn end_process_session(root: &ProjectRoot) -> Result<(), LockPoisoned> {
    if let Some(sessions) = crate::util::lock(&SESSIONS, "findings_sessions")?.as_mut() {
        sessions.remove(&activation_dir(root));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn take_session_for_test(root: &ProjectRoot) -> Result<Option<Session>, LockPoisoned> {
    Ok(crate::util::lock(&SESSIONS, "findings_sessions")?
        .as_mut()
        .and_then(|sessions| sessions.remove(&activation_dir(root))))
}

#[cfg(test)]
pub(crate) fn put_session_for_test(
    root: &ProjectRoot,
    session: Session,
) -> Result<Option<Session>, LockPoisoned> {
    Ok(crate::util::lock(&SESSIONS, "findings_sessions")?
        .get_or_insert_with(HashMap::new)
        .insert(activation_dir(root), session))
}

#[cfg(test)]
pub(crate) fn forget_root_for_test(root: &ProjectRoot) -> Result<(), LockPoisoned> {
    drop(take_session_for_test(root)?);
    Ok(())
}

#[cfg(test)]
#[path = "findings_session_tests.rs"]
mod tests;
