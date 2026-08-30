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
    let binary = dir.path().join("slugaudit-mcp-x86_64-unknown-linux-gnu");
    let contents = b"#!/bin/sh\necho fake binary\n";
    std::fs::write(&binary, contents).expect("write fake binary");

    // Compute the expected hash and write a SHA256SUMS line for it.
    let expected = {
        let output = std::process::Command::new("sha256sum")
            .current_dir(dir.path())
            .arg("slugaudit-mcp-x86_64-unknown-linux-gnu")
            .output()
            .expect("run sha256sum");
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    };
    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(
        &sums_file,
        format!("{expected}  slugaudit-mcp-x86_64-unknown-linux-gnu\n"),
    )
    .expect("write sha256sums");

    verify_checksum(&binary, &sums_file).expect("matching checksum must verify");
}

/// The temp binary no longer needs to be named exactly the asset filename:
/// verification compares digests, not filenames, so a unique temp name (as
/// `apply_update` now uses) must verify fine. This is the regression test
/// for the clobber-a-same-named-file fix.
#[test]
fn verify_checksum_accepts_a_file_with_a_unique_temp_name() {
    if which::which("sha256sum").is_err() {
        eprintln!("skipping: sha256sum not available");
        return;
    }
    let dir = tempfile::tempdir().expect("temp dir");
    // Deliberately NOT named the asset — a unique `.update-<pid>` name.
    let binary = dir
        .path()
        .join(".slugaudit-mcp-update-12345-slugaudit-mcp-x86_64-unknown-linux-gnu");
    std::fs::write(&binary, b"#!/bin/sh\necho fake binary\n").expect("write fake binary");

    let expected = {
        let output = std::process::Command::new("sha256sum")
            .arg(&binary)
            .output()
            .expect("run sha256sum");
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    };
    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(
        &sums_file,
        format!("{expected}  slugaudit-mcp-x86_64-unknown-linux-gnu\n"),
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
    let binary = dir.path().join("slugaudit-mcp-x86_64-unknown-linux-gnu");
    std::fs::write(&binary, b"different bytes\n").expect("write fake binary");

    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(
        &sums_file,
        format!(
            "{}  slugaudit-mcp-x86_64-unknown-linux-gnu\n",
            "0".repeat(64)
        ),
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
    let slugthug = temp.path().join("bin");
    std::fs::create_dir_all(&slugthug).expect("create dir");
    let stable = slugthug.join("slugaudit-mcp");
    std::fs::write(&stable, b"fake installed binary").expect("write fake install");

    temp_env::with_var("SLUGTHUG_HOME", Some(temp.path().as_os_str()), || {
        let target = target_binary().expect("target binary resolves");
        assert_eq!(target, stable);
    });
}
