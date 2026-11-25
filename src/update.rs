//! Self-update: fetch the latest GitHub release and atomically replace the
//! installed binary in place.
//!
//! SlugAudit is a single binary. All durable state lives per-project in
//! `<project>/.planning/slugaudit/project.db`, not in the binary's own
//! location, and stale databases are auto-rebuilt from source on first use
//! (`SourceSyncManager::ensure_current`). So "upgrade" really is just
//! replacing the executable at the same path: agent configs point at the
//! stable path and keep working, and project indexes are disposable.
//!
//! Design notes:
//! - We shell out to `curl` rather than pulling in an HTTP client
//!   dependency. The repo deliberately keeps its dependency graph lean and
//!   hand-reviews every direct dependency (see `deny.toml` + the README's
//!   dependency policy), so an HTTP stack for one subcommand isn't worth
//!   the tree.
//! - The download is verified against the release's `SHA256SUMS` before the
//!   old binary is touched. If verification fails, nothing is replaced and
//!   the current binary stays intact.
//! - Replacement is atomic: the new binary is written to a temp file in the
//!   target directory, then `rename`d over the old one, so a crash mid-copy
//!   never leaves a truncated executable behind. On the same filesystem,
//!   `std::fs::rename` is atomic.
//! - The displaced binary is retained as `slugaudit.prev` beside the new one
//!   (copied before the rename, so the atomicity above still holds and the copy
//!   comes from a complete file). An update that turns out to be worse than the
//!   version it replaced can therefore be undone without a download. The copy
//!   is overwritten by the next install or
//!   update, and nothing else reads it.

use std::path::{Path, PathBuf};
use thiserror::Error;
use update_fetch::{
    ASSET, ASSET_ARCH, ASSET_OS, CHECKSUMS_MAX_BYTES, LatestRelease, UpdateDecision, download,
    download_capped, fetch_latest_release, platform_supported, update_decision,
};

#[cfg(test)]
use update_proc::parse_binary_version;
use update_proc::{installed_version, parse_expected_digest};

#[path = "update_fetch.rs"]
mod update_fetch;

#[path = "update_proc.rs"]
mod update_proc;

#[derive(Debug, Error)]
/// Self-update failures. Each names the real cause (spawn, network, checksum,
/// filesystem) rather than a guessed one.
pub enum UpdateError {
    /// A helper tool required for self-update could not be run.
    #[error(
        "could not run {tool} (required to self-update): {source}; \
         if {tool} is genuinely missing, download the release manually from GitHub"
    )]
    ToolSpawn {
        /// Helper executable that could not be spawned.
        tool: &'static str,
        /// Underlying spawn error.
        #[source]
        source: std::io::Error,
    },
    /// The binary that the update should replace could not be determined.
    #[error("could not determine the binary to update: {0}")]
    Target(std::io::Error),
    /// The network request for release metadata or an asset failed.
    #[error("network request failed: {0}")]
    Network(String),
    /// The GitHub release response could not be decoded as JSON.
    #[error("the latest release response was not JSON decodable: {0}")]
    Json(String),
    /// The release response carried no tag name.
    #[error("the GitHub release response did not include a tag name")]
    NoTag,
    /// The published release tag is not a plain dot-separated numeric version.
    #[error("published release version is not a numeric release: {0}")]
    InvalidReleaseVersion(String),
    /// The downloaded binary did not verify against the release checksums.
    #[error("checksum verification failed: {0}")]
    Checksum(String),
    /// The machine's platform has no published release asset.
    #[error(
        "slugaudit update only fetches the {ASSET_OS}/{ASSET_ARCH} release, and this machine \
         is {os}/{arch}; replacing the binary with it would leave it unable to start. \
         Install a build for this platform instead"
    )]
    UnsupportedPlatform {
        /// Operating system that has no asset.
        os: &'static str,
        /// Architecture that has no asset.
        arch: &'static str,
    },
    /// A filesystem operation failed.
    #[error("could not operate on the filesystem: {0}")]
    Io(#[from] std::io::Error),
    /// Downloading a release asset failed.
    #[error("could not {action} {path}: {source}")]
    Download {
        /// Action being attempted, for the error message.
        action: &'static str,
        /// Destination path.
        path: PathBuf,
        /// Underlying download failure.
        #[source]
        source: Box<UpdateError>,
    },
    /// Staging or publishing the downloaded binary failed.
    #[error("could not {action} {path}: {source}")]
    Stage {
        /// Action being attempted, for the error message.
        action: &'static str,
        /// Path being operated on.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: std::io::Error,
    },
}

/// The running executable and the absolute path `slugaudit update` should
/// replace (preferring the stable install path when present, matching
/// `connect`).
fn resolve_binaries() -> Result<(PathBuf, PathBuf), UpdateError> {
    let running = crate::install::running_binary().map_err(UpdateError::Target)?;
    let target = crate::connect::prefer_slugthug_binary(&running).map_err(UpdateError::Target)?;
    Ok((running, target))
}

#[cfg(test)]
fn target_binary() -> Result<PathBuf, UpdateError> {
    resolve_binaries().map(|(_, target)| target)
}

/// Downloads the release, verifies its checksum, and atomically replaces the
/// target binary. Leaves the existing binary untouched on any failure.
fn apply_update(latest: &LatestRelease, target: &Path) -> Result<(), UpdateError> {
    let dir = target.parent().ok_or_else(|| UpdateError::Stage {
        action: "prepare",
        path: target.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "target has no parent directory",
        ),
    })?;

    // Unique temp names: checksum verification compares digests directly
    // (see `verify_checksum`), not by `sha256sum -c` filename matching, so
    // the temp binary need not be named exactly `ASSET` — which
    // would otherwise overwrite a same-named file a user had placed in the
    // directory before verification had a chance to run.
    let temp_exe = dir.join(format!(".slugaudit-update-{}-{ASSET}", std::process::id()));
    let temp_sums = dir.join(format!("{ASSET}-{}-SHA256SUMS", std::process::id()));

    let result = (|| -> Result<(), UpdateError> {
        download(&latest.asset_url, &temp_exe, "download binary to")?;
        download_capped(
            &latest.checksums_url,
            &temp_sums,
            "download checksums to",
            CHECKSUMS_MAX_BYTES,
        )?;
        verify_checksum(&temp_exe, &temp_sums)?;
        crate::install_place::place_staged_binary(&temp_exe, target).map_err(|e| {
            UpdateError::Stage {
                action: e.action,
                path: e.path,
                source: e.source,
            }
        })
    })();

    // Best-effort cleanup of temp files whether or not we succeeded.
    crate::install_place::remove_leftover(&temp_exe, "could not remove update temporary file");
    crate::install_place::remove_leftover(&temp_sums, "could not remove update temporary file");
    result
}

/// Verifies `binary_path` against the `ASSET` entry in the SHA256SUMS text
/// in `sums_path` by comparing SHA-256 hex digests directly. Unlike
/// `sha256sum -c` (which matches entries by filename relative to the working
/// directory), this needs no filename agreement, so the temp binary can carry
/// a unique name instead of the exact asset filename. Fails if the sums file
/// has no `ASSET` entry, or if the digests differ — the old binary is never
/// touched on failure.
fn verify_checksum(binary_path: &Path, sums_path: &Path) -> Result<(), UpdateError> {
    let sums = crate::util::read_file_capped(sums_path, CHECKSUMS_MAX_BYTES)
        .map_err(|error| UpdateError::Checksum(format!("reading checksums: {error}")))?;
    let expected = parse_expected_digest(&sums)?;

    let output = std::process::Command::new("sha256sum")
        .arg("--")
        .arg(binary_path)
        .output()
        .map_err(|error| UpdateError::ToolSpawn {
            tool: "sha256sum",
            source: error,
        })?;
    if !output.status.success() {
        return Err(UpdateError::Checksum(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    let actual = parse_digest(&String::from_utf8_lossy(&output.stdout))?;
    if actual != expected {
        return Err(UpdateError::Checksum(format!(
            "checksum mismatch for {ASSET}: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

/// The lowercase digest `sha256sum` printed (its first token). Output with no
/// token is a parse failure, named as such rather than compared as `""`.
fn parse_digest(stdout: &str) -> Result<String, UpdateError> {
    stdout
        .split_whitespace()
        .next()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| UpdateError::Checksum("sha256sum printed no digest".to_owned()))
}

/// Refuses a platform the single published asset cannot run on. Takes the
/// OS/architecture so the refusal is testable on the supported platform.
fn ensure_platform_supported(os: &'static str, arch: &'static str) -> Result<(), UpdateError> {
    if platform_supported(os, arch) {
        Ok(())
    } else {
        Err(UpdateError::UnsupportedPlatform { os, arch })
    }
}

/// Entry point for `slugaudit update`. Prints human-facing progress to
/// stdout; never touches the MCP transport.
///
/// # Errors
///
/// Fails when the platform has no published asset, when the latest release
/// cannot be fetched or parsed, when the running binary's path cannot be
/// determined, when downloading, verifying, or replacing the binary fails, or
/// when writing the progress text fails.
pub fn run_update() -> Result<(), UpdateError> {
    // Refused before any network call: the only published asset is for one
    // platform, and its checksum would pass on any other.
    ensure_platform_supported(std::env::consts::OS, std::env::consts::ARCH)?;

    let (running, target) = resolve_binaries()?;
    let current = installed_version(&running, &target);
    let latest = fetch_latest_release()?;

    // Compare by version when the target build carries a numeric version; if
    // it doesn't parse (e.g. a dev build) and the published latest is a valid
    // numeric release, assume the published latest is what we want and proceed.
    match update_decision(&current, &latest.version)? {
        UpdateDecision::UpToDate => {
            crate::cli::out(format_args!(
                "slugaudit is already up to date (v{current}).\n"
            ))?;
            return Ok(());
        }
        UpdateDecision::Newer => {
            crate::cli::out(format_args!(
                "A newer release (tag {}) is available; fetching...\n",
                latest.tag
            ))?;
        }
        UpdateDecision::Incomparable => {
            // The comparison could not be made — one side is not a plain
            // dotted number, which a dev build is not. Falling back to
            // fetching is deliberate (the comment above records that), but
            // saying "a newer release is available" would assert a comparison
            // that never happened.
            crate::cli::out(format_args!(
                "This build's version ({current}) could not be compared with the \
                 published {tag}; fetching it as though it were newer.\n",
                tag = latest.tag
            ))?;
        }
    }

    crate::cli::out(format_args!(
        "Updating slugaudit v{current} -> {} at {}...\n",
        latest.version,
        target.display()
    ))?;

    apply_update(&latest, &target)?;

    crate::cli::out(format_args!(
        "Done. Restart any running AI sessions so they launch the new binary.\n"
    ))?;
    Ok(())
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "update_version_tests.rs"]
mod version_tests;
