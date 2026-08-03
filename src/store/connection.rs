use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;
use thiserror::Error;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Owner-only access for newly created database files. Existing files keep
/// whatever mode they already have — we never widen permissions.
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
}

/// Opens with `SQLITE_OPEN_NOFOLLOW` so a path that is (or becomes) a
/// symlink is rejected atomically by SQLite rather than by a separate
/// `lstat` that races with the open. The pre-check remains as a clearer
/// error for the common case where the path is already a symlink.
fn reject_symlink(path: &Path) -> Result<(), StoreError> {
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
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
    let connection =
        Connection::open_with_flags(path, open_flags(false)).map_err(StoreError::Open)?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::Configure)?;
    Ok(connection)
}

/// Refuses to open a database whose path resides on a network filesystem
/// (NFS, CIFS, or SMB). SQLite's WAL journal mode — which `configure`
/// enables on every read-write connection — is unreliable on network
/// mounts: POSIX locking semantics don't translate cleanly across the
/// network protocol, which can produce locking corruption, stale reads, or
/// persistent `SQLITE_BUSY`/`SQLITE_IOERR` failures. The 5-second busy
/// timeout compounds the problem by masking the underlying issue as a
/// transient contention rather than a fundamental incompatibility.
///
/// On Linux, the filesystem type is determined by reading
/// `/proc/self/mountinfo` and finding the most specific mount point that
/// contains the database path, then checking its filesystem type against
/// known network-filesystem names. On non-Linux platforms the check is
/// skipped (returns `Ok(())`) since `/proc/self/mountinfo` is Linux-
/// specific; network filesystems on other platforms are a much narrower
/// slice of the audience, and a wrong positive here would block legitimate
/// local-filesystem use.
fn reject_network_filesystem(path: &Path) -> Result<(), StoreError> {
    if is_on_network_filesystem(path) {
        return Err(StoreError::NetworkFilesystem);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn is_on_network_filesystem(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string("/proc/self/mountinfo") else {
        // If /proc/self/mountinfo is unavailable (e.g. sandboxed
        // environment), err on the side of allowing the open rather
        // than blocking legitimate use. The NFS risk is unchanged, but
        // a missing mount table is not itself evidence of a network
        // mount.
        return false;
    };

    let path_str = match path.to_str() {
        Some(p) => p,
        None => return false,
    };

    is_mountpoint_network_filesystem(&content, path_str)
}

/// Given the raw contents of `/proc/self/mountinfo` and a target path,
/// returns true if the most specific mount point covering the path is a
/// known network filesystem. Extracted as a pure function so the parsing
/// logic can be unit-tested without an actual `/proc` mount table.
#[cfg(target_os = "linux")]
fn is_mountpoint_network_filesystem(mountinfo: &str, path_str: &str) -> bool {
    let mut best_fs_type: Option<&str> = None;
    let mut best_mount_len: usize = 0;

    for line in mountinfo.lines() {
        let Some(dash_idx) = line.find(" - ") else {
            continue;
        };
        let after_dash = &line[dash_idx + 3..];
        let Some(fs_type) = after_dash.split_whitespace().next() else {
            continue;
        };

        let before_dash = &line[..dash_idx];
        let mut fields = before_dash.split_whitespace();
        // Skip: mount ID, parent ID, major:minor, root
        let mount_point = fields.nth(4).unwrap_or("");

        if path_str.starts_with(mount_point) && mount_point.len() > best_mount_len {
            best_mount_len = mount_point.len();
            best_fs_type = Some(fs_type);
        }
    }

    let Some(fs_type) = best_fs_type else {
        return false;
    };
    matches!(
        fs_type,
        "nfs" | "nfs4" | "cifs" | "smb" | "smb3" | "ncp" | "ncpfs" | "fusectl"
    )
}

#[cfg(not(target_os = "linux"))]
fn is_on_network_filesystem(_path: &Path) -> bool {
    false
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
mod tests {
    use super::*;

    #[test]
    fn foreign_keys_are_enabled_on_a_read_write_connection() {
        let directory = tempfile::tempdir().expect("temp dir");
        let connection = open_read_write(&directory.path().join("project.db")).expect("open");
        let enabled: i64 = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("read pragma");
        assert_eq!(enabled, 1);
    }

    #[test]
    fn a_read_only_connection_cannot_write() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        open_read_write(&path).expect("create database");

        let read_only = open_read_only(&path).expect("open read-only");
        let result = read_only.execute("DELETE FROM findings", []);
        assert!(result.is_err());
    }

    #[test]
    fn read_only_open_fails_against_a_missing_database() {
        let directory = tempfile::tempdir().expect("temp dir");
        let result = open_read_only(&directory.path().join("missing.db"));
        assert!(matches!(result, Err(StoreError::Open(_))));
    }

    /// A corrupted/non-SQLite file at the database path must fail closed
    /// with a typed error, not panic and not silently treat garbage bytes
    /// as an empty database (SQLite validates lazily on first real access,
    /// not at `open_with_flags` itself, so this only surfaces once
    /// `configure` issues its first pragma).
    #[test]
    fn a_corrupted_database_file_fails_closed_with_a_typed_error() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        std::fs::write(&path, b"not a sqlite database, just garbage bytes").expect("write garbage");

        let result = open_read_write(&path);
        assert!(
            matches!(result, Err(StoreError::Configure(_))),
            "expected a typed Configure error, got {result:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_db_path_is_rejected_for_both_read_write_and_read_only() {
        let directory = tempfile::tempdir().expect("temp dir");
        let real_target = directory.path().join("elsewhere.db");
        let link_path = directory.path().join("project.db");
        std::os::unix::fs::symlink(&real_target, &link_path).expect("create symlink");

        assert!(matches!(
            open_read_write(&link_path),
            Err(StoreError::Symlink)
        ));
        assert!(matches!(
            open_read_only(&link_path),
            Err(StoreError::Symlink)
        ));
        assert!(
            !real_target.exists(),
            "the symlink target must never be created/opened"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_newly_created_database_gets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        open_read_write(&path).expect("create database");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, PRIVATE_MODE);
    }

    /// The file is created with `O_CREAT | O_EXCL` and the private mode in
    /// one syscall (see `create_with_private_permissions_if_missing`), so
    /// there is no separate "create, then chmod" step for a single-threaded
    /// test to catch mid-window. What we can prove here: losing the create
    /// race to another process (simulated by pre-creating the file with a
    /// wider mode, as a concurrent creator might under a permissive umask)
    /// doesn't panic, doesn't get chmod'd out from under its actual owner,
    /// and still yields a working, migrated connection.
    #[cfg(unix)]
    #[test]
    fn losing_the_concurrent_create_race_still_opens_cleanly_without_touching_permissions() {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");

        // Simulate another process winning the create race with a wider,
        // umask-derived mode.
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(&path)
            .expect("simulate a concurrent creator");

        let connection = open_read_write(&path).expect("open the already-created database");
        let enabled: i64 = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("connection is usable");
        assert_eq!(enabled, 1);

        let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o644,
            "must never chmod a file this call didn't create"
        );
    }

    /// Distinct from `a_symlinked_db_path_is_rejected_for_both_read_write_and_read_only`,
    /// where the path is a symlink from the very first open: this proves the
    /// TOCTOU case, where a path that was legitimately a real file at one
    /// point in time gets replaced by a symlink later (e.g. a compromised or
    /// misbehaving process racing the legitimate owner). Both `open_flags`'
    /// `SQLITE_OPEN_NOFOLLOW` and the `reject_symlink` pre-check must catch
    /// this on the *next* open, not just the first.
    #[cfg(unix)]
    #[test]
    fn a_db_path_replaced_by_a_symlink_after_a_successful_open_is_rejected_on_the_next_open() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");

        // A real, legitimate first open succeeds and migrates the schema.
        let connection = open_read_write(&path).expect("first open on a real file succeeds");
        drop(connection);
        assert!(!path.symlink_metadata().unwrap().file_type().is_symlink());

        // The path is now replaced by a symlink pointing elsewhere, as a
        // TOCTOU attacker (or a racing process) might.
        let elsewhere = directory.path().join("elsewhere.db");
        std::fs::remove_file(&path).expect("remove the real file");
        std::os::unix::fs::symlink(&elsewhere, &path).expect("replace it with a symlink");

        assert!(matches!(open_read_write(&path), Err(StoreError::Symlink)));
        assert!(matches!(open_read_only(&path), Err(StoreError::Symlink)));
        assert!(
            !elsewhere.exists(),
            "the symlink target must never be created/opened through the replaced path"
        );
    }

    /// A real temp-dir database on a local filesystem must open cleanly —
    /// the NFS check must not produce false positives against ordinary
    /// local paths. This is the most important invariant: blocking a
    /// legitimate local open is strictly worse than missing an NFS mount.
    #[test]
    fn a_local_filesystem_database_is_not_mistaken_for_network_filesystem() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        let result = open_read_write(&path);
        assert!(
            !matches!(result, Err(StoreError::NetworkFilesystem)),
            "a local temp-dir database must not be rejected as a network filesystem, got: {result:?}"
        );
    }

    /// The mountinfo parser must pick the most specific (longest) mount
    /// point that covers the path, not just the first match. A path under
    /// `/mnt/nfs/project` should match the `nfs` mount, not the root `/`
    /// ext4 mount.
    #[cfg(target_os = "linux")]
    #[test]
    fn mountinfo_parser_picks_the_most_specific_mount_point() {
        let mountinfo = "\
26 23 0:24 / /sys rw,nosuid,nodev shared:7 - sysfs sysfs rw
27 23 0:25 / /proc rw,nosuid,nodev shared:8 - proc proc rw
1 23 259:1 / / rw shared:1 - ext4 /dev/sda1 rw
100 1 0:100 / /mnt/nfs rw,relatime shared:50 - nfs server:/export rw
101 100 0:101 /project /mnt/nfs/project rw,relatime shared:51 - nfs server:/export/project rw
";
        // Path under the most specific NFS mount.
        assert!(is_mountpoint_network_filesystem(
            mountinfo,
            "/mnt/nfs/project/lib.rs"
        ));
        // Path under root (ext4) — not a network filesystem.
        assert!(!is_mountpoint_network_filesystem(mountinfo, "/home/user/project/lib.rs"));
        // Path under /sys — not a network filesystem.
        assert!(!is_mountpoint_network_filesystem(mountinfo, "/sys/class/net"));
    }

    /// The mountinfo parser must recognize all the common network
    /// filesystem type names, not just `nfs`.
    #[cfg(target_os = "linux")]
    #[test]
    fn mountinfo_parser_recognizes_cifs_and_smb_as_network_filesystems() {
        for fs_type in ["nfs", "nfs4", "cifs", "smb", "smb3"] {
            let mountinfo = format!("1 0 0:1 / /mnt/share rw - {} server:/share rw", fs_type);
            assert!(
                is_mountpoint_network_filesystem(&mountinfo, "/mnt/share/file.txt"),
                "{} should be detected as a network filesystem",
                fs_type
            );
        }
    }

    /// A mountinfo line with optional fields (the `shared:NN` tokens and
    /// others before the ` - ` separator) must still parse correctly —
    /// the ` - ` is the reliable anchor, not field position from the start.
    #[cfg(target_os = "linux")]
    #[test]
    fn mountinfo_parser_handles_optional_fields_before_the_dash_separator() {
        let mountinfo = "52 23 0:44 / /home/user rw,relatime shared:32 master:1 - nfs4 server:/data rw,vers=4.2";
        assert!(is_mountpoint_network_filesystem(
            mountinfo,
            "/home/user/project/lib.rs"
        ));
    }

    /// Malformed mountinfo lines (missing the ` - ` separator, missing
    /// fields, empty lines) must be skipped gracefully rather than
    /// panicking or misidentifying the filesystem.
    #[cfg(target_os = "linux")]
    #[test]
    fn mountinfo_parser_skips_malformed_lines_without_panicking() {
        let mountinfo = "\
not a valid line at all
1 0 0:1 / / rw - ext4 /dev/sda1 rw
another bad line
missing dash separator / rw ext4
";
        // Should not panic; should identify / as ext4 (not network).
        assert!(!is_mountpoint_network_filesystem(mountinfo, "/home/user/file.txt"));
    }
}
