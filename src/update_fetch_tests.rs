//! Tests for the network/version helpers in `update_fetch.rs` — pure URL and
//! version tests, plus hermetic fake-`curl` tests that never touch the network.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult};

/// Runs `f` with `PATH` pointing at exactly `bin_dir`: the inherited entries
/// are dropped, so `curl` resolves only to the fake planted there and no real
/// network request can run.
fn with_replaced_path<T>(bin_dir: &std::path::Path, f: impl FnOnce() -> T) -> TestResult<T> {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    Ok(temp_env::with_var("PATH", Some(bin_dir.as_os_str()), f))
}

fn make_executable(path: &std::path::Path) -> TestResult {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)
            .check("stat fake CLI")?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).check("chmod fake CLI")?;
    }
    Ok(())
}

/// Plants a fake `curl` that appends its argv to `log`, prints `stdout` and
/// `stderr`, then exits with `code`.
fn plant_fake_curl(
    bin_dir: &std::path::Path,
    log: &std::path::Path,
    stdout: &str,
    stderr: &str,
    code: i32,
) -> TestResult {
    let script = bin_dir.join("curl");
    let body = format!(
        "#!/bin/sh\n\
         printf '%s\\n' \"$@\" > '{log}'\n\
         printf '%s' '{stdout}'\n\
         printf '%s' '{stderr}' >&2\n\
         exit {code}\n",
        log = log.display(),
    );
    std::fs::write(&script, body).check("write fake curl")?;
    make_executable(&script)?;
    Ok(())
}

#[test]
fn from_tag_builds_the_asset_and_checksum_urls() {
    let release = LatestRelease::from_tag("v1.0.3");
    assert_eq!(release.version, "1.0.3");
    assert!(release.asset_url.ends_with(
        "/SlugThugLabs/slugaudit/releases/download/v1.0.3/slugaudit-x86_64-unknown-linux-gnu"
    ));
    assert!(
        release
            .checksums_url
            .ends_with("/SlugThugLabs/slugaudit/releases/download/v1.0.3/SHA256SUMS")
    );
}

#[test]
fn from_tag_trims_a_leading_v_from_the_version() {
    assert_eq!(LatestRelease::from_tag("1.0.3").version, "1.0.3");
    assert_eq!(LatestRelease::from_tag("v1.0.3").version, "1.0.3");
}

#[test]
fn compare_versions_orders_numeric_releases() {
    use std::cmp::Ordering::*;
    assert_eq!(compare_versions("1.0.2", "1.0.3"), Ok(Less));
    assert_eq!(compare_versions("1.0.3", "1.0.3"), Ok(Equal));
    assert_eq!(compare_versions("1.1.0", "1.0.99"), Ok(Greater));
    assert_eq!(compare_versions("v1.0.3", "1.0.3"), Ok(Equal));
    assert_eq!(
        compare_versions("2.0.0", "1.0.3"),
        Ok(Greater),
        "major bump outranks any patch"
    );
}

#[test]
fn compare_versions_rejects_non_numeric_local_versions() -> TestResult {
    assert!(compare_versions("dev", "1.0.3").is_err());
    assert!(compare_versions("1.0.3", "abc").is_err());
    assert_eq!(
        compare_versions("1.0.3", "1.0.3").check("comparable")?,
        std::cmp::Ordering::Equal
    );
    Ok(())
}

#[test]
fn compare_versions_treats_missing_components_as_zero() -> TestResult {
    use std::cmp::Ordering::{Equal, Greater, Less};
    assert_eq!(compare_versions("1.0", "1.0.0").check("comparable")?, Equal);
    assert_eq!(compare_versions("1", "1.0.0").check("comparable")?, Equal);
    assert_eq!(compare_versions("1.0.0", "1.0").check("comparable")?, Equal);
    assert_eq!(compare_versions("1.0", "1.0.1").check("comparable")?, Less);
    assert_eq!(
        compare_versions("1.1", "1.0.9").check("comparable")?,
        Greater
    );
    Ok(())
}

#[test]
fn compare_versions_error_names_the_unparseable_version_and_keeps_the_cause() -> TestResult {
    let error = compare_versions("1.0.0-rc1", "1.0.0").check_err("pre-release is not numeric")?;

    assert!(error.to_string().contains("1.0.0-rc1"), "{error}");
    assert!(std::error::Error::source(&error).is_some());
    Ok(())
}

#[test]
fn from_json_prefers_actual_asset_urls_when_present() -> TestResult {
    let json = serde_json::json!({
        "tag_name": "v1.0.10",
        "assets": [
            {
                "name": "slugaudit-x86_64-unknown-linux-gnu",
                "browser_download_url": "https://custom.download/slugaudit-bin"
            },
            {
                "name": "SHA256SUMS",
                "browser_download_url": "https://custom.download/SHA256SUMS"
            }
        ]
    });
    let release = LatestRelease::from_json(&json).check("parses")?;
    assert_eq!(release.version, "1.0.10");
    assert_eq!(release.asset_url, "https://custom.download/slugaudit-bin");
    assert_eq!(release.checksums_url, "https://custom.download/SHA256SUMS");
    Ok(())
}

#[test]
fn from_json_without_a_tag_name_reports_no_tag() -> TestResult {
    let json = serde_json::json!({ "assets": [] });

    let Err(error) = LatestRelease::from_json(&json) else {
        return Err("no tag name must not parse as a release".to_owned())
            .check("unexpected outcome");
    };

    assert!(matches!(error, UpdateError::NoTag), "{error:?}");
    Ok(())
}

#[test]
fn fetch_latest_release_reports_undecodable_json() -> TestResult {
    let bin = tempfile::tempdir().check("fake bin dir")?;
    let log = bin.path().join("curl-argv.log");
    plant_fake_curl(bin.path(), &log, "not json at all", "", 0)?;

    let Err(error) = with_replaced_path(bin.path(), fetch_latest_release)? else {
        return Err("invalid JSON must not parse as a release".to_owned())
            .check("unexpected outcome");
    };

    let UpdateError::Json(message) = error else {
        return Err(format!("expected Json, got {error:?}")).check("unexpected outcome");
    };
    assert!(
        !message.is_empty(),
        "the decode failure must keep its reason"
    );
    let argv = std::fs::read_to_string(&log).check("the fake curl ran")?;
    assert!(
        argv.contains("https://api.github.com/repos/SlugThugLabs/slugaudit/releases/latest"),
        "the release metadata came from the fake, not the network: {argv}"
    );
    Ok(())
}

#[test]
fn fetch_latest_release_reports_a_missing_curl_as_a_tool_spawn_failure() -> TestResult {
    let empty = tempfile::tempdir().check("empty bin dir")?;

    let Err(error) = with_replaced_path(empty.path(), fetch_latest_release)? else {
        return Err("a PATH with no curl cannot fetch a release".to_owned())
            .check("unexpected outcome");
    };

    let UpdateError::ToolSpawn { tool, source } = error else {
        return Err(format!("expected ToolSpawn, got {error:?}")).check("unexpected outcome");
    };
    assert_eq!(tool, "curl");
    assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
    Ok(())
}

#[test]
fn fetch_latest_release_reports_a_failing_curl_as_network() -> TestResult {
    let bin = tempfile::tempdir().check("fake bin dir")?;
    let log = bin.path().join("curl-argv.log");
    plant_fake_curl(bin.path(), &log, "", "could not resolve host", 1)?;

    let Err(error) = with_replaced_path(bin.path(), fetch_latest_release)? else {
        return Err("a failing curl must not yield a release".to_owned())
            .check("unexpected outcome");
    };

    let UpdateError::Network(message) = error else {
        return Err(format!("expected Network, got {error:?}")).check("unexpected outcome");
    };
    assert_eq!(message, "could not resolve host");
    let argv = std::fs::read_to_string(&log).check("the fake curl ran")?;
    assert!(
        argv.contains("https://api.github.com/repos/SlugThugLabs/slugaudit/releases/latest"),
        "the failing request went to the fake, not the network: {argv}"
    );
    Ok(())
}

#[test]
fn download_reports_a_missing_curl_as_a_tool_spawn_failure() -> TestResult {
    let empty = tempfile::tempdir().check("empty bin dir")?;
    let destination = empty.path().join("asset");

    let result = with_replaced_path(empty.path(), || {
        download(
            "https://example.invalid/asset",
            &destination,
            "download binary to",
        )
    })?;

    let Err(UpdateError::Download {
        action,
        path,
        source,
    }) = result
    else {
        return Err(format!("expected Download, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(action, "download binary to");
    assert_eq!(path, destination);
    let UpdateError::ToolSpawn { tool, source } = *source else {
        return Err(format!(
            "expected the download cause to be ToolSpawn, got {source:?}"
        ))
        .check("unexpected outcome");
    };
    assert_eq!(tool, "curl");
    assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
    Ok(())
}

#[test]
fn download_reports_a_failing_curl_as_network() -> TestResult {
    let bin = tempfile::tempdir().check("fake bin dir")?;
    let log = bin.path().join("curl-argv.log");
    plant_fake_curl(bin.path(), &log, "", "404: Not Found", 1)?;
    let destination = bin.path().join("asset");

    let result = with_replaced_path(bin.path(), || {
        download(
            "https://example.invalid/asset",
            &destination,
            "download checksums to",
        )
    })?;

    let Err(UpdateError::Download {
        action,
        path,
        source,
    }) = result
    else {
        return Err(format!("expected Download, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(action, "download checksums to");
    assert_eq!(path, destination);
    let UpdateError::Network(message) = *source else {
        return Err(format!(
            "expected the download cause to be Network, got {source:?}"
        ))
        .check("unexpected outcome");
    };
    assert_eq!(message, "404: Not Found");
    assert!(
        !destination.exists(),
        "a failed download must not leave a destination file"
    );
    let argv = std::fs::read_to_string(&log).check("the fake curl ran")?;
    assert!(
        argv.contains("https://example.invalid/asset") && argv.contains("-o"),
        "the failing download went to the fake, not the network: {argv}"
    );
    Ok(())
}

#[test]
fn update_fetch_malformed_asset_entry_names_its_index() -> TestResult {
    let json = serde_json::json!({
        "tag_name": "v1.0.3",
        "assets": [
            {"name": "SHA256SUMS", "browser_download_url": "https://x/SHA256SUMS"},
            {"name": "slugaudit-x86_64-unknown-linux-gnu"}
        ]
    });

    let Err(UpdateError::Json(message)) = LatestRelease::from_json(&json) else {
        return Err("expected a Json error for the malformed entry".to_owned())
            .check("unexpected outcome");
    };

    assert!(message.contains("asset entry 1"), "{message}");
    Ok(())
}

#[test]
fn update_fetch_failed_download_removes_the_partial_file() -> TestResult {
    let bin = tempfile::tempdir().check("temp dir")?;
    let script = bin.path().join("curl");
    std::fs::write(
        &script,
        "#!/bin/sh\nfor a; do last=\"$a\"; done\nprintf partial > \"$last\"\nexit 22\n",
    )
    .check("write fake curl")?;
    make_executable(&script)?;
    let destination = bin.path().join("asset");

    let result = with_replaced_path(bin.path(), || {
        download(
            "https://example.invalid/asset",
            &destination,
            "download binary to",
        )
    })?;

    assert!(
        matches!(result, Err(UpdateError::Download { .. })),
        "{result:?}"
    );
    assert!(!destination.exists(), "the partial file must be removed");
    Ok(())
}

#[test]
fn both_curl_calls_share_one_bounded_https_only_argument_list() {
    let command = curl_command("https://example.test/x", 30, 10_000_000);

    let args: Vec<_> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();

    assert_eq!(
        args,
        [
            "-fsSL",
            "--max-time",
            "30",
            "--max-filesize",
            "10000000",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "https://example.test/x",
        ]
    );
}
