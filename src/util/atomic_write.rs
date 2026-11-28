//! Atomic temp-file replacement and open-descriptor identity helpers shared by
//! config writers, `.gitignore` updates, and session lock files.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static ATOMIC_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes `chunks` to a new sibling temp file in `destination`'s parent
/// directory, preserves `destination`'s permissions when present, calls
/// `sync_file`, renames over `destination`, and commits the directory entry via
/// `sync_directory` (logging any post-rename directory-sync failure without
/// undoing the committed write).
pub(crate) fn write_atomic_chunks_with(
    destination: &Path,
    chunks: &[&[u8]],
    sync_file: impl FnOnce(&File) -> io::Result<()>,
    sync_directory: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic write destination has no parent directory",
        )
    })?;
    let seq = ATOMIC_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".slugaudit-tmp-{}-{seq}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let written = preserve_mode(&file, destination)
        .and_then(|()| {
            for chunk in chunks {
                file.write_all(chunk)?;
            }
            Ok(())
        })
        .and_then(|()| sync_file(&file));
    drop(file);
    if let Err(error) = written.and_then(|()| fs::rename(&temp, destination)) {
        remove_abandoned_temp(&temp);
        return Err(error);
    }
    if let Err(error) = sync_directory(parent) {
        tracing::warn!(
            path = %parent.display(),
            error = %error,
            "file renamed into place, but syncing its parent directory failed"
        );
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn sync_directory(directory: &Path) -> io::Result<()> {
    File::open(directory).and_then(|handle| handle.sync_all())
}

#[cfg(not(unix))]
pub(crate) fn sync_directory(_directory: &Path) -> io::Result<()> {
    Ok(())
}

fn preserve_mode(file: &File, destination: &Path) -> io::Result<()> {
    match fs::metadata(destination) {
        Ok(metadata) => file.set_permissions(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_abandoned_temp(temp: &Path) {
    match fs::remove_file(temp) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(
                path = %temp.display(),
                error = %error,
                "failed to remove temporary file after an aborted atomic write"
            );
        }
    }
}

/// Returns `Ok(true)` when `path` currently resolves to the exact same `(dev,
/// ino)` as the open `file` handle, `Ok(false)` when `path` is gone or replaced.
#[cfg(unix)]
pub(crate) fn same_open_file(file: &File, path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt as _;
    let held = file.metadata()?;
    match fs::metadata(path) {
        Ok(named) => Ok(named.dev() == held.dev() && named.ino() == held.ino()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
pub(crate) fn same_open_file(file: &File, path: &Path) -> io::Result<bool> {
    let held = file.metadata()?;
    match fs::metadata(path) {
        Ok(named) => {
            if !held.is_file() || !named.is_file() || held.len() != named.len() {
                return Ok(false);
            }
            match (held.modified(), named.modified()) {
                (Ok(held_mtime), Ok(named_mtime)) if held_mtime != named_mtime => {
                    return Ok(false);
                }
                (Ok(_), Err(_)) | (Err(_), Ok(_)) => return Ok(false),
                _ => {}
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt as _;
                return Ok(held.creation_time() == named.creation_time()
                    && held.last_write_time() == named.last_write_time()
                    && held.file_attributes() == named.file_attributes());
            }
            #[cfg(not(windows))]
            Ok(true)
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}
