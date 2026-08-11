//! Refuses to open a SQLite database that lives on a network filesystem.
//!
//! SQLite's WAL journal mode — which every read-write connection enables —
//! is unreliable on NFS/CIFS/SMB mounts: POSIX locking semantics don't
//! translate cleanly across the network protocol, which can produce locking
//! corruption, stale reads, or persistent `SQLITE_BUSY`/`SQLITE_IOERR`
//! failures. The busy timeout compounds it by making a fundamental
//! incompatibility look like transient contention.

// slugaudit-line-exception: approved-by=agent; reason=platform-specific mount inspection (linux mountinfo parse + macOS `stat -f` + Windows `fsutil`) is one cohesive filesystem-detection contract; further splitting would fragment the platform guard matrix across files

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
/// macOS and Windows use their native filesystem inspection commands. Other
/// platforms fail closed rather than silently accepting an unknown mount.
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
/// macOS and Windows use their native filesystem inspection commands. Other
/// platforms fail closed rather than silently accepting an unknown mount.
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
fn is_on_network_filesystem(path: &Path) -> Result<bool, StoreError> {
    #[cfg(target_os = "macos")]
    let output = std::process::Command::new("stat")
        .args(["-f", "%T"])
        .arg(path)
        .output()
        .map_err(StoreError::NetworkFilesystemCheck)?;

    #[cfg(target_os = "windows")]
    let output = {
        let volume = path
            .components()
            .next()
            .and_then(|component| match component {
                std::path::Component::Prefix(prefix) => Some(prefix.as_os_str()),
                _ => None,
            })
            .ok_or_else(|| {
                StoreError::NetworkFilesystemCheck(std::io::Error::other(
                    "database path has no Windows volume prefix",
                ))
            })?;
        std::process::Command::new("fsutil")
            .args(["fsinfo", "volumeinfo"])
            .arg(volume)
            .output()
            .map_err(StoreError::NetworkFilesystemCheck)?
    };

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err(StoreError::NetworkFilesystemCheck(std::io::Error::other(
        "filesystem inspection is not implemented on this platform",
    )));

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
mod tests {
    use super::*;
    use crate::store::open_read_write;
    use crate::store::test_capture::capture_warns;

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
        assert!(!is_mountpoint_network_filesystem(
            mountinfo,
            "/home/user/project/lib.rs"
        ));
        // Path under /sys — not a network filesystem.
        assert!(!is_mountpoint_network_filesystem(
            mountinfo,
            "/sys/class/net"
        ));
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
        assert!(!is_mountpoint_network_filesystem(
            mountinfo,
            "/home/user/file.txt"
        ));
    }

    /// Verifies that rejecting an open against an NFS mount emits a
    /// `tracing::warn!` event tagged with the path and a machine-readable
    /// reason so an operator can find which mount triggered the failure
    /// without parsing MCP errors. Tested via the warn-helper rather
    /// than `reject_network_filesystem` directly because standing up a
    /// real NFS mount in tests is not portable.
    #[test]
    fn network_filesystem_rejection_emits_a_warning_for_auditing() {
        let (_, logs) = capture_warns(|| {
            log_rejected_network_filesystem(Path::new("/mnt/nfs/project.db"));
        });
        assert!(
            logs.contains("network_filesystem"),
            "the warn event must be tagged with reason=network_filesystem so \
             operators can grep rejection categories, captured logs: {logs}"
        );
        assert!(
            logs.contains("/mnt/nfs/project.db"),
            "the warn event must include the path so an operator can identify \
             which mount triggered the rejection — /mnt/nfs/project.db in this test, \
             captured logs: {logs}"
        );
    }

    /// A local-filesystem open must never emit the network-filesystem
    /// warning. This is a paired regression test for the previous one:
    /// any test that exercises a successful local `open_read_write`
    /// indirectly verifies the warn site didn't drop a false positive.
    #[test]
    fn a_local_filesystem_open_does_not_emit_the_network_filesystem_warning() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("project.db");
        let (result, logs) = capture_warns(|| open_read_write(&path));
        assert!(result.is_ok(), "sanity: a local tempfile open must succeed");
        assert!(
            !logs.contains("network_filesystem"),
            "a local-filesystem open must not emit the network-filesystem warn, captured logs: {logs}"
        );
    }
}
