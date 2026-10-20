//! Manifest comparison and aggregate-hash computation: what changed
//! between a stored revision and current disk state ([`compare`]), and the
//! single deterministic hash over a file set
//! ([`compute_manifest_hash`]) that makes a no-op publish detectable
//! without diffing every file.

use crate::util::Deadline;
use rusqlite::Connection;
use std::collections::{BTreeMap, HashSet};

use super::hash;
use super::reconcile::ReconcileError;
use super::revision;
use super::revision_skipped::SkippedRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// How a path differs between the walked tree and the stored revision.
pub enum ChangeStatus {
    /// The path exists on disk but not in the stored revision.
    Added,
    /// The path exists in both, with a different content hash.
    Modified,
    /// The path exists in the stored revision but not on disk.
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One path's classification in the manifest diff.
pub struct FileChange {
    /// Project-relative path that changed.
    pub relative_path: String,
    /// How the path differs between disk and the stored revision.
    pub status: ChangeStatus,
}

/// Compares freshly hashed disk state against what a revision currently has
/// stored, returning only files that changed. An unchanged file — same
/// path, same hash — never appears in the result.
pub fn compare<'a>(
    stored: impl IntoIterator<Item = (&'a str, &'a str)>,
    current: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<FileChange> {
    let stored: BTreeMap<&str, &str> = stored.into_iter().collect();
    let current: BTreeMap<&str, &str> = current.into_iter().collect();

    let mut changes = Vec::new();
    for (path, hash) in &current {
        match stored.get(path) {
            None => changes.push(FileChange {
                relative_path: (*path).to_owned(),
                status: ChangeStatus::Added,
            }),
            Some(stored_hash) if stored_hash != hash => changes.push(FileChange {
                relative_path: (*path).to_owned(),
                status: ChangeStatus::Modified,
            }),
            _ => {}
        }
    }
    for path in stored.keys() {
        if !current.contains_key(path) {
            changes.push(FileChange {
                relative_path: (*path).to_owned(),
                status: ChangeStatus::Deleted,
            });
        }
    }
    changes.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    changes
}

/// SQL subquery yielding the paths of the JSON array bound to its single
/// parameter (built by [`paths_json_array`]).
///
/// Every query that filters on a caller-sized set of paths binds ONE JSON
/// array and reads it through `json_each`, never one `?` per path: SQLite
/// rejects a statement with more than `SQLITE_MAX_VARIABLE_NUMBER` bound
/// variables (32766 in the bundled build, probed 2026-10-18: 32766 binds
/// succeed, 32767 fail with "too many SQL variables"), and a project's first
/// publish touches every file, so a per-path bind list made any project with
/// more than 32766 files impossible to index. One parameter keeps the work
/// set-based and inside the caller's transaction.
pub(crate) const PATH_SET_SUBQUERY: &str = "SELECT value FROM json_each(?)";

/// Serializes `paths` into the JSON array [`PATH_SET_SUBQUERY`] binds.
///
/// The whole array becomes ONE SQL parameter, so every caller must pass a
/// bounded batch (4096 paths is the existing unit) rather than a set whose
/// size scales with the project: an unbounded path set allocates and binds
/// proportionally to it. Callers that already hold a caller-sized set split
/// it and execute once per batch, as `reconcile::queries::query_existing_hashes`
/// and `revision::invalidate_stale_findings` do.
pub(crate) fn paths_json_array<'a>(
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<String, rusqlite::Error> {
    let paths: Vec<&str> = paths.into_iter().collect();
    serde_json::to_string(&paths).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

/// How many stored rows [`compute_manifest_hash`] reads per query. The
/// exclusion set (upserts + deletions + skipped) can be as large as the
/// project's file count — `max_total_import_bytes` bounds bytes, not the
/// number of (e.g. empty) files — so neither one JSON parameter for the
/// whole set nor one unpaged read may allocate proportionally to it. Paging
/// bounds each query, and the shared sync deadline is checked between pages.
const STORED_ROW_BATCH: i64 = 4096;

/// Computes the aggregate manifest hash from a post-update file set:
/// existing DB state, updated with upserts and skipped records, minus
/// deletions. The skipped records matter twice: their paths stay in the set
/// (a skipped file is on disk and must not be hashed out of existence), and
/// a refreshed stat identity must replace the stored one or the hash would
/// describe a row the transaction is about to overwrite.
///
/// Stored rows are read in bounded pages and filtered in memory against the
/// exclusion set; the deadline is checked before every page, so a spent sync
/// budget fails here — before any write — rather than after one unbounded
/// allocation. The caller's publish is the only writer, and it re-checks the
/// CAS baseline inside its transaction, so this read never has to span one.
///
/// `ReconcileError` is the error type because this runs only inside the
/// reconcile commit and its budget failure must keep the typed
/// `TimeBudgetExceeded` variant the caller maps to `sync_budget_exhausted`.
pub(crate) fn compute_manifest_hash(
    connection: &Connection,
    upserts: &[revision::FileRecord],
    deletions: &[String],
    skipped: &[SkippedRecord],
    deadline: &Deadline,
) -> Result<String, ReconcileError> {
    hash_with_batch(
        connection,
        upserts,
        deletions,
        skipped,
        deadline,
        STORED_ROW_BATCH,
    )
}

/// [`compute_manifest_hash`] with an injectable stored-row page size, so the
/// multi-page path is testable without a `STORED_ROW_BATCH`-row fixture.
///
/// Chunked `NOT IN` would be wrong: a per-chunk `NOT IN` query returns rows
/// belonging to other chunks, so the chunks would need quadratic
/// re-querying or a temp table. Filtering the already-materialized upsert /
/// deletion / skipped slices in memory is exact, and the read it replaces
/// returned every kept row anyway. Keyset pagination (`id > last_id`) is
/// `O(rows)` in total, unlike `LIMIT/OFFSET`, which rescans from the start
/// for every page; the deadline is checked before each page.
pub(super) fn hash_with_batch(
    connection: &Connection,
    upserts: &[revision::FileRecord],
    deletions: &[String],
    skipped: &[SkippedRecord],
    deadline: &Deadline,
    batch_size: i64,
) -> Result<String, ReconcileError> {
    let excluded: HashSet<&str> = upserts
        .iter()
        .map(|record| record.relative_path.as_str())
        .chain(deletions.iter().map(String::as_str))
        .chain(skipped.iter().map(|record| record.relative_path.as_str()))
        .collect();
    let mut current_refs = BTreeMap::new();
    let mut statement = connection
        .prepare("SELECT id, path, content_hash FROM files WHERE id > ?1 ORDER BY id LIMIT ?2")?;
    let mut after_id = 0_i64;
    loop {
        if let Some(elapsed) = deadline.exceeded() {
            return Err(ReconcileError::TimeBudgetExceeded {
                path: " while computing the manifest hash".to_owned(),
                elapsed_ms: elapsed.as_millis(),
            });
        }
        let page: Vec<(i64, String, String)> = statement
            .query_map(rusqlite::params![after_id, batch_size], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<_, _>>()?;
        let Some(last) = page.last() else {
            break;
        };
        after_id = last.0;
        for (_, path, content_hash) in page {
            if !excluded.contains(path.as_str()) {
                current_refs.insert(path, content_hash);
            }
        }
    }
    let overlays = upserts
        .iter()
        .map(|record| (&record.relative_path, &record.identity.content_hash))
        .chain(
            skipped
                .iter()
                .map(|record| (&record.relative_path, &record.identity.content_hash)),
        );
    for (path, content_hash) in overlays {
        current_refs.insert(path.clone(), content_hash.clone());
    }
    Ok(hash::aggregate_manifest_hash(
        current_refs.iter().map(|(k, v)| (k.as_str(), v.as_str())),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_file_produces_no_change() {
        let changes = compare([("a.rs", "hash1")], [("a.rs", "hash1")]);
        assert!(changes.is_empty());
    }

    #[test]
    fn same_size_content_change_is_modified() {
        let changes = compare([("a.rs", "hash1")], [("a.rs", "hash2")]);
        assert_eq!(
            changes,
            vec![FileChange {
                relative_path: "a.rs".to_owned(),
                status: ChangeStatus::Modified,
            }]
        );
    }

    #[test]
    fn new_file_is_added() {
        let changes = compare([], [("a.rs", "hash1")]);
        assert_eq!(
            changes,
            vec![FileChange {
                relative_path: "a.rs".to_owned(),
                status: ChangeStatus::Added,
            }]
        );
    }

    #[test]
    fn missing_file_is_deleted() {
        let changes = compare([("a.rs", "hash1")], []);
        assert_eq!(
            changes,
            vec![FileChange {
                relative_path: "a.rs".to_owned(),
                status: ChangeStatus::Deleted,
            }]
        );
    }

    #[test]
    fn mixed_changes_are_all_reported_and_sorted() {
        let changes = compare(
            [
                ("deleted.rs", "h1"),
                ("modified.rs", "h2"),
                ("same.rs", "h3"),
            ],
            [("added.rs", "h4"), ("modified.rs", "h5"), ("same.rs", "h3")],
        );
        assert_eq!(
            changes,
            vec![
                FileChange {
                    relative_path: "added.rs".to_owned(),
                    status: ChangeStatus::Added,
                },
                FileChange {
                    relative_path: "deleted.rs".to_owned(),
                    status: ChangeStatus::Deleted,
                },
                FileChange {
                    relative_path: "modified.rs".to_owned(),
                    status: ChangeStatus::Modified,
                },
            ]
        );
    }
}
