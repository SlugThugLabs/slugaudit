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
#[path = "hash_tests.rs"]
mod tests;
