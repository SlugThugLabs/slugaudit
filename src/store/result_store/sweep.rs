//! Startup sweep of dead-owner and orphaned result store files.
//!
//! Deletes the files of a session whose `<session>.lock` file can be locked
//! (its owner is dead) and of a session with no lock file; never touches a live
//! owner's files or this process's own session files.

use super::cursor::canonical_uuid;
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// What the startup sweep did; visible in the store stats.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepOutcome {
    /// Files removed (dead owners' databases and lock files, orphans).
    pub removed_files: usize,
    /// Sessions whose lock another live process holds; their files were left.
    pub live_owners_skipped: usize,
    /// Operations that failed; the sweep continued with the rest.
    pub errors: usize,
    /// The first failure's message, kept for the stats.
    pub first_error: Option<String>,
}

/// Adds one to a sweep tally.
fn tally(count: &mut usize) {
    // KEEP saturating: a stats tally has a designed ceiling; at usize::MAX it reads "at least this many" and never wraps to a small number
    *count = count.saturating_add(1);
}

impl SweepOutcome {
    /// Counts a failure and logs it once, with its cause.
    fn failed(&mut self, context: &'static str, error: &io::Error) {
        tracing::warn!(context, error = %error, "result store sweep problem; paging stays enabled");
        tally(&mut self.errors);
        self.first_error
            .get_or_insert_with(|| format!("{context}: {error}"));
    }

    /// Removes one file; a file that is already gone is not a failure.
    pub(super) fn remove(&mut self, path: &Path, context: &'static str) {
        match std::fs::remove_file(path) {
            Ok(()) => tally(&mut self.removed_files),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => self.failed(context, &error),
        }
    }
}

/// `<uuid>.lock` -> the session.
fn parse_lock(name: &str) -> Option<Uuid> {
    canonical_uuid(name.strip_suffix(".lock")?)
}

/// `<uuid>-<uuid>.db` -> the owning session.
fn parse_db(name: &str) -> Option<Uuid> {
    let stem = name.strip_suffix(".db")?;
    canonical_uuid(stem.get(..36)?).filter(|_| {
        stem.get(36..37) == Some("-") && stem.get(37..).and_then(canonical_uuid).is_some()
    })
}

/// Removes the leftovers of dead owners and of lock-less sessions.
pub(super) fn sweep(dir: &Path, own: Uuid) -> SweepOutcome {
    let mut outcome = SweepOutcome::default();
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(error) => {
            outcome.failed("list the results directory", &error);
            return outcome;
        }
    };
    let (mut locks, mut dbs) = (Vec::new(), Vec::new());
    for entry in listing {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                outcome.failed("read a results directory entry", &error);
                continue;
            }
        };
        match entry.file_type() {
            Ok(kind) if kind.is_file() => {}
            Ok(_) => continue,
            Err(error) => {
                outcome.failed("inspect a results directory entry", &error);
                continue;
            }
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if let Some(session) = parse_lock(&name) {
            locks.push((session, entry.path()));
        } else if let Some(session) = parse_db(&name) {
            dbs.push((session, entry.path()));
        }
    }
    for (session, lock_path) in locks.iter().filter(|(session, _)| *session != own) {
        sweep_owner(*session, lock_path, &dbs, &mut outcome);
    }
    for (session, path) in &dbs {
        let has_lock = locks.iter().any(|(owner, _)| owner == session);
        if *session != own && !has_lock {
            sweep_orphan(path, lock_exists(dir, *session), &mut outcome);
        }
    }
    outcome
}

/// Re-checks the filesystem before deleting a lock-less session's file: a
/// live process creates its lock before its first `.db`.
fn lock_exists(dir: &Path, session: Uuid) -> io::Result<bool> {
    lock_presence(
        dir.join(format!("{}.lock", session.hyphenated()))
            .symlink_metadata(),
    )
}

/// Removes a lock-less session's file when its lock is known to be absent.
pub(super) fn sweep_orphan(
    path: &Path,
    lock_present: io::Result<bool>,
    outcome: &mut SweepOutcome,
) {
    match lock_present {
        Ok(false) => outcome.remove(path, "remove an orphaned result file"),
        Ok(true) => {}
        Err(error) => outcome.failed("check for a lock before removing a result file", &error),
    }
}

/// Maps a lock file's metadata lookup to present, absent (`NotFound`) or error.
pub(super) fn lock_presence(metadata: io::Result<std::fs::Metadata>) -> io::Result<bool> {
    match metadata {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Removes a dead owner's files, or skips a live owner.
fn sweep_owner(
    session: Uuid,
    lock_path: &Path,
    dbs: &[(Uuid, PathBuf)],
    outcome: &mut SweepOutcome,
) {
    let lock = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
    {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => return outcome.failed("open another session's lock", &error),
    };
    match lock.try_lock() {
        Ok(()) => {
            for (owner, path) in dbs {
                if *owner == session {
                    outcome.remove(path, "remove a dead owner's result file");
                }
            }
            outcome.remove(lock_path, "remove a dead owner's lock file");
        }
        Err(std::fs::TryLockError::WouldBlock) => tally(&mut outcome.live_owners_skipped),
        Err(std::fs::TryLockError::Error(error)) => {
            outcome.failed("lock another session's lock", &error);
        }
    }
}
