//! The results directory, this process's owner lock, and the startup sweep.
//!
//! Layout: `results/<session>.lock` (advisory lock held for the store's life)
//! and `results/<session>-<result>.db` (unlinked at creation, so a `.db` is
//! only ever seen if the process died in the instant before the unlink). The
//! lock is taken BEFORE any `.db` exists. The sweep deletes the files of a
//! session whose lock it can take (the owner is dead) and of a session with no
//! lock file; it never touches a live owner's files or its own session's.

use super::error::ResultStoreError;
use super::platform;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Attempts at taking a lock file that a sweeper deleted under us.
const LOCK_ATTEMPTS: usize = 3;

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

impl SweepOutcome {
    /// Counts a failure and logs it once, with its cause.
    fn failed(&mut self, context: &'static str, error: &io::Error) {
        tracing::warn!(context, error = %error, "result store sweep problem; paging stays enabled");
        self.errors += 1;
        self.first_error
            .get_or_insert_with(|| format!("{context}: {error}"));
    }

    /// Removes one file; a file that is already gone is not a failure.
    pub(super) fn remove(&mut self, path: &Path, context: &'static str) {
        match std::fs::remove_file(path) {
            Ok(()) => self.removed_files += 1,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => self.failed(context, &error),
        }
    }
}

/// The directory and the lock this process holds on it.
pub(super) struct Owner {
    pub(super) dir: PathBuf,
    pub(super) lock: File,
    pub(super) lock_path: PathBuf,
    pub(super) sweep: SweepOutcome,
}

/// Creates the directory, takes the owner lock, then sweeps.
pub(super) fn open_owner(runtime_dir: &Path, session: Uuid) -> Result<Owner, ResultStoreError> {
    let dir = runtime_dir.join("results");
    platform::ensure_private_dir(&dir)
        .map_err(|error| ResultStoreError::unavailable("prepare the results directory", error))?;
    let lock_path = dir.join(format!("{}.lock", session.hyphenated()));
    let lock = take_lock(&lock_path)?;
    let sweep = sweep(&dir, session);
    Ok(Owner {
        dir,
        lock,
        lock_path,
        sweep,
    })
}

/// Opens and exclusively locks the lock file, verifying the path still names
/// the locked inode (a sweeper may have deleted it between open and lock).
fn take_lock(path: &Path) -> Result<File, ResultStoreError> {
    for _ in 0..LOCK_ATTEMPTS {
        let file = platform::open_lock_file(path)
            .map_err(|error| ResultStoreError::unavailable("open the owner lock", error))?;
        file.try_lock()
            .map_err(|error| ResultStoreError::unavailable("take the owner lock", error))?;
        let current = platform::names_same_file(&file, path)
            .map_err(|error| ResultStoreError::unavailable("verify the owner lock", error))?;
        if current {
            return Ok(file);
        }
    }
    Err(ResultStoreError::unavailable(
        "take the owner lock",
        io::Error::other("the lock file was replaced repeatedly"),
    ))
}

/// `<uuid>.lock` -> the session.
fn parse_lock(name: &str) -> Option<Uuid> {
    parse_hyphenated(name.strip_suffix(".lock")?)
}

/// `<uuid>-<uuid>.db` -> the owning session.
fn parse_db(name: &str) -> Option<Uuid> {
    let stem = name.strip_suffix(".db")?;
    parse_hyphenated(stem.get(..36)?).filter(|_| {
        stem.get(36..37) == Some("-") && stem.get(37..).and_then(parse_hyphenated).is_some()
    })
}

fn parse_hyphenated(text: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(text).ok()?;
    (id.hyphenated().to_string() == text).then_some(id)
}

/// Removes the leftovers of dead owners and of lock-less sessions.
fn sweep(dir: &Path, own: Uuid) -> SweepOutcome {
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
        if *session != own && !has_lock && !lock_exists(dir, *session) {
            outcome.remove(path, "remove an orphaned result file");
        }
    }
    outcome
}

/// Re-checks the filesystem before deleting a lock-less session's file: a
/// live process creates its lock before its first `.db`.
fn lock_exists(dir: &Path, session: Uuid) -> bool {
    dir.join(format!("{}.lock", session.hyphenated()))
        .symlink_metadata()
        .is_ok()
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
        Err(std::fs::TryLockError::WouldBlock) => outcome.live_owners_skipped += 1,
        Err(std::fs::TryLockError::Error(error)) => {
            outcome.failed("lock another session's lock", &error);
        }
    }
}
