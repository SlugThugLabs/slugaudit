//! Results directory setup, exclusive file creation, and session owner lock.
//!
//! Layout: `results/<session>.lock` (advisory lock held for the store's life)
//! and `results/<session>-<result>.db`. The lock is taken before any `.db`
//! exists and unlinked on [`OwnerLock`] drop.

use super::error::ResultStoreError;
use super::sweep::{self, SweepOutcome};
#[cfg(test)]
use super::sweep::{lock_presence, sweep_orphan};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const LOCK_ATTEMPTS: usize = 3;

/// Creates `dir` if missing or accepts an existing directory without creating
/// missing parents (a missing `.planning/slugaudit` means the project is off).
pub(super) fn ensure_dir(dir: &Path) -> io::Result<()> {
    match std::fs::create_dir(dir) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && dir.is_dir() => Ok(()),
        other => other,
    }
}

/// Creates a brand-new scratch file; an existing path is an error.
pub(super) fn create_new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
}

fn open_lock_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        // FILE_SHARE_READ | FILE_SHARE_WRITE (disallow delete while held).
        options.share_mode(0x0000_0001 | 0x0000_0002);
    }
    options.open(path)
}

/// Guard holding this session's OS lock file and unlinking it on drop.
pub(super) struct OwnerLock {
    lock: Option<File>,
    lock_path: PathBuf,
}

impl Drop for OwnerLock {
    fn drop(&mut self) {
        #[cfg(not(unix))]
        drop(self.lock.take());
        match std::fs::remove_file(&self.lock_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(error = %error, "could not remove the result store lock file");
            }
        }
        #[cfg(unix)]
        drop(self.lock.take());
    }
}

/// The directory, owner lock guard, and sweep outcome for an open store.
pub(super) struct Owner {
    pub(super) dir: PathBuf,
    pub(super) lock: OwnerLock,
    pub(super) sweep: SweepOutcome,
}

/// Creates `results/`, takes the owner lock, and sweeps dead-session leftovers.
pub(super) fn open_owner(runtime_dir: &Path, session: Uuid) -> Result<Owner, ResultStoreError> {
    let dir = runtime_dir.join("results");
    ensure_dir(&dir)
        .map_err(|error| ResultStoreError::unavailable("prepare the results directory", error))?;
    let lock_path = dir.join(format!("{}.lock", session.hyphenated()));
    let lock = take_lock(&lock_path)?;
    let sweep = sweep::sweep(&dir, session);
    Ok(Owner {
        dir,
        lock: OwnerLock {
            lock: Some(lock),
            lock_path,
        },
        sweep,
    })
}

fn take_lock(path: &Path) -> Result<File, ResultStoreError> {
    for _ in 0..LOCK_ATTEMPTS {
        let file = open_lock_file(path)
            .map_err(|error| ResultStoreError::unavailable("open the owner lock", error))?;
        file.try_lock()
            .map_err(|error| ResultStoreError::unavailable("take the owner lock", error))?;
        let current = crate::util::same_open_file(&file, path)
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

#[cfg(test)]
#[path = "owner_tests.rs"]
mod tests;
