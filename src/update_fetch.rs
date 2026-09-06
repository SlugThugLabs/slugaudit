//! Network + version helpers for self-update, kept in their own module so
//! `update.rs` stays under the small-file rule cap. Nothing here touches the
//! filesystem; it only builds URLs, parses the GitHub releases API, and
//! compares versions — all unit-testable without a network call.

use super::UpdateError;

/// Default repo owner/name and the single published Linux x86_64 asset.
const OWNER: &str = "SlugThugLabs";
const REPO: &str = "slugaudit";
pub(crate) const ASSET: &str = "slugaudit-x86_64-unknown-linux-gnu";

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
        .args(["-fsSL", url])
        .output()
        .map_err(|_| UpdateError::CurlMissing)?;
    if !output.status.success() {
        return Err(UpdateError::Network(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Compares two dot-separated numeric versions. Only the (major.minor.patch)
/// prefix is considered; `v1.0.3` == `1.0.3`. Returns `Ok(Ordering)` when
/// both parse as sequences of numeric components, `Err` otherwise.
pub(crate) fn compare_versions(current: &str, latest: &str) -> Result<std::cmp::Ordering, ()> {
    fn components(v: &str) -> Result<Vec<u64>, ()> {
        v.trim_start_matches('v')
            .split('.')
            .map(|part| part.parse::<u64>().map_err(|_| ()))
            .collect()
    }
    let current = components(current)?;
    let latest = components(latest)?;
    Ok(current.cmp(&latest))
}

#[cfg(test)]
#[path = "update_fetch_tests.rs"]
mod tests;
