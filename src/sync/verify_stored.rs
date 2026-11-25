//! The database-side half of "the index equals the source": every stored
//! `content` must still hash to its stored `content_hash`.
//!
//! The publish diff compares *disk* hashes against stored hashes, so an edit
//! made to `files.content` without touching `content_hash` is invisible to
//! it. This pass recomputes BLAKE3 over each stored row's text; a mismatch
//! names the path, and the publish that follows force-rewrites that row from
//! the file on disk. The database cannot answer with text it derived from
//! nothing.
// Owner decision (2026-10-31): the database only matters if it is exactly the same as the code base, so stored text is re-hashed against its stored hash on every call.

use super::publish::PublishError;
use crate::util::Deadline;
use rusqlite::Connection;
use std::collections::HashSet;

/// Paths whose stored content violates the storage-class/nullness invariants or
/// no longer hashes to its stored hash.
///
/// Rows with `file_kind != 'indexed'` and `content IS NULL` (binary and skipped
/// existence rows) are not checked: they deliberately retain no text. Any
/// `indexed` row with `content IS NULL` or a non-`Text` storage class
/// (`Integer`, `Real`, `Blob`), and any non-`indexed` row with `content IS NOT
/// NULL`, is flagged as a mismatch so the publish repairs it from disk.
pub(super) fn content_mismatches(
    connection: &Connection,
    deadline: &Deadline,
) -> Result<HashSet<String>, PublishError> {
    let mut statement = connection.prepare(
        "SELECT path, file_kind, content, content_hash FROM files \
         WHERE content IS NOT NULL OR file_kind = 'indexed' ORDER BY id",
    )?;
    let mut rows = statement.query([])?;
    let mut mismatched = HashSet::new();
    while let Some(row) = rows.next()? {
        if let Some(elapsed) = deadline.exceeded() {
            return Err(PublishError::TimeBudgetExceeded {
                path: " while verifying stored content".to_owned(),
                elapsed_ms: elapsed.as_millis(),
            });
        }
        let file_kind = row.get_ref(1)?.as_str();
        let is_indexed = matches!(file_kind, Ok("indexed"));
        if !is_indexed {
            // Selected only because `content IS NOT NULL`; non-indexed rows
            // must never retain content.
            let path: String = row.get(0)?;
            mismatched.insert(path);
            continue;
        }
        let content: rusqlite::types::ValueRef<'_> = row.get_ref(2)?;
        let rusqlite::types::ValueRef::Text(content_bytes) = content else {
            // `indexed` rows must store valid TEXT, never NULL/Integer/Real/Blob.
            let path: String = row.get(0)?;
            mismatched.insert(path);
            continue;
        };
        let stored_hash: rusqlite::types::ValueRef<'_> = row.get_ref(3)?;
        let stored_hash_str = stored_hash.as_str().map_err(|error| {
            PublishError::Database(rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(error),
            ))
        })?;
        if blake3::hash(content_bytes).to_hex().as_str() != stored_hash_str {
            let path: String = row.get(0)?;
            mismatched.insert(path);
        }
    }
    Ok(mismatched)
}

#[cfg(test)]
#[path = "verify_stored_tests.rs"]
mod tests;
