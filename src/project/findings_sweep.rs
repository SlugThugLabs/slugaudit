//! Deleting the findings files of sessions that ended. A session's file is
//! live while its process holds the exclusive lock on the matching
//! `findings-<id>.lock`; a lock that can be taken means the owner is gone.
//! The lock sits on its own empty file, not on the database, because on
//! Windows a locked file also blocks SQLite's own handle on it.

use super::activation::ActivationError;
use super::findings_session::{LEGACY_FINDINGS_FILENAME, db_name, lock_name};
use crate::store::remove_database_files;
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

/// Logs a failure once and shapes the error.
pub(super) fn session_error(path: &Path, source: std::io::Error) -> ActivationError {
    tracing::error!(path = %path.display(), error = %source, "could not set up the session findings files");
    ActivationError::FindingsSession {
        path: path.to_path_buf(),
        source,
    }
}

/// The error for a lock on this process's own, freshly named file that stayed
/// held by someone else for the whole wait.
pub(super) fn lock_error(path: &Path) -> ActivationError {
    session_error(
        path,
        std::io::Error::other("the new session's lock file stayed locked by another holder"),
    )
}

/// The session id in a `findings-<id>.db` or `findings-<id>.lock` file name.
fn session_id_of(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("findings-")?;
    let id = rest
        .strip_suffix(".db")
        .or_else(|| rest.strip_suffix(".lock"))?;
    (!id.is_empty()).then_some(id)
}

/// Deletes the old build's `findings.db` and every other session's file whose
/// lock can be taken, leaving files whose lock is held. `own_id` is skipped.
/// The caller already holds its own lock.
pub(super) fn sweep_dead_sessions(dir: &Path, own_id: &str) -> Result<(), ActivationError> {
    let mut others = BTreeSet::new();
    for entry in std::fs::read_dir(dir).map_err(|source| session_error(dir, source))? {
        let entry = entry.map_err(|source| session_error(dir, source))?;
        // A name that is not UTF-8 is not one of ours.
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

/// Deletes session `id`'s files if its lock can be taken, holding the lock
/// while deleting so no other sweep races it. A session with no lock file is
/// finished too: a live one creates its lock file before anything else, and
/// only a sweep that held the lock ever deletes it. The sweep never creates a
/// lock file, so a deleted one stays deleted.
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
    match std::fs::remove_file(&lock_path) {
        Ok(()) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(session_error(&lock_path, source)),
    }
    tracing::info!("deleted a finished session's findings database");
    Ok(())
}
