//! Tests for `update`: target discovery, platform gating, `O_EXCL` staging,
//! error formatting, and placement failure handling.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn target_binary() -> Result<PathBuf, UpdateError> {
    resolve_binaries().map(|(_, target)| target)
}

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

#[test]
fn create_staging_file_refuses_an_existing_path_without_overwriting() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let existing = dir.path().join(".slugaudit-update-existing.tmp");
    std::fs::write(&existing, b"keep-me").check("plant existing file")?;

    let err = create_staging_file(dir.path(), ".slugaudit-update-existing.tmp")
        .check_err("O_EXCL must refuse existing staging filename")?;

    let UpdateError::Stage {
        action,
        path,
        source,
    } = err
    else {
        return Err(format!("expected Stage error, got {err:?}")).check("unexpected outcome");
    };
    ensure_eq(
        &action,
        &"create staging file",
        "names create staging file step",
    )?;
    ensure_eq(&path, &existing, "names colliding staging path")?;
    ensure_eq(
        &source.kind(),
        &std::io::ErrorKind::AlreadyExists,
        "preserves AlreadyExists kind",
    )?;
    ensure_eq(
        &std::fs::read(&existing).check("read existing")?,
        &b"keep-me",
        "existing file at staging path stays intact",
    )?;
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
            ensure_eq(&target, &stable, "prefers installed stable path")?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn update_error_display_covers_all_variants() -> TestResult {
    ensure(
        UpdateError::ToolSpawn {
            tool: "curl",
            source: std::io::Error::other("Permission denied (os error 13)"),
        }
        .to_string()
        .contains("could not run curl (required to self-update): Permission denied"),
        "ToolSpawn display includes tool name and source cause",
    )?;
    ensure(
        UpdateError::Target(std::io::Error::other("fail"))
            .to_string()
            .contains("could not determine"),
        "Target display explains binary lookup failure",
    )?;
    ensure(
        UpdateError::Network("conn reset".into())
            .to_string()
            .contains("network request failed"),
        "Network display states network request failed",
    )?;
    ensure(
        UpdateError::Json("bad json".into())
            .to_string()
            .contains("JSON decodable"),
        "Json display states response was not JSON decodable",
    )?;
    ensure(
        UpdateError::NoTag.to_string().contains("tag name"),
        "NoTag display mentions missing tag name",
    )?;
    ensure(
        UpdateError::InvalidReleaseVersion("1.0.0-rc1".into())
            .to_string()
            .contains("numeric release"),
        "InvalidReleaseVersion display mentions numeric release",
    )?;
    ensure(
        UpdateError::Checksum("bad hash".into())
            .to_string()
            .contains("checksum verification failed"),
        "Checksum display states checksum verification failed",
    )?;
    ensure(
        UpdateError::Io(std::io::Error::other("io"))
            .to_string()
            .contains("filesystem"),
        "Io display mentions filesystem failure",
    )?;
    ensure(
        UpdateError::Stage {
            action: "create",
            path: PathBuf::from("/test"),
            source: std::io::Error::other("perm"),
        }
        .to_string()
        .contains("could not create /test"),
        "Stage display includes action and path",
    )?;
    Ok(())
}

#[test]
fn a_download_failure_keeps_its_typed_cause() -> TestResult {
    let err = UpdateError::Download {
        action: "download binary to",
        path: PathBuf::from("/test"),
        source: Box::new(UpdateError::Network("conn reset".into())),
    };

    ensure(
        err.to_string()
            .contains("could not download binary to /test"),
        "Download display includes action and destination",
    )?;
    ensure(
        std::error::Error::source(&err)
            .is_some_and(|cause| cause.to_string().contains("network request failed")),
        "the cause must stay reachable through source()",
    )?;
    Ok(())
}

#[test]
fn only_the_published_platform_may_self_update() -> TestResult {
    ensure(
        platform_supported("linux", "x86_64"),
        "accepts linux/x86_64",
    )?;
    ensure(
        !platform_supported("linux", "aarch64"),
        "rejects linux/aarch64",
    )?;
    ensure(
        !platform_supported("macos", "x86_64"),
        "rejects macos/x86_64",
    )?;
    ensure(
        !platform_supported("macos", "aarch64"),
        "rejects macos/aarch64",
    )?;
    Ok(())
}

#[test]
fn an_unsupported_platform_is_the_exact_unsupported_variant() -> TestResult {
    let error = ensure_platform_supported("macos", "aarch64")
        .check_err("a platform with no published asset must be refused")?;

    let UpdateError::UnsupportedPlatform { os, arch } = error else {
        return Err(format!("expected UnsupportedPlatform, got {error:?}"))
            .check("unexpected outcome");
    };
    ensure_eq(&os, &"macos", "records unsupported os")?;
    ensure_eq(&arch, &"aarch64", "records unsupported arch")?;
    Ok(())
}

#[test]
fn the_published_platform_is_accepted() -> TestResult {
    ensure_platform_supported("linux", "x86_64").check("the published asset must be accepted")?;
    Ok(())
}

#[test]
fn the_unsupported_platform_error_names_both_sides() -> TestResult {
    let message = UpdateError::UnsupportedPlatform {
        os: "macos",
        arch: "aarch64",
    }
    .to_string();

    ensure(message.contains("linux/x86_64"), &message)?;
    ensure(message.contains("macos/aarch64"), &message)?;
    Ok(())
}

#[test]
fn update_fetches_only_a_newer_or_incomparable_release() -> TestResult {
    let cases = [
        ("1.0.10", "1.0.10", UpdateDecision::UpToDate),
        ("1.1.0", "1.0.10", UpdateDecision::UpToDate),
        ("1.0.9", "1.0.10", UpdateDecision::Newer),
        ("1.0.10-dev", "1.0.10", UpdateDecision::Incomparable),
    ];

    for (current, latest, expected) in cases {
        let label = format!("{current} vs {latest}");
        let actual = update_decision(current, latest).check(&label)?;
        ensure_eq(&actual, &expected, label)?;
    }
    Ok(())
}

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
    ensure_eq(
        &source.kind(),
        &std::io::ErrorKind::NotADirectory,
        "preserves NotADirectory kind",
    )?;
    Ok(())
}

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
    ensure_eq(&action, &"prepare", "names prepare action")?;
    ensure_eq(&path, &PathBuf::from("/"), "names root path")?;
    ensure_eq(
        &source.kind(),
        &std::io::ErrorKind::InvalidInput,
        "reports InvalidInput for parentless target",
    )?;
    Ok(())
}

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
    ensure_eq(
        &action,
        &"retain the previous binary at",
        "names retain step",
    )?;
    ensure_eq(
        &path,
        &install_dir.join("slugaudit.prev"),
        "names slugaudit.prev path",
    )?;
    ensure_eq(
        &source.kind(),
        &std::io::ErrorKind::InvalidInput,
        "refuses directory source with InvalidInput",
    )?;
    ensure(target.is_dir(), "the target must be left in place")?;
    let argv = std::fs::read_to_string(&argv_log).check("the fake curl ran")?;
    ensure(
        argv.contains(
            "https://github.com/SlugThugLabs/slugaudit/releases/download/v1.0.3/slugaudit-x86_64-unknown-linux-gnu"
        ),
        format!("the fake curl must have served the asset download: {argv}"),
    )?;
    Ok(())
}
