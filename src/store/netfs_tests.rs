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

#[test]
fn every_known_network_magic_is_classified_as_network() {
    for magic in [
        0x6969_u32,
        0x517B,
        0xFF53_4D42,
        0xFE53_4D42,
        0x564C,
        0x7375_7245,
        0x5346_414F,
        0x6B41_4653,
        0x00C3_6400,
        0x0BD0_0BD0,
        0x0102_1997,
    ] {
        assert!(
            is_network_filesystem_magic(magic),
            "{magic:#x} must be network"
        );
    }
}

#[test]
fn local_and_fuse_magics_are_not_classified_as_network() {
    // ext4, tmpfs, btrfs, xfs, overlayfs, generic FUSE, fusectl, procfs.
    for magic in [
        0xEF53_u32,
        0x0102_1994,
        0x9123_683E,
        0x5846_5342,
        0x794C_7630,
        0x6573_5546,
        0x6573_5543,
        0x9FA0,
    ] {
        assert!(
            !is_network_filesystem_magic(magic),
            "{magic:#x} must be local"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn f_type_is_reduced_to_its_low_32_bits_whatever_its_sign() {
    assert_eq!(
        magic_from_f_type(0xFF53_4D42_i64).expect("positive"),
        0xFF53_4D42
    );
    assert_eq!(
        magic_from_f_type(0xFF53_4D42_u32 as i32).expect("negative"),
        0xFF53_4D42
    );
    assert_eq!(
        magic_from_f_type((-1_i64 << 32) | 0x6969).expect("sign extended"),
        0x6969
    );
}

#[test]
fn macos_type_names_classify_network_and_local_filesystems() {
    for name in ["nfs", "smbfs", "afpfs", "webdav", "cifs", "ftp"] {
        assert!(is_network_filesystem_type_name(name), "{name}");
    }
    for name in ["apfs", "hfs", "msdos", "exfat", "tmpfs", ""] {
        assert!(!is_network_filesystem_type_name(name), "{name}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn a_local_tempdir_is_detected_as_local_by_fstatfs() {
    let directory = tempfile::tempdir().expect("temp dir");
    let verdict = is_on_network_filesystem(&directory.path().join("project.db"));
    assert!(matches!(verdict, Ok(false)), "got: {verdict:?}");
}

/// The database file and even its directory may not exist yet; the nearest
/// existing ancestor decides, and detection never reads a mount table.
#[cfg(target_os = "linux")]
#[test]
fn a_not_yet_created_directory_is_judged_by_its_nearest_existing_ancestor() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("a").join("b").join("project.db");
    let verdict = is_on_network_filesystem(&path);
    assert!(matches!(verdict, Ok(false)), "got: {verdict:?}");
}

#[cfg(target_os = "linux")]
#[test]
fn a_non_utf8_database_path_is_inspected_rather_than_refused() {
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff/project.db"));
    let verdict = is_on_network_filesystem(path);
    assert!(matches!(verdict, Ok(false)), "got: {verdict:?}");
}

#[cfg(target_os = "linux")]
#[test]
fn an_fstatfs_failure_fails_closed_with_the_source_error_and_a_log() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project.db");
    let (result, logs) =
        capture_warns(|| is_network_with(&path, |_| Err(std::io::Error::from_raw_os_error(5))));
    match result {
        Err(StoreError::NetworkFilesystemCheck(source)) => {
            assert_eq!(source.raw_os_error(), Some(5));
        }
        other => panic!("expected NetworkFilesystemCheck, got: {other:?}"),
    }
    assert!(
        logs.contains("could not determine the filesystem type"),
        "the failure must be logged once, captured logs: {logs}"
    );
}
