//! The `files` row for a discovered file that was deliberately not read.
//!
//! A `files` row used to mean "this file was sampled successfully", so a
//! file the walk found but could not index — oversize, unreadable — had no
//! row at all and was invisible in every count the tools report (a bare
//! `count(*) FROM files`). It now means "this file is in the project": one
//! row per discovered file, with `file_kind = 'skipped'` and a
//! `skip_reason` naming why there is no content.
//!
//! Split out of `revision.rs` (the same pattern as `revision_edges.rs`) so
//! the indexed-file write path stays focused on content rows.

use super::classify::classify;
use crate::model::SourceIdentity;
use rusqlite::{Transaction, params};

/// `files.skip_reason` for a file over the per-file import ceiling. A code
/// rather than the human message, so `report` can group by it; the schema's
/// column comment names it too.
pub(super) const SKIP_REASON_OVERSIZE: &str = "oversize";
/// `files.skip_reason` for a file discovery could not read (the sniff/stat
/// failed), so its content was never sampled.
pub(super) const SKIP_REASON_UNREADABLE: &str = "unreadable";

/// One discovered-but-not-indexed file's existence row: enough to say the
/// file is in the project, why it was not indexed, and a stat-derived
/// identity to detect the *cheap* changes (size, mtime) without a read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SkippedRecord {
    pub(super) relative_path: String,
    /// A [`SKIP_REASON_OVERSIZE`]/[`SKIP_REASON_UNREADABLE`] code.
    pub(super) reason: String,
    /// `hash::stat_identity` — never a content hash.
    pub(super) identity: SourceIdentity,
    pub(super) byte_len: u64,
    pub(super) modified_unix_seconds: Option<i64>,
}

/// Writes every skipped file's existence row for one publish, in the order
/// given. A batch entry point so the publish transaction's write sequence
/// reads as one call — the rows are still written one at a time.
///
/// # Errors
///
/// Returns a database error from the first row that fails.
pub(super) fn upsert_skipped_files(
    tx: &Transaction<'_>,
    records: &[SkippedRecord],
    revision_db_id: i64,
) -> Result<(), rusqlite::Error> {
    for record in records {
        upsert_skipped_file(tx, record, revision_db_id)?;
    }
    Ok(())
}

/// Writes the existence row for a skipped file, replacing any previous row
/// for the same path. Everything that describes content is explicitly
/// cleared, because none of it was retained: `content` is NULL,
/// `extraction_completeness = 'Unavailable'` (never `ContentOnly`, which
/// claims the text is indexed and readable) and `parse_outcome =
/// 'NotAttempted'`. A file that shrank back under the ceiling is re-indexed
/// by `revision::upsert_file`, which clears `skip_reason` in turn.
///
/// # Errors
///
/// Returns a database error if the upsert, the id lookup, or the evidence /
/// edge cleanup fails.
pub(super) fn upsert_skipped_file(
    tx: &Transaction<'_>,
    record: &SkippedRecord,
    revision_db_id: i64,
) -> Result<(), rusqlite::Error> {
    tx.execute(
        "INSERT INTO files (\
            path, file_kind, skip_reason, content, content_hash, hash_algorithm, byte_len, \
            modified_unix_seconds, language, language_detected, parser_availability, \
            parse_outcome, parse_error_reason, extraction_completeness, last_revision_id, \
            file_class\
         ) VALUES (?1, 'skipped', ?2, NULL, ?3, ?4, ?5, ?6, NULL, 0, 'Unavailable', \
                   'NotAttempted', NULL, 'Unavailable', ?7, ?8) \
         ON CONFLICT(path) DO UPDATE SET \
            file_kind = 'skipped', \
            file_class = excluded.file_class, \
            skip_reason = excluded.skip_reason, \
            content = NULL, \
            content_hash = excluded.content_hash, \
            hash_algorithm = excluded.hash_algorithm, \
            byte_len = excluded.byte_len, \
            modified_unix_seconds = excluded.modified_unix_seconds, \
            language = NULL, \
            language_detected = 0, \
            parser_availability = 'Unavailable', \
            parse_outcome = 'NotAttempted', \
            parse_error_reason = NULL, \
            extraction_completeness = 'Unavailable', \
            last_revision_id = excluded.last_revision_id",
        params![
            record.relative_path,
            record.reason,
            record.identity.content_hash,
            record.identity.hash_algorithm,
            record.byte_len,
            record.modified_unix_seconds,
            revision_db_id,
            classify(&record.relative_path).as_sql_text(),
        ],
    )?;
    let file_id: i64 = tx.query_row(
        "SELECT id FROM files WHERE path = ?1",
        params![record.relative_path],
        |row| row.get(0),
    )?;
    // A file that was indexed and became skipped must not keep the evidence
    // or outgoing edges of content nobody retained: `Unavailable` extraction
    // beside live Symbol rows would be the same overclaim `ContentOnly`
    // exists to remove, and an import edge from content we no longer hold
    // would resolve against a file whose text is gone.
    tx.execute("DELETE FROM evidence WHERE file_id = ?1", params![file_id])?;
    tx.execute(
        "DELETE FROM dependency_edges WHERE from_file_id = ?1",
        params![file_id],
    )?;
    Ok(())
}
