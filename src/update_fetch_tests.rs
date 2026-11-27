//! Tests for the network/release helpers in `update_fetch.rs` — release URL
//! and payload parsing, plus hermetic fake-`curl` execution tests.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

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
fn from_tag_builds_the_asset_and_checksum_urls() -> TestResult {
    let release = LatestRelease::from_tag("v1.0.3");
    ensure_eq(&release.version, &"1.0.3", "strips v prefix from tag")?;
    ensure(
        release.asset_url.ends_with(
            "/SlugThugLabs/slugaudit/releases/download/v1.0.3/slugaudit-x86_64-unknown-linux-gnu",
        ),
        "builds tag-pinned Linux x86_64 asset URL",
    )?;
    ensure(
        release
            .checksums_url
            .ends_with("/SlugThugLabs/slugaudit/releases/download/v1.0.3/SHA256SUMS"),
        "builds tag-pinned SHA256SUMS URL",
    )?;
    Ok(())
}

#[test]
fn from_tag_trims_a_leading_v_from_the_version() -> TestResult {
    ensure_eq(
        &LatestRelease::from_tag("1.0.3").version,
        &"1.0.3",
        "preserves bare numeric tag",
    )?;
    ensure_eq(
        &LatestRelease::from_tag("v1.0.3").version,
        &"1.0.3",
        "strips leading v from tag",
    )?;
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
    ensure_eq(&release.version, &"1.0.10", "parses release version")?;
    ensure_eq(
        &release.asset_url,
        &"https://custom.download/slugaudit-bin",
        "prefers explicit binary asset URL",
    )?;
    ensure_eq(
        &release.checksums_url,
        &"https://custom.download/SHA256SUMS",
        "prefers explicit SHA256SUMS URL",
    )?;
    Ok(())
}

#[test]
fn from_json_without_a_tag_name_reports_no_tag() -> TestResult {
    let json = serde_json::json!({ "assets": [] });

    let Err(error) = LatestRelease::from_json(&json) else {
        return Err("no tag name must not parse as a release".to_owned())
            .check("unexpected outcome");
    };

    ensure(matches!(error, UpdateError::NoTag), format!("{error:?}"))?;
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
    ensure(
        !message.is_empty(),
        "the decode failure must keep its reason",
    )?;
    let argv = std::fs::read_to_string(&log).check("the fake curl ran")?;
    ensure(
        argv.contains("https://api.github.com/repos/SlugThugLabs/slugaudit/releases/latest"),
        format!("the release metadata came from the fake, not the network: {argv}"),
    )?;
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
    ensure_eq(&tool, &"curl", "ToolSpawn names curl")?;
    ensure_eq(
        &source.kind(),
        &std::io::ErrorKind::NotFound,
        "preserves NotFound error kind",
    )?;
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
    ensure_eq(
        &message,
        &"could not resolve host",
        "preserves curl stderr message",
    )?;
    let argv = std::fs::read_to_string(&log).check("the fake curl ran")?;
    ensure(
        argv.contains("https://api.github.com/repos/SlugThugLabs/slugaudit/releases/latest"),
        format!("the failing request went to the fake, not the network: {argv}"),
    )?;
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
    ensure_eq(
        &action,
        &"download binary to",
        "Download preserves action description",
    )?;
    ensure_eq(&path, &destination, "Download preserves destination path")?;
    let UpdateError::ToolSpawn { tool, source } = *source else {
        return Err(format!(
            "expected the download cause to be ToolSpawn, got {source:?}"
        ))
        .check("unexpected outcome");
    };
    ensure_eq(&tool, &"curl", "ToolSpawn names curl")?;
    ensure_eq(
        &source.kind(),
        &std::io::ErrorKind::NotFound,
        "preserves NotFound kind",
    )?;
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
    ensure_eq(
        &action,
        &"download checksums to",
        "Download preserves checksum action",
    )?;
    ensure_eq(&path, &destination, "Download preserves destination path")?;
    let UpdateError::Network(message) = *source else {
        return Err(format!(
            "expected the download cause to be Network, got {source:?}"
        ))
        .check("unexpected outcome");
    };
    ensure_eq(&message, &"404: Not Found", "preserves HTTP failure text")?;
    ensure(
        !destination.exists(),
        "a failed download must not leave a destination file",
    )?;
    let argv = std::fs::read_to_string(&log).check("the fake curl ran")?;
    ensure(
        argv.contains("https://example.invalid/asset") && argv.contains("-o"),
        format!("the failing download went to the fake, not the network: {argv}"),
    )?;
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

    ensure(message.contains("asset entry 1"), &message)?;
    Ok(())
}

#[test]
fn update_fetch_failed_download_removes_the_partial_file() -> TestResult {
    let bin = tempfile::tempdir().check("temp dir")?;
    let script = bin.path().join("curl");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         dest=\n\
         while [ $# -gt 0 ]; do\n\
           if [ \"$1\" = \"-o\" ]; then dest=$2; shift 2; continue; fi\n\
           shift\n\
         done\n\
         printf partial > \"$dest\"\n\
         exit 22\n",
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

    ensure(
        matches!(result, Err(UpdateError::Download { .. })),
        format!("{result:?}"),
    )?;
    ensure(!destination.exists(), "the partial file must be removed")?;
    Ok(())
}

#[test]
fn both_curl_calls_share_one_bounded_https_only_argument_list() -> TestResult {
    let command = curl_command("https://example.test/x", 30, 10_000_000, None);

    let args: Vec<_> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();

    ensure_eq(
        &args,
        &[
            "-q",
            "-fsSL",
            "--globoff",
            "--max-time",
            "30",
            "--max-filesize",
            "10000000",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--tlsv1.2",
            "--",
            "https://example.test/x",
        ],
        "metadata curl args start with -q and include --globoff, --tlsv1.2, and -- before url",
    )?;

    let dl_command = curl_command(
        "https://example.test/SHA256SUMS",
        300,
        CHECKSUMS_MAX_BYTES,
        Some(Path::new("/tmp/sums")),
    );
    let dl_args: Vec<_> = dl_command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    ensure_eq(
        &dl_args,
        &[
            "-q",
            "-fsSL",
            "--globoff",
            "--max-time",
            "300",
            "--max-filesize",
            &CHECKSUMS_MAX_BYTES.to_string(),
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--tlsv1.2",
            "-o",
            "/tmp/sums",
            "--",
            "https://example.test/SHA256SUMS",
        ],
        "download curl args start with -q and place -o before -- and url",
    )?;
    Ok(())
}

#[test]
fn curl_get_capped_rejects_oversized_stdout() -> TestResult {
    let bin = tempfile::tempdir().check("fake bin dir")?;
    let log = bin.path().join("curl-argv.log");
    plant_fake_curl(bin.path(), &log, "0123456789abcdefX", "", 0)?;

    let err = with_replaced_path(bin.path(), || {
        curl_get_capped("https://example.invalid/x", 16)
    })?
    .check_err("oversized stdout must fail")?;
    ensure(
        matches!(&err, UpdateError::Network(msg) if msg.contains("exceeded 16 bytes")),
        format!("expected Network size error, got {err:?}"),
    )?;
    Ok(())
}

#[test]
fn download_capped_removes_and_rejects_oversized_file() -> TestResult {
    let bin = tempfile::tempdir().check("temp dir")?;
    let script = bin.path().join("curl");
    std::fs::write(
        &script,
        "#!/bin/sh\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = \"-o\" ]; then dest=$2; shift 2; continue; fi\n  shift\ndone\nprintf '0123456789abcdefX' > \"$dest\"\n",
    )
    .check("write fake curl")?;
    make_executable(&script)?;
    let dest = bin.path().join("asset");

    let err = with_replaced_path(bin.path(), || {
        download_capped("https://example.invalid/x", &dest, "download to", 16)
    })?
    .check_err("oversized download must fail")?;
    ensure(
        matches!(&err, UpdateError::Download { source, .. } if matches!(source.as_ref(), UpdateError::Network(m) if m.contains("exceeded 16 bytes"))),
        format!("expected Download(Network) size error, got {err:?}"),
    )?;
    ensure(!dest.exists(), "oversized download must be removed")?;
    Ok(())
}
