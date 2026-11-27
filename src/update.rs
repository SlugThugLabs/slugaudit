//! Self-update: fetch the latest GitHub release and atomically replace the
//! installed binary in place.
//!
//! SlugAudit is a single binary. All durable state lives per-project in
//! `<project>/.planning/slugaudit/project.db`, not in the binary's own
//! location, and stale databases are auto-rebuilt from source on first use
//! (`SourceSyncManager::ensure_current`). So "upgrade" really is just
//! replacing the executable at the same path: agent configs point at the
//! stable path and keep working, and project indexes are disposable.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use update_fetch::{
    ASSET, ASSET_ARCH, ASSET_OS, CHECKSUMS_MAX_BYTES, LatestRelease, download, download_capped,
    fetch_latest_release, platform_supported,
};
use update_verify::{UpdateDecision, installed_version, update_decision, verify_checksum};

#[path = "update_fetch.rs"]
mod update_fetch;

#[path = "update_proc.rs"]
mod update_proc;

#[path = "update_verify.rs"]
mod update_verify;

static UPDATE_STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

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

/// Atomically reserves a brand-new staging file inside `dir` with `O_EXCL`
/// (`create_new(true)`) so `curl -o` never follows a pre-existing symlink or
/// clobbers an existing file.
fn create_staging_file(dir: &Path, filename: &str) -> Result<PathBuf, UpdateError> {
    let path = dir.join(filename);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map(drop)
        .map_err(|source| UpdateError::Stage {
            action: "create staging file",
            path: path.clone(),
            source,
        })?;
    Ok(path)
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

    let seq = UPDATE_STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let temp_exe = create_staging_file(dir, &format!(".slugaudit-update-{pid}-{seq}-{ASSET}.tmp"))?;
    let temp_sums = match create_staging_file(
        dir,
        &format!(".slugaudit-update-{pid}-{seq}-SHA256SUMS.tmp"),
    ) {
        Ok(path) => path,
        Err(error) => {
            crate::install::remove_leftover(&temp_exe, "could not remove update temporary file");
            return Err(error);
        }
    };

    let result = (|| -> Result<(), UpdateError> {
        download(&latest.asset_url, &temp_exe, "download binary to")?;
        download_capped(
            &latest.checksums_url,
            &temp_sums,
            "download checksums to",
            CHECKSUMS_MAX_BYTES,
        )?;
        verify_checksum(&temp_exe, &temp_sums)?;
        crate::install::place_staged_binary(&temp_exe, target).map_err(|e| UpdateError::Stage {
            action: e.action,
            path: e.path,
            source: e.source,
        })
    })();

    // Best-effort cleanup of temp files whether or not we succeeded.
    crate::install::remove_leftover(&temp_exe, "could not remove update temporary file");
    crate::install::remove_leftover(&temp_sums, "could not remove update temporary file");
    result
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
