//! Bounded file reads and stat-derived identities for file discovery.
//!
//! [`super::hash`] owns the hash types and in-memory digests; this module
//! holds the streaming read path ([`hash_file`]) and the stat identity for
//! files whose content is deliberately not retained ([`stat_identity`]).
//! Both are re-exported from `hash`, so their existing paths are unchanged.

use super::hash::{HashError, STAT_IDENTITY_ALGORITHM};
use crate::model::SourceIdentity;
use std::io::Read;
use std::path::Path;

/// Chunk size for bounded hashing: one fixed buffer per call, so hashing a
/// multi-gigabyte file never allocates memory proportional to the file.
const HASH_CHUNK_BYTES: usize = 64 * 1024;

/// # Errors
///
/// Returns [`HashError::Read`] if `absolute_path` can't be opened or read,
/// and [`HashError::TooLarge`] if it holds more than `max_bytes` bytes. The
/// read is bounded by `take(max_bytes + 1)`, so a file that grows after the
/// caller's stat cannot allocate an unbounded buffer; no byte past the cap
/// is ever fed to the hasher.
pub fn hash_file(
    relative_path: &str,
    absolute_path: &Path,
    max_bytes: u64,
) -> Result<SourceIdentity, HashError> {
    let file = std::fs::File::open(absolute_path).map_err(|source| HashError::Read {
        path: absolute_path.to_path_buf(),
        source,
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut chunk = [0_u8; HASH_CHUNK_BYTES];
    let mut observed = 0_u64;
    let mut reader = file.take(max_bytes.saturating_add(1));
    loop {
        let read = match reader.read(&mut chunk) {
            Ok(read) => read,
            // `Read` contract: an interrupted read is retried by the caller.
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(source) => {
                return Err(HashError::Read {
                    path: absolute_path.to_path_buf(),
                    source,
                });
            }
        };
        if read == 0 {
            break;
        }
        observed = observed.saturating_add(read as u64);
        if observed > max_bytes {
            return Err(HashError::TooLarge {
                path: absolute_path.to_path_buf(),
                observed,
                limit: max_bytes,
            });
        }
        hasher.update(&chunk[..read]);
    }
    Ok(SourceIdentity::new(
        relative_path.to_owned(),
        hasher.finalize().to_string(),
    ))
}

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
