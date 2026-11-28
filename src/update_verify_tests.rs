//! Unit tests for `update_verify`: SHA-256 manifest parsing, bounded
//! `sha256sum` invocation, and digest comparison.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn require_sha256sum() -> TestResult {
    which::which("sha256sum")
        .check("sha256sum (coreutils) is required to run the checksum tests")?;
    Ok(())
}

fn with_replaced_path<T>(bin_dir: &std::path::Path, f: impl FnOnce() -> T) -> TestResult<T> {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    Ok(temp_env::with_var("PATH", Some(bin_dir.as_os_str()), f))
}

#[test]
fn verify_checksum_accepts_a_matching_file() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    require_sha256sum()?;
    let dir = tempfile::tempdir().check("temp dir")?;
    let binary = dir.path().join("slugaudit-x86_64-unknown-linux-gnu");
    let contents = b"#!/bin/sh\necho fake binary\n";
    std::fs::write(&binary, contents).check("write fake binary")?;

    let expected = {
        let output = std::process::Command::new("sha256sum")
            .current_dir(dir.path())
            .arg("slugaudit-x86_64-unknown-linux-gnu")
            .output()
            .check("run sha256sum")?;
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .check("sha256sum stdout format has digest as first token")?
            .to_string()
    };
    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(
        &sums_file,
        format!("{expected}  slugaudit-x86_64-unknown-linux-gnu\n"),
    )
    .check("write sha256sums")?;

    verify_checksum(&binary, &sums_file).check("matching checksum must verify")?;
    Ok(())
}

#[test]
fn verify_checksum_accepts_a_unique_temporary_file_path() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    require_sha256sum()?;
    let dir = tempfile::tempdir().check("temp dir")?;
    let binary = dir
        .path()
        .join(".slugaudit-update-12345-slugaudit-x86_64-unknown-linux-gnu");
    std::fs::write(&binary, b"#!/bin/sh\necho fake binary\n").check("write fake binary")?;

    let expected = {
        let output = std::process::Command::new("sha256sum")
            .arg(&binary)
            .output()
            .check("run sha256sum")?;
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .check("sha256sum stdout format has digest as first token")?
            .to_string()
    };
    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(
        &sums_file,
        format!("{expected}  slugaudit-x86_64-unknown-linux-gnu\n"),
    )
    .check("write sha256sums")?;

    verify_checksum(&binary, &sums_file).check("unique temp name must verify")?;
    Ok(())
}

#[test]
fn verify_checksum_rejects_a_mismatching_file() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    require_sha256sum()?;
    let dir = tempfile::tempdir().check("temp dir")?;
    let binary = dir.path().join("slugaudit-x86_64-unknown-linux-gnu");
    std::fs::write(&binary, b"different bytes\n").check("write fake binary")?;

    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(
        &sums_file,
        format!("{}  slugaudit-x86_64-unknown-linux-gnu\n", "0".repeat(64)),
    )
    .check("write sha256sums")?;

    let err = verify_checksum(&binary, &sums_file).check_err("a mismatch must be rejected")?;
    match err {
        UpdateError::Checksum(msg) => {
            ensure(
                msg.contains("checksum mismatch"),
                format!("unexpected error message: {msg}"),
            )?;
        }
        other => {
            return Err(format!("expected Checksum error, got: {other:?}"))
                .check("unexpected outcome");
        }
    }
    Ok(())
}

#[test]
fn verify_checksum_fails_on_missing_sums_file() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let binary = dir.path().join("fake_bin");
    std::fs::write(&binary, b"content").check("write binary")?;
    let missing_sums = dir.path().join("nonexistent_SHA256SUMS");
    let err = verify_checksum(&binary, &missing_sums).check_err("the call must fail")?;
    ensure(
        err.to_string().contains("reading checksums"),
        "reports failure reading checksums file",
    )?;
    Ok(())
}

#[test]
fn verify_checksum_fails_when_asset_entry_missing() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let binary = dir.path().join("fake_bin");
    std::fs::write(&binary, b"content").check("write binary")?;
    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(&sums_file, "1234567890abcdef  other-asset.tar.gz\n").check("write sums")?;
    let err = verify_checksum(&binary, &sums_file).check_err("the call must fail")?;
    ensure(
        err.to_string().contains("has no entry for"),
        "reports missing asset entry in SHA256SUMS",
    )?;
    Ok(())
}

#[test]
fn verify_checksum_fails_when_binary_file_missing() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    require_sha256sum()?;
    let dir = tempfile::tempdir().check("temp dir")?;
    let missing_binary = dir.path().join("missing_bin");
    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(&sums_file, format!("{}  {ASSET}\n", "0".repeat(64))).check("write sums")?;
    let err = verify_checksum(&missing_binary, &sums_file).check_err("the call must fail")?;
    ensure(
        err.to_string().contains("checksum verification failed"),
        "reports checksum verification failure when binary is missing",
    )?;
    Ok(())
}

#[test]
fn verify_checksum_rejects_oversized_sums_file() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let binary = dir.path().join("fake_bin");
    std::fs::write(&binary, b"content").check("write binary")?;
    let sums_file = dir.path().join("SHA256SUMS");
    let oversized_len = usize::try_from(CHECKSUMS_MAX_BYTES)
        .check("cap")?
        .checked_add(1)
        .check("oversized len")?;
    let oversized = "a".repeat(oversized_len);
    std::fs::write(&sums_file, oversized).check("write oversized sums")?;

    let err = verify_checksum(&binary, &sums_file).check_err("oversized sums must fail")?;
    ensure(
        err.to_string().contains("reading checksums"),
        format!("unexpected error: {err}"),
    )?;
    Ok(())
}

#[test]
fn an_empty_sha256sum_output_is_a_named_parse_failure() -> TestResult {
    let error = parse_digest("  \n").check_err("no token is no digest")?;

    ensure(
        error.to_string().contains("printed no digest"),
        format!("{error}"),
    )?;
    Ok(())
}

#[test]
fn the_first_sha256sum_token_is_the_lowercase_digest() -> TestResult {
    ensure_eq(
        &parse_digest("ABCDEF  file\n").check("digest")?,
        &"abcdef",
        "lowercases first hex token from sha256sum output",
    )?;
    Ok(())
}

#[test]
fn verify_checksum_reports_a_missing_sha256sum_as_a_tool_spawn_failure() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let binary = directory.path().join("fake_bin");
    std::fs::write(&binary, b"content").check("write binary")?;
    let sums = directory.path().join("SHA256SUMS");
    std::fs::write(&sums, format!("{}  {ASSET}\n", "0".repeat(64))).check("write sums")?;
    let empty = tempfile::tempdir().check("empty bin dir")?;

    let result = with_replaced_path(empty.path(), || verify_checksum(&binary, &sums))?;

    let Err(UpdateError::ToolSpawn { tool, source }) = result else {
        return Err(format!("expected ToolSpawn, got {result:?}")).check("unexpected outcome");
    };
    ensure_eq(&tool, &"sha256sum", "ToolSpawn names sha256sum")?;
    ensure_eq(
        &source.kind(),
        &std::io::ErrorKind::NotFound,
        "preserves NotFound kind",
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn installed_version_returns_none_when_distinct_target_fails_or_prints_unexpected_output()
-> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let temp = tempfile::tempdir().check("temp dir")?;
    let running = temp.path().join("running-slugaudit");
    std::fs::write(&running, "#!/bin/sh\necho 'slugaudit 9.9.9'\n").check("write running")?;

    let valid = temp.path().join("valid-target");
    std::fs::write(&valid, "#!/bin/sh\necho 'slugaudit 1.0.4'\n").check("write valid target")?;
    std::fs::set_permissions(&valid, std::fs::Permissions::from_mode(0o755))
        .check("chmod valid")?;
    ensure_eq(
        &installed_version(&running, &valid),
        &Some("1.0.4".to_owned()),
        "valid distinct target returns Some(version)",
    )?;
    ensure_eq(
        &installed_version(&running, &running),
        &Some(env!("CARGO_PKG_VERSION").to_owned()),
        "same binary path returns Some(CARGO_PKG_VERSION)",
    )?;

    let failing = temp.path().join("failing-target");
    std::fs::write(&failing, "#!/bin/sh\nexit 1\n").check("write failing target")?;
    std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755))
        .check("chmod failing")?;
    ensure_eq(
        &installed_version(&running, &failing),
        &None,
        "non-zero exit from target returns None instead of running version",
    )?;

    let garbled = temp.path().join("garbled-target");
    std::fs::write(&garbled, "#!/bin/sh\necho 'unexpected output'\n").check("write garbled")?;
    std::fs::set_permissions(&garbled, std::fs::Permissions::from_mode(0o755))
        .check("chmod garbled")?;
    ensure_eq(
        &installed_version(&running, &garbled),
        &None,
        "unexpected --version output from target returns None",
    )?;

    let missing = temp.path().join("missing-target");
    ensure_eq(
        &installed_version(&running, &missing),
        &None,
        "unreadable/missing target binary returns None",
    )?;
    Ok(())
}

#[test]
fn update_decision_for_treats_missing_installed_version_as_incomparable() -> TestResult {
    ensure_eq(
        &update_decision_for(None, "1.0.10").check("None resolves to Incomparable")?,
        &UpdateDecision::Incomparable,
        "missing installed version is Incomparable rather than UpToDate",
    )?;
    ensure_eq(
        &update_decision_for(Some("1.0.10"), "1.0.10").check("equal versions")?,
        &UpdateDecision::UpToDate,
        "matching installed version is UpToDate",
    )?;
    let err = update_decision_for(None, "1.0.0-rc1")
        .check_err("non-numeric latest must still be rejected when current is None")?;
    ensure(
        matches!(err, UpdateError::InvalidReleaseVersion(_)),
        format!("expected InvalidReleaseVersion, got {err:?}"),
    )?;
    Ok(())
}
