//! Network + version helpers for self-update, kept in their own module so
//! `update.rs` stays under the small-file rule cap. Nothing here touches the
//! filesystem; it only builds URLs, parses the GitHub releases API, and
//! compares versions — all unit-testable without a network call.

use super::UpdateError;
use std::path::Path;

/// Default repo owner/name and the single published Linux x86_64 asset.
const OWNER: &str = "SlugThugLabs";
const REPO: &str = "slugaudit";
pub(crate) const ASSET: &str = "slugaudit-x86_64-unknown-linux-gnu";
/// The `std::env::consts` OS and architecture [`ASSET`] runs on.
pub(crate) const ASSET_OS: &str = "linux";
pub(crate) const ASSET_ARCH: &str = "x86_64";

/// curl flags that refuse any non-HTTPS URL, including a redirect target.
pub(crate) const HTTPS_ONLY: [&str; 4] = ["--proto", "=https", "--proto-redir", "=https"];

const GITHUB_API: &str = "https://api.github.com";
const GITHUB_DL: &str = "https://github.com";

/// Result of inspecting the latest release, separated from the fetch so the
/// version comparison can be unit-tested without a network call.
pub(crate) struct LatestRelease {
    pub tag: String,
    /// Trims a leading `v` so `v1.0.3` and `1.0.3` compare cleanly.
    pub version: String,
    pub asset_url: String,
    pub checksums_url: String,
}

impl LatestRelease {
    /// The asset + checksum download URLs derived from a release tag.
    pub(crate) fn from_tag(tag: &str) -> Self {
        let base = format!("{GITHUB_DL}/{OWNER}/{REPO}/releases/download/{tag}");
        LatestRelease {
            tag: tag.to_string(),
            version: tag.trim_start_matches('v').to_string(),
            asset_url: format!("{base}/{ASSET}"),
            checksums_url: format!("{base}/SHA256SUMS"),
        }
    }

    /// Parses release metadata from GitHub release JSON, preferring asset URLs
    /// from the release payload over formatted URL templates.
    pub(crate) fn from_json(json: &serde_json::Value) -> Result<Self, UpdateError> {
        let tag = json
            .get("tag_name")
            .and_then(|value| value.as_str())
            .ok_or(UpdateError::NoTag)?;

        let mut release = Self::from_tag(tag);
        if let Some(assets) = json.get("assets").and_then(|a| a.as_array()) {
            for asset in assets {
                let name = asset.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let url = asset.get("browser_download_url").and_then(|u| u.as_str());
                if name == ASSET
                    && let Some(u) = url
                {
                    release.asset_url = u.to_string();
                } else if name == "SHA256SUMS"
                    && let Some(u) = url
                {
                    release.checksums_url = u.to_string();
                }
            }
        }
        Ok(release)
    }
}

/// Fetches the newest published release metadata.
pub(crate) fn fetch_latest_release() -> Result<LatestRelease, UpdateError> {
    let url = format!("{GITHUB_API}/repos/{OWNER}/{REPO}/releases/latest");
    let body = curl_get(&url)?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|error| UpdateError::Json(error.to_string()))?;
    LatestRelease::from_json(&json)
}

/// Runs a single `curl -fsSL <url>` and returns stdout. `-f` fails on HTTP
/// errors (404 etc.), `-sS` keeps errors visible but suppresses the progress
/// meter so the download body or error text is clean to read.
fn curl_get(url: &str) -> Result<String, UpdateError> {
    let output = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "30", "--max-filesize", "10000000"])
        .args(HTTPS_ONLY)
        .arg(url)
        .output()
        .map_err(|error| UpdateError::ToolSpawn {
            tool: "curl",
            source: error,
        })?;
    if !output.status.success() {
        return Err(UpdateError::Network(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Fetches `url` into `dest` over HTTPS only (a redirect off HTTPS is refused
/// too). A failure keeps its typed cause under the `action` that was attempted.
pub(crate) fn download(url: &str, dest: &Path, action: &'static str) -> Result<(), UpdateError> {
    let wrap = |cause: UpdateError| UpdateError::Download {
        action,
        path: dest.to_path_buf(),
        source: Box::new(cause),
    };
    let output = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "300", "--max-filesize", "150000000"])
        .args(HTTPS_ONLY)
        .arg(url)
        .arg("-o")
        .arg(dest)
        .output()
        .map_err(|error| {
            wrap(UpdateError::ToolSpawn {
                tool: "curl",
                source: error,
            })
        })?;
    if !output.status.success() {
        return Err(wrap(UpdateError::Network(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )));
    }
    Ok(())
}

/// Whether the one published asset can run on `os`/`arch`.
pub(crate) fn platform_supported(os: &str, arch: &str) -> bool {
    os == ASSET_OS && arch == ASSET_ARCH
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

/// Compares two dot-separated numeric versions; `v1.0.3` == `1.0.3` and a
/// missing trailing component counts as 0, so `1.0` == `1.0.0`. Returns
/// `Ok(Ordering)` when both parse as sequences of numeric components, and
/// a [`VersionParseError`] naming the offending string otherwise (so a
/// pre-release suffix is incomparable rather than guessed at).
pub(crate) fn compare_versions(
    current: &str,
    latest: &str,
) -> Result<std::cmp::Ordering, VersionParseError> {
    fn components(v: &str) -> Result<Vec<u64>, VersionParseError> {
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
    let mut current = components(current)?;
    let mut latest = components(latest)?;
    let width = current.len().max(latest.len());
    current.resize(width, 0);
    latest.resize(width, 0);
    Ok(current.cmp(&latest))
}

#[cfg(test)]
#[path = "update_fetch_tests.rs"]
mod tests;
