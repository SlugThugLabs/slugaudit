//! The few Unix-only file operations the store needs. Elsewhere they report
//! `Unsupported`, and `ResultStore::open` refuses before reaching them: a
//! platform that cannot keep an unlinked open file readable gets no store.

use std::fs::File;
use std::io;
use std::path::Path;

#[cfg(unix)]
mod imp {
    use super::{File, Path, io};
    use std::fs::OpenOptions;
    use std::os::unix::fs::MetadataExt;

    /// Creates `dir` if it does not exist yet.
    pub(in super::super) fn ensure_dir(dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }

    /// A new file; an existing one is an error.
    pub(in super::super) fn create_new_file(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
    }

    /// The owner-lock file, created if missing.
    pub(in super::super) fn open_lock_file(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
    }

    /// True when `path` still names the inode `file` holds.
    pub(in super::super) fn names_same_file(file: &File, path: &Path) -> io::Result<bool> {
        let held = file.metadata()?;
        match std::fs::metadata(path) {
            Ok(named) => Ok(named.dev() == held.dev() && named.ino() == held.ino()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use super::{File, Path, io};

    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    pub(in super::super) fn ensure_dir(_dir: &Path) -> io::Result<()> {
        unsupported()
    }

    pub(in super::super) fn create_new_file(_path: &Path) -> io::Result<File> {
        unsupported()
    }

    pub(in super::super) fn open_lock_file(_path: &Path) -> io::Result<File> {
        unsupported()
    }

    pub(in super::super) fn names_same_file(_file: &File, _path: &Path) -> io::Result<bool> {
        unsupported()
    }
}

pub(super) use imp::{create_new_file, ensure_dir, names_same_file, open_lock_file};
