//! The per-file `files` / `evidence` writes for one publish.
//!
//! Split out of `revision.rs` (the same pattern as `revision_edges.rs` and
//! `revision_skipped.rs`) so that module reads as the transaction it is: the
//! input struct, the compare-and-swap baseline check, the write ordering, and
//! the current-pointer swap. How one indexed file's row and evidence are
//! written lives here.

use super::FileRecord;
use super::RevisionError;
use super::classify::classify;
use crate::evidence::{self, EvidenceRow};
use rusqlite::params;

/// Upserts one indexed file's `files` row, then replaces its evidence.
///
/// The row is inserted or updated in place: a path that previously existed as
/// a skipped existence row becomes `indexed` again, and `skip_reason` is
/// cleared so a stale reason cannot outlive the skip it described. A file
/// that was `binary` and is now text (or the reverse) is re-classified the
/// same way — `file_kind` always describes the current sample.
///
/// # Errors
///
/// Returns [`RevisionError::InvalidParserRun`] if the record's parser columns
/// contradict each other, or a database error from any write.
pub(super) fn upsert_file(
    tx: &rusqlite::Transaction<'_>,
    file: &FileRecord,
    revision_db_id: i64,
) -> Result<i64, RevisionError> {
    file.run
        .validate()
        .map_err(|reason| RevisionError::InvalidParserRun {
            path: file.relative_path.clone(),
            reason,
        })?;
    tx.execute(
        "INSERT INTO files (\
            path, file_kind, content, content_hash, hash_algorithm, byte_len, \
            modified_unix_seconds, language, language_detected, parser_availability, \
            parse_outcome, parse_error_reason, extraction_completeness, last_revision_id, \
            file_class\
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
         ON CONFLICT(path) DO UPDATE SET \
            file_kind = excluded.file_kind, \
            file_class = excluded.file_class, \
            skip_reason = NULL, \
            content = excluded.content, \
            content_hash = excluded.content_hash, \
            hash_algorithm = excluded.hash_algorithm, \
            byte_len = excluded.byte_len, \
            modified_unix_seconds = excluded.modified_unix_seconds, \
            language = excluded.language, \
            language_detected = excluded.language_detected, \
            parser_availability = excluded.parser_availability, \
            parse_outcome = excluded.parse_outcome, \
            parse_error_reason = excluded.parse_error_reason, \
            extraction_completeness = excluded.extraction_completeness, \
            last_revision_id = excluded.last_revision_id",
        params![
            file.relative_path,
            if file.is_binary { "binary" } else { "indexed" },
            file.content,
            file.identity.content_hash,
            file.identity.hash_algorithm,
            file.byte_len,
            file.modified_unix_seconds,
            file.language,
            file.language_detected,
            file.run.availability.as_sql_text(),
            file.run.outcome.as_sql_text(),
            file.run.error_reason(),
            file.run.completeness.as_sql_text(),
            revision_db_id,
            classify(&file.relative_path).as_sql_text(),
        ],
    )?;

    let file_id: i64 = tx.query_row(
        "SELECT id FROM files WHERE path = ?1",
        params![file.relative_path],
        |row| row.get(0),
    )?;
    tx.execute("DELETE FROM evidence WHERE file_id = ?1", params![file_id])?;
    for item in &file.evidence {
        insert_evidence(tx, file_id, &evidence::to_row(item))?;
    }
    Ok(file_id)
}

/// Inserts one normalized evidence row for a file. The caller has already
/// deleted the file's previous evidence, so this is a plain insert.
fn insert_evidence(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    row: &EvidenceRow,
) -> Result<(), rusqlite::Error> {
    tx.execute(
        "INSERT INTO evidence (\
            file_id, key, kind, origin, span_availability, \
            start_byte, end_byte, start_line, start_column, end_line, end_column, payload\
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            file_id,
            row.key,
            row.kind,
            row.origin,
            row.span_availability,
            row.start_byte,
            row.end_byte,
            row.start_line,
            row.start_column,
            row.end_line,
            row.end_column,
            row.payload,
        ],
    )?;
    Ok(())
}
