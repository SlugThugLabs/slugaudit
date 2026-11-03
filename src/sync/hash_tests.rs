use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};

#[test]
fn identical_bytes_hash_identically() -> TestResult {
    let first = hash_bytes("a.rs", b"fn a() {}");
    let second = hash_bytes("a.rs", b"fn a() {}");
    ensure_eq(
        &first.content_hash,
        &second.content_hash,
        "first.content_hash == second.content_hash",
    )?;
    Ok(())
}

#[test]
fn one_changed_byte_changes_the_hash() -> TestResult {
    let first = hash_bytes("a.rs", b"fn a() {}");
    let second = hash_bytes("a.rs", b"fn b() {}");
    ensure_ne(
        &first.content_hash,
        &second.content_hash,
        "first.content_hash != second.content_hash",
    )?;
    Ok(())
}

#[test]
fn hash_algorithm_is_recorded() -> TestResult {
    let identity = hash_bytes("a.rs", b"content");
    ensure_eq(
        &identity.hash_algorithm,
        &"blake3-v1",
        "identity.hash_algorithm == \"blake3-v1\"",
    )?;
    Ok(())
}

#[test]
fn aggregate_hash_is_independent_of_input_order() -> TestResult {
    let forward = aggregate_manifest_hash([("a.rs", "h1"), ("b.rs", "h2")]);
    let backward = aggregate_manifest_hash([("b.rs", "h2"), ("a.rs", "h1")]);
    ensure_eq(&forward, &backward, "forward == backward")?;
    Ok(())
}

#[test]
fn aggregate_hash_changes_when_a_file_hash_changes() -> TestResult {
    let before = aggregate_manifest_hash([("a.rs", "h1")]);
    let after = aggregate_manifest_hash([("a.rs", "h2")]);
    ensure_ne(&before, &after, "before != after")?;
    Ok(())
}

#[test]
fn reads_and_hashes_a_real_file() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("a.rs");
    std::fs::write(&path, b"fn a() {}").check("write fixture file")?;

    let identity = hash_file("a.rs", &path, 1024).check("hash file")?;
    ensure_eq(
        &identity.content_hash,
        &hash_bytes("a.rs", b"fn a() {}").content_hash,
        "identity.content_hash == hash_bytes(\"a.rs\", b\"fn a() {}\").content_hash",
    )?;
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

    ensure_eq(
        &identity,
        &hash_bytes("under.rs", bytes),
        "identity == hash_bytes(\"under.rs\", bytes)",
    )?;
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

    ensure_eq(
        &identity,
        &hash_bytes("at-cap.rs", bytes),
        "identity == hash_bytes(\"at-cap.rs\", bytes)",
    )?;
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
            ensure_eq(&reported, &path, "reported == path")?;
            ensure_eq(
                &observed,
                &9,
                "the observed length is reported, not truncated",
            )?;
            ensure_eq(&limit, &8, "limit == 8")?;
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

    ensure(
        message.contains("exceeds the 8-byte per-file hash ceiling"),
        format!("the message must name the ceiling: {message}"),
    )?;
    ensure(
        message.contains("at least 9 bytes were read"),
        format!("the message must report the bounded read length as a lower bound: {message}"),
    )?;
    ensure(
        !message.contains("is 9 bytes"),
        format!("the bounded read length must not be presented as the file's size: {message}"),
    )?;
    ensure(
        !message.contains("64"),
        format!("a value the read never observed must not appear: {message}"),
    )?;
    Ok(())
}

#[test]
fn hash_file_reports_a_missing_path_as_a_read_error() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("missing.rs");

    let error = hash_file("missing.rs", &path, 1024).check_err("a missing file is unreadable")?;

    ensure(
        matches!(error, HashError::Read { .. }),
        format!("a missing file must stay a read error, got {error:?}"),
    )?;
    Ok(())
}

/// A stat identity must be distinguishable from a content hash and must
/// actually track the two stat fields it stands in for — otherwise a
/// skipped file would look unchanged after any edit that kept its path.
#[test]
fn stat_identity_is_tagged_and_tracks_size_and_mtime() -> TestResult {
    let baseline = stat_identity("a.rs", Some(10), Some(100));
    ensure_eq(
        &baseline.hash_algorithm,
        &STAT_IDENTITY_ALGORITHM,
        "baseline.hash_algorithm == STAT_IDENTITY_ALGORITHM",
    )?;
    ensure_ne(
        &baseline.hash_algorithm,
        &"blake3-v1",
        "a stat identity must never be mistakable for a content hash",
    )?;
    ensure_eq(
        &baseline.content_hash,
        &stat_identity("a.rs", Some(10), Some(100)).content_hash,
        "identical stat state must hash identically",
    )?;
    ensure_ne(
        &baseline.content_hash,
        &stat_identity("a.rs", Some(11), Some(100)).content_hash,
        "baseline.content_hash != stat_identity(\"a.rs\", Some(11), Some(100)).content_hash",
    )?;
    ensure_ne(
        &baseline.content_hash,
        &stat_identity("a.rs", Some(10), Some(101)).content_hash,
        "baseline.content_hash != stat_identity(\"a.rs\", Some(10), Some(101)).content_hash",
    )?;
    ensure_ne(
        &baseline.content_hash,
        &stat_identity("a.rs", Some(10), None).content_hash,
        "a missing mtime must not collide with any recorded one",
    )?;
    ensure_ne(
        &stat_identity("a.rs", Some(0), Some(100)).content_hash,
        &stat_identity("a.rs", None, Some(100)).content_hash,
        "an unknown size must not collide with a recorded empty file",
    )?;
    Ok(())
}
