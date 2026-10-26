// slugaudit-line-exception: approved-by=slugthug; reason=the reconcile SQL (stored state, version, atomic commit) is one cohesive query layer for the pipeline
//! The SQL and commit step the reconcile pipeline uses: stored content
//! hashes for dirty paths (to skip unchanged files), the current parser
//! pack version (to pin the new revision against), and the atomic revision
//! write that closes one reconcile pass.

use super::ReconcileError;
use crate::sync::FileRecord;
use crate::sync::manifest::{self, PATH_SET_SUBQUERY, paths_json_array};
use crate::sync::revision;
use crate::sync::revision_skipped::SkippedRecord;
use crate::util::Deadline;
use rusqlite::{Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};

/// How many paths one stored-hash lookup binds.
///
/// `PATH_SET_SUBQUERY` binds the whole path set as ONE JSON parameter, which
/// stays far below SQLite's variable limit no matter the batch size; this
/// batch exists to bound the serialized array and the allocation a bulk
/// watcher event burst can materialize before the reconciler sees a deadline.
/// Callers check the budget between batches, so a pathological set fails
/// closed instead of building one unbounded array first.
pub(super) const MAX_PATH_LOOKUP_BATCH: usize = 4096;

/// What the files table already holds for a set of dirty paths.
#[derive(Debug, Default)]
pub(super) struct StoredState {
    /// Stored content hash per path.
    pub(super) hashes: HashMap<String, String>,
    /// Paths whose stored parse is `LoadFailed`: the grammar could not be
    /// fetched or loaded, so an unchanged hash must not skip them.
    pub(super) retry: HashSet<String>,
}

impl StoredState {
    fn absorb(&mut self, other: StoredState) {
        self.hashes.extend(other.hashes);
        self.retry.extend(other.retry);
    }
}

/// Queries the stored content hashes (and which rows are `LoadFailed`) for
/// the given paths. Callers pass at most [`MAX_PATH_LOOKUP_BATCH`] paths per
/// call so the serialized JSON parameter stays bounded.
pub(super) fn query_existing_hashes<'a>(
    connection: &Connection,
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<StoredState, rusqlite::Error> {
    let paths: Vec<&str> = paths.into_iter().collect();
    let mut state = StoredState::default();
    if paths.is_empty() {
        return Ok(state);
    }
    let wanted = paths_json_array(paths)?;
    let mut stmt = connection.prepare(&format!(
        "SELECT path, content_hash, parser_availability = 'LoadFailed' FROM files \
         WHERE path IN ({PATH_SET_SUBQUERY})"
    ))?;
    let rows = stmt.query_map([wanted], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, bool>(2)?,
        ))
    })?;
    for row in rows {
        let (path, hash, load_failed) = row?;
        if load_failed {
            state.retry.insert(path.clone());
        }
        state.hashes.insert(path, hash);
    }
    Ok(state)
}

/// Reads the parser pack version of the current revision. When no revision
/// has been published yet, the pack this build parses with
/// ([`crate::parse::PACK_VERSION`]) is the truthful answer: any revision
/// this reconcile creates is parsed by it, and recording a made-up version
/// would make the next full publish see a pack change and re-parse every
/// file.
pub(super) fn query_current_parser_pack_version(
    connection: &Connection,
) -> Result<String, rusqlite::Error> {
    let version: Option<String> = connection
        .query_row(
            "SELECT parser_pack_version FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(version.unwrap_or_else(|| crate::parse::PACK_VERSION.to_owned()))
}

/// The dirty set can be the whole event burst from a bulk filesystem
/// change. Look the stored hashes up in bounded batches, checking the
/// budget before each query, instead of serializing the entire set into
/// one JSON array before the first deadline check can run.
pub(super) fn load_existing_hashes(
    connection: &Connection,
    dirty: &HashSet<String>,
    deadline: Deadline,
) -> Result<StoredState, ReconcileError> {
    let mut existing = StoredState::default();
    let mut batch: Vec<&str> = Vec::with_capacity(MAX_PATH_LOOKUP_BATCH.min(dirty.len()));
    for path in dirty {
        batch.push(path.as_str());
        if batch.len() == MAX_PATH_LOOKUP_BATCH {
            existing.absorb(query_stored_hash_batch(connection, &mut batch, deadline)?);
        }
    }
    if !batch.is_empty() {
        existing.absorb(query_stored_hash_batch(connection, &mut batch, deadline)?);
    }
    Ok(existing)
}

/// Refreshes the stored size and mtime for a dirty path whose content hash
/// still matches, so a touch with unchanged bytes is not re-indexed.
pub(super) fn touch_unchanged(
    connection: &Connection,
    path: &str,
    mtime: Option<i64>,
    byte_len: i64,
) -> Result<(), ReconcileError> {
    connection.execute(
        "UPDATE files SET modified_unix_seconds = ?1, byte_len = ?2 WHERE path = ?3",
        rusqlite::params![mtime, byte_len, path],
    )?;
    Ok(())
}

/// Queries one bounded batch of stored path hashes, failing closed when the
/// shared budget is spent. The deadline is checked before the query, so a
/// bulk watcher event burst cannot serialize and execute one huge lookup
/// before the budget gets a chance to trip.
pub(super) fn query_stored_hash_batch(
    connection: &Connection,
    batch: &mut Vec<&str>,
    deadline: Deadline,
) -> Result<StoredState, ReconcileError> {
    if let Some(elapsed) = deadline.exceeded() {
        return Err(ReconcileError::TimeBudgetExceeded {
            path: " while reading stored path hashes".to_owned(),
            elapsed_ms: elapsed.as_millis(),
        });
    }
    Ok(query_existing_hashes(connection, batch.drain(..))?)
}

/// Commits one reconcile pass's upserts, deletions, and skipped existence
/// rows as a single compare-and-swap revision.
pub(super) fn commit_reconcile_changes(
    connection: &mut Connection,
    expected_revision: Option<&str>,
    discovered_paths: &HashSet<String>,
    upserts: &[FileRecord],
    deletions: &[String],
    skipped_records: &[SkippedRecord],
    deadline: Deadline,
) -> Result<(), ReconcileError> {
    let manifest_hash = manifest::compute_manifest_hash(
        connection,
        upserts,
        deletions,
        skipped_records,
        &deadline,
    )?;
    let parser_pack_version = query_current_parser_pack_version(connection)?;
    // The last cheap point to fail before the write transaction opens: a
    // budget spent while the hash was computed must not be followed by a
    // commit that runs for an unbounded time.
    if let Some(elapsed) = deadline.exceeded() {
        return Err(ReconcileError::TimeBudgetExceeded {
            path: " before committing the reconciled revision".to_owned(),
            elapsed_ms: elapsed.as_millis(),
        });
    }

    revision::publish_revision(
        connection,
        revision::RevisionInputs {
            expected_current: expected_revision,
            manifest_hash: &manifest_hash,
            parser_pack_version: &parser_pack_version,
            upserts,
            deletions,
            skipped: skipped_records,
            discovered_paths,
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_database_with_no_revision_reports_the_pack_this_build_parses_with() {
        let db_dir = tempfile::tempdir().expect("db dir");
        let connection =
            crate::store::open_read_write(&db_dir.path().join("project.db")).expect("open db");

        let version = query_current_parser_pack_version(&connection).expect("query version");

        assert_eq!(version, crate::parse::PACK_VERSION);
    }

    #[test]
    fn stored_hashes_are_found_for_a_path_set_beyond_the_sqlite_variable_limit() {
        let (_project, _db_dir, connection, _revision) = crate::sync::test_support::setup_project();
        let mut paths: HashSet<String> = (0..40_000).map(|i| format!("missing/{i}.rs")).collect();
        paths.insert("a.rs".to_owned());

        let hashes = query_existing_hashes(&connection, paths.iter().map(String::as_str))
            .expect("40k-path lookup")
            .hashes;

        assert_eq!(hashes.len(), 1, "only the stored path matches: {hashes:?}");
        assert!(hashes.contains_key("a.rs"));
    }

    /// Every batch a bulk lookup is split into must still find its own
    /// stored hashes — batching bounds the array without silently dropping
    /// paths that fall in later batches.
    #[test]
    fn stored_hashes_are_found_in_every_lookup_batch() {
        let (_project, _db_dir, connection, _revision) = crate::sync::test_support::setup_project();
        let revision_id: i64 = connection
            .query_row("SELECT id FROM revisions WHERE is_current = 1", [], |row| {
                row.get(0)
            })
            .expect("current revision id");
        let expected: Vec<String> = (0..MAX_PATH_LOOKUP_BATCH + 2)
            .map(|index| format!("synthetic/{index}.rs"))
            .collect();
        for path in &expected {
            connection
                .execute(
                    "INSERT INTO files \
                        (path, content_hash, hash_algorithm, parser_availability, \
                         parse_outcome, extraction_completeness, last_revision_id) \
                     VALUES (?1, ?2, 'blake3-v1', 'Available', 'Succeeded', 'Full', ?3)",
                    rusqlite::params![path, format!("hash:{path}"), revision_id],
                )
                .expect("insert stored hash fixture");
        }

        for batch in expected.chunks(MAX_PATH_LOOKUP_BATCH) {
            let hashes = query_existing_hashes(&connection, batch.iter().map(String::as_str))
                .expect("batch lookup")
                .hashes;

            assert_eq!(
                hashes.len(),
                batch.len(),
                "every path in the batch must find its stored hash"
            );
            for path in batch {
                assert_eq!(
                    hashes.get(path.as_str()),
                    Some(&format!("hash:{path}")),
                    "{path} is missing from its own batch"
                );
            }
        }
    }
}
