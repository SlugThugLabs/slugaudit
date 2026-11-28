//! Bounded BLAKE3 content hashing, single-pass binary sniffing, manifest
//! digests, and stat-derived identities for skipped files.

use super::discovery::{BINARY_SNIFF_BYTES, FileKind};
use crate::model::SourceIdentity;
use std::cell::RefCell;
use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Chunk size for bounded hashing: one fixed buffer per worker thread, so
/// hashing a large file never churns heap allocations proportional to file size.
pub(crate) const HASH_CHUNK_BYTES: usize = 64 * 1024;

thread_local! {
    static HASH_BUFFER: RefCell<Vec<u8>> = RefCell::new(vec![0_u8; HASH_CHUNK_BYTES]);
}

/// The `hash_algorithm` value for a stat-derived identity (see
/// [`stat_identity`]). Distinct from `blake3-v1` so nothing mistakes a stat
/// identity for a content hash.
pub(super) const STAT_IDENTITY_ALGORITHM: &str = "stat-identity";

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
    /// bounded read's length — a lower bound on the file's size, never a
    /// claimed exact size.
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
/// Computes the BLAKE3 content identity of `bytes` in the same form
/// `files.content_hash` stores.
pub fn hash_bytes(relative_path: &str, bytes: &[u8]) -> SourceIdentity {
    let digest = blake3::hash(bytes);
    SourceIdentity::new(relative_path.to_owned(), digest.to_string())
}

/// Runs `f` with this thread's reusable 64 KiB chunk buffer, falling back to a
/// temporary buffer only if re-entered by a custom `Read` implementation.
fn with_hash_buffer<R>(f: impl FnOnce(&mut [u8]) -> R) -> R {
    HASH_BUFFER.with(|cell| {
        if let Ok(mut buf) = cell.try_borrow_mut() {
            f(&mut buf)
        } else {
            let mut fallback = vec![0_u8; HASH_CHUNK_BYTES];
            f(&mut fallback)
        }
    })
}

/// Streams bytes from `reader`, sniffing the first [`BINARY_SNIFF_BYTES`] for
/// NUL bytes while computing the BLAKE3 digest, bounded by `max_bytes`.
pub(super) fn sniff_and_hash_reader(
    mut reader: impl Read,
    relative_path: &str,
    absolute_path: &Path,
    max_bytes: u64,
) -> Result<(FileKind, SourceIdentity), HashError> {
    with_hash_buffer(|chunk| {
        let mut hasher = blake3::Hasher::new();
        let mut observed = 0_u64;
        let mut sniffed = 0_usize;
        let mut is_binary = false;
        loop {
            let read = match reader.read(chunk) {
                Ok(read) => read,
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
            let Some(filled) = chunk.get(..read) else {
                return Err(HashError::Read {
                    path: absolute_path.to_path_buf(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "reader reported {read} bytes for a {}-byte buffer",
                            chunk.len()
                        ),
                    ),
                });
            };
            if !is_binary
                && let Some(remaining) = BINARY_SNIFF_BYTES.checked_sub(sniffed)
                && remaining > 0
            {
                let to_sniff = remaining.min(filled.len());
                if let Some(prefix) = filled.get(..to_sniff)
                    && prefix.contains(&0)
                {
                    is_binary = true;
                }
                // KEEP saturating: sniffed is capped at BINARY_SNIFF_BYTES
                sniffed = sniffed.saturating_add(to_sniff);
            }
            // KEEP saturating: `observed` is documented as a lower bound ("at least this many bytes"), which a clamped count still states
            observed = observed.saturating_add(read as u64);
            if observed > max_bytes {
                return Err(HashError::TooLarge {
                    path: absolute_path.to_path_buf(),
                    observed,
                    limit: max_bytes,
                });
            }
            hasher.update(filled);
        }
        let kind = if is_binary {
            FileKind::Binary
        } else {
            FileKind::Indexed
        };
        Ok((
            kind,
            SourceIdentity::new(relative_path.to_owned(), hasher.finalize().to_string()),
        ))
    })
}

/// Streams and hashes bytes from `reader`, bounded by `max_bytes`.
#[cfg(test)]
pub(crate) fn hash_reader(
    reader: impl Read,
    relative_path: &str,
    absolute_path: &Path,
    max_bytes: u64,
) -> Result<SourceIdentity, HashError> {
    sniff_and_hash_reader(reader, relative_path, absolute_path, max_bytes)
        .map(|(_kind, identity)| identity)
}

/// Opens `absolute_path` once as a regular nonblocking file and simultaneously
/// classifies binary vs. text and computes its bounded BLAKE3 content hash.
pub(super) fn sniff_and_hash_file(
    relative_path: &str,
    absolute_path: &Path,
    max_bytes: u64,
) -> Result<(FileKind, SourceIdentity), HashError> {
    let (file, metadata) =
        crate::util::open_regular_file_nonblocking(absolute_path).map_err(|source| {
            HashError::Read {
                path: absolute_path.to_path_buf(),
                source,
            }
        })?;
    if metadata.len() > max_bytes {
        return Err(HashError::TooLarge {
            path: absolute_path.to_path_buf(),
            observed: metadata.len(),
            limit: max_bytes,
        });
    }
    // KEEP saturating: at a cap of u64::MAX no read can exceed it, so reading up to u64::MAX bytes loses nothing
    let reader = file.take(max_bytes.saturating_add(1));
    sniff_and_hash_reader(reader, relative_path, absolute_path, max_bytes)
}

/// Streams and BLAKE3-hashes `absolute_path` up to `max_bytes`.
///
/// # Errors
///
/// Returns [`HashError::Read`] if `absolute_path` cannot be opened as a regular
/// file or read, and [`HashError::TooLarge`] if it exceeds `max_bytes`.
pub fn hash_file(
    relative_path: &str,
    absolute_path: &Path,
    max_bytes: u64,
) -> Result<SourceIdentity, HashError> {
    sniff_and_hash_file(relative_path, absolute_path, max_bytes).map(|(_kind, identity)| identity)
}

/// Stat-derived identity for a skipped existence row (`size` + `mtime`, no
/// file read).
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

#[cfg(test)]
#[path = "hash_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "hash_io_tests.rs"]
mod io_tests;
