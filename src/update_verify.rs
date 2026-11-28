//! Checksum verification, installed-binary version inspection, and numeric
//! release version comparison for `slugaudit update`.

use super::UpdateError;
use super::update_fetch::{ASSET, CHECKSUMS_MAX_BYTES};
use super::update_proc::run_bounded;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Largest stdout accepted from `sha256sum`, in bytes.
const SHA256SUM_STDOUT_MAX_BYTES: u64 = 16 * 1024;

/// Wall-clock deadline for `sha256sum` verification.
const SHA256SUM_TIMEOUT: Duration = Duration::from_secs(30);

/// Wall-clock deadline for querying `target --version`.
const VERSION_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Verifies `binary_path` against the `ASSET` entry in the SHA256SUMS text
/// in `sums_path` by comparing SHA-256 hex digests directly. Unlike
/// `sha256sum -c` (which matches entries by filename relative to the working
/// directory), this needs no filename agreement, so the temp binary can carry
/// a unique name instead of the exact asset filename.
pub(super) fn verify_checksum(binary_path: &Path, sums_path: &Path) -> Result<(), UpdateError> {
    let sums = crate::util::read_file_capped(sums_path, CHECKSUMS_MAX_BYTES)
        .map_err(|error| UpdateError::Checksum(format!("reading checksums: {error}")))?;
    let expected = parse_expected_digest(&sums)?;

    let mut command = Command::new("sha256sum");
    command.arg("--").arg(binary_path);
    let stdout = match run_bounded(
        &mut command,
        "sha256sum",
        Some(SHA256SUM_STDOUT_MAX_BYTES),
        SHA256SUM_TIMEOUT,
    ) {
        Ok(stdout) => stdout,
        Err(UpdateError::ToolSpawn { tool, source }) => {
            return Err(UpdateError::ToolSpawn { tool, source });
        }
        Err(UpdateError::Network(reason)) => {
            return Err(UpdateError::Checksum(reason));
        }
        Err(other) => return Err(other),
    };
    let actual = parse_digest(&String::from_utf8_lossy(&stdout))?;
    if actual != expected {
        return Err(UpdateError::Checksum(format!(
            "checksum mismatch for {ASSET}: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

/// The lowercase digest `sha256sum` printed (its first token). Output with no
/// token is a parse failure, named as such rather than compared as `""`.
pub(super) fn parse_digest(stdout: &str) -> Result<String, UpdateError> {
    stdout
        .split_whitespace()
        .next()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| UpdateError::Checksum("sha256sum printed no digest".to_owned()))
}

/// Parses the 64-hex-digit SHA-256 entry for `ASSET` from `SHA256SUMS` text,
/// accepting optional binary-mode `*` and `./` filename prefixes.
pub(super) fn parse_expected_digest(sums: &str) -> Result<String, UpdateError> {
    sums.lines()
        .find_map(|line| {
            let mut tokens = line.split_whitespace();
            let hash = tokens.next()?;
            let raw_name = tokens.next()?;
            if tokens.next().is_some() {
                return None;
            }
            let name = match raw_name.strip_prefix('*') {
                Some(stripped) => stripped,
                None => raw_name,
            };
            let name = match name.strip_prefix("./") {
                Some(stripped) => stripped,
                None => name,
            };
            let valid_hex = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
            (name == ASSET && valid_hex).then(|| hash.to_ascii_lowercase())
        })
        .ok_or_else(|| {
            UpdateError::Checksum(format!(
                "SHA256SUMS has no entry for {ASSET}; refusing to update"
            ))
        })
}

/// Whether two binary paths refer to the same file, comparing canonical paths
/// when both resolve and falling back to raw path equality otherwise.
fn same_binary_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Extracts `<ver>` from `slugaudit <ver>` printed by `--version`.
pub(super) fn parse_binary_version(stdout: &str) -> Option<String> {
    let mut tokens = stdout.split_whitespace();
    let name = tokens.next()?;
    let version = tokens.next()?;
    (name == "slugaudit" && tokens.next().is_none()).then(|| version.to_owned())
}

/// Returns the version of `target`: the compiled-in version when `target` is
/// the running binary, otherwise the version reported by `target --version`,
/// or `None` when a distinct target binary cannot be queried or does not
/// report a recognizable `slugaudit <ver>` output.
pub(super) fn installed_version(running: &Path, target: &Path) -> Option<String> {
    if same_binary_path(running, target) {
        return Some(env!("CARGO_PKG_VERSION").to_owned());
    }
    let mut cmd = Command::new(target);
    cmd.arg("--version");
    match run_bounded(&mut cmd, "slugaudit", Some(1024), VERSION_QUERY_TIMEOUT) {
        Ok(stdout) => {
            let text = String::from_utf8_lossy(&stdout);
            if let Some(version) = parse_binary_version(&text) {
                return Some(version);
            }
            tracing::warn!(
                path = %target.display(),
                output = text.trim(),
                "unexpected --version output from target binary",
            );
        }
        Err(error) => {
            tracing::warn!(
                path = %target.display(),
                error = %error,
                "could not query target binary version",
            );
        }
    }
    None
}

/// A version string that is not a plain dot-separated numeric version
/// (a dev build or a pre-release such as `1.0.0-rc1`).
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("version {version:?} is not a dot-separated numeric version: {source}")]
pub(crate) struct VersionParseError {
    version: String,
    #[source]
    source: std::num::ParseIntError,
}

/// Parses a dot-separated numeric version (with an optional leading `v`) into
/// its numeric components.
pub(crate) fn parse_version(v: &str) -> Result<Vec<u64>, VersionParseError> {
    v.trim_start_matches('v')
        .split('.')
        .map(|part| {
            part.parse::<u64>().map_err(|source| VersionParseError {
                version: v.to_owned(),
                source,
            })
        })
        .collect()
}

/// What `update` does after comparing this build with the published release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateDecision {
    /// This build is the published version or newer: nothing to fetch.
    UpToDate,
    /// The published release is newer: fetch it.
    Newer,
    /// This build is not a plain dotted version (a dev build) while the
    /// published release is numeric: fetch it anyway without claiming newer.
    Incomparable,
}

/// Decides whether `latest` should replace an optional installed `current`
/// version; `None` (unqueryable target binary) resolves to
/// [`UpdateDecision::Incomparable`] after validating that `latest` is numeric.
pub(super) fn update_decision_for(
    current: Option<&str>,
    latest: &str,
) -> Result<UpdateDecision, UpdateError> {
    let Some(current) = current else {
        let _parts = parse_version(latest)
            .map_err(|error| UpdateError::InvalidReleaseVersion(error.to_string()))?;
        tracing::info!(
            latest,
            "target binary version could not be determined; fetching release anyway"
        );
        return Ok(UpdateDecision::Incomparable);
    };
    update_decision(current, latest)
}

/// Decides whether `latest` should replace `current`. Refuses any non-numeric
/// published `latest` tag; allows an incomparable fallback only when `current`
/// itself is a non-numeric dev build.
pub(crate) fn update_decision(current: &str, latest: &str) -> Result<UpdateDecision, UpdateError> {
    let mut latest_parts = parse_version(latest)
        .map_err(|error| UpdateError::InvalidReleaseVersion(error.to_string()))?;
    let Ok(mut current_parts) = parse_version(current) else {
        tracing::info!(
            current,
            latest,
            "local version is not numeric; fetching release anyway"
        );
        return Ok(UpdateDecision::Incomparable);
    };
    let width = current_parts.len().max(latest_parts.len());
    current_parts.resize(width, 0);
    latest_parts.resize(width, 0);
    Ok(if current_parts < latest_parts {
        UpdateDecision::Newer
    } else {
        UpdateDecision::UpToDate
    })
}

/// Compares two dot-separated numeric versions; `v1.0.3` == `1.0.3` and a
/// missing trailing component counts as 0, so `1.0` == `1.0.0`.
#[cfg(test)]
pub(crate) fn compare_versions(
    current: &str,
    latest: &str,
) -> Result<std::cmp::Ordering, VersionParseError> {
    let mut current = parse_version(current)?;
    let mut latest = parse_version(latest)?;
    let width = current.len().max(latest.len());
    current.resize(width, 0);
    latest.resize(width, 0);
    Ok(current.cmp(&latest))
}

#[cfg(test)]
#[path = "update_verify_tests.rs"]
mod tests;
