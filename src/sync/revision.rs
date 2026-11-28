// slugaudit-line-exception: approved-by=split-judge; reason=atomic project.db revision state transitions, CAS baseline check, deletions, skipped rows, and pointer flip
//! Atomic revision commit transaction on `project.db`: baseline CAS check,
//! deletions, staged row copy, skipped existence rows, `unrepresentable_paths`,
//! dependency edge handoff, and the `is_current` revision pointer flip.

use super::classify::classify;
use super::revision_edge_invalidation::EdgeChangeScope;
use super::revision_edges::{UpsertedEdgeInput, resolve_upserted_edges};
use super::revision_graph_context::load_path_to_id;
#[cfg(test)]
use super::revision_path_state::collect_prior_rust_path_state;
use super::revision_path_state::{PriorPathAttrInvalidation, StagedFileEdgeMeta};
use super::staging::StagingDb;
#[cfg(test)]
use super::test_support::scope_for_test_inputs;
use crate::model::{EvidenceItem, ParserRun, SourceIdentity};
use crate::util::{self, Deadline};
#[cfg(test)]
use rusqlite::TransactionBehavior;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::HashSet;
use thiserror::Error;

/// SQL subquery yielding the paths of the JSON array bound to its single
/// parameter (built by [`paths_json_array`]) so batch updates never hit
/// `SQLITE_MAX_VARIABLE_NUMBER` (32,766).
const PATH_SET_SUBQUERY: &str = "SELECT value FROM json_each(?)";

/// How many deleted paths one edge-nulling statement binds.
const DELETED_PATH_BATCH: usize = 4096;

/// `files.skip_reason` for a file over the per-file import ceiling.
pub(super) const SKIP_REASON_OVERSIZE: &str = "oversize";
/// `files.skip_reason` for a file whose bytes are not valid UTF-8.
pub(super) const SKIP_REASON_NOT_UTF8: &str = "not_utf8";
/// `files.skip_reason` for a file discovery could not read.
pub(super) const SKIP_REASON_UNREADABLE: &str = "unreadable";
/// `files.skip_reason` for a skipped file whose `stat` failed (`byte_len` NULL).
pub(super) const SKIP_REASON_STAT_FAILED: &str = "stat_failed";

pub(super) const UPSERT_FILE_SQL: &str = "INSERT INTO files (\
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
        last_revision_id = excluded.last_revision_id \
     RETURNING id";

pub(super) const INSERT_EVIDENCE_SQL: &str = "INSERT INTO evidence (\
        file_id, key, kind, origin, span_availability, \
        start_byte, end_byte, start_line, start_column, end_line, end_column, payload\
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)";

fn paths_json_array<'a>(
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<String, rusqlite::Error> {
    let paths: Vec<&str> = paths.into_iter().collect();
    serde_json::to_string(&paths).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One file's fully-sampled state entering a revision: identity, content and
/// byte length, language and parse outcome, and its extracted evidence.
pub struct FileRecord {
    /// Project-relative path with `/` separators.
    pub relative_path: String,
    /// Whether the walk classified the file as binary (`content` is `None`).
    pub is_binary: bool,
    /// Decoded text content, or `None` for a binary file.
    pub content: Option<String>,
    /// Content identity of the sampled bytes.
    pub identity: SourceIdentity,
    /// Raw byte length of the file.
    pub byte_len: u64,
    /// File mtime in unix seconds at sample time, kept for reporting only.
    pub modified_unix_seconds: Option<i64>,
    /// Detected language name, or `None` when no grammar matched the path.
    pub language: Option<String>,
    /// Whether `language` was detected from the file's path.
    pub language_detected: bool,
    /// Parse and extraction status recorded for this revision.
    pub run: ParserRun,
    /// Evidence extracted from the file during this revision.
    pub evidence: Vec<EvidenceItem>,
}

/// Existence row for a discovered-but-skipped file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SkippedRecord {
    pub(super) relative_path: String,
    pub(super) reason: String,
    pub(super) identity: SourceIdentity,
    pub(super) byte_len: Option<u64>,
    pub(super) modified_unix_seconds: Option<i64>,
}

#[derive(Debug, Error)]
/// Revision publish failures, including the CAS `StaleBaseline` verdict.
pub enum RevisionError {
    /// A SQLite operation failed.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// A sampled file's parser run contradicts its language or evidence.
    #[error("{path}: invalid parser run: {reason}")]
    InvalidParserRun {
        /// Project-relative path of the file with the invalid run.
        path: String,
        /// Static description of the invariant that failed.
        reason: &'static str,
    },
    /// Another publisher committed between our sample and this write.
    #[error(
        "concurrent publish won the race (expected current {expected}, found {found}); retry the call"
    )]
    StaleBaseline {
        /// Revision id the publish expected to still be current.
        expected: String,
        /// Revision id actually current when the transaction began.
        found: String,
    },
    /// The system clock cannot timestamp the revision.
    #[error(transparent)]
    Clock(#[from] crate::util::ClockError),
    /// The revision write transaction exceeded the shared wall-clock deadline.
    #[error("revision publish exceeded its wall-clock time budget after {elapsed_ms} ms")]
    TimeBudgetExceeded {
        /// Elapsed wall-clock milliseconds when the budget tripped.
        elapsed_ms: u128,
    },
}

#[cfg(test)]
pub(super) struct RevisionInputs<'a> {
    pub(super) expected_current: Option<&'a str>,
    pub(super) parser_pack_version: &'a str,
    pub(super) upserts: &'a [FileRecord],
    pub(super) deletions: &'a [String],
    pub(super) skipped: &'a [SkippedRecord],
    pub(super) discovered_paths: &'a HashSet<String>,
    pub(super) re_resolve_unresolved: bool,
    pub(super) unrepresentable_paths: &'a HashSet<Vec<u8>>,
    pub(super) deadline: Option<&'a util::Deadline>,
}

/// Fails closed when an optional wall-clock [`util::Deadline`] is spent.
pub(super) fn check_deadline(deadline: Option<&util::Deadline>) -> Result<(), RevisionError> {
    if let Some(elapsed) = deadline.and_then(util::Deadline::exceeded) {
        return Err(RevisionError::TimeBudgetExceeded {
            elapsed_ms: elapsed.as_millis(),
        });
    }
    Ok(())
}

pub(super) fn validate_record(file: &FileRecord) -> Result<(), RevisionError> {
    file.run
        .validate()
        .map_err(|reason| RevisionError::InvalidParserRun {
            path: file.relative_path.clone(),
            reason,
        })
}

/// Streams staged `files` and `evidence` rows one file at a time from
/// `staging` into `tx`, checking `deadline` cooperatively and recording which
/// files changed stored Rust `#[path]` attributes.
pub(super) fn copy_staged_files_to_tx(
    tx: &Transaction<'_>,
    staging: &StagingDb,
    staged_metas: &[StagedFileEdgeMeta],
    revision_db_id: i64,
    changed_path_attr_seqs: &HashSet<i64>,
    deadline: Option<&Deadline>,
) -> Result<Vec<UpsertedEdgeInput>, RevisionError> {
    let mut edge_inputs = Vec::with_capacity(staged_metas.len());
    for meta in staged_metas {
        check_deadline(deadline)?;
        let rust_path_attr_changed = changed_path_attr_seqs.contains(&meta.seq);
        let file_id = staging.copy_seq_to_tx(tx, meta.seq, revision_db_id)?;
        edge_inputs.push(meta.to_edge_input(file_id, rust_path_attr_changed));
    }
    Ok(edge_inputs)
}

/// Publishes one revision atomically from in-memory [`FileRecord`]s (test helper).
#[cfg(test)]
pub(super) fn publish_revision(
    connection: &mut Connection,
    inputs: &RevisionInputs<'_>,
) -> Result<String, RevisionError> {
    let RevisionInputs {
        expected_current,
        parser_pack_version,
        upserts,
        deletions,
        skipped,
        discovered_paths,
        re_resolve_unresolved,
        unrepresentable_paths,
        deadline,
    } = *inputs;
    check_deadline(deadline)?;
    let mut staging = StagingDb::open_in_memory()?;
    let mut staged_metas = Vec::with_capacity(upserts.len());
    {
        let staging_tx = staging.transaction()?;
        for (idx, file) in upserts.iter().enumerate() {
            check_deadline(deadline)?;
            let seq = i64::try_from(idx)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            StagingDb::stage_file(&staging_tx, seq, file)?;
            staged_metas.push(StagedFileEdgeMeta::from_record(seq, file));
        }
        staging_tx.commit()?;
    }

    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    assert_baseline(&tx, expected_current)?;
    let scope = scope_for_test_inputs(&tx, upserts, deletions, re_resolve_unresolved)?;
    let (revision_db_id, revision_id) = insert_revision_row(&tx, parser_pack_version)?;
    let removed_or_skipped = deletions
        .iter()
        .map(String::as_str)
        .chain(skipped.iter().map(|s| s.relative_path.as_str()));
    let prior_rust_paths =
        collect_prior_rust_path_state(&tx, removed_or_skipped, &staged_metas, deadline)?;
    apply_deletions(&tx, deletions, deadline)?;

    let edge_inputs = copy_staged_files_to_tx(
        &tx,
        &staging,
        &staged_metas,
        revision_db_id,
        &prior_rust_paths.changed_staged_seqs,
        deadline,
    )?;
    drop(staging);

    check_deadline(deadline)?;
    upsert_skipped_files(&tx, skipped, revision_db_id, deadline)?;
    replace_unrepresentable_paths_in_tx(&tx, unrepresentable_paths)?;
    finish_edges_and_flip(
        &tx,
        &edge_inputs,
        discovered_paths,
        &scope,
        &prior_rust_paths.invalidation,
        revision_db_id,
        deadline,
    )?;

    tx.commit()?;
    Ok(revision_id)
}

/// Resolves edges against the transaction's complete `files` table and
/// `discovered_paths`, then flips `is_current` to `revision_db_id`.
pub(super) fn finish_edges_and_flip(
    tx: &Transaction<'_>,
    edge_inputs: &[UpsertedEdgeInput],
    discovered_paths: &HashSet<String>,
    scope: &EdgeChangeScope,
    prior_path_attrs: &PriorPathAttrInvalidation,
    revision_db_id: i64,
    deadline: Option<&Deadline>,
) -> Result<(), RevisionError> {
    let path_to_id = load_path_to_id(tx)?;
    let known_ref: HashSet<&str> = discovered_paths
        .iter()
        .map(String::as_str)
        .chain(path_to_id.keys().map(String::as_str))
        .collect();
    resolve_upserted_edges(
        tx,
        edge_inputs,
        &path_to_id,
        &known_ref,
        scope,
        prior_path_attrs,
        deadline,
    )?;
    flip_current_revision(tx, revision_db_id)
}

/// Inserts the revision row and stamps its derived `revision_id`.
pub(super) fn insert_revision_row(
    tx: &Transaction<'_>,
    parser_pack_version: &str,
) -> Result<(i64, String), RevisionError> {
    let persisted_time: Option<i64> =
        tx.query_row("SELECT max(created_at_unix) FROM revisions", [], |row| {
            row.get(0)
        })?;
    let created_at = util::at_least_timestamp(util::now_unix()?, persisted_time);
    tx.execute(
        "INSERT INTO revisions (revision_id, parser_pack_version, created_at_unix, is_current) \
         VALUES ('pending', ?1, ?2, 0)",
        params![parser_pack_version, created_at],
    )?;
    let revision_db_id = tx.last_insert_rowid();
    let revision_id = format!("rev-{revision_db_id}");
    tx.execute(
        "UPDATE revisions SET revision_id = ?1 WHERE id = ?2",
        params![revision_id, revision_db_id],
    )?;
    Ok((revision_db_id, revision_id))
}

/// Deletes removed files; their evidence and outgoing edges cascade via
/// foreign keys while inbound edges survive as `Unresolved`.
pub(super) fn apply_deletions(
    tx: &Transaction<'_>,
    deletions: &[String],
    deadline: Option<&Deadline>,
) -> Result<(), RevisionError> {
    if deletions.is_empty() {
        return Ok(());
    }
    let statement_sql = format!(
        "UPDATE dependency_edges SET to_file_id = NULL, resolution_kind = 'Unresolved', \
         confidence = NULL WHERE to_file_id IN \
         (SELECT id FROM files WHERE path IN ({PATH_SET_SUBQUERY}))"
    );
    {
        let mut statement = tx.prepare_cached(&statement_sql)?;
        for batch in deletions.chunks(DELETED_PATH_BATCH) {
            check_deadline(deadline)?;
            let paths = paths_json_array(batch.iter().map(String::as_str))?;
            statement.execute([paths])?;
        }
    }
    let mut delete_file = tx.prepare_cached("DELETE FROM files WHERE path = ?1")?;
    for path in deletions {
        check_deadline(deadline)?;
        delete_file.execute(params![path])?;
    }
    Ok(())
}

/// Writes every skipped file's existence row inside `tx`, clearing any prior
/// content, evidence, and outgoing edges.
pub(super) fn upsert_skipped_files(
    tx: &Transaction<'_>,
    records: &[SkippedRecord],
    revision_db_id: i64,
    deadline: Option<&Deadline>,
) -> Result<(), RevisionError> {
    for record in records {
        check_deadline(deadline)?;
        upsert_skipped_file(tx, record, revision_db_id)?;
    }
    Ok(())
}

fn upsert_skipped_file(
    tx: &Transaction<'_>,
    record: &SkippedRecord,
    revision_db_id: i64,
) -> Result<(), rusqlite::Error> {
    let mut upsert_stmt = tx.prepare_cached(
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
            last_revision_id = excluded.last_revision_id \
         RETURNING id",
    )?;
    let byte_len = record
        .byte_len
        .map(i64::try_from)
        .transpose()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let file_id: i64 = upsert_stmt.query_row(
        params![
            record.relative_path,
            record.reason,
            record.identity.content_hash,
            record.identity.hash_algorithm,
            byte_len,
            record.modified_unix_seconds,
            revision_db_id,
            classify(&record.relative_path).as_sql_text(),
        ],
        |row| row.get(0),
    )?;
    tx.prepare_cached("DELETE FROM evidence WHERE file_id = ?1")?
        .execute(params![file_id])?;
    tx.prepare_cached("DELETE FROM dependency_edges WHERE from_file_id = ?1")?
        .execute(params![file_id])?;
    Ok(())
}

/// Replaces the persisted set of discovered non-UTF-8 paths inside an active
/// revision transaction after the CAS baseline check has succeeded.
pub(super) fn replace_unrepresentable_paths_in_tx(
    tx: &Transaction<'_>,
    paths: &HashSet<Vec<u8>>,
) -> Result<(), rusqlite::Error> {
    if stored_unrepresentable_paths(tx)? == *paths {
        return Ok(());
    }
    tx.execute("DELETE FROM unrepresentable_paths", [])?;
    let mut insert =
        tx.prepare("INSERT OR IGNORE INTO unrepresentable_paths (path) VALUES (?1)")?;
    for path in paths {
        insert.execute([path])?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn replace_unrepresentable_paths(
    connection: &mut Connection,
    paths: &HashSet<Vec<u8>>,
) -> Result<(), rusqlite::Error> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    replace_unrepresentable_paths_in_tx(&tx, paths)?;
    tx.commit()
}

pub(super) fn stored_unrepresentable_paths(
    connection: &Connection,
) -> Result<HashSet<Vec<u8>>, rusqlite::Error> {
    let mut statement = connection.prepare("SELECT path FROM unrepresentable_paths")?;
    statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))?
        .collect()
}

/// Flips the `is_current` pointer last — clear the old current, set this revision.
pub(super) fn flip_current_revision(
    tx: &Transaction<'_>,
    revision_db_id: i64,
) -> Result<(), RevisionError> {
    tx.execute(
        "UPDATE revisions SET is_current = 0 WHERE is_current = 1",
        [],
    )?;
    tx.execute(
        "UPDATE revisions SET is_current = 1 WHERE id = ?1",
        params![revision_db_id],
    )?;
    Ok(())
}

/// Returns the `revision_id` of the current revision, or `None` if no
/// revision has been published yet.
pub(crate) fn current_revision_id(
    connection: &Connection,
) -> Result<Option<String>, rusqlite::Error> {
    Ok(current_revision_row(connection)?.map(|(id, _)| id))
}

/// Like [`current_revision_id`], returning `(revision_id, parser_pack_version)`.
pub(crate) fn current_revision_row(
    connection: &Connection,
) -> Result<Option<(String, String)>, rusqlite::Error> {
    connection
        .query_row(
            "SELECT revision_id, parser_pack_version FROM revisions WHERE is_current = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
}

pub(super) fn assert_baseline(
    tx: &Transaction<'_>,
    expected_current: Option<&str>,
) -> Result<(), RevisionError> {
    let actual = current_revision_id(tx)?;
    match (expected_current, actual.as_deref()) {
        (None, None) => Ok(()),
        (Some(expected), Some(found)) if expected == found => Ok(()),
        (expected, found) => Err(RevisionError::StaleBaseline {
            expected: baseline_label(expected),
            found: baseline_label(found),
        }),
    }
}

pub(super) fn baseline_label(revision_id: Option<&str>) -> String {
    match revision_id {
        Some(id) => id.to_owned(),
        None => "none".to_owned(),
    }
}

#[cfg(test)]
#[path = "revision_tests.rs"]
mod tests;
