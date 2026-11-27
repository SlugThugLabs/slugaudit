// slugaudit-line-exception: approved-by=split-judge; reason=scratch SQLite StagingDb lifecycle, staged_files/staged_evidence schema, chunk writes, and row copy
//! Scratch SQLite staging database (`StagingDb`) storing Pass 2's parsed
//! [`FileRecord`] and `evidence::EvidenceRow` rows outside `project.db`'s
//! write lock and streaming them into the commit transaction by sequence id.

use super::classify::classify;
use super::revision::{
    FileRecord, INSERT_EVIDENCE_SQL, RevisionError, UPSERT_FILE_SQL, validate_record,
};
use crate::evidence;
use rusqlite::{Connection, OpenFlags, Transaction, params};
use std::path::{Path, PathBuf};

const STAGING_SCHEMA: &str = "\
    PRAGMA journal_mode=OFF; \
    PRAGMA synchronous=OFF; \
    PRAGMA locking_mode=EXCLUSIVE; \
    CREATE TABLE staged_files (\
        seq INTEGER PRIMARY KEY, \
        path TEXT NOT NULL, \
        file_kind TEXT NOT NULL, \
        file_class TEXT NOT NULL, \
        content TEXT, \
        content_hash TEXT NOT NULL, \
        hash_algorithm TEXT NOT NULL, \
        byte_len INTEGER NOT NULL, \
        modified_unix_seconds INTEGER, \
        language TEXT, \
        language_detected INTEGER NOT NULL, \
        parser_availability TEXT NOT NULL, \
        parse_outcome TEXT NOT NULL, \
        parse_error_reason TEXT, \
        extraction_completeness TEXT NOT NULL\
    ); \
    CREATE TABLE staged_evidence (\
        seq INTEGER NOT NULL, \
        ord INTEGER NOT NULL, \
        key TEXT NOT NULL, \
        kind TEXT NOT NULL, \
        origin TEXT NOT NULL, \
        span_availability TEXT NOT NULL, \
        start_byte INTEGER, \
        end_byte INTEGER, \
        start_line INTEGER, \
        start_column INTEGER, \
        end_line INTEGER, \
        end_column INTEGER, \
        payload TEXT NOT NULL, \
        PRIMARY KEY (seq, ord)\
    );";

struct StagingFileCleanup(Option<PathBuf>);

impl Drop for StagingFileCleanup {
    fn drop(&mut self) {
        if let Some(path) = self.0.as_deref() {
            remove_staging_file(path);
        }
    }
}

fn remove_staging_file(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "could not remove scratch staging database"
            );
        }
    }
}

/// Uncontended scratch SQLite database holding Pass 2's chunked [`FileRecord`]
/// and `evidence::EvidenceRow` output before `project.db`'s write lock is
/// acquired. `conn` is declared before `_cleanup` so the file descriptor closes
/// before fallback file removal on `Drop`.
pub(super) struct StagingDb {
    conn: Connection,
    _cleanup: StagingFileCleanup,
}

impl StagingDb {
    pub(super) fn open(root: &Path) -> Result<Self, RevisionError> {
        let activation = root.join(crate::ignore_rules::ACTIVATION_RELATIVE_DIR);
        let dir = if activation.is_dir() {
            activation
        } else {
            std::env::temp_dir()
        };
        let path = dir.join(format!("staging-{}.db", uuid::Uuid::new_v4().hyphenated()));
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        #[cfg(unix)]
        remove_staging_file(&path);
        let staging = Self {
            conn,
            _cleanup: StagingFileCleanup(Some(path)),
        };
        staging.conn.execute_batch(STAGING_SCHEMA)?;
        Ok(staging)
    }

    #[cfg(test)]
    pub(super) fn open_in_memory() -> Result<Self, RevisionError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(STAGING_SCHEMA)?;
        Ok(Self {
            conn,
            _cleanup: StagingFileCleanup(None),
        })
    }

    pub(super) fn transaction(&mut self) -> Result<Transaction<'_>, rusqlite::Error> {
        self.conn.transaction()
    }

    /// Validates and writes one [`FileRecord`] and its `evidence::EvidenceRow`s
    /// into `staging_tx`.
    pub(super) fn stage_file(
        staging_tx: &Transaction<'_>,
        seq: i64,
        file: &FileRecord,
    ) -> Result<(), RevisionError> {
        validate_record(file)?;
        staging_tx
            .prepare_cached(
                "INSERT INTO staged_files (\
                    seq, path, file_kind, file_class, content, content_hash, hash_algorithm, \
                    byte_len, modified_unix_seconds, language, language_detected, \
                    parser_availability, parse_outcome, parse_error_reason, extraction_completeness\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            )?
            .execute(params![
                seq,
                file.relative_path,
                if file.is_binary { "binary" } else { "indexed" },
                classify(&file.relative_path).as_sql_text(),
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
            ])?;
        if file.evidence.is_empty() {
            return Ok(());
        }
        let mut insert_ev = staging_tx.prepare_cached(
            "INSERT INTO staged_evidence (\
                seq, ord, key, kind, origin, span_availability, \
                start_byte, end_byte, start_line, start_column, end_line, end_column, payload\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        )?;
        for (ord, item) in file.evidence.iter().enumerate() {
            let ord_i64 = i64::try_from(ord)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let row = evidence::to_row(item);
            insert_ev.execute(params![
                seq,
                ord_i64,
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
            ])?;
        }
        Ok(())
    }

    /// Copies the staged `files` row and ordered `evidence` rows for `seq` into
    /// `tx`, returning the upserted `files.id`.
    pub(super) fn copy_seq_to_tx(
        &self,
        tx: &Transaction<'_>,
        seq: i64,
        revision_db_id: i64,
    ) -> Result<i64, RevisionError> {
        let file_id = self.upsert_file_row(tx, seq, revision_db_id)?;
        self.replace_evidence_rows(tx, seq, file_id)?;
        Ok(file_id)
    }

    fn upsert_file_row(
        &self,
        tx: &Transaction<'_>,
        seq: i64,
        revision_db_id: i64,
    ) -> Result<i64, RevisionError> {
        let mut select_file = self.conn.prepare_cached(
            "SELECT path, file_kind, file_class, content, content_hash, hash_algorithm, \
                    byte_len, modified_unix_seconds, language, language_detected, \
                    parser_availability, parse_outcome, parse_error_reason, extraction_completeness \
             FROM staged_files WHERE seq = ?1",
        )?;
        let row = select_file.query_row(params![seq], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, String>(13)?,
            ))
        })?;
        let file_id = tx.prepare_cached(UPSERT_FILE_SQL)?.query_row(
            params![
                row.0,
                row.1,
                row.3,
                row.4,
                row.5,
                row.6,
                row.7,
                row.8,
                row.9,
                row.10,
                row.11,
                row.12,
                row.13,
                revision_db_id,
                row.2,
            ],
            |out| out.get(0),
        )?;
        Ok(file_id)
    }

    fn replace_evidence_rows(
        &self,
        tx: &Transaction<'_>,
        seq: i64,
        file_id: i64,
    ) -> Result<(), RevisionError> {
        tx.prepare_cached("DELETE FROM evidence WHERE file_id = ?1")?
            .execute(params![file_id])?;
        let mut select_ev = self.conn.prepare_cached(
            "SELECT key, kind, origin, span_availability, \
                    start_byte, end_byte, start_line, start_column, end_line, end_column, payload \
             FROM staged_evidence WHERE seq = ?1 ORDER BY ord",
        )?;
        let mut insert_ev = tx.prepare_cached(INSERT_EVIDENCE_SQL)?;
        let mut rows = select_ev.query(params![seq])?;
        while let Some(row) = rows.next()? {
            insert_ev.execute(params![
                file_id,
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, String>(10)?,
            ])?;
        }
        Ok(())
    }
}
