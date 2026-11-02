// slugaudit-line-exception: approved-by=slugthug; reason=the revision transaction: baseline check, deletions, upserts, edge resolution and the current-pointer swap
use super::manifest::{PATH_SET_SUBQUERY, paths_json_array};
use super::revision_edges;
use super::revision_files;
use super::revision_skipped::{self, SkippedRecord};
use crate::model::{EvidenceItem, ParserRun, SourceIdentity};
use crate::util;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
/// One file's fully-sampled state entering a revision: identity, content and
/// byte length, language and parse outcome, and its extracted evidence.
pub struct FileRecord {
    /// Project-relative path with `/` separators.
    pub relative_path: String,
    /// Whether the walk classified the file as binary; binary files keep no
    /// `content`.
    pub is_binary: bool,
    /// Decoded text content, or `None` for a binary file.
    pub content: Option<String>,
    /// Content identity of the sampled bytes.
    pub identity: SourceIdentity,
    /// Raw byte length of the file.
    pub byte_len: u64,
    /// File mtime in unix seconds at sample time, kept for reporting. It is
    /// never used to skip a read: freshness is decided by content hash.
    pub modified_unix_seconds: Option<i64>,
    /// Detected language name, or `None` when no grammar matched the path.
    pub language: Option<String>,
    /// Whether `language` was detected from the file's path; false when
    /// nothing matched.
    pub language_detected: bool,
    /// Parse and extraction status recorded for this revision.
    pub run: ParserRun,
    /// Evidence extracted from the file during this revision.
    pub evidence: Vec<EvidenceItem>,
}

#[derive(Debug, Error)]
/// Revision publish failures, including the CAS `StaleBaseline` verdict — the
/// signal to re-sample and retry, never to force the write.
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
    /// Another publisher committed between our filesystem sample and this
    /// write. The caller must re-sample and retry rather than overwrite a
    /// newer revision with older disk state.
    #[error(
        "concurrent publish won the race (expected current {expected}, found {found}); retry the call"
    )]
    StaleBaseline {
        /// Revision id the publish expected to still be current.
        expected: String,
        /// Revision id actually current when the transaction began.
        found: String,
    },
}

/// The inputs one publish transaction needs, grouped by role.
///
/// Grouped rather than passed positionally for a concrete reason:
/// `manifest_hash` and `parser_pack_version` are both `&str`, and `deletions`
/// and `discovered_paths` are both path collections, so a positional call
/// site can silently swap two same-typed arguments — a mistake the compiler
/// cannot catch.
pub(super) struct RevisionInputs<'a> {
    /// The revision this publish expects to still be current (`None` when the
    /// project has never been published). CAS: a mismatch fails with
    /// [`RevisionError::StaleBaseline`] rather than overwriting a newer
    /// revision with older disk state.
    pub(super) expected_current: Option<&'a str>,
    /// Aggregate hash over the post-update file set:
    /// `aggregate_manifest_hash` over the sampled *and* skipped set.
    pub(super) manifest_hash: &'a str,
    /// Parser pack version this revision is pinned against.
    pub(super) parser_pack_version: &'a str,
    /// Added/modified files with their parsed evidence.
    pub(super) upserts: &'a [FileRecord],
    /// Project-relative paths no longer on disk; their rows (and cascading
    /// evidence and edges) are deleted.
    pub(super) deletions: &'a [String],
    /// Existence rows for the discovered-but-not-indexed files: one
    /// `file_kind = 'skipped'` row each, so the file is countable instead of
    /// invisible in every `count(*) FROM files`. Written before edge
    /// resolution so an import targeting one resolves to a real row.
    pub(super) skipped: &'a [SkippedRecord],
    /// Project-relative paths the caller walked on disk. Edge resolution
    /// needs them because a discovered file that sampling skipped (oversize,
    /// unreadable) exists; importing it must resolve, not read as dead code.
    pub(super) discovered_paths: &'a HashSet<String>,
    /// Re-resolve stored `Unresolved` edges against the post-transaction
    /// path set. True when this revision added or removed a path, because
    /// either can change an unchanged importer's resolution.
    pub(super) re_resolve_unresolved: bool,
}

/// Publishes one revision atomically under compare-and-swap: the write
/// transaction only proceeds if the current revision is still the expected
/// one (see [`RevisionInputs::expected_current`]). Discovery/hashing happen
/// outside this lock; CAS is what stops an older sample from committing
/// after a newer one.
///
/// Upserts added/modified files and their evidence, writes the skipped
/// files' existence rows, deletes removed files (cascading evidence and
/// dependency edges via foreign keys), and flips `is_current` only as the
/// transaction's last statements. Nothing here is visible to another
/// connection until `commit` succeeds. The ordering is load-bearing: the
/// files, evidence, and skipped rows must all be in place before edge
/// resolution reads the known-path set, and the pointer swap comes last.
pub(super) fn publish_revision(
    connection: &mut Connection,
    inputs: &RevisionInputs<'_>,
) -> Result<String, RevisionError> {
    let RevisionInputs {
        expected_current,
        manifest_hash,
        parser_pack_version,
        upserts,
        deletions,
        skipped,
        discovered_paths,
        re_resolve_unresolved,
    } = *inputs;
    // The sequence below is the load-bearing ordering stated in the doc
    // comment; each step is a private helper in THIS file so the mutation
    // scope over revision.rs keeps covering the whole publish path.
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    assert_baseline(&tx, expected_current)?;
    let (revision_db_id, revision_id) =
        insert_revision_row(&tx, manifest_hash, parser_pack_version)?;

    apply_deletions(&tx, deletions)?;
    let upserted_ids = write_upserts_and_skipped(&tx, upserts, skipped, revision_db_id)?;
    let known = revision_edges::known_paths(&tx, discovered_paths)?;
    let known_ref: HashSet<&str> = known.iter().map(String::as_str).collect();
    resolve_edges(&tx, upserts, &upserted_ids, &known_ref)?;
    if re_resolve_unresolved {
        revision_edges::re_resolve_unresolved(&tx, &known_ref)?;
    }
    flip_current_revision(&tx, revision_db_id)?;

    tx.commit()?;
    Ok(revision_id)
}

/// Inserts the revision row and stamps its derived `revision_id`. The id is
/// derived from the row's own autoincrement id, which SQLite guarantees
/// unique — safer than deriving it from `manifest_hash`, which can
/// legitimately repeat if source reverts to an earlier state.
fn insert_revision_row(
    tx: &rusqlite::Transaction<'_>,
    manifest_hash: &str,
    parser_pack_version: &str,
) -> Result<(i64, String), RevisionError> {
    let persisted_time: Option<i64> =
        tx.query_row("SELECT max(created_at_unix) FROM revisions", [], |row| {
            row.get(0)
        })?;
    let created_at = util::at_least_timestamp(util::now_unix(), persisted_time);
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
    Ok((revision_db_id, revision_id))
}

/// Deletes removed files; their evidence and outgoing edges cascade via
/// foreign keys. Inbound edges survive as `Unresolved` evidence first: the
/// import is still written in the importer's source, so dropping the raw
/// text would erase a fact.
fn apply_deletions(
    tx: &rusqlite::Transaction<'_>,
    deletions: &[String],
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
        let mut statement = tx.prepare(&statement_sql)?;
        for batch in deletions.chunks(DELETED_PATH_BATCH) {
            let paths = paths_json_array(batch.iter().map(String::as_str))?;
            statement.execute([paths])?;
        }
    }
    for path in deletions {
        tx.execute("DELETE FROM files WHERE path = ?1", params![path])?;
    }
    Ok(())
}

/// How many deleted paths one edge-nulling statement binds.
const DELETED_PATH_BATCH: usize = 4096;

/// Writes upserted files with their evidence, then the skipped files'
/// existence rows — before edge resolution, so an import targeting a skipped
/// file resolves to a real `files.id`. Returns the upserted rows' ids in the
/// same order as `upserts`.
fn write_upserts_and_skipped(
    tx: &rusqlite::Transaction<'_>,
    upserts: &[FileRecord],
    skipped: &[SkippedRecord],
    revision_db_id: i64,
) -> Result<Vec<i64>, RevisionError> {
    let mut upserted_ids = Vec::with_capacity(upserts.len());
    for file in upserts {
        upserted_ids.push(revision_files::upsert_file(tx, file, revision_db_id)?);
    }
    revision_skipped::upsert_skipped_files(tx, skipped, revision_db_id)?;
    Ok(upserted_ids)
}

/// Resolves import edges after every upsert/deletion has landed, against
/// the complete post-transaction path set — so a file added earlier in this
/// same revision is a valid resolution target for one added later, and a
/// discovered-but-skipped file (no `files` row) still counts as existing.
fn resolve_edges(
    tx: &rusqlite::Transaction<'_>,
    upserts: &[FileRecord],
    upserted_ids: &[i64],
    known_paths: &HashSet<&str>,
) -> Result<(), RevisionError> {
    for (file, file_id) in upserts.iter().zip(upserted_ids.iter().copied()) {
        revision_edges::resolve_and_store(tx, file, file_id, known_paths)?;
    }
    Ok(())
}

/// Flips the `is_current` pointer last — two statements: clear the old
/// current, set this revision. Until the transaction commits, no other
/// connection sees any of it.
fn flip_current_revision(
    tx: &rusqlite::Transaction<'_>,
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
/// revision has been published yet. This is the one reader of the
/// `is_current` pointer; `flip_current_revision` is its writer.
pub(crate) fn current_revision_id(
    connection: &Connection,
) -> Result<Option<String>, rusqlite::Error> {
    connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .optional()
}

/// Like [`current_revision_id`], but also returns the current revision's
/// `parser_pack_version`, as `(revision_id, parser_pack_version)`.
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

fn assert_baseline(
    tx: &rusqlite::Transaction<'_>,
    expected_current: Option<&str>,
) -> Result<(), RevisionError> {
    let actual = current_revision_id(tx)?;
    match (expected_current, actual.as_deref()) {
        (None, None) => Ok(()),
        (Some(expected), Some(found)) if expected == found => Ok(()),
        (expected, found) => Err(RevisionError::StaleBaseline {
            expected: expected.unwrap_or("none").to_owned(),
            found: found.unwrap_or("none").to_owned(),
        }),
    }
}

#[cfg(test)]
#[path = "revision_tests.rs"]
mod tests;
