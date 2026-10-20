use crate::model::SourceIdentity;
use std::path::PathBuf;
use thiserror::Error;

pub use super::hash_io::hash_file;
pub(super) use super::hash_io::stat_identity;

#[derive(Debug, Error)]
/// File-identity read failures (gone, unreadable, over-cap) — always
/// surfaced to the caller, never swallowed.
pub enum HashError {
    /// The file could not be opened or read.
    #[error("failed to read {path} for hashing: {source}")]
    Read {
        /// Path of the file that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file holds more than the caller's byte cap. `observed` is the
    /// bounded read's length — how much was actually read before the cap
    /// stopped it — so the message reports a lower bound on the file's
    /// size, never the size itself.
    #[error(
        "{path} exceeds the {limit}-byte per-file hash ceiling; \
         at least {observed} bytes were read before the cap stopped the read"
    )]
    TooLarge {
        /// Path of the oversize file.
        path: PathBuf,
        /// Bytes read before the cap stopped the read; a lower bound on the
        /// file's size.
        observed: u64,
        /// The caller's per-file byte ceiling.
        limit: u64,
    },
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

        let identity = hash_file("a.rs", &path, 1024).expect("hash file");
        assert_eq!(
            identity.content_hash,
            hash_bytes("a.rs", b"fn a() {}").content_hash
        );
    }

    #[test]
    fn hash_file_under_the_cap_matches_hash_bytes() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("under.rs");
        let bytes = b"fn under() {}";
        std::fs::write(&path, bytes).expect("write fixture file");

        let identity = hash_file("under.rs", &path, bytes.len() as u64 + 1)
            .expect("a file under the cap hashes");

        assert_eq!(identity, hash_bytes("under.rs", bytes));
    }

    #[test]
    fn hash_file_exactly_at_the_cap_succeeds() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("at-cap.rs");
        let bytes = b"exactly!";
        std::fs::write(&path, bytes).expect("write fixture file");

        let identity =
            hash_file("at-cap.rs", &path, bytes.len() as u64).expect("the cap is inclusive");

        assert_eq!(identity, hash_bytes("at-cap.rs", bytes));
    }

    #[test]
    fn hash_file_one_byte_over_the_cap_reports_the_observed_size() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("over.rs");
        std::fs::write(&path, b"ninebytes").expect("write fixture file");

        let error = hash_file("over.rs", &path, 8).expect_err("over-cap file is refused");

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
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn the_too_large_message_reports_a_lower_bound_not_the_file_size() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("large.rs");
        // The bounded read stops at cap + 1, so the message can only state
        // how much was read — never the 64-byte file's real size.
        std::fs::write(&path, vec![b'x'; 64]).expect("write fixture file");

        let error = hash_file("large.rs", &path, 8).expect_err("over-cap file is refused");
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
    }

    #[test]
    fn hash_file_reports_a_missing_path_as_a_read_error() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("missing.rs");

        let error = hash_file("missing.rs", &path, 1024).expect_err("a missing file is unreadable");

        assert!(
            matches!(error, HashError::Read { .. }),
            "a missing file must stay a read error, got {error:?}"
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
