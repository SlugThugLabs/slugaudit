use crate::evidence::{self, EvidenceRow};
use crate::model::{EvidenceItem, SourceIdentity};
use rusqlite::{Connection, params};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecord {
    pub relative_path: String,
    pub is_binary: bool,
    pub content: Option<String>,
    pub identity: SourceIdentity,
    pub byte_len: u64,
    pub language: Option<String>,
    pub language_detected: bool,
    pub parser_availability: &'static str,
    pub parse_outcome: &'static str,
    pub parse_error_reason: Option<String>,
    pub extraction_completeness: &'static str,
    pub evidence: Vec<EvidenceItem>,
}

#[derive(Debug, Error)]
pub enum RevisionError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}

/// Publishes one revision atomically: upserts added/modified files and
/// their evidence, deletes removed files (cascading their evidence and
/// dependency edges via foreign keys), and flips `is_current` only as the
/// transaction's last statements. Nothing here is visible to another
/// connection until `commit` succeeds, so a reader never observes a
/// partially published revision.
pub fn publish_revision(
    connection: &mut Connection,
    manifest_hash: &str,
    parser_pack_version: &str,
    upserts: &[FileRecord],
    deletions: &[String],
) -> Result<String, RevisionError> {
    let created_at = now_unix();
    let tx = connection.transaction()?;

    // `revision_id` is derived from the row's own autoincrement id, which
    // SQLite guarantees unique — safer than deriving it from manifest_hash,
    // which can legitimately repeat if source reverts to an earlier state.
    tx.execute(
        "INSERT INTO revisions (revision_id, manifest_hash, parser_pack_version, created_at_unix, is_current) \
         VALUES ('pending', ?1, ?2, ?3, 0)",
        params![manifest_hash, parser_pack_version, created_at],
    )?;
    let revision_db_id = tx.last_insert_rowid();
    let revision_id = format!("rev-{revision_db_id}");
    tx.execute(
        "UPDATE revisions SET revision_id = ?1 WHERE id = ?2",
        params![revision_id, revision_db_id],
    )?;

    for path in deletions {
        tx.execute("DELETE FROM files WHERE path = ?1", params![path])?;
    }

    for file in upserts {
        tx.execute(
            "INSERT INTO files (\
                path, file_kind, content, content_hash, hash_algorithm, byte_len, \
                language, language_detected, parser_availability, parse_outcome, \
                parse_error_reason, extraction_completeness, last_revision_id\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
             ON CONFLICT(path) DO UPDATE SET \
                file_kind = excluded.file_kind, \
                content = excluded.content, \
                content_hash = excluded.content_hash, \
                hash_algorithm = excluded.hash_algorithm, \
                byte_len = excluded.byte_len, \
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
                file.language,
                file.language_detected,
                file.parser_availability,
                file.parse_outcome,
                file.parse_error_reason,
                file.extraction_completeness,
                revision_db_id,
            ],
        )?;

        let file_id: i64 = tx.query_row(
            "SELECT id FROM files WHERE path = ?1",
            params![file.relative_path],
            |row| row.get(0),
        )?;
        tx.execute("DELETE FROM evidence WHERE file_id = ?1", params![file_id])?;
        for item in &file.evidence {
            insert_evidence(&tx, file_id, &evidence::to_row(item))?;
        }
    }

    let touched_paths: Vec<&str> = upserts
        .iter()
        .map(|file| file.relative_path.as_str())
        .chain(deletions.iter().map(String::as_str))
        .collect();
    invalidate_stale_findings(&tx, &touched_paths)?;

    tx.execute(
        "UPDATE revisions SET is_current = 0 WHERE is_current = 1",
        [],
    )?;
    tx.execute(
        "UPDATE revisions SET is_current = 1 WHERE id = ?1",
        params![revision_db_id],
    )?;

    tx.commit()?;
    Ok(revision_id)
}

/// A finding is stale the moment its file's content hash no longer matches
/// the hash it was recorded against — including when the file is gone
/// entirely. This runs inside the publish transaction so a finding can
/// never be read as current against evidence that has already changed.
fn invalidate_stale_findings(
    tx: &rusqlite::Transaction,
    touched_paths: &[&str],
) -> Result<(), rusqlite::Error> {
    if touched_paths.is_empty() {
        return Ok(());
    }
    let placeholders = vec!["?"; touched_paths.len()].join(", ");
    let sql = format!(
        "UPDATE findings SET status = 'stale' \
         WHERE status = 'current' AND path IN ({placeholders}) \
         AND NOT EXISTS ( \
             SELECT 1 FROM files \
             WHERE files.path = findings.path AND files.content_hash = findings.source_hash \
         )"
    );
    let params: Vec<&dyn rusqlite::ToSql> = touched_paths
        .iter()
        .map(|path| path as &dyn rusqlite::ToSql)
        .collect();
    tx.execute(&sql, params.as_slice())?;
    Ok(())
}

fn insert_evidence(
    tx: &rusqlite::Transaction,
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

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}
