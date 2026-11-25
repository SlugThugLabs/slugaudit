use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn update_refuses_non_numeric_remote_release_versions() -> TestResult {
    for (current, latest) in [
        ("1.1.0", "1.0.0-rc1"),
        ("1.1.0", "nightly"),
        ("1.1.0-dev", "1.0.0-rc1"),
    ] {
        let label = format!("non-numeric latest {latest} must be refused");
        let err = update_decision(current, latest).check_err(&label)?;
        ensure(
            matches!(err, UpdateError::InvalidReleaseVersion(_)),
            format!("expected InvalidReleaseVersion for {current} vs {latest}, got {err:?}"),
        )?;
    }
    Ok(())
}

/// On a platform with no published asset `run_update` must refuse before any
/// network call. Compiled only where the refusal is reachable; Linux/x86_64
/// (the platform this repository builds and tests) never takes it.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[test]
fn run_update_refuses_a_platform_with_no_published_asset() -> TestResult {
    let result = run_update();

    let Err(UpdateError::UnsupportedPlatform { os, arch }) = result else {
        return Err(format!("expected UnsupportedPlatform, got {result:?}"))
            .check("unexpected outcome");
    };
    ensure_eq(&os, &std::env::consts::OS, "os == std::env::consts::OS")?;
    ensure_eq(
        &arch,
        &std::env::consts::ARCH,
        "arch == std::env::consts::ARCH",
    )?;
    Ok(())
}

#[test]
fn parse_binary_version_extracts_slugaudit_version_token() -> TestResult {
    ensure_eq(
        &parse_binary_version("slugaudit 1.0.9\n"),
        &Some("1.0.9".to_owned()),
        "standard --version output",
    )?;
    ensure_eq(
        &parse_binary_version("other-tool 1.0.9\n"),
        &None,
        "wrong binary name rejected",
    )?;
    ensure_eq(
        &parse_binary_version("slugaudit 1.0.9 extra\n"),
        &None,
        "extra tokens rejected",
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn installed_version_queries_target_binary_when_different_from_running() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let temp = tempfile::tempdir().check("temp dir")?;
    let running = temp.path().join("running-slugaudit");
    let target = temp.path().join("installed-slugaudit");
    std::fs::write(&running, "#!/bin/sh\necho 'slugaudit 9.9.9'\n").check("write running")?;
    std::fs::write(&target, "#!/bin/sh\necho 'slugaudit 1.0.4'\n").check("write target")?;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).check("chmod")?;

    ensure_eq(
        &installed_version(&running, &target),
        &"1.0.4".to_owned(),
        "must query target --version instead of CARGO_PKG_VERSION",
    )?;
    ensure_eq(
        &installed_version(&running, &running),
        &env!("CARGO_PKG_VERSION").to_owned(),
        "same path uses compiled-in version",
    )?;
    Ok(())
}

#[test]
fn parse_expected_digest_accepts_binary_star_and_dot_slash_prefixes() -> TestResult {
    let digest = "0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef";
    let expected = digest.to_ascii_lowercase();
    for line in [
        format!("{digest}  {ASSET}\n"),
        format!("{digest} *{ASSET}\n"),
        format!("{digest}  ./{ASSET}\n"),
        format!("{digest} *./{ASSET}\n"),
    ] {
        let actual = parse_expected_digest(&line).check(&line)?;
        ensure_eq(&actual, &expected, format!("line: {line}"))?;
    }
    for bad in [
        format!("abc  {ASSET}\n"),
        format!("{}  {ASSET}\n", "g".repeat(64)),
        format!("{digest}  {ASSET}  extra\n"),
    ] {
        parse_expected_digest(&bad).check_err(&format!("invalid line must fail: {bad}"))?;
    }
    Ok(())
}
