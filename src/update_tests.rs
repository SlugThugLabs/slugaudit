//! Tests for the self-update helpers that don't need the network:
//! release URL building, version comparison, and checksum verification
//! against a real `sha256sum` invocation.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult};

/// The checksum tests drive a real `sha256sum`; a machine without it must fail
/// them, not report green for tests that ran nothing. They resolve and execute
/// it through `PATH`, so each holds `TEST_ENV_LOCK` for its whole body: the
/// update failure-path tests replace `PATH` with a fake bin directory, and an
/// unguarded checksum test would otherwise execute the fake.
fn require_sha256sum() {
    assert!(
        which::which("sha256sum").is_ok(),
        "sha256sum (coreutils) is required to run the checksum tests"
    );
}

/// Runs `f` with `PATH` pointing at exactly `bin_dir`: the inherited entries
/// are dropped, so `curl` and `sha256sum` resolve only to the fakes planted
/// there and no real network request or coreutils process can run.
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

/// Plants a fake `curl` that answers the `SHA256SUMS` URL with `sums` and
/// every other URL with `binary`, writing the body to the `-o` destination and
/// appending each invocation's argv to `log`.
fn plant_fake_curl(
    bin_dir: &std::path::Path,
    log: &std::path::Path,
    binary: &str,
    sums: &str,
) -> TestResult {
    let script = bin_dir.join("curl");
    let body = format!(
        "#!/bin/sh\n\
         printf '%s\\n' \"$@\" >> '{log}'\n\
         dest=\n\
         while [ $# -gt 0 ]; do\n\
           if [ \"$1\" = \"-o\" ]; then dest=$2; shift 2; continue; fi\n\
           url=$1; shift\n\
         done\n\
         case \"$url\" in\n\
           *SHA256SUMS*) printf '%s' '{sums}' > \"$dest\" ;;\n\
           *) printf '%s' '{binary}' > \"$dest\" ;;\n\
         esac\n",
        log = log.display(),
        binary = binary,
        sums = sums,
    );
    std::fs::write(&script, body).check("write fake curl")?;
    make_executable(&script)?;
    Ok(())
}

/// Plants a fake `sha256sum` that reports `digest` for whatever path it is
/// given; `verify_checksum` compares digests, never filenames.
fn plant_fake_sha256sum(bin_dir: &std::path::Path, digest: &str) -> TestResult {
    let script = bin_dir.join("sha256sum");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s  %s\\n' '{digest}' \"$1\"\n"),
    )
    .check("write fake sha256sum")?;
    make_executable(&script)?;
    Ok(())
}

/// Verifies a real (mock) binary against a SHA256SUMS file by invoking
/// `sha256sum`.
#[test]
fn verify_checksum_accepts_a_matching_file() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    require_sha256sum();
    let dir = tempfile::tempdir().check("temp dir")?;
    let binary = dir.path().join("slugaudit-x86_64-unknown-linux-gnu");
    let contents = b"#!/bin/sh\necho fake binary\n";
    std::fs::write(&binary, contents).check("write fake binary")?;

    // Compute the expected hash and write a SHA256SUMS line for it.
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

/// The temp binary need not be named exactly the asset filename:
/// verification compares digests, not filenames, so a unique temp name (as
/// `apply_update` uses) must verify fine. This guards against a temp name
/// that has to equal the asset filename.
#[test]
fn verify_checksum_accepts_a_unique_temporary_file_path() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    require_sha256sum();
    let dir = tempfile::tempdir().check("temp dir")?;
    // Deliberately NOT named the asset — a unique `.update-<pid>` name.
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

/// A SHA256SUMS entry that doesn't match must be rejected so the old binary
/// is never replaced with a corrupt download.
#[test]
fn verify_checksum_rejects_a_mismatching_file() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    require_sha256sum();
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
            assert!(
                msg.contains("checksum mismatch"),
                "unexpected error message: {msg}"
            );
        }
        other => {
            return Err(format!("expected Checksum error, got: {other:?}"))
                .check("unexpected outcome");
        }
    }
    Ok(())
}

#[test]
fn target_binary_prefers_the_installed_stable_path() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let slugthug = temp.path().join("slugaudit");
    std::fs::create_dir_all(&slugthug).check("create dir")?;
    let stable = slugthug.join("slugaudit");
    std::fs::write(&stable, b"fake installed binary").check("write fake install")?;

    temp_env::with_var(
        "SLUGTHUG_HOME",
        Some(temp.path().as_os_str()),
        || -> TestResult {
            let target = target_binary().check("target binary resolves")?;
            assert_eq!(target, stable);
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn verify_checksum_fails_on_missing_sums_file() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let binary = dir.path().join("fake_bin");
    std::fs::write(&binary, b"content").check("write binary")?;
    let missing_sums = dir.path().join("nonexistent_SHA256SUMS");
    let err = verify_checksum(&binary, &missing_sums).check_err("the call must fail")?;
    assert!(err.to_string().contains("reading checksums"));
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
    assert!(err.to_string().contains("has no entry for"));
    Ok(())
}

#[test]
fn verify_checksum_fails_when_binary_file_missing() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    require_sha256sum();
    let dir = tempfile::tempdir().check("temp dir")?;
    let missing_binary = dir.path().join("missing_bin");
    let sums_file = dir.path().join("SHA256SUMS");
    std::fs::write(&sums_file, format!("abc  {ASSET}\n")).check("write sums")?;
    let err = verify_checksum(&missing_binary, &sums_file).check_err("the call must fail")?;
    assert!(err.to_string().contains("checksum verification failed"));
    Ok(())
}

#[test]
fn update_error_display_covers_all_variants() {
    assert!(
        UpdateError::ToolSpawn {
            tool: "curl",
            source: std::io::Error::other("Permission denied (os error 13)"),
        }
        .to_string()
        .contains("could not run curl (required to self-update): Permission denied")
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
fn a_download_failure_keeps_its_typed_cause() {
    let err = UpdateError::Download {
        action: "download binary to",
        path: PathBuf::from("/test"),
        source: Box::new(UpdateError::Network("conn reset".into())),
    };

    assert!(
        err.to_string()
            .contains("could not download binary to /test")
    );
    assert!(
        std::error::Error::source(&err)
            .is_some_and(|cause| cause.to_string().contains("network request failed")),
        "the cause must stay reachable through source()"
    );
}

#[test]
fn only_the_published_platform_may_self_update() {
    assert!(platform_supported("linux", "x86_64"));
    assert!(!platform_supported("linux", "aarch64"));
    assert!(!platform_supported("macos", "x86_64"));
    assert!(!platform_supported("macos", "aarch64"));
}

/// The refusal `run_update` takes on an unsupported machine is reachable on
/// this one through the same pure check, with the actual platform kept.
#[test]
fn an_unsupported_platform_is_the_exact_unsupported_variant() -> TestResult {
    let error = ensure_platform_supported("macos", "aarch64")
        .check_err("a platform with no published asset must be refused")?;

    let UpdateError::UnsupportedPlatform { os, arch } = error else {
        return Err(format!("expected UnsupportedPlatform, got {error:?}"))
            .check("unexpected outcome");
    };
    assert_eq!(os, "macos");
    assert_eq!(arch, "aarch64");
    Ok(())
}

#[test]
fn the_published_platform_is_accepted() -> TestResult {
    ensure_platform_supported("linux", "x86_64").check("the published asset must be accepted")?;
    Ok(())
}

#[test]
fn the_unsupported_platform_error_names_both_sides() {
    let message = UpdateError::UnsupportedPlatform {
        os: "macos",
        arch: "aarch64",
    }
    .to_string();

    assert!(message.contains("linux/x86_64"), "{message}");
    assert!(message.contains("macos/aarch64"), "{message}");
}

#[test]
fn update_fetches_only_a_newer_or_incomparable_release() {
    let cases = [
        ("1.0.10", "1.0.10", UpdateDecision::UpToDate),
        ("1.1.0", "1.0.10", UpdateDecision::UpToDate),
        ("1.0.9", "1.0.10", UpdateDecision::Newer),
        ("1.0.10-dev", "1.0.10", UpdateDecision::Incomparable),
    ];

    for (current, latest, expected) in cases {
        assert_eq!(
            update_decision(current, latest),
            expected,
            "{current} vs {latest}"
        );
    }
}

/// A `sha256sum` that exits 0 but prints nothing must be named as a parse
/// failure, not surface as a mismatch against an empty digest.
#[test]
fn an_empty_sha256sum_output_is_a_named_parse_failure() -> TestResult {
    let error = parse_digest("  \n").check_err("no token is no digest")?;

    assert!(error.to_string().contains("printed no digest"), "{error}");
    Ok(())
}

#[test]
fn the_first_sha256sum_token_is_the_lowercase_digest() -> TestResult {
    assert_eq!(parse_digest("ABCDEF  file\n").check("digest")?, "abcdef");
    Ok(())
}

/// A stat failure on the installed path is reported rather than read as "not
/// installed", which would make the update replace a temporary running path.
#[test]
fn target_binary_reports_a_failing_stat_of_the_installed_path() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let directory = tempfile::tempdir().check("temp dir")?;
    let home_is_a_file = directory.path().join("not-a-directory");
    std::fs::write(&home_is_a_file, b"a file where the home belongs").check("plant file")?;

    let result = temp_env::with_var(
        "SLUGTHUG_HOME",
        Some(home_is_a_file.as_os_str()),
        target_binary,
    );

    let Err(UpdateError::Target(source)) = result else {
        return Err(format!("expected Target, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(source.kind(), std::io::ErrorKind::NotADirectory);
    Ok(())
}

/// A target with no parent cannot host the temp files the update stages, so
/// the failure is named as the prepare stage rather than passed to the
/// filesystem.
#[test]
fn apply_update_with_a_target_that_has_no_parent_reports_the_prepare_stage() -> TestResult {
    let latest = LatestRelease::from_tag("v1.0.3");

    let result = apply_update(&latest, Path::new("/"));

    let Err(UpdateError::Stage {
        action,
        path,
        source,
    }) = result
    else {
        return Err(format!("expected Stage, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(action, "prepare");
    assert_eq!(path, PathBuf::from("/"));
    assert_eq!(source.kind(), std::io::ErrorKind::InvalidInput);
    Ok(())
}

/// A placement failure after a verified download keeps the failing step and
/// path under `Stage`, so the operator sees which move failed.
#[test]
fn apply_update_maps_a_placement_failure_to_the_stage_variant() -> TestResult {
    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let directory = tempfile::tempdir().check("temp dir")?;
    let bin = tempfile::tempdir().check("fake bin dir")?;
    let argv_log = bin.path().join("curl-argv.log");
    plant_fake_curl(
        bin.path(),
        &argv_log,
        "fake binary",
        &format!("{DIGEST}  {ASSET}"),
    )?;
    plant_fake_sha256sum(bin.path(), DIGEST)?;

    let install_dir = directory.path().join("install");
    let target = install_dir.join("slugaudit");
    std::fs::create_dir_all(&target).check("plant a directory at the update target")?;
    std::fs::write(target.join("keep"), b"kept").check("make the target non-empty")?;
    let latest = LatestRelease::from_tag("v1.0.3");

    let result = with_replaced_path(bin.path(), || apply_update(&latest, &target))?;

    let Err(UpdateError::Stage {
        action,
        path,
        source,
    }) = result
    else {
        return Err(format!("expected Stage, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(action, "retain the previous binary at");
    assert_eq!(path, install_dir.join("slugaudit.prev"));
    // `std::fs::copy` refuses a directory as its source before it reads any
    // bytes; the target directory is what made the retain step fail.
    assert_eq!(source.kind(), std::io::ErrorKind::InvalidInput);
    assert!(target.is_dir(), "the target must be left in place");
    let argv = std::fs::read_to_string(&argv_log).check("the fake curl ran")?;
    assert!(
        argv.contains(
            "https://github.com/SlugThugLabs/slugaudit/releases/download/v1.0.3/slugaudit-x86_64-unknown-linux-gnu"
        ),
        "the fake curl must have served the asset download: {argv}"
    );
    Ok(())
}

/// Without `sha256sum` on `PATH` the checksum step reports the missing tool
/// rather than comparing against an empty digest.
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
    assert_eq!(tool, "sha256sum");
    assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
    Ok(())
}

/// On a platform with no published asset `run_update` must refuse before any
/// network call. Compiled only where the refusal is reachable; Linux/x86_64
/// (the platform this repository builds and tests) never takes it.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[test]
fn run_update_refuses_a_platform_with_no_published_asset() {
    let result = run_update();

    let Err(UpdateError::UnsupportedPlatform { os, arch }) = result else {
        panic!("expected UnsupportedPlatform, got {result:?}");
    };
    assert_eq!(os, std::env::consts::OS);
    assert_eq!(arch, std::env::consts::ARCH);
}
