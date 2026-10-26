#![forbid(unsafe_code)]
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

//! End-to-end tests for `slugaudit update` on the shipped platform.
//!
//! `run_update` had no test on Linux/x86_64: the only direct call was the
//! compile-gated refusal in `src/update_tests.rs`, which never runs here. The
//! UpToDate, Newer, and Incomparable branches and their user-visible text were
//! asserted nowhere.
//!
//! These spawn the real binary because the branch text goes to stdout through
//! `cli::out`; an in-process call cannot capture it without a production seam.
//! Nothing outside the temp dir is read or written: `HOME`, `SLUGTHUG_HOME`
//! and `SLUGTHUG_CONFIG_HOME` point into it, the installed binary `update`
//! replaces is a temp file in it, and `PATH` is *replaced* (not prepended to)
//! by a directory holding a fake `curl` and `sha256sum`, so no real network
//! request or checksum process can run.
//!
//! The whole file is compiled only where the one published asset runs
//! (`linux/x86_64`). On any other platform `run_update` refuses before these
//! branches; that refusal is the compile-gated test in `src/update_tests.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The single published asset; mirrors `ASSET` in `src/update_fetch.rs`.
const ASSET: &str = "slugaudit-x86_64-unknown-linux-gnu";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const OLD_BINARY: &str = "the installed binary before the update";
const NEW_BINARY: &str = "the verified release asset";
/// The digest the fake `sha256sum` prints and the fake `SHA256SUMS` carries.
const MATCHING_DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// A digest the fake `sha256sum` prints when verification must fail.
const MISMATCHING_DIGEST: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

const API_URL: &str = "https://api.github.com/repos/SlugThugLabs/slugaudit/releases/latest";

/// A temp home with an installed binary at the stable path `target_binary`
/// prefers, plus a bin directory that replaces `PATH` for the child.
struct UpdateSandbox {
    _temp: tempfile::TempDir,
    home: PathBuf,
    bin_dir: PathBuf,
    target: PathBuf,
    curl_log: PathBuf,
    sha_log: PathBuf,
}

impl UpdateSandbox {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp dir");
        let home = temp.path().join("home");
        let install_dir = home.join(".slugthug").join("slugaudit");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&install_dir).expect("create install dir");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        let target = install_dir.join("slugaudit");
        fs::write(&target, OLD_BINARY).expect("plant the installed binary");

        UpdateSandbox {
            curl_log: temp.path().join("curl-argv.log"),
            sha_log: temp.path().join("sha256sum-argv.log"),
            _temp: temp,
            home,
            bin_dir,
            target,
        }
    }

    /// Plants a fake `curl` that answers the releases API with `tag`, the
    /// `SHA256SUMS` URL with a manifest carrying `digest`, and every other URL
    /// with [`NEW_BINARY`], appending each invocation's argv to a log.
    fn plant_curl(&self, tag: &str, digest: &str) {
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
        write_executable(&self.bin_dir.join("curl"), &script);
    }

    /// Plants a fake `sha256sum` that reports `digest` for whatever path it is
    /// given and records its argv.
    fn plant_sha256sum(&self, digest: &str) {
        let script = format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$@\" >> '{log}'\n\
             printf '%s  %s\\n' '{digest}' \"$1\"\n",
            log = self.sha_log.display(),
        );
        write_executable(&self.bin_dir.join("sha256sum"), &script);
    }

    fn run(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_slugaudit"))
            .arg("update")
            .env("HOME", &self.home)
            .env("SLUGTHUG_HOME", self.home.join(".slugthug"))
            .env("SLUGTHUG_CONFIG_HOME", &self.home)
            .env("PATH", &self.bin_dir)
            .env_remove("SLUGAUDIT_LOG_FORMAT")
            .output()
            .expect("spawn slugaudit update")
    }

    fn previous(&self) -> PathBuf {
        self.target.with_file_name("slugaudit.prev")
    }
}

fn write_executable(path: &Path, script: &str) {
    fs::write(path, script).expect("write fake tool");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path).expect("stat fake tool").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod fake tool");
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The up-to-date branch must print the exact message, fetch nothing beyond
/// the release metadata, and leave the installed binary untouched.
#[test]
fn run_update_reports_the_exact_up_to_date_message_without_fetching() {
    let sandbox = UpdateSandbox::new();
    sandbox.plant_curl(&format!("v{CURRENT_VERSION}"), MATCHING_DIGEST);
    sandbox.plant_sha256sum(MATCHING_DIGEST);

    let output = sandbox.run();

    assert!(
        output.status.success(),
        "update failed: {}",
        text(&output.stderr)
    );
    assert_eq!(
        text(&output.stdout),
        format!("slugaudit is already up to date (v{CURRENT_VERSION}).\n")
    );
    assert_eq!(
        fs::read(&sandbox.target).expect("read the installed binary"),
        OLD_BINARY.as_bytes(),
        "the up-to-date branch must not replace the binary"
    );
    assert!(
        !sandbox.previous().exists(),
        "nothing may be retained when nothing is replaced"
    );
    let curl = fs::read_to_string(&sandbox.curl_log).expect("the fake curl ran");
    assert!(
        curl.contains(API_URL),
        "the release metadata must be fetched: {curl}"
    );
    assert!(
        !curl.contains("/releases/download/"),
        "the up-to-date branch must not download an asset: {curl}"
    );
    assert!(
        !sandbox.sha_log.exists(),
        "the up-to-date branch must not verify a download"
    );
}

/// The newer-release branch must print the exact messages, download and verify
/// the asset, place it at the target, and retain the displaced binary.
#[test]
fn run_update_replaces_the_target_with_the_newer_release_asset() {
    let sandbox = UpdateSandbox::new();
    sandbox.plant_curl("v1.2.0", MATCHING_DIGEST);
    sandbox.plant_sha256sum(MATCHING_DIGEST);

    let output = sandbox.run();

    assert!(
        output.status.success(),
        "update failed: {}",
        text(&output.stderr)
    );
    assert_eq!(
        text(&output.stdout),
        format!(
            "A newer release (tag v1.2.0) is available; fetching...\n\
             Updating slugaudit v{CURRENT_VERSION} -> 1.2.0 at {}...\n\
             Done. Restart any running AI sessions so they launch the new binary.\n",
            sandbox.target.display()
        )
    );
    assert_eq!(
        fs::read(&sandbox.target).expect("read the updated binary"),
        NEW_BINARY.as_bytes(),
        "the verified release asset must be placed at the target"
    );
    assert_eq!(
        fs::read(sandbox.previous()).expect("read the retained binary"),
        OLD_BINARY.as_bytes(),
        "the displaced binary must be retained for a revert"
    );
    let curl = fs::read_to_string(&sandbox.curl_log).expect("the fake curl ran");
    let asset_url =
        format!("https://github.com/SlugThugLabs/slugaudit/releases/download/v1.2.0/{ASSET}");
    let sums_url = "https://github.com/SlugThugLabs/slugaudit/releases/download/v1.2.0/SHA256SUMS";
    assert!(curl.contains(API_URL), "metadata fetch: {curl}");
    assert!(curl.contains(&asset_url), "asset download: {curl}");
    assert!(curl.contains(sums_url), "checksum download: {curl}");
}

/// The incomparable branch must say the comparison could not be made — never
/// that a newer release was found — and a failed verification must leave the
/// installed binary untouched. The branch deliberately falls through to the
/// fetch (the message says so); the mismatching digest is what makes this test
/// assert "no replacement", without claiming the branch itself skips it.
#[test]
fn run_update_reports_the_exact_incomparable_message_without_replacing() {
    let sandbox = UpdateSandbox::new();
    sandbox.plant_curl("v1.2.0-rc1", MATCHING_DIGEST);
    sandbox.plant_sha256sum(MISMATCHING_DIGEST);

    let output = sandbox.run();

    assert_eq!(
        output.status.code(),
        Some(1),
        "a failed checksum must exit nonzero: {}",
        text(&output.stderr)
    );
    assert_eq!(
        text(&output.stdout),
        format!(
            "This build's version ({CURRENT_VERSION}) could not be compared with the \
             published v1.2.0-rc1; fetching it as though it were newer.\n\
             Updating slugaudit v{CURRENT_VERSION} -> 1.2.0-rc1 at {}...\n",
            sandbox.target.display()
        ),
        "the incomparable message must not claim a comparison happened"
    );
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains(&format!(
            "Error: checksum verification failed: checksum mismatch for {ASSET}: \
             expected {MATCHING_DIGEST}, got {MISMATCHING_DIGEST}"
        )),
        "the checksum failure must be reported: {stderr}"
    );
    assert_eq!(
        fs::read(&sandbox.target).expect("read the installed binary"),
        OLD_BINARY.as_bytes(),
        "a failed verification must leave the target untouched"
    );
    assert!(
        !sandbox.previous().exists(),
        "nothing may be retained when nothing is replaced"
    );
    let curl = fs::read_to_string(&sandbox.curl_log).expect("the fake curl ran");
    assert!(
        curl.contains(&format!(
            "https://github.com/SlugThugLabs/slugaudit/releases/download/v1.2.0-rc1/{ASSET}"
        )),
        "the incomparable branch must still fetch the release: {curl}"
    );
}
