#![forbid(unsafe_code)]
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

//! End-to-end tests for `slugaudit update` on the shipped platform.
//!
//! The compile-gated refusal in `src/update_tests.rs` never runs on
//! `Linux/x86_64`, so these tests are what assert the `UpToDate`, `Newer`, and
//! `Incomparable` branches of `run_update` and their user-visible text.

use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ASSET: &str = "slugaudit-x86_64-unknown-linux-gnu";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const NEW_BINARY: &str = "the verified release asset";
const MATCHING_DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const MISMATCHING_DIGEST: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

const API_URL: &str = "https://api.github.com/repos/SlugThugLabs/slugaudit/releases/latest";

fn old_binary_script() -> String {
    format!("#!/bin/sh\necho 'slugaudit {CURRENT_VERSION}'\n")
}

struct UpdateSandbox {
    _temp: tempfile::TempDir,
    home: PathBuf,
    bin_dir: PathBuf,
    target: PathBuf,
    curl_log: PathBuf,
    sha_log: PathBuf,
}

impl UpdateSandbox {
    fn new() -> TestResult<Self> {
        let temp = tempfile::tempdir().check("create temp dir")?;
        let home = temp.path().join("home");
        let install_dir = home.join(".slugthug").join("slugaudit");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&install_dir).check("create install dir")?;
        fs::create_dir_all(&bin_dir).check("create bin dir")?;
        let target = install_dir.join("slugaudit");
        write_executable(&target, &old_binary_script())?;

        Ok(UpdateSandbox {
            curl_log: temp.path().join("curl-argv.log"),
            sha_log: temp.path().join("sha256sum-argv.log"),
            _temp: temp,
            home,
            bin_dir,
            target,
        })
    }

    fn plant_curl(&self, tag: &str, digest: &str) -> TestResult {
        let json = format!(r#"{{"tag_name":"{tag}"}}"#);
        let sums = format!("{digest}  {ASSET}");
        let script = format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$@\" >> '{log}'\n\
             dest=\n\
             url=\n\
             while [ $# -gt 0 ]; do\n\
               if [ \"$1\" = \"-o\" ]; then dest=$2; shift 2; continue; fi\n\
               url=$1; shift\n\
             done\n\
             case \"$url\" in\n\
               */releases/latest) printf '%s' '{json}' ;;\n\
               *SHA256SUMS*) printf '%s' '{sums}' > \"$dest\" ;;\n\
               *) printf '%s' '{NEW_BINARY}' > \"$dest\" ;;\n\
             esac\n",
            log = self.curl_log.display(),
        );
        write_executable(&self.bin_dir.join("curl"), &script)
    }

    fn plant_sha256sum(&self, digest: &str) -> TestResult {
        let script = format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$@\" >> '{log}'\n\
             printf '%s  %s\\n' '{digest}' \"$1\"\n",
            log = self.sha_log.display(),
        );
        write_executable(&self.bin_dir.join("sha256sum"), &script)
    }

    fn run(&self) -> TestResult<Output> {
        Command::new(env!("CARGO_BIN_EXE_slugaudit"))
            .arg("update")
            .env("HOME", &self.home)
            .env("SLUGTHUG_HOME", self.home.join(".slugthug"))
            .env("SLUGTHUG_CONFIG_HOME", &self.home)
            .env("PATH", &self.bin_dir)
            .env_remove("SLUGAUDIT_LOG_FORMAT")
            .output()
            .check("spawn slugaudit update")
    }

    fn previous(&self) -> PathBuf {
        self.target.with_file_name("slugaudit.prev")
    }
}

fn write_executable(path: &Path, script: &str) -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, script).check("write fake tool")?;
    let mut permissions = fs::metadata(path).check("stat fake tool")?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).check("chmod fake tool")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn run_update_reports_the_exact_up_to_date_message_without_fetching() -> TestResult {
    let sandbox = UpdateSandbox::new()?;
    sandbox.plant_curl(&format!("v{CURRENT_VERSION}"), MATCHING_DIGEST)?;
    sandbox.plant_sha256sum(MATCHING_DIGEST)?;

    let output = sandbox.run()?;

    ensure(
        output.status.success(),
        format!("update failed: {}", text(&output.stderr)),
    )?;
    ensure_eq(
        &text(&output.stdout),
        &format!("slugaudit is already up to date (v{CURRENT_VERSION}).\n"),
        "prints exact up-to-date message",
    )?;
    ensure_eq(
        &fs::read(&sandbox.target).check("read the installed binary")?,
        &old_binary_script().as_bytes(),
        "the up-to-date branch must not replace the binary",
    )?;
    ensure(
        !sandbox.previous().exists(),
        "nothing may be retained when nothing is replaced",
    )?;
    let curl = fs::read_to_string(&sandbox.curl_log).check("read the fake curl log")?;
    ensure(
        curl.contains(API_URL),
        format!("the release metadata must be fetched: {curl}"),
    )?;
    ensure(
        !curl.contains("/releases/download/"),
        format!("the up-to-date branch must not download an asset: {curl}"),
    )?;
    ensure(
        !sandbox.sha_log.exists(),
        "the up-to-date branch must not verify a download",
    )?;
    Ok(())
}

#[test]
fn run_update_replaces_the_target_with_the_newer_release_asset() -> TestResult {
    let sandbox = UpdateSandbox::new()?;
    sandbox.plant_curl("v1.2.0", MATCHING_DIGEST)?;
    sandbox.plant_sha256sum(MATCHING_DIGEST)?;

    let output = sandbox.run()?;

    ensure(
        output.status.success(),
        format!("update failed: {}", text(&output.stderr)),
    )?;
    ensure_eq(
        &text(&output.stdout),
        &format!(
            "A newer release (tag v1.2.0) is available; fetching...\n\
             Updating slugaudit v{CURRENT_VERSION} -> 1.2.0 at {}...\n\
             Done. Restart any running AI sessions so they launch the new binary.\n",
            sandbox.target.display()
        ),
        "prints exact newer-release progress output",
    )?;
    ensure_eq(
        &fs::read(&sandbox.target).check("read the updated binary")?,
        &NEW_BINARY.as_bytes(),
        "the verified release asset must be placed at the target",
    )?;
    ensure_eq(
        &fs::read(sandbox.previous()).check("read the retained binary")?,
        &old_binary_script().as_bytes(),
        "the displaced binary must be retained for a revert",
    )?;
    let curl = fs::read_to_string(&sandbox.curl_log).check("read the fake curl log")?;
    let asset_url =
        format!("https://github.com/SlugThugLabs/slugaudit/releases/download/v1.2.0/{ASSET}");
    let sums_url = "https://github.com/SlugThugLabs/slugaudit/releases/download/v1.2.0/SHA256SUMS";
    ensure(curl.contains(API_URL), format!("metadata fetch: {curl}"))?;
    ensure(curl.contains(&asset_url), format!("asset download: {curl}"))?;
    ensure(
        curl.contains(sums_url),
        format!("checksum download: {curl}"),
    )?;
    Ok(())
}

/// When the installed target binary reports a non-numeric dev version (such as
/// `1.1.0-dev`) and the published release is numeric, `run_update` takes the
/// `Incomparable` branch, states honestly that the versions could not be
/// compared, fetches and verifies the release, and retains the dev binary.
#[test]
fn run_update_fetches_release_when_installed_binary_has_incomparable_dev_version() -> TestResult {
    let sandbox = UpdateSandbox::new()?;
    let dev_script = "#!/bin/sh\nprintf 'slugaudit 1.1.0-dev\\n'\n";
    write_executable(&sandbox.target, dev_script)?;
    sandbox.plant_curl("v1.2.0", MATCHING_DIGEST)?;
    sandbox.plant_sha256sum(MATCHING_DIGEST)?;

    let output = sandbox.run()?;

    ensure(
        output.status.success(),
        format!("incomparable update failed: {}", text(&output.stderr)),
    )?;
    ensure_eq(
        &text(&output.stdout),
        &format!(
            "This build's version (1.1.0-dev) could not be compared with the published v1.2.0; fetching it as though it were newer.\n\
             Updating slugaudit v1.1.0-dev -> 1.2.0 at {}...\n\
             Done. Restart any running AI sessions so they launch the new binary.\n",
            sandbox.target.display()
        ),
        "prints exact Incomparable branch message without claiming newer",
    )?;
    ensure_eq(
        &fs::read(&sandbox.target).check("read updated target")?,
        &NEW_BINARY.as_bytes(),
        "replaces dev target with verified release asset",
    )?;
    ensure_eq(
        &fs::read(sandbox.previous()).check("read retained dev binary")?,
        &dev_script.as_bytes(),
        "retains displaced dev binary at slugaudit.prev",
    )?;
    Ok(())
}

#[test]
fn run_update_refuses_non_numeric_remote_release_without_downloading() -> TestResult {
    let sandbox = UpdateSandbox::new()?;
    sandbox.plant_curl("v1.2.0-rc1", MATCHING_DIGEST)?;
    sandbox.plant_sha256sum(MATCHING_DIGEST)?;

    let output = sandbox.run()?;

    ensure_eq(
        &output.status.code(),
        &Some(1),
        format!(
            "a non-numeric remote release must exit nonzero: {}",
            text(&output.stderr)
        ),
    )?;
    let stderr = text(&output.stderr);
    ensure(
        stderr.contains("published release version is not a numeric release"),
        format!("the refusal reason must be reported: {stderr}"),
    )?;
    ensure_eq(
        &fs::read(&sandbox.target).check("read the installed binary")?,
        &old_binary_script().as_bytes(),
        "a refused remote tag must leave the target untouched",
    )?;
    ensure(
        !sandbox.previous().exists(),
        "nothing may be retained when nothing is replaced",
    )?;
    let curl = fs::read_to_string(&sandbox.curl_log).check("read the fake curl log")?;
    ensure(
        !curl.contains("/releases/download/"),
        format!("a refused remote tag must not download any asset: {curl}"),
    )?;
    Ok(())
}

#[test]
fn run_update_checksum_mismatch_leaves_target_untouched() -> TestResult {
    let sandbox = UpdateSandbox::new()?;
    sandbox.plant_curl("v1.2.0", MATCHING_DIGEST)?;
    sandbox.plant_sha256sum(MISMATCHING_DIGEST)?;

    let output = sandbox.run()?;

    ensure_eq(
        &output.status.code(),
        &Some(1),
        format!(
            "a failed checksum must exit nonzero: {}",
            text(&output.stderr)
        ),
    )?;
    let stderr = text(&output.stderr);
    ensure(
        stderr.contains(&format!(
            "Error: checksum verification failed: checksum mismatch for {ASSET}: \
             expected {MATCHING_DIGEST}, got {MISMATCHING_DIGEST}"
        )),
        format!("the checksum failure must be reported: {stderr}"),
    )?;
    ensure_eq(
        &fs::read(&sandbox.target).check("read the installed binary")?,
        &old_binary_script().as_bytes(),
        "a failed verification must leave the target untouched",
    )?;
    ensure(
        !sandbox.previous().exists(),
        "nothing may be retained when nothing is replaced",
    )?;
    Ok(())
}

#[test]
fn run_update_replaces_broken_target_binary_when_version_probe_fails() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    const BROKEN_TARGET: &[u8] = b"corrupted non-executable binary";
    let sandbox = UpdateSandbox::new()?;
    fs::write(&sandbox.target, BROKEN_TARGET).check("overwrite target with broken bytes")?;
    let mut permissions = fs::metadata(&sandbox.target)
        .check("stat broken target")?
        .permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(&sandbox.target, permissions).check("make broken target non-executable")?;
    sandbox.plant_curl(&format!("v{CURRENT_VERSION}"), MATCHING_DIGEST)?;
    sandbox.plant_sha256sum(MATCHING_DIGEST)?;

    let output = sandbox.run()?;

    ensure(
        output.status.success(),
        format!("update over broken target failed: {}", text(&output.stderr)),
    )?;
    ensure_eq(
        &text(&output.stdout),
        &format!(
            "The installed binary's version at {path} could not be determined; \
             fetching v{CURRENT_VERSION} anyway.\n\
             Updating slugaudit -> {CURRENT_VERSION} at {path}...\n\
             Done. Restart any running AI sessions so they launch the new binary.\n",
            path = sandbox.target.display()
        ),
        "prints exact broken-target Incomparable message and updates without claiming UpToDate",
    )?;
    ensure_eq(
        &fs::read(&sandbox.target).check("read replaced target")?,
        &NEW_BINARY.as_bytes(),
        "replaces broken target with verified release asset",
    )?;
    ensure_eq(
        &fs::read(sandbox.previous()).check("read retained broken target")?,
        &BROKEN_TARGET,
        "retains broken target at slugaudit.prev",
    )?;
    Ok(())
}
