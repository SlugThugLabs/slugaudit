//! Tests for the network/version helpers in `update_fetch.rs` — all pure,
//! no network calls.

use super::{LatestRelease, compare_versions};

#[test]
fn from_tag_builds_the_asset_and_checksum_urls() {
    let release = LatestRelease::from_tag("v1.0.3");
    assert_eq!(release.version, "1.0.3");
    assert!(release.asset_url.ends_with(
        "/SlugThugLabs/slugaudit/releases/download/v1.0.3/slugaudit-x86_64-unknown-linux-gnu"
    ));
    assert!(
        release
            .checksums_url
            .ends_with("/SlugThugLabs/slugaudit/releases/download/v1.0.3/SHA256SUMS")
    );
}

#[test]
fn from_tag_trims_a_leading_v_from_the_version() {
    assert_eq!(LatestRelease::from_tag("1.0.3").version, "1.0.3");
    assert_eq!(LatestRelease::from_tag("v1.0.3").version, "1.0.3");
}

#[test]
fn compare_versions_orders_numeric_releases() {
    use std::cmp::Ordering::*;
    assert_eq!(compare_versions("1.0.2", "1.0.3"), Ok(Less));
    assert_eq!(compare_versions("1.0.3", "1.0.3"), Ok(Equal));
    assert_eq!(compare_versions("1.1.0", "1.0.99"), Ok(Greater));
    assert_eq!(compare_versions("v1.0.3", "1.0.3"), Ok(Equal));
    assert_eq!(
        compare_versions("2.0.0", "1.0.3"),
        Ok(Greater),
        "major bump outranks any patch"
    );
}

#[test]
fn compare_versions_rejects_non_numeric_local_versions() {
    assert!(compare_versions("dev", "1.0.3").is_err());
    assert!(compare_versions("1.0.3", "abc").is_err());
}

#[test]
fn from_json_prefers_actual_asset_urls_when_present() {
    let json = serde_json::json!({
        "tag_name": "v1.0.10",
        "assets": [
            {
                "name": "slugaudit-x86_64-unknown-linux-gnu",
                "browser_download_url": "https://custom.download/slugaudit-bin"
            },
            {
                "name": "SHA256SUMS",
                "browser_download_url": "https://custom.download/SHA256SUMS"
            }
        ]
    });
    let release = LatestRelease::from_json(&json).expect("parses");
    assert_eq!(release.version, "1.0.10");
    assert_eq!(release.asset_url, "https://custom.download/slugaudit-bin");
    assert_eq!(release.checksums_url, "https://custom.download/SHA256SUMS");
}
