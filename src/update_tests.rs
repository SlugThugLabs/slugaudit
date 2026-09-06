//! Tests for the self-update helpers that don't need the network:
//! release URL building, version comparison, and checksum verification
//! against a real `sha256sum` invocation.

use super::*;

/// Verifies a real (mock) binary against a SHA256SUMS file by invoking
/// `sha256sum`. Skipped when `sha256sum` isn't on PATH.
#[test]
fn verify_checksum_accepts_a_matching_file() {
    if which::which("sha256sum").is_err() {
        eprintln!("skipping: sha256sum not available");
        return;
    }
    let dir = tempfile::tempdir().expect("temp dir");
    let binary = dir.path().join("slugaudit-x86_64-unknown-linux-gnu");
    let contents = b"#!/bin/sh\necho fake binary\n";
    std::fs::write(&binary, contents).expect("write fake binary");

    // Compute the expected hash and write a SHA256SUMS line for it.
    let expected = {
        let output = std::process::Command::new("sha256sum")
            .current_dir(dir.path())
            .arg("slugaudit-x86_64-unknown-linux-gnu")
            .output()
            .expect("run sha256sum");
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .expect("sha256sum stdout format has digest as first token")
            .to_string()
    };
    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(
        &sums_file,
        format!("{expected}  slugaudit-x86_64-unknown-linux-gnu\n"),
    )
    .expect("write sha256sums");

    verify_checksum(&binary, &sums_file).expect("matching checksum must verify");
}

/// The temp binary no longer needs to be named exactly the asset filename:
/// verification compares digests, not filenames, so a unique temp name (as
/// `apply_update` now uses) must verify fine. This is the regression test
/// for Task 17.5.
#[test]
fn verify_checksum_accepts_a_unique_temporary_file_path() {
    if which::which("sha256sum").is_err() {
        eprintln!("skipping: sha256sum not available");
        return;
    }
    let dir = tempfile::tempdir().expect("temp dir");
    // Deliberately NOT named the asset — a unique `.update-<pid>` name.
    let binary = dir
        .path()
        .join(".slugaudit-update-12345-slugaudit-x86_64-unknown-linux-gnu");
    std::fs::write(&binary, b"#!/bin/sh\necho fake binary\n").expect("write fake binary");

    let expected = {
        let output = std::process::Command::new("sha256sum")
            .arg(&binary)
            .output()
            .expect("run sha256sum");
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .expect("sha256sum stdout format has digest as first token")
            .to_string()
    };
    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(
        &sums_file,
        format!("{expected}  slugaudit-x86_64-unknown-linux-gnu\n"),
    )
    .expect("write sha256sums");

    verify_checksum(&binary, &sums_file).expect("unique temp name must verify");
}

/// A SHA256SUMS entry that doesn't match must be rejected so the old binary
/// is never replaced with a corrupt download.
#[test]
fn verify_checksum_rejects_a_mismatching_file() {
    if which::which("sha256sum").is_err() {
        eprintln!("skipping: sha256sum not available");
        return;
    }
    let dir = tempfile::tempdir().expect("temp dir");
    let binary = dir.path().join("slugaudit-x86_64-unknown-linux-gnu");
    std::fs::write(&binary, b"different bytes\n").expect("write fake binary");

    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(
        &sums_file,
        format!("{}  slugaudit-x86_64-unknown-linux-gnu\n", "0".repeat(64)),
    )
    .expect("write sha256sums");

    assert!(
        verify_checksum(&binary, &sums_file).is_err(),
        "a mismatch must be rejected"
    );
}

#[test]
fn target_binary_prefers_the_installed_stable_path() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    let slugthug = temp.path().join("slugaudit");
    std::fs::create_dir_all(&slugthug).expect("create dir");
    let stable = slugthug.join("slugaudit");
    std::fs::write(&stable, b"fake installed binary").expect("write fake install");

    temp_env::with_var("SLUGTHUG_HOME", Some(temp.path().as_os_str()), || {
        let target = target_binary().expect("target binary resolves");
        assert_eq!(target, stable);
    });
}

#[test]
fn verify_checksum_fails_on_missing_sums_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let binary = dir.path().join("fake_bin");
    std::fs::write(&binary, b"content").expect("write binary");
    let missing_sums = dir.path().join("nonexistent_SHA256SUMS");
    let err = verify_checksum(&binary, &missing_sums).unwrap_err();
    assert!(err.to_string().contains("reading checksums"));
}

#[test]
fn verify_checksum_fails_when_asset_entry_missing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let binary = dir.path().join("fake_bin");
    std::fs::write(&binary, b"content").expect("write binary");
    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(&sums_file, "1234567890abcdef  other-asset.tar.gz\n").expect("write sums");
    let err = verify_checksum(&binary, &sums_file).unwrap_err();
    assert!(err.to_string().contains("has no entry for"));
}

#[test]
fn verify_checksum_fails_when_binary_file_missing() {
    if which::which("sha256sum").is_err() {
        return;
    }
    let dir = tempfile::tempdir().expect("temp dir");
    let missing_binary = dir.path().join("missing_bin");
    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(&sums_file, format!("abc  {ASSET}\n")).expect("write sums");
    let err = verify_checksum(&missing_binary, &sums_file).unwrap_err();
    assert!(err.to_string().contains("checksum verification failed"));
}

#[test]
fn update_error_display_covers_all_variants() {
    assert!(
        UpdateError::CurlMissing
            .to_string()
            .contains("curl is required")
    );
    assert!(
        UpdateError::Target(std::io::Error::other("fail"))
            .to_string()
            .contains("could not determine")
    );
    assert!(
        UpdateError::Network("conn reset".into())
            .to_string()
            .contains("network request failed")
    );
    assert!(
        UpdateError::Json("bad json".into())
            .to_string()
            .contains("JSON decodable")
    );
    assert!(UpdateError::NoTag.to_string().contains("tag name"));
    assert!(
        UpdateError::Checksum("bad hash".into())
            .to_string()
            .contains("checksum verification failed")
    );
    assert!(
        UpdateError::Io(std::io::Error::other("io"))
            .to_string()
            .contains("filesystem")
    );
    assert!(
        UpdateError::Stage {
            action: "create",
            path: PathBuf::from("/test"),
            source: std::io::Error::other("perm"),
        }
        .to_string()
        .contains("could not create /test")
    );
}

#[test]
fn io_from_string_helper_creates_error() {
    let err = io_from_string("network err".into());
    assert_eq!(err.to_string(), "network err");
}
