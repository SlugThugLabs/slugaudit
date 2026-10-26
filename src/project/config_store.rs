// slugaudit-line-exception: approved-by=slugthug; reason=the one write path of config.json: the cross-process lock, the read-modify-write and the atomic replace guard each other and must not be split apart
//! The one write path of `.planning/slugaudit/config.json`.
//!
//! Every writer (`project_control on` setting `profile`, `mechanic` persisting
//! or removing an entry) goes through [`ProjectConfig::update_raw`], so no
//! writer can replace the file with only its own key. One update is:
//!
//! 1. an exclusive advisory lock on the sibling `config.lock`, held for the
//!    whole read-modify-write, so two servers (or two tools) on one project
//!    cannot lose each other's change. The wait is bounded.
//! 2. a read of the current file as a JSON object (a missing file is an empty
//!    object). A file that cannot be read, is not JSON, or is not an object is
//!    refused and never overwritten.
//! 3. the caller's change, a size check against the cap the server reads
//!    back, and an atomic replace: a temp file in the same directory, written
//!    and synced, then renamed over `config.json`, then the directory synced.
//!    A crash or a full disk leaves the old file or the new one, never a
//!    truncated one.
//!
//! `config.json` and `config.lock` are refused when they are a symlink or not
//! a regular file (A3), and `config.json` is checked again immediately before
//! the rename. The rename replaces the directory entry itself, never a link
//! target.

use super::config::{ConfigError, ProjectConfig};
use serde_json::{Map, Value};
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;

/// How long an update waits for another holder of `config.lock` before it
/// gives up with [`ConfigWriteError::Locked`].
const LOCK_WAIT: Duration = Duration::from_secs(5);
/// Pause between two attempts to take the lock.
const LOCK_POLL: Duration = Duration::from_millis(20);

/// Sequence making temp-file names unique within one process; the process id
/// makes them unique across processes.
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Why an update of `.planning/slugaudit/config.json` did not happen. On
/// every variant except [`ConfigWriteError::NotDurable`] the file is
/// unchanged.
#[derive(Debug, Error)]
pub enum ConfigWriteError {
    /// `config.json` or `config.lock` is a symlink or not a regular file, so
    /// writing would follow a link out of the project or clobber a non-file.
    #[error(
        "refusing to write config file at '{path}': it is a symlink or not a regular file; replace it with a regular file or remove it"
    )]
    NotRegularFile {
        /// The refused path.
        path: PathBuf,
    },
    /// The existing `config.json` is not JSON; it is not overwritten, so
    /// nothing it holds is lost.
    #[error(
        "config file at '{path}' is not valid JSON, so it was not changed; fix or remove it: {source}"
    )]
    Malformed {
        /// Path to the configuration file.
        path: PathBuf,
        /// The parse error.
        source: serde_json::Error,
    },
    /// The existing `config.json` holds JSON that is not an object.
    #[error(
        "config file at '{path}' is not a JSON object, so it was not changed; fix or remove it"
    )]
    NotAnObject {
        /// Path to the configuration file.
        path: PathBuf,
    },
    /// The updated file would be larger than the server reads back.
    #[error("config.json would be {bytes} bytes, over the {limit}-byte limit the server reads")]
    TooLarge {
        /// Encoded size of the updated file.
        bytes: usize,
        /// The read cap.
        limit: u64,
    },
    /// Another process held `config.lock` for the whole bounded wait.
    #[error(
        "config file at '{path}' is locked by another process (waited {waited_ms} ms); nothing was changed"
    )]
    Locked {
        /// Path to the lock file.
        path: PathBuf,
        /// How long this update waited.
        waited_ms: u128,
    },
    /// Locking, reading, or writing failed; nothing was changed.
    #[error("failed to update config file at '{path}': {source}")]
    Io {
        /// The path the failed operation was on.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// `config.json` was replaced, but syncing its directory failed, so the
    /// change may not survive a crash or power loss.
    #[error(
        "config file at '{path}' was updated, but syncing its directory failed, so the update may not survive a crash: {source}"
    )]
    NotDurable {
        /// Path to the configuration file.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> ConfigWriteError + '_ {
    move |source| ConfigWriteError::Io {
        path: path.to_path_buf(),
        source,
    }
}

impl ProjectConfig {
    /// Read-modify-write of `.planning/slugaudit/config.json` under
    /// `project_root`: `change` receives the file's object (empty when the
    /// file does not exist) and every key it does not touch is written back
    /// as read. See the module docs for the lock, the refusals and the
    /// atomic replace.
    ///
    /// # Errors
    ///
    /// `change`'s own error (nothing is written), or a [`ConfigWriteError`]
    /// converted into `E`.
    pub fn update_raw<E: From<ConfigWriteError>>(
        project_root: &Path,
        change: impl FnOnce(&mut Map<String, Value>) -> Result<(), E>,
    ) -> Result<(), E> {
        update_with(project_root, LOCK_WAIT, &mut |_| Ok(()), change)
    }
}

/// The whole `config.json` as JSON, read under the same size cap
/// [`ProjectConfig::load`] uses; `None` when the file does not exist. Takes
/// no lock: the atomic replace means a reader sees the old file or the new
/// one.
pub(super) fn read_value(project_root: &Path) -> Result<Option<Value>, ConfigError> {
    let path = config_path(project_root);
    let exists = path.try_exists().map_err(|source| ConfigError::Io {
        path: path.clone(),
        source,
    })?;
    if !exists {
        return Ok(None);
    }
    let content = crate::util::read_file_capped(&path, crate::util::MAX_FOREIGN_TEXT_BYTES)
        .map_err(|source| ConfigError::Io {
            path: path.clone(),
            source,
        })?;
    serde_json::from_str(&content)
        .map(Some)
        .map_err(|source| ConfigError::Parse { path, source })
}

pub(super) fn config_path(project_root: &Path) -> PathBuf {
    project_root
        .join(".planning")
        .join("slugaudit")
        .join("config.json")
}

/// [`ProjectConfig::update_raw`] with the lock wait and a hook that runs
/// after the temp file is written and synced and before the target is
/// re-checked and replaced (tests use it to fail or race that window).
fn update_with<E: From<ConfigWriteError>>(
    project_root: &Path,
    wait: Duration,
    before_rename: &mut dyn FnMut(&Path) -> std::io::Result<()>,
    change: impl FnOnce(&mut Map<String, Value>) -> Result<(), E>,
) -> Result<(), E> {
    let target = config_path(project_root);
    let lock = lock_path(&target)?;
    let _held = acquire_lock(&lock, wait)?;
    let mut object = read_object(&target)?;
    change(&mut object)?;
    // `{:#}` is the pretty `Display` of a `Value`, which cannot fail.
    let encoded = format!("{:#}", Value::Object(object));
    let limit = crate::util::MAX_FOREIGN_TEXT_BYTES;
    if !u64::try_from(encoded.len()).is_ok_and(|bytes| bytes <= limit) {
        return Err(ConfigWriteError::TooLarge {
            bytes: encoded.len(),
            limit,
        }
        .into());
    }
    replace_atomically(&target, encoded.as_bytes(), before_rename)?;
    Ok(())
}

pub(super) fn lock_path(target: &Path) -> Result<PathBuf, ConfigWriteError> {
    let directory = target.parent().ok_or_else(|| ConfigWriteError::Io {
        path: target.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the config path has no parent directory",
        ),
    })?;
    Ok(directory.join("config.lock"))
}

/// What `symlink_metadata` says about a path this module writes.
fn regular_or_missing(path: &Path) -> Result<Option<std::fs::Metadata>, ConfigWriteError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(Some(metadata)),
        Ok(_) => Err(ConfigWriteError::NotRegularFile {
            path: path.to_path_buf(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigWriteError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Opens `config.lock` without ever following a link: a missing file is
/// created with `create_new` (which refuses any existing entry, a dangling
/// link included); an existing one must be a regular file before the open,
/// and the opened file must still be that same directory entry after it.
fn open_lock_file(path: &Path) -> Result<File, ConfigWriteError> {
    // Two attempts: a concurrent creator can win the `create_new` race, and
    // the second attempt then opens the file it created.
    for _ in 0..2 {
        if regular_or_missing(path)?.is_none() {
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(file) => return Ok(file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => return Err(io_error(path)(source)),
            }
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(io_error(path))?;
        let opened = file.metadata().map_err(io_error(path))?;
        match regular_or_missing(path)? {
            Some(entry) if same_file(&opened, &entry) => return Ok(file),
            _ => {
                return Err(ConfigWriteError::NotRegularFile {
                    path: path.to_path_buf(),
                });
            }
        }
    }
    Err(ConfigWriteError::NotRegularFile {
        path: path.to_path_buf(),
    })
}

/// True when the opened file is the directory entry at the path.
#[cfg(unix)]
fn same_file(opened: &std::fs::Metadata, entry: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    opened.dev() == entry.dev() && opened.ino() == entry.ino()
}

/// Without inode numbers the check is that both are regular files (the
/// entry was refused above otherwise).
#[cfg(not(unix))]
fn same_file(opened: &std::fs::Metadata, entry: &std::fs::Metadata) -> bool {
    opened.file_type().is_file() && entry.file_type().is_file()
}

/// Takes the exclusive lock on `config.lock`, polling for at most `wait`.
/// The lock is released when the returned file is dropped.
pub(super) fn acquire_lock(path: &Path, wait: Duration) -> Result<File, ConfigWriteError> {
    let file = open_lock_file(path)?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(source)) => return Err(io_error(path)(source)),
        }
        if started.elapsed() >= wait {
            return Err(ConfigWriteError::Locked {
                path: path.to_path_buf(),
                waited_ms: started.elapsed().as_millis(),
            });
        }
        std::thread::sleep(LOCK_POLL);
    }
}

/// The current file as an object; a missing file is an empty one.
fn read_object(target: &Path) -> Result<Map<String, Value>, ConfigWriteError> {
    if regular_or_missing(target)?.is_none() {
        return Ok(Map::new());
    }
    let content = crate::util::read_file_capped(target, crate::util::MAX_FOREIGN_TEXT_BYTES)
        .map_err(io_error(target))?;
    match serde_json::from_str(&content) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(_) => Err(ConfigWriteError::NotAnObject {
            path: target.to_path_buf(),
        }),
        Err(source) => Err(ConfigWriteError::Malformed {
            path: target.to_path_buf(),
            source,
        }),
    }
}

/// Writes `contents` to a new temp file beside `target`, syncs it, re-checks
/// `target`, renames the temp file over it and syncs the directory. The temp
/// file is removed on every failure before the rename.
fn replace_atomically(
    target: &Path,
    contents: &[u8],
    before_rename: &mut dyn FnMut(&Path) -> std::io::Result<()>,
) -> Result<(), ConfigWriteError> {
    let directory = target.parent().ok_or_else(|| ConfigWriteError::Io {
        path: target.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the config path has no parent directory",
        ),
    })?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = directory.join(format!("config.json.{}.{sequence}.tmp", std::process::id()));
    // `create_new` refuses any existing entry, so this never follows a link
    // and only ever removes a temp file this call created.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(io_error(&temp))?;
    let staged = write_temp(&mut file, target, contents)
        .map_err(io_error(&temp))
        .and_then(|()| {
            drop(file);
            before_rename(&temp).map_err(io_error(&temp))
        })
        // The A3 rule, checked again as late as possible: the target must
        // still be a regular file (or absent) right before it is replaced.
        .and_then(|()| regular_or_missing(target).map(|_| ()))
        .and_then(|()| std::fs::rename(&temp, target).map_err(io_error(target)));
    if let Err(error) = staged {
        remove_abandoned_temp(&temp);
        return Err(error);
    }
    sync_directory(directory).map_err(|source| ConfigWriteError::NotDurable {
        path: target.to_path_buf(),
        source,
    })
}

/// Commits the rename: on Unix a directory is opened and `fsync`ed like a
/// file.
#[cfg(unix)]
fn sync_directory(directory: &Path) -> std::io::Result<()> {
    File::open(directory).and_then(|handle| handle.sync_all())
}

/// Windows cannot open a directory as a `File` without extra flags, and its
/// `MoveFileEx` replace is not made durable by a directory flush; the rename
/// itself is still atomic.
#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Writes and syncs the temp file, carrying the target's permissions over.
/// `sync_all` surfaces a deferred write error while the rename can still be
/// skipped (`File::drop` would discard it).
fn write_temp(file: &mut File, target: &Path, contents: &[u8]) -> std::io::Result<()> {
    file.write_all(contents)?;
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_file() => {
            file.set_permissions(metadata.permissions())?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    file.sync_all()
}

/// Removes the temp file of a failed update. The update's own error is
/// already on its way to the caller; a cleanup failure is logged once and
/// never replaces it.
fn remove_abandoned_temp(temp: &Path) {
    match std::fs::remove_file(temp) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!(
            path_id = %crate::util::redacted_path(temp),
            %error,
            "failed to remove an abandoned config.json temp file"
        ),
    }
}

#[cfg(test)]
#[path = "config_store_tests.rs"]
mod tests;
