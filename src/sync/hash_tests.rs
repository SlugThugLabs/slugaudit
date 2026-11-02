use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult};

#[test]
fn identical_bytes_hash_identically() {
    let first = hash_bytes("a.rs", b"fn a() {}");
    let second = hash_bytes("a.rs", b"fn a() {}");
    assert_eq!(first.content_hash, second.content_hash);
}

#[test]
fn one_changed_byte_changes_the_hash() {
    let first = hash_bytes("a.rs", b"fn a() {}");
    let second = hash_bytes("a.rs", b"fn b() {}");
    assert_ne!(first.content_hash, second.content_hash);
}

#[test]
fn hash_algorithm_is_recorded() {
    let identity = hash_bytes("a.rs", b"content");
    assert_eq!(identity.hash_algorithm, "blake3-v1");
}

#[test]
fn aggregate_hash_is_independent_of_input_order() {
    let forward = aggregate_manifest_hash([("a.rs", "h1"), ("b.rs", "h2")]);
    let backward = aggregate_manifest_hash([("b.rs", "h2"), ("a.rs", "h1")]);
    assert_eq!(forward, backward);
}

#[test]
fn aggregate_hash_changes_when_a_file_hash_changes() {
    let before = aggregate_manifest_hash([("a.rs", "h1")]);
    let after = aggregate_manifest_hash([("a.rs", "h2")]);
    assert_ne!(before, after);
}

#[test]
fn reads_and_hashes_a_real_file() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("a.rs");
    std::fs::write(&path, b"fn a() {}").check("write fixture file")?;

    let identity = hash_file("a.rs", &path, 1024).check("hash file")?;
    assert_eq!(
        identity.content_hash,
        hash_bytes("a.rs", b"fn a() {}").content_hash
    );
    Ok(())
}

#[test]
fn hash_file_under_the_cap_matches_hash_bytes() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("under.rs");
    let bytes = b"fn under() {}";
    std::fs::write(&path, bytes).check("write fixture file")?;

    let identity = hash_file("under.rs", &path, bytes.len() as u64 + 1)
        .check("a file under the cap hashes")?;

    assert_eq!(identity, hash_bytes("under.rs", bytes));
    Ok(())
}

#[test]
fn hash_file_exactly_at_the_cap_succeeds() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("at-cap.rs");
    let bytes = b"exactly!";
    std::fs::write(&path, bytes).check("write fixture file")?;

    let identity =
        hash_file("at-cap.rs", &path, bytes.len() as u64).check("the cap is inclusive")?;

    assert_eq!(identity, hash_bytes("at-cap.rs", bytes));
    Ok(())
}

#[test]
fn hash_file_one_byte_over_the_cap_reports_the_observed_size() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("over.rs");
    std::fs::write(&path, b"ninebytes").check("write fixture file")?;

    let error = hash_file("over.rs", &path, 8).check_err("over-cap file is refused")?;

    match error {
        HashError::TooLarge {
            path: reported,
            observed,
            limit,
        } => {
            assert_eq!(reported, path);
            assert_eq!(
                observed, 9,
                "the observed length is reported, not truncated"
            );
            assert_eq!(limit, 8);
        }
        other @ HashError::Read { .. } => {
            return Err(format!("expected TooLarge, got {other:?}")).check("unexpected outcome");
        }
    }
    Ok(())
}

#[test]
fn the_too_large_message_reports_a_lower_bound_not_the_file_size() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("large.rs");
    // The bounded read stops at cap + 1, so the message can only state
    // how much was read — never the 64-byte file's real size.
    std::fs::write(&path, vec![b'x'; 64]).check("write fixture file")?;

    let error = hash_file("large.rs", &path, 8).check_err("over-cap file is refused")?;
    let message = error.to_string();

    assert!(
        message.contains("exceeds the 8-byte per-file hash ceiling"),
        "the message must name the ceiling: {message}"
    );
    assert!(
        message.contains("at least 9 bytes were read"),
        "the message must report the bounded read length as a lower bound: {message}"
    );
    assert!(
        !message.contains("is 9 bytes"),
        "the bounded read length must not be presented as the file's size: {message}"
    );
    assert!(
        !message.contains("64"),
        "a value the read never observed must not appear: {message}"
    );
    Ok(())
}

#[test]
fn hash_file_reports_a_missing_path_as_a_read_error() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("missing.rs");

    let error = hash_file("missing.rs", &path, 1024).check_err("a missing file is unreadable")?;

    assert!(
        matches!(error, HashError::Read { .. }),
        "a missing file must stay a read error, got {error:?}"
    );
    Ok(())
}

/// A stat identity must be distinguishable from a content hash and must
/// actually track the two stat fields it stands in for — otherwise a
/// skipped file would look unchanged after any edit that kept its path.
#[test]
fn stat_identity_is_tagged_and_tracks_size_and_mtime() {
    let baseline = stat_identity("a.rs", Some(10), Some(100));
    assert_eq!(baseline.hash_algorithm, STAT_IDENTITY_ALGORITHM);
    assert_ne!(
        baseline.hash_algorithm, "blake3-v1",
        "a stat identity must never be mistakable for a content hash"
    );
    assert_eq!(
        baseline.content_hash,
        stat_identity("a.rs", Some(10), Some(100)).content_hash,
        "identical stat state must hash identically"
    );
    assert_ne!(
        baseline.content_hash,
        stat_identity("a.rs", Some(11), Some(100)).content_hash
    );
    assert_ne!(
        baseline.content_hash,
        stat_identity("a.rs", Some(10), Some(101)).content_hash
    );
    assert_ne!(
        baseline.content_hash,
        stat_identity("a.rs", Some(10), None).content_hash,
        "a missing mtime must not collide with any recorded one"
    );
    assert_ne!(
        stat_identity("a.rs", Some(0), Some(100)).content_hash,
        stat_identity("a.rs", None, Some(100)).content_hash,
        "an unknown size must not collide with a recorded empty file"
    );
}
