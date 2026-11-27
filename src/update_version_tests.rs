use super::update_verify::{compare_versions, parse_binary_version, parse_expected_digest};
use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn compare_versions_orders_numeric_releases() -> TestResult {
    use std::cmp::Ordering::{Equal, Greater, Less};
    ensure_eq(
        &compare_versions("1.0.2", "1.0.3"),
        &Ok(Less),
        "older patch compares Less",
    )?;
    ensure_eq(
        &compare_versions("1.0.3", "1.0.3"),
        &Ok(Equal),
        "identical version compares Equal",
    )?;
    ensure_eq(
        &compare_versions("1.1.0", "1.0.99"),
        &Ok(Greater),
        "minor bump outranks patch",
    )?;
    ensure_eq(
        &compare_versions("v1.0.3", "1.0.3"),
        &Ok(Equal),
        "leading v compares Equal to bare version",
    )?;
    ensure_eq(
        &compare_versions("2.0.0", "1.0.3"),
        &Ok(Greater),
        "major bump outranks any patch",
    )?;
    Ok(())
}

#[test]
fn compare_versions_rejects_non_numeric_local_versions() -> TestResult {
    ensure(
        compare_versions("dev", "1.0.3").is_err(),
        "rejects non-numeric local version",
    )?;
    ensure(
        compare_versions("1.0.3", "abc").is_err(),
        "rejects non-numeric remote version",
    )?;
    ensure_eq(
        &compare_versions("1.0.3", "1.0.3").check("comparable")?,
        &std::cmp::Ordering::Equal,
        "valid dotted versions compare Equal",
    )?;
    Ok(())
}

#[test]
fn compare_versions_treats_missing_components_as_zero() -> TestResult {
    use std::cmp::Ordering::{Equal, Greater, Less};
    ensure_eq(
        &compare_versions("1.0", "1.0.0").check("comparable")?,
        &Equal,
        "1.0 equals 1.0.0",
    )?;
    ensure_eq(
        &compare_versions("1", "1.0.0").check("comparable")?,
        &Equal,
        "1 equals 1.0.0",
    )?;
    ensure_eq(
        &compare_versions("1.0.0", "1.0").check("comparable")?,
        &Equal,
        "1.0.0 equals 1.0",
    )?;
    ensure_eq(
        &compare_versions("1.0", "1.0.1").check("comparable")?,
        &Less,
        "1.0 is less than 1.0.1",
    )?;
    ensure_eq(
        &compare_versions("1.1", "1.0.9").check("comparable")?,
        &Greater,
        "1.1 is greater than 1.0.9",
    )?;
    Ok(())
}

#[test]
fn compare_versions_error_names_the_unparseable_version_and_keeps_the_cause() -> TestResult {
    let error = compare_versions("1.0.0-rc1", "1.0.0").check_err("pre-release is not numeric")?;

    ensure(error.to_string().contains("1.0.0-rc1"), format!("{error}"))?;
    std::error::Error::source(&error).check("the error carries a source")?;
    Ok(())
}

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
