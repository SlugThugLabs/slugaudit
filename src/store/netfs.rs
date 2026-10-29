//! Refuses to open a SQLite database that lives on a network filesystem.
//!
//! SQLite's WAL journal mode — which every read-write connection enables —
//! is unreliable on NFS/CIFS/SMB mounts: POSIX locking semantics don't
//! translate cleanly across the network protocol, which can produce locking
//! corruption, stale reads, or persistent `SQLITE_BUSY`/`SQLITE_IOERR`
//! failures. The busy timeout compounds it by making a fundamental
//! incompatibility look like transient contention.

use super::connection::StoreError;
use std::path::Path;

/// Refuses to open a database whose path resides on a network filesystem
/// (NFS, CIFS, or SMB). SQLite's WAL journal mode — which `configure`
/// enables on every read-write connection — is unreliable on network
/// mounts: POSIX locking semantics don't translate cleanly across the
/// network protocol, which can produce locking corruption, stale reads, or
/// persistent `SQLITE_BUSY`/`SQLITE_IOERR` failures. The 5-second busy
/// timeout compounds the problem by masking the underlying issue as a
/// transient contention rather than a fundamental incompatibility.
///
/// The filesystem is identified with `fstatfs` on the database's directory
/// (Linux `f_type` magic numbers, macOS `f_fstypename`); no mount table is
/// read. Other platforms are unsupported by the product and fail closed
/// rather than silently accepting an unknown mount.
/// Emits the operational warning that a network-filesystem rejection
/// happened. Split out so tests can verify the warning site fires
/// without standing up a real NFS mount — the verdict determination
/// still goes through [`reject_network_filesystem`] in production code,
/// which passes the verdict to [`reject_if_network`], the helper that
/// calls this one on the positive case.
///
/// `pub(super)` so the test module can call it directly with a known
/// `path` argument and assert the emitted event format without making
/// the function part of the external API.
pub(super) fn log_rejected_network_filesystem(path: &Path) {
    // The `fstatfs` probe already returns its own error variant on the
    // boundary cases (directory vanished, syscall failed, …); this is the
    // *positive* "yep, this is on NFS" verdict. Logged with the path so an operator enabling a
    // project on `/team-share/foo` (via `project_control`) can tell which
    // filesystem the server diagnosed, not just that the open failed.
    tracing::warn!(
        target: "slugaudit::store",
        path = %path.display(),
        reason = "network_filesystem",
        "refusing to open a database on a network filesystem; \
         SQLite WAL locking is unreliable on NFS/CIFS/SMB and can \
         produce locking corruption, stale reads, or SQLITE_BUSY/\
         SQLITE_IOERR errors. Move the project to a local \
         filesystem, or deactivate and re-enable after relocating"
    );
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
/// The filesystem is identified with `fstatfs` on the database's directory
/// (Linux `f_type` magic numbers, macOS `f_fstypename`); no mount table is
/// read. Other platforms are unsupported by the product and fail closed
/// rather than silently accepting an unknown mount.
pub(super) fn reject_network_filesystem(path: &Path) -> Result<(), StoreError> {
    reject_if_network(path, is_on_network_filesystem(path)?)
}

/// The rejection decision once the filesystem verdict is known. Split out
/// so the positive branch — the typed `NetworkFilesystem` refusal and its
/// warning — is testable with a synthetic verdict, without standing up a
/// real network mount.
fn reject_if_network(path: &Path, on_network: bool) -> Result<(), StoreError> {
    if on_network {
        log_rejected_network_filesystem(path);
        return Err(StoreError::NetworkFilesystem);
    }
    Ok(())
}

/// Linux `statfs` magic numbers (`linux/magic.h`) of network and cluster
/// filesystems on which SQLite WAL locking is unreliable: NFS, SMB, CIFS,
/// SMB2, NCP, Coda, AFS (both the legacy and kAFS magics), Ceph, Lustre and
/// 9P. Generic FUSE (`0x65735546`) is deliberately absent: the kernel reports
/// the same magic for `sshfs` and for local FUSE filesystems such as
/// `gocryptfs`, so FUSE cannot be classified without false positives, and
/// blocking a legitimate local open is strictly worse than missing a mount.
#[cfg(any(target_os = "linux", test))]
const LINUX_NETWORK_MAGICS: &[u32] = &[
    0x0000_6969, // NFS_SUPER_MAGIC
    0x0000_517B, // SMB_SUPER_MAGIC
    0xFF53_4D42, // CIFS_MAGIC_NUMBER
    0xFE53_4D42, // SMB2_MAGIC_NUMBER
    0x0000_564C, // NCP_SUPER_MAGIC
    0x7375_7245, // CODA_SUPER_MAGIC
    0x5346_414F, // AFS_SUPER_MAGIC
    0x6B41_4653, // AFS_FS_MAGIC (kAFS)
    0x00C3_6400, // CEPH_SUPER_MAGIC
    0x0BD0_0BD0, // LL_SUPER_MAGIC (Lustre)
    0x0102_1997, // V9FS_MAGIC (9P)
];

/// Pure classifier: whether a Linux `statfs` `f_type` magic is a network
/// filesystem.
#[cfg(any(target_os = "linux", test))]
fn is_network_filesystem_magic(magic: u32) -> bool {
    LINUX_NETWORK_MAGICS.contains(&magic)
}

/// Pure classifier: whether a macOS/BSD `f_fstypename` is a network
/// filesystem.
#[cfg(any(target_os = "macos", test))]
fn is_network_filesystem_type_name(name: &str) -> bool {
    matches!(name, "nfs" | "smbfs" | "afpfs" | "webdav" | "cifs" | "ftp")
}

/// Keeps the low 32 bits of a `statfs` `f_type`. The kernel's `f_type` is a
/// signed word on some targets, where a magic such as CIFS's `0xFF534D42`
/// is reported sign-extended or truncated depending on the word width.
#[cfg(target_os = "linux")]
fn magic_from_f_type(f_type: impl Into<i128>) -> std::io::Result<u32> {
    u32::try_from(f_type.into() & 0xFFFF_FFFF)
        .map_err(|error| std::io::Error::other(format!("statfs f_type out of range: {error}")))
}

/// What `fstatfs` says about the filesystem holding a database directory.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fstatfs_is_network(directory: &std::fs::File) -> std::io::Result<bool> {
    let stat = rustix::fs::fstatfs(directory)?;
    #[cfg(target_os = "linux")]
    {
        Ok(is_network_filesystem_magic(magic_from_f_type(stat.f_type)?))
    }
    #[cfg(target_os = "macos")]
    {
        let name: Vec<u8> = stat
            .f_fstypename
            .iter()
            .map(|byte| byte.to_ne_bytes()[0])
            .take_while(|byte| *byte != 0)
            .collect();
        let name = std::str::from_utf8(&name)
            .map_err(|error| std::io::Error::other(format!("f_fstypename: {error}")))?;
        Ok(is_network_filesystem_type_name(name))
    }
}

/// Opens the nearest existing ancestor directory of `path`. The database
/// file may not exist yet (the first open creates it) and its directory may
/// not either, but whichever directory will hold it lives on the filesystem
/// of its nearest existing ancestor. Only "not found" moves to the next
/// ancestor; every other error fails closed.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_nearest_existing_directory(path: &Path) -> std::io::Result<std::fs::File> {
    let mut last_error = None;
    for ancestor in path.ancestors().skip(1) {
        let candidate = if ancestor.as_os_str().is_empty() {
            Path::new(".")
        } else {
            ancestor
        };
        match std::fs::File::open(candidate) {
            Ok(directory) => return Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => last_error = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last_error
        .unwrap_or_else(|| std::io::Error::other("the database path has no directory to inspect")))
}

/// Probes the filesystem under `path` with `probe` (production: `fstatfs`).
/// Any failure to open the directory or run the probe fails closed with the
/// source error preserved, and is logged once here.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_network_with(
    path: &Path,
    probe: impl FnOnce(&std::fs::File) -> std::io::Result<bool>,
) -> Result<bool, StoreError> {
    open_nearest_existing_directory(path)
        .and_then(|directory| probe(&directory))
        .map_err(|error| {
            tracing::warn!(
                target: "slugaudit::store",
                path = %path.display(),
                error = %error,
                "could not determine the filesystem type of the database path"
            );
            StoreError::NetworkFilesystemCheck(error)
        })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_on_network_filesystem(path: &Path) -> Result<bool, StoreError> {
    is_network_with(path, fstatfs_is_network)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn is_on_network_filesystem(path: &Path) -> Result<bool, StoreError> {
    let _ = path;
    Err(StoreError::NetworkFilesystemCheck(std::io::Error::other(
        "this target is unsupported; SlugAudit supports Linux and macOS",
    )))
}
#[cfg(test)]
#[path = "netfs_tests.rs"]
mod tests;
