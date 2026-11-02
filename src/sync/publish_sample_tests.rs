//! Unit tests for the pure helpers of `publish_sample`.

use super::*;
use crate::sync::discovery::{DiscoveredFile, FileKind};
use slugaudit_test_check::{Check, TestResult};
use std::path::PathBuf;

fn file(path: &str) -> DiscoveredFile {
    DiscoveredFile {
        relative_path: path.to_owned(),
        absolute_path: PathBuf::from("/root").join(path),
        kind: FileKind::Indexed,
    }
}

fn identity(path: &str, hash: &str) -> SourceIdentity {
    SourceIdentity::new(path.to_owned(), hash.to_owned())
}

fn change(path: &str, status: ChangeStatus) -> FileChange {
    FileChange {
        relative_path: path.to_owned(),
        status,
    }
}

fn paths(files: &[DiscoveredFile]) -> Vec<&str> {
    files.iter().map(|f| f.relative_path.as_str()).collect()
}

fn sample_of(path: &str, hash: &str) -> sample::Sample {
    sample::Sample {
        relative_path: path.to_owned(),
        is_binary: false,
        content: Some(String::new()),
        identity: identity(path, hash),
        byte_len: 0,
        modified_unix_seconds: None,
    }
}

#[test]
fn parser_version_change_samples_every_hashed_file() {
    let discovered = [file("a.rs"), file("b.rs"), file("unhashed.rs")];
    let identities = [identity("a.rs", "h1"), identity("b.rs", "h2")];
    let picked = files_to_sample(&discovered, &identities, &[], true);
    assert_eq!(paths(&picked), vec!["a.rs", "b.rs"]);
}

#[test]
fn unchanged_parser_samples_only_changed_hashed_files() {
    let discovered = [
        file("added.rs"),
        file("same.rs"),
        file("modified.rs"),
        file("unhashed.rs"),
    ];
    let identities = [
        identity("added.rs", "h1"),
        identity("same.rs", "h2"),
        identity("modified.rs", "h3"),
    ];
    let changes = [
        change("added.rs", ChangeStatus::Added),
        change("modified.rs", ChangeStatus::Modified),
        change("unhashed.rs", ChangeStatus::Modified),
        change("gone.rs", ChangeStatus::Deleted),
    ];
    let picked = files_to_sample(&discovered, &identities, &changes, false);
    assert_eq!(paths(&picked), vec!["added.rs", "modified.rs"]);
}

#[test]
fn deleted_change_is_never_sampled() {
    let discovered = [file("a.rs")];
    let identities = [identity("a.rs", "h1")];
    let changes = [change("a.rs", ChangeStatus::Deleted)];
    assert!(files_to_sample(&discovered, &identities, &changes, false).is_empty());
}

#[test]
fn matching_sample_hashes_pass() {
    let samples = [sample_of("a.rs", "h1"), sample_of("b.rs", "h2")];
    let identities = [identity("a.rs", "h1"), identity("b.rs", "h2")];
    assert!(check_samples_match_pass1(&samples, &identities).is_ok());
}

#[test]
fn sample_absent_from_pass1_is_not_a_mismatch() {
    let samples = [sample_of("new.rs", "h9")];
    assert!(check_samples_match_pass1(&samples, &[identity("a.rs", "h1")]).is_ok());
}

#[test]
fn mismatched_sample_hash_is_changed_during_sample() -> TestResult {
    let samples = [sample_of("a.rs", "h1"), sample_of("b.rs", "other")];
    let identities = [identity("a.rs", "h1"), identity("b.rs", "h2")];
    match check_samples_match_pass1(&samples, &identities) {
        Err(PublishError::ChangedDuringSample { path }) => assert_eq!(path, "b.rs"),
        other => {
            return Err(format!("expected ChangedDuringSample, got {other:?}"))
                .check("unexpected outcome");
        }
    }
    Ok(())
}
