//! The few Unix-only file operations the store needs. Elsewhere they report
//! `Unsupported`, and `ResultStore::open` refuses before reaching them: a
//! platform that cannot keep an unlinked open file readable gets no store.

use std::fs::File;
use std::io;
use std::path::Path;

#[cfg(unix)]
mod imp {
    use super::{File, Path, io};
    use std::fs::{DirBuilder, OpenOptions};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

    /// Creates `dir` with mode 0700, or accepts an existing plain directory
    /// (tightening its mode). A symlink or non-directory is refused.
    pub(in super::super) fn ensure_private_dir(dir: &Path) -> io::Result<()> {
        match std::fs::symlink_metadata(dir) {
            Ok(meta) if meta.is_dir() => {
                if meta.permissions().mode() & 0o077 != 0 {
                    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
                }
                Ok(())
            }
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "results path is a symlink or not a directory",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                DirBuilder::new().mode(0o700).create(dir)
            }
            Err(error) => Err(error),
        }
    }

    /// A new file (never an existing one or a symlink), mode 0600.
    pub(in super::super) fn create_private_file(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }

    /// The owner-lock file, created if missing, never through a symlink.
    /// A new file is made with `create_new` (which does not follow a
    /// symlink); an existing one must be a regular file by `symlink_metadata`
    /// and the opened handle must be that same inode.
    pub(in super::super) fn open_lock_file(path: &Path) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).mode(0o600);
        match options.clone().create_new(true).open(path) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            created => return created,
        }
        let named = std::fs::symlink_metadata(path)?;
        if !named.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "lock path is a symlink or not a file",
            ));
        }
        let file = options.open(path)?;
        let held = file.metadata()?;
        if held.dev() != named.dev() || held.ino() != named.ino() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "lock path changed while opening",
            ));
        }
        Ok(file)
    }

    /// True when `path` still names the inode `file` holds.
    pub(in super::super) fn names_same_file(file: &File, path: &Path) -> io::Result<bool> {
        let held = file.metadata()?;
        match std::fs::symlink_metadata(path) {
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

    pub(in super::super) fn ensure_private_dir(_dir: &Path) -> io::Result<()> {
        unsupported()
    }

    pub(in super::super) fn create_private_file(_path: &Path) -> io::Result<File> {
        unsupported()
    }

    pub(in super::super) fn open_lock_file(_path: &Path) -> io::Result<File> {
        unsupported()
    }

    pub(in super::super) fn names_same_file(_file: &File, _path: &Path) -> io::Result<bool> {
        unsupported()
    }
}

pub(super) use imp::{create_private_file, ensure_private_dir, names_same_file, open_lock_file};
