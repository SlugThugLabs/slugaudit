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

/// Paths whose stored content no longer hashes to its stored hash.
///
/// Rows without content (binary and skipped existence rows) are not checked:
/// they deliberately retain no text, so there is nothing to compare.
pub(super) fn content_mismatches(
    connection: &Connection,
    deadline: &Deadline,
) -> Result<HashSet<String>, PublishError> {
    let mut statement = connection.prepare(
        "SELECT path, content, content_hash FROM files \
         WHERE content IS NOT NULL ORDER BY id",
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
        let path: String = row.get(0)?;
        let content: String = row.get(1)?;
        let stored_hash: String = row.get(2)?;
        if blake3::hash(content.as_bytes()).to_string() != stored_hash {
            mismatched.insert(path);
        }
    }
    Ok(mismatched)
}

#[cfg(test)]
#[path = "verify_stored_tests.rs"]
mod tests;
