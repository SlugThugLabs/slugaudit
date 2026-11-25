//! Network + version helpers for self-update, kept in their own module so
//! `update.rs` stays under the small-file rule cap. It builds URLs, parses the GitHub releases API, and
//! compares versions — all unit-testable without a network call. The one
//! file it touches is a partial download it removes on failure.

use super::UpdateError;
use std::path::Path;

/// Default repo owner/name and the single published Linux `x86_64` asset.
const OWNER: &str = "SlugThugLabs";
const REPO: &str = "slugaudit";
pub(crate) const ASSET: &str = "slugaudit-x86_64-unknown-linux-gnu";
/// The `std::env::consts` OS and architecture [`ASSET`] runs on.
pub(crate) const ASSET_OS: &str = "linux";
pub(crate) const ASSET_ARCH: &str = "x86_64";

/// curl flags that refuse any non-HTTPS URL (including a redirect target) and
/// require TLS 1.2 or newer.
pub(crate) const HTTPS_ONLY: [&str; 5] =
    ["--proto", "=https", "--proto-redir", "=https", "--tlsv1.2"];

/// Wall-clock limit, in seconds, for fetching release metadata.
const METADATA_MAX_TIME_SECS: u64 = 30;
/// Largest release-metadata body curl accepts, in bytes.
const METADATA_MAX_BYTES: u64 = 10_000_000;
/// Wall-clock limit, in seconds, for downloading a release asset.
const DOWNLOAD_MAX_TIME_SECS: u64 = 300;
/// Largest asset download curl accepts, in bytes.
const DOWNLOAD_MAX_BYTES: u64 = 150_000_000;
/// Largest `SHA256SUMS` manifest download/read accepted, in bytes.
pub(crate) const CHECKSUMS_MAX_BYTES: u64 = 64 * 1024;

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
            for (index, asset) in assets.iter().enumerate() {
                let field = |key: &str| asset.get(key).and_then(|v| v.as_str());
                let (Some(name), Some(url)) = (field("name"), field("browser_download_url")) else {
                    return Err(UpdateError::Json(format!(
                        "release asset entry {index} is missing a string `name` or \
                         `browser_download_url`"
                    )));
                };
                if name == ASSET {
                    release.asset_url = url.to_string();
                } else if name == "SHA256SUMS" {
                    release.checksums_url = url.to_string();
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

/// A `curl -fsSL` command bounded to `max_time_secs` and `max_bytes`, HTTPS
/// only, for `url`. `-f` fails on HTTP errors (404 etc.), `-sS` keeps errors
/// visible but suppresses the progress meter, `--globoff` disables URL glob
/// expansion, and `--` terminates option parsing before `url`.
fn curl_command(
    url: &str,
    max_time_secs: u64,
    max_bytes: u64,
    output: Option<&Path>,
) -> std::process::Command {
    let mut command = std::process::Command::new("curl");
    command
        .arg("-fsSL")
        .arg("--globoff")
        .args(["--max-time", &max_time_secs.to_string()])
        .args(["--max-filesize", &max_bytes.to_string()])
        .args(HTTPS_ONLY);
    if let Some(dest) = output {
        command.arg("-o").arg(dest);
    }
    command.arg("--").arg(url);
    command
}

/// Runs `curl -fsSL <url>` and returns stdout capped at `max_bytes`.
fn curl_get_capped(url: &str, max_bytes: u64) -> Result<String, UpdateError> {
    let mut cmd = curl_command(url, METADATA_MAX_TIME_SECS, max_bytes, None);
    let stdout = super::update_proc::run_bounded(&mut cmd, "curl", Some(max_bytes))?;
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

/// Runs a single `curl -fsSL <url>` and returns stdout. `-f` fails on HTTP
/// errors (404 etc.), `-sS` keeps errors visible but suppresses the progress
/// meter so the download body or error text is clean to read.
fn curl_get(url: &str) -> Result<String, UpdateError> {
    curl_get_capped(url, METADATA_MAX_BYTES)
}

/// Fetches `url` into `dest` over HTTPS only (a redirect off HTTPS is refused
/// too), enforcing `max_bytes`. A failure keeps its typed cause under the
/// `action` that was attempted.
pub(crate) fn download_capped(
    url: &str,
    dest: &Path,
    action: &'static str,
    max_bytes: u64,
) -> Result<(), UpdateError> {
    let wrap = |cause: UpdateError| UpdateError::Download {
        action,
        path: dest.to_path_buf(),
        source: Box::new(cause),
    };
    let mut cmd = curl_command(url, DOWNLOAD_MAX_TIME_SECS, max_bytes, Some(dest));
    if let Err(cause) = super::update_proc::run_bounded(&mut cmd, "curl", None) {
        // curl may have left a partial file at `dest`.
        crate::install_place::remove_leftover(dest, "could not remove partial download");
        return Err(wrap(cause));
    }
    let size = std::fs::metadata(dest)
        .map_err(|error| {
            crate::install_place::remove_leftover(dest, "could not remove partial download");
            wrap(UpdateError::Io(error))
        })?
        .len();
    if size > max_bytes {
        crate::install_place::remove_leftover(dest, "could not remove partial download");
        return Err(wrap(UpdateError::Network(format!(
            "downloaded file exceeded {max_bytes} bytes ({size} bytes)"
        ))));
    }
    Ok(())
}

/// Fetches `url` into `dest` over HTTPS only (a redirect off HTTPS is refused
/// too). A failure keeps its typed cause under the `action` that was attempted.
pub(crate) fn download(url: &str, dest: &Path, action: &'static str) -> Result<(), UpdateError> {
    download_capped(url, dest, action, DOWNLOAD_MAX_BYTES)
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
/// missing trailing component counts as 0, so `1.0` == `1.0.0`. Returns
/// `Ok(Ordering)` when both parse as sequences of numeric components, and
/// a [`VersionParseError`] naming the offending string otherwise (so a
/// pre-release suffix is incomparable rather than guessed at).
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
#[path = "update_fetch_tests.rs"]
mod tests;
