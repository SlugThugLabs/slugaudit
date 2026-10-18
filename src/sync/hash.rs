use crate::model::SourceIdentity;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
/// File-identity read failures (gone, unreadable, over-cap) — always
/// surfaced to the caller, never swallowed.
pub enum HashError {
    #[error("failed to read {path} for hashing: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// # Errors
///
/// Returns an error if `absolute_path` can't be read.
pub fn hash_file(relative_path: &str, absolute_path: &Path) -> Result<SourceIdentity, HashError> {
    let bytes = std::fs::read(absolute_path).map_err(|source| HashError::Read {
        path: absolute_path.to_path_buf(),
        source,
    })?;
    Ok(hash_bytes(relative_path, &bytes))
}

#[must_use]
/// The content hash of `bytes` in the same form `files.content_hash` stores,
/// so an in-memory digest can be compared against a published row.
pub fn hash_bytes(relative_path: &str, bytes: &[u8]) -> SourceIdentity {
    let digest = blake3::hash(bytes);
    SourceIdentity::new(relative_path.to_owned(), digest.to_string())
}

/// The `hash_algorithm` value for a stat-derived identity (see
/// [`stat_identity`]). Distinct from `blake3-v1` on purpose: nothing should
/// be able to mistake a stat identity for a hash of the file's content.
pub(super) const STAT_IDENTITY_ALGORITHM: &str = "stat-identity";

/// A cheap identity for a file whose content is deliberately not retained
/// (an existence row for a skipped file — see
/// [`crate::sync::revision_skipped`]). Hashes the path with the file's size
/// and mtime instead of its bytes, so identity costs one `stat` rather than
/// a full read — the read is exactly what skipping avoids. The price is the
/// stat sweep's documented blind spot: same size and same mtime means the
/// same identity even if the bytes changed.
#[must_use]
pub(super) fn stat_identity(
    relative_path: &str,
    byte_len: Option<u64>,
    modified_unix_seconds: Option<i64>,
) -> SourceIdentity {
    let mut hasher = blake3::Hasher::new();
    hasher.update(relative_path.as_bytes());
    hasher.update(&[0]);
    match byte_len {
        Some(length) => {
            hasher.update(b"l");
            hasher.update(&length.to_le_bytes());
        }
        None => {
            hasher.update(b"u");
        }
    }
    match modified_unix_seconds {
        Some(seconds) => {
            hasher.update(b"m");
            hasher.update(&seconds.to_le_bytes());
        }
        None => {
            hasher.update(b"n");
        }
    }
    SourceIdentity {
        relative_path: relative_path.to_owned(),
        content_hash: hasher.finalize().to_string(),
        hash_algorithm: STAT_IDENTITY_ALGORITHM.to_owned(),
    }
}

/// A single deterministic hash over an entire file set: sorted by path so
/// insertion order never affects the result, changing if any path or
/// content hash in the set changes.
pub fn aggregate_manifest_hash<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> String {
    let mut hasher = blake3::Hasher::new();
    let mut sorted: Vec<(&str, &str)> = entries.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));

    for (path, hash) in sorted {
        hasher.update(path.as_bytes());
        hasher.update(&[0]);
        hasher.update(hash.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn reads_and_hashes_a_real_file() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("a.rs");
        std::fs::write(&path, b"fn a() {}").expect("write fixture file");

        let identity = hash_file("a.rs", &path).expect("hash file");
        assert_eq!(
            identity.content_hash,
            hash_bytes("a.rs", b"fn a() {}").content_hash
        );
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
}
