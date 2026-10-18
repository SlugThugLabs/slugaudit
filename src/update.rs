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
//!   version it replaced can therefore be undone without a download — see
//!   `docs/TROUBLESHOOTING.md`. The copy is overwritten by the next install or
//!   update, and nothing else reads it.
#![allow(clippy::print_stdout)]

use std::path::{Path, PathBuf};
use thiserror::Error;
use update_fetch::{
    ASSET, ASSET_ARCH, ASSET_OS, LatestRelease, compare_versions, download, fetch_latest_release,
    platform_supported,
};

#[path = "update_fetch.rs"]
mod update_fetch;

#[derive(Debug, Error)]
/// Self-update failures. Each names the real cause (spawn, network, checksum,
/// filesystem) rather than a guessed one.
pub enum UpdateError {
    #[error(
        "could not run {tool} (required to self-update): {source}; \
         if {tool} is genuinely missing, download the release manually from GitHub"
    )]
    ToolSpawn {
        tool: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("could not determine the binary to update: {0}")]
    Target(std::io::Error),
    #[error("network request failed: {0}")]
    Network(String),
    #[error("the latest release response was not JSON decodable: {0}")]
    Json(String),
    #[error("the GitHub release response did not include a tag name")]
    NoTag,
    #[error("checksum verification failed: {0}")]
    Checksum(String),
    #[error(
        "slugaudit update only fetches the {ASSET_OS}/{ASSET_ARCH} release, and this machine \
         is {os}/{arch}; replacing the binary with it would leave it unable to start. \
         Install a build for this platform instead"
    )]
    UnsupportedPlatform {
        os: &'static str,
        arch: &'static str,
    },
    #[error("could not operate on the filesystem: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not {action} {path}: {source}")]
    Download {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: Box<UpdateError>,
    },
    #[error("could not {action} {path}: {source}")]
    Stage {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// The absolute path the binary that `slugaudit update` should replace.
/// Prefers the stable install path when present (matches what `connect`
/// registers), otherwise the running executable.
fn target_binary() -> Result<PathBuf, UpdateError> {
    let running = crate::install::running_binary().map_err(UpdateError::Target)?;
    Ok(crate::connect::prefer_slugthug_binary(&running))
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
    // the temp binary no longer needs to be named exactly `ASSET` — which
    // would otherwise overwrite a same-named file a user had placed in the
    // directory before verification had a chance to run.
    let temp_exe = dir.join(format!(".slugaudit-update-{}-{ASSET}", std::process::id()));
    let temp_sums = dir.join(format!("{ASSET}-{}-SHA256SUMS", std::process::id()));

    let result = (|| -> Result<(), UpdateError> {
        download(&latest.asset_url, &temp_exe, "download binary to")?;
        download(&latest.checksums_url, &temp_sums, "download checksums to")?;

        verify_checksum(&temp_exe, &temp_sums)?;

        crate::install_place::place_staged_binary(&temp_exe, target).map_err(|error| {
            UpdateError::Stage {
                action: error.action,
                path: error.path,
                source: error.source,
            }
        })?;
        Ok(())
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
    let sums = std::fs::read_to_string(sums_path)
        .map_err(|error| UpdateError::Checksum(format!("reading checksums: {error}")))?;
    // SHA256SUMS lines are `<hex>  <filename>`; match by the trailing
    // filename so entries for other assets (if the release ever ships more)
    // are ignored.
    let expected = sums
        .lines()
        .filter_map(|line| line.rsplit_once(' '))
        .find_map(|(hash, name)| (name.trim() == ASSET).then(|| hash.trim().to_ascii_lowercase()))
        .ok_or_else(|| {
            UpdateError::Checksum(format!(
                "SHA256SUMS has no entry for {ASSET}; refusing to update"
            ))
        })?;

    let output = std::process::Command::new("sha256sum")
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
    let actual = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if actual != expected {
        return Err(UpdateError::Checksum(format!(
            "checksum mismatch for {ASSET}: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

/// What `update` does after comparing this build with the published release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateDecision {
    /// This build is the published version or newer: nothing to fetch.
    UpToDate,
    /// The published release is newer: fetch it.
    Newer,
    /// One side is not a plain dotted version (a dev build): fetch it anyway,
    /// without claiming it is newer.
    Incomparable,
}

fn update_decision(current: &str, latest: &str) -> UpdateDecision {
    match compare_versions(current, latest) {
        Ok(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal) => UpdateDecision::UpToDate,
        Ok(std::cmp::Ordering::Less) => UpdateDecision::Newer,
        Err(error) => {
            tracing::info!(%error, "versions are not comparable; fetching the release anyway");
            UpdateDecision::Incomparable
        }
    }
}

/// Entry point for `slugaudit update`. Prints human-facing progress to
/// stdout; never touches the MCP transport.
pub fn run_update() -> Result<(), UpdateError> {
    let current = env!("CARGO_PKG_VERSION");

    // Refused before any network call: the only published asset is for one
    // platform, and its checksum would pass on any other.
    if !platform_supported(std::env::consts::OS, std::env::consts::ARCH) {
        return Err(UpdateError::UnsupportedPlatform {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        });
    }

    let latest = fetch_latest_release()?;

    // Compare by version when the local build carries a numeric version; if
    // it doesn't parse (e.g. a dev build), assume the published latest is
    // what we want and proceed to fetch it.
    match update_decision(current, &latest.version) {
        UpdateDecision::UpToDate => {
            println!("slugaudit is already up to date (v{current}).");
            return Ok(());
        }
        UpdateDecision::Newer => {
            println!(
                "A newer release (tag {}) is available; fetching...",
                latest.tag
            );
        }
        UpdateDecision::Incomparable => {
            // The comparison could not be made — one side is not a plain
            // dotted number, which a dev build is not. Falling back to
            // fetching is deliberate (the comment above records that), but
            // saying "a newer release is available" would assert a comparison
            // that never happened.
            println!(
                "This build's version ({current}) could not be compared with the \
                 published {tag}; fetching it as though it were newer.",
                tag = latest.tag
            );
        }
    }

    let target = target_binary()?;
    println!(
        "Updating slugaudit v{current} -> {} at {}...",
        latest.version,
        target.display()
    );

    apply_update(&latest, &target)?;

    println!("Done. Restart any running AI sessions so they launch the new binary.");
    Ok(())
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
