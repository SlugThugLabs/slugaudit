// slugaudit-line-exception: approved-by=agent; reason=open/configure/permissions/symlink guards are a single atomic open contract; network-filesystem detection already split into netfs.rs
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;
use thiserror::Error;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Owner-only access for newly created database files. Existing files with
/// broader access are rejected rather than silently exposed.
#[cfg(unix)]
const PRIVATE_MODE: u32 = 0o600;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("failed to open database: {0}")]
    Open(#[source] rusqlite::Error),
    #[error("failed to configure database connection: {0}")]
    Configure(#[source] rusqlite::Error),
    #[error(transparent)]
    Migration(#[from] super::migrations::MigrationError),
    #[error("refusing to open a database path that is a symlink")]
    Symlink,
    #[error("failed to set database file permissions: {0}")]
    Permissions(#[source] std::io::Error),
    #[error(
        "refusing to open a database on a network filesystem (NFS/CIFS/SMB): \
         SQLite WAL mode is unreliable on network mounts and can produce locking \
         corruption, stale reads, or SQLITE_BUSY/SQLITE_IOERR errors. \
         Move the project to a local filesystem, or deactivate and re-enable \
         SlugAudit after relocating it."
    )]
    NetworkFilesystem,
    #[error("could not verify whether the database is on a network filesystem: {0}")]
    NetworkFilesystemCheck(#[source] std::io::Error),
}

impl StoreError {
    #[must_use]
    pub fn is_corruption(&self) -> bool {
        match self {
            Self::Open(error) | Self::Configure(error) => matches!(
                error,
                rusqlite::Error::SqliteFailure(failure, _)
                    if matches!(
                        failure.code,
                        rusqlite::ErrorCode::DatabaseCorrupt
                            | rusqlite::ErrorCode::NotADatabase
                    )
            ),
            Self::Migration(error) => error.is_corruption(),
            Self::Symlink
            | Self::Permissions(_)
            | Self::NetworkFilesystem
            | Self::NetworkFilesystemCheck(_) => false,
        }
    }
}

/// Discards a corrupt derived database and its SQLite journal sidecars.
/// Callers must recreate the schema and republish from project files.
pub fn discard_corrupt_database(path: &Path) -> Result<(), std::io::Error> {
    for suffix in ["", "-wal", "-shm"] {
        let candidate = if suffix.is_empty() {
            path.to_path_buf()
        } else {
            Path::new(&format!("{}{}", path.display(), suffix)).to_path_buf()
        };
        match std::fs::remove_file(candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Opens with `SQLITE_OPEN_NOFOLLOW` so a path that is (or becomes) a
/// symlink is rejected atomically by SQLite rather than by a separate
/// `lstat` that races with the open. The pre-check remains as a clearer
/// error for the common case where the path is already a symlink.
use super::netfs::reject_network_filesystem;

fn reject_symlink(path: &Path) -> Result<(), StoreError> {
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        // Logged at the rejection moment so an operator investigating
        // "why did this open fail" sees the exact path that was a
        // symlink, even when only the typed `StoreError::Symlink`
        // surfaces to the tool-call failure log. The `target` keeps
        // these warnings filterable as a single category.
        tracing::warn!(
            target: "slugaudit::store",
            path = %path.display(),
            reason = "symlink",
            "refusing to open a database path that resolves to a symlink; \
             SQLite opens follow targets, which would silently cross a \
             permission or backing-file boundary"
        );
        return Err(StoreError::Symlink);
    }
    Ok(())
}

fn open_flags(read_write: bool) -> OpenFlags {
    let mut flags = OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    if read_write {
        flags |= OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
    } else {
        flags |= OpenFlags::SQLITE_OPEN_READ_ONLY;
    }
    flags
}

/// If `path` doesn't exist yet, creates it with owner-only permissions set
/// atomically at creation time (`O_CREAT | O_EXCL` plus the mode, in one
/// syscall) so there is never a window where the file exists with a wider,
/// umask-derived mode before being tightened after the fact. If another
/// process wins the race and creates the file first, `create_new` fails
/// with `AlreadyExists`; that's fine — the file already exists, so we fall
/// through and let `Connection::open_with_flags` open it as-is. We never
/// `chmod` a file this call didn't create: doing so on a path we didn't
/// just create atomically would be a TOCTOU/symlink race against whatever
/// actually created it (the `reject_symlink` pre-check plus
/// `SQLITE_OPEN_NOFOLLOW` already guard the open itself).
#[cfg(unix)]
fn create_with_private_permissions_if_missing(path: &Path) -> Result<(), StoreError> {
    use std::io::ErrorKind;
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PRIVATE_MODE)
        .open(path)
    {
        Ok(_file) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(StoreError::Permissions(error)),
    }
}

#[cfg(not(unix))]
fn create_with_private_permissions_if_missing(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

/// Refuse existing database files that are readable or writable by group or
/// other users. The index can contain source-derived evidence and must not be
/// opened under weaker permissions merely because it was created elsewhere.
#[cfg(unix)]
fn reject_insecure_permissions(path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(StoreError::Permissions(error)),
    };
    let mode = metadata.permissions().mode();
    if mode & 0o077 != 0 {
        tracing::warn!(
            target: "slugaudit::store",
            path = %path.display(),
            mode = format!("{mode:o}"),
            reason = "insecure_permissions",
            "refusing to open a database file with broader-than-owner permissions; \
             the index can contain source-derived evidence and must not be opened \
             under group- or world-readable/writable modes"
        );
        return Err(StoreError::Permissions(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("database mode {mode:o} is broader than owner-only 600"),
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn reject_insecure_permissions(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

/// Opens a read-write connection, creating the database file if needed, and
/// brings its schema up to date. This is the only connection sync/store
/// repositories write through.
///
/// # Errors
///
/// Returns an error if the file can't be opened/created, if pragmas can't
/// be applied, if the schema can't be migrated to the current version
/// (including when the database is from a newer, unsupported version), or
/// if the database path resides on a network filesystem (NFS/CIFS/SMB)
/// where SQLite WAL mode is unreliable.
pub fn open_read_write(path: &Path) -> Result<Connection, StoreError> {
    reject_symlink(path)?;
    reject_network_filesystem(path)?;
    create_with_private_permissions_if_missing(path)?;
    reject_insecure_permissions(path)?;
    let mut connection =
        Connection::open_with_flags(path, open_flags(true)).map_err(StoreError::Open)?;
    configure(&connection)?;
    super::migrations::ensure_current_schema(&mut connection)?;
    Ok(connection)
}

/// Opens a connection that cannot write no matter what SQL it executes.
/// This is the safety boundary for the `query` tool: correctness comes from
/// the connection itself, never from inspecting query text. Requires an
/// already-migrated database; it never creates or alters the schema.
///
/// # Errors
///
/// Returns an error if the file doesn't exist or can't be opened read-only,
/// if the busy timeout can't be configured, or if the database path resides
/// on a network filesystem (NFS/CIFS/SMB) where SQLite WAL mode is
/// unreliable.
pub fn open_read_only(path: &Path) -> Result<Connection, StoreError> {
    reject_symlink(path)?;
    reject_network_filesystem(path)?;
    reject_insecure_permissions(path)?;
    let connection =
        Connection::open_with_flags(path, open_flags(false)).map_err(StoreError::Open)?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::Configure)?;
    Ok(connection)
}

fn configure(connection: &Connection) -> Result<(), StoreError> {
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(StoreError::Configure)?;
    connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |_row| Ok(()))
        .map_err(StoreError::Configure)?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::Configure)?;
    Ok(())
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
