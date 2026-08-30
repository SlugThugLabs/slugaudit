//! Tests for the network/version helpers in `update_fetch.rs` — all pure,
//! no network calls.

use super::{LatestRelease, compare_versions};

#[test]
fn from_tag_builds_the_asset_and_checksum_urls() {
    let release = LatestRelease::from_tag("v1.0.3");
    assert_eq!(release.version, "1.0.3");
    assert!(release.asset_url.ends_with(
        "/SlugThugLabs/slugaudit/releases/download/v1.0.3/slugaudit-mcp-x86_64-unknown-linux-gnu"
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
