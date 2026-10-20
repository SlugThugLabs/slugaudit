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

/// A network filesystem mounted at `/` covers every path under it. The
/// mount-point boundary check must treat `/` as matching every absolute
/// path: `strip_prefix("/")` leaves a tail without its leading slash, so
/// a prefix-tail check alone would classify an NFS-root host as local and
/// let `open_read_write` enable WAL on NFS.
#[cfg(target_os = "linux")]
#[test]
fn mountinfo_parser_detects_a_network_filesystem_mounted_at_root() {
    let mountinfo = "\
1 0 0:1 / / rw shared:1 - nfs4 server:/export rw
2 1 259:1 / /home/user/local rw shared:2 - ext4 /dev/sda2 rw
";
    assert!(
        is_mountpoint_network_filesystem(
            mountinfo,
            "/home/user/project/.planning/slugaudit/project.db"
        ),
        "a path under an NFS root must be classified as network"
    );
    // A more specific local mount still wins over the network root.
    assert!(
        !is_mountpoint_network_filesystem(mountinfo, "/home/user/local/project.db"),
        "a local mount below a network root must cover its own descendants"
    );
}

/// A local filesystem mounted at `/` must stay local, including for the
/// database path that lives directly under it.
#[cfg(target_os = "linux")]
#[test]
fn mountinfo_parser_keeps_a_local_root_local() {
    let mountinfo = "1 0 0:1 / / rw shared:1 - ext4 /dev/sda1 rw\n";
    assert!(!is_mountpoint_network_filesystem(
        mountinfo,
        "/home/user/project/.planning/slugaudit/project.db"
    ));
    assert!(!is_mountpoint_network_filesystem(mountinfo, "/"));
}

/// The root special case must not replace the path-boundary matching for
/// non-root mounts: a network mount below a local root still matches its
/// descendants and still rejects a sibling that only shares a prefix.
#[cfg(target_os = "linux")]
#[test]
fn mountinfo_parser_still_matches_non_root_network_mounts_by_path_boundary() {
    let mountinfo = "\
1 0 0:1 / / rw shared:1 - ext4 /dev/sda1 rw
100 1 0:100 / /mnt/nfs rw,relatime shared:50 - nfs server:/export rw
";
    assert!(is_mountpoint_network_filesystem(
        mountinfo,
        "/mnt/nfs/project/lib.rs"
    ));
    assert!(!is_mountpoint_network_filesystem(
        mountinfo,
        "/mnt/nfs2/project/lib.rs"
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

/// A path that merely *begins with* a network mount point string but
/// is not actually under it — `/home/user2` vs an NFS mount at
/// `/home/user` — must not be rejected as a network filesystem. A
/// bare `starts_with` prefix check would false-positive here, and
/// blocking a legitimate local open is strictly worse than missing
/// an NFS mount.
#[cfg(target_os = "linux")]
#[test]
fn mountinfo_parser_respects_path_boundaries_not_string_prefixes() {
    let mountinfo = "\
1 0 0:1 / /home/user rw - nfs server:/export rw
";
    // `/home/user2` is a sibling directory with its own (presumably
    // local) mount — it must not match the `/home/user` NFS mount.
    assert!(!is_mountpoint_network_filesystem(
        mountinfo,
        "/home/user2/project/lib.rs"
    ));
    // The mount point itself and real children under it still match.
    assert!(is_mountpoint_network_filesystem(
        mountinfo,
        "/home/user/project/lib.rs"
    ));
    assert!(is_mountpoint_network_filesystem(mountinfo, "/home/user"));
}

/// A mountinfo line with optional fields (the `shared:NN` tokens and
/// others before the ` - ` separator) must still parse correctly —
/// the ` - ` is the reliable anchor, not field position from the start.
#[cfg(target_os = "linux")]
#[test]
fn mountinfo_parser_handles_optional_fields_before_the_dash_separator() {
    let mountinfo =
        "52 23 0:44 / /home/user rw,relatime shared:32 master:1 - nfs4 server:/data rw,vers=4.2";
    assert!(is_mountpoint_network_filesystem(
        mountinfo,
        "/home/user/project/lib.rs"
    ));
}

/// The kernel escapes space, tab, newline and backslash in mount points as
/// `\040`, `\011`, `\012`, `\134`; matching must happen on the decoded path.
#[cfg(target_os = "linux")]
#[test]
fn mountinfo_octal_escapes_in_mount_points_are_decoded_before_matching() {
    let mountinfo = "\
30 1 0:25 / /mnt/my\\040share rw - cifs //srv/share rw
31 1 0:26 / /mnt/tab\\011dir rw - nfs4 srv:/x rw
32 1 0:27 / /mnt/back\\134slash rw - nfs srv:/y rw
33 1 0:28 / /mnt/new\\012line rw - nfs srv:/z rw
";

    assert!(is_mountpoint_network_filesystem(
        mountinfo,
        "/mnt/my share/project.db"
    ));
    assert!(is_mountpoint_network_filesystem(
        mountinfo,
        "/mnt/tab\tdir/project.db"
    ));
    assert!(is_mountpoint_network_filesystem(
        mountinfo,
        "/mnt/back\\slash/project.db"
    ));
    assert!(is_mountpoint_network_filesystem(
        mountinfo,
        "/mnt/new\nline/project.db"
    ));
    assert!(!is_mountpoint_network_filesystem(
        mountinfo,
        "/mnt/my/project.db"
    ));
}

/// An escape that is not a full three-digit octal sequence is kept
/// literally rather than dropped or guessed at.
#[cfg(target_os = "linux")]
#[test]
fn mountinfo_malformed_octal_escapes_are_kept_literally() {
    let mountinfo = "30 1 0:25 / /mnt/a\\04 rw - nfs srv:/x rw\n";

    assert!(is_mountpoint_network_filesystem(mountinfo, "/mnt/a\\04/f"));
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
/// without parsing MCP errors.
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
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn unsupported_targets_fail_closed() {
    let error = is_on_network_filesystem(Path::new("project.db"))
        .expect_err("unsupported targets must fail closed");
    assert!(matches!(error, StoreError::NetworkFilesystemCheck(_)));
}

/// The positive verdict is the branch that returns the typed refusal: a
/// synthetic "this path is on a network filesystem" verdict must produce
/// exactly `StoreError::NetworkFilesystem` and the operator warning, with
/// no real network mount involved.
#[test]
fn a_positive_network_verdict_is_rejected_with_the_typed_error() {
    let path = Path::new("/mnt/nfs/project.db");
    let (result, logs) = capture_warns(|| reject_if_network(path, true));

    assert!(
        matches!(&result, Err(StoreError::NetworkFilesystem)),
        "a positive verdict must be refused, got: {result:?}"
    );
    assert!(
        logs.contains("network_filesystem"),
        "the refusal must be logged with its machine-readable reason, captured logs: {logs}"
    );
    assert!(
        logs.contains("/mnt/nfs/project.db"),
        "the warning must name the offending path, captured logs: {logs}"
    );
}

/// The negative control: a local verdict is accepted and emits no
/// rejection warning. The liveness line proves the absence check is not
/// passing because the capture went blind.
#[test]
fn a_local_verdict_is_accepted_without_a_rejection_warning() {
    let path = Path::new("/home/user/project.db");
    let (result, logs) = capture_warns(|| {
        tracing::warn!("verdict liveness control");
        reject_if_network(path, false)
    });

    assert!(
        matches!(result, Ok(())),
        "a local verdict must be accepted, got: {result:?}"
    );
    assert!(
        logs.contains("verdict liveness control"),
        "the capture must see this thread's events, captured logs: {logs}"
    );
    assert!(
        !logs.contains("network_filesystem"),
        "a local verdict must not emit the rejection warning, captured logs: {logs}"
    );
}

#[test]
fn a_local_filesystem_open_does_not_emit_the_network_filesystem_warning() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    let (result, logs) = capture_warns(|| {
        // Liveness control: proves the absence check below cannot pass
        // because the capture went blind.
        tracing::warn!("capture liveness control");
        open_read_write(&path)
    });
    assert!(result.is_ok(), "sanity: a local tempfile open must succeed");
    assert!(
        logs.contains("capture liveness control"),
        "the capture must see this thread's events, or the absence check \
         below proves nothing, captured logs: {logs}"
    );
    assert!(
        !logs.contains("network_filesystem"),
        "a local-filesystem open must not emit the network-filesystem warn, captured logs: {logs}"
    );
}

const MACOS_MOUNT_TABLE: &str = "\
/dev/disk3s1s1 on / (apfs, sealed, local, read-only, journaled)
/dev/disk3s5 on /System/Volumes/Data (apfs, local, journaled, nobrowse)
map auto_home on /System/Volumes/Data/home (autofs, automounted, nobrowse)
//user@nas/share on /Volumes/share (smbfs, nodev, nosuid, mounted by user)
nas:/export on /Volumes/share/nfs (nfs, nodev, nosuid)
";

#[test]
fn the_macos_mount_table_treats_a_local_apfs_path_as_local() {
    assert!(!is_mount_table_network_filesystem(
        MACOS_MOUNT_TABLE,
        "/System/Volumes/Data/Users/dev/project/.planning/slugaudit/project.db"
    ));
    assert!(!is_mount_table_network_filesystem(
        MACOS_MOUNT_TABLE,
        "/private/var/folders/n6/T/project.db"
    ));
}

#[test]
fn the_macos_mount_table_recognizes_smbfs_and_the_most_specific_nfs_mount() {
    assert!(is_mount_table_network_filesystem(
        MACOS_MOUNT_TABLE,
        "/Volumes/share/project.db"
    ));
    assert!(is_mount_table_network_filesystem(
        MACOS_MOUNT_TABLE,
        "/Volumes/share/nfs/project.db"
    ));
}

#[test]
fn the_macos_mount_table_respects_path_boundaries_and_skips_malformed_lines() {
    let table = format!("{MACOS_MOUNT_TABLE}garbage line\non without parens\n");
    assert!(!is_mount_table_network_filesystem(
        &table,
        "/Volumes/share2/project.db"
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn a_non_utf8_database_path_fails_the_network_filesystem_check_closed() {
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff/project.db"));

    let error = is_on_network_filesystem(path).expect_err("must fail closed");

    assert!(matches!(error, StoreError::NetworkFilesystemCheck(_)));
}

#[cfg(target_os = "linux")]
#[test]
fn mountinfo_parser_recognizes_remote_fuse_and_cluster_filesystems_but_not_fusectl() {
    for fs_type in ["fuse.sshfs", "ceph", "glusterfs", "lustre", "afs"] {
        let mountinfo = format!("1 0 0:1 / /mnt/remote rw - {fs_type} host:/x rw");
        assert!(
            is_mountpoint_network_filesystem(&mountinfo, "/mnt/remote/file"),
            "{fs_type} should be detected as a network filesystem"
        );
    }
    let fusectl = "1 0 0:1 / /sys/fs/fuse/connections rw - fusectl none rw";
    assert!(!is_mountpoint_network_filesystem(
        fusectl,
        "/sys/fs/fuse/connections/x"
    ));
}
