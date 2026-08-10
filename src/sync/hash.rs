use crate::model::SourceIdentity;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
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
pub fn hash_bytes(relative_path: &str, bytes: &[u8]) -> SourceIdentity {
    let digest = blake3::hash(bytes);
    SourceIdentity::new(relative_path.to_owned(), digest.to_string())
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
}
