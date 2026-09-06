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
/// On Linux, the filesystem type is determined from `/proc/self/mountinfo`.
/// macOS uses its native filesystem inspection command. Other platforms are
/// unsupported by the product and fail closed rather than silently accepting
/// an unknown mount.
/// Emits the operational warning that a network-filesystem rejection
/// happened. Split out so tests can verify the warning site fires
/// without standing up a real NFS mount — the verdict determination
/// still goes through [`reject_network_filesystem`] in production code,
/// which calls this helper on the positive case.
///
/// `pub(super)` so the test module can call it directly with a known
/// `path` argument and assert the emitted event format without making
/// the function part of the external API.
pub(super) fn log_rejected_network_filesystem(path: &Path) {
    // The mountinfo / `stat -f` / `fsutil` round-trip already returns
    // its own error variant on the boundary cases (file vanished,
    // command unavailable, …); this is the *positive* "yep, this is
    // on NFS" verdict. Logged with the path so an operator enabling a
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
/// On Linux, the filesystem type is determined from `/proc/self/mountinfo`.
/// macOS uses its native filesystem inspection command. Other platforms are
/// unsupported by the product and fail closed rather than silently accepting
/// an unknown mount.
pub(super) fn reject_network_filesystem(path: &Path) -> Result<(), StoreError> {
    if is_on_network_filesystem(path)? {
        log_rejected_network_filesystem(path);
        return Err(StoreError::NetworkFilesystem);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn is_on_network_filesystem(path: &Path) -> Result<bool, StoreError> {
    let content = std::fs::read_to_string("/proc/self/mountinfo")
        .map_err(StoreError::NetworkFilesystemCheck)?;

    let path_str = match path.to_str() {
        Some(p) => p,
        None => return Ok(false),
    };

    Ok(is_mountpoint_network_filesystem(&content, path_str))
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

        // Path-boundary match: the path must be the mount point itself or
        // fall under it as a child (`/mnt/nfs/x`), not merely share a
        // string prefix (`/mnt/nfs2/x` is a *different* mount). A bare
        // `starts_with` would falsely reject a local path whose name
        // begins with a network mount point — and per this module's own
        // invariant, blocking a legitimate local open is strictly worse
        // than missing an NFS mount.
        let under_mount = path_str
            .strip_prefix(mount_point)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
        if under_mount && mount_point.len() > best_mount_len {
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
fn is_on_network_filesystem(path: &Path) -> Result<bool, StoreError> {
    #[cfg(target_os = "macos")]
    let output = std::process::Command::new("stat")
        .args(["-f", "%T"])
        .arg(path)
        .output()
        .map_err(StoreError::NetworkFilesystemCheck)?;

    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        return Err(StoreError::NetworkFilesystemCheck(std::io::Error::other(
            "this target is unsupported; SlugAudit supports Linux and macOS",
        )));
    }

    if !output.status.success() {
        return Err(StoreError::NetworkFilesystemCheck(std::io::Error::other(
            "filesystem inspection command failed",
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    Ok(["nfs", "nfs4", "cifs", "smb", "smb3", "ncp", "ncpfs"]
        .iter()
        .any(|kind| text.contains(kind)))
}
#[cfg(test)]
#[path = "netfs_tests.rs"]
mod tests;

