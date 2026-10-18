//! Manifest comparison and aggregate-hash computation: what changed
//! between a stored revision and current disk state ([`compare`]), and the
//! single deterministic hash over a file set
//! ([`compute_manifest_hash`]) that makes a no-op publish detectable
//! without diffing every file.

use rusqlite::Connection;
use std::collections::BTreeMap;

use super::hash;
use super::revision;
use super::revision_skipped::SkippedRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// How a path differs between the walked tree and the stored revision.
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One path's classification in the manifest diff.
pub struct FileChange {
    pub relative_path: String,
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
pub(crate) fn paths_json_array<'a>(
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<String, rusqlite::Error> {
    let paths: Vec<&str> = paths.into_iter().collect();
    serde_json::to_string(&paths).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

/// Computes the aggregate manifest hash from a post-update file set:
/// existing DB state, updated with upserts and skipped records, minus
/// deletions. The skipped records matter twice: their paths stay in the set
/// (a skipped file is on disk and must not be hashed out of existence), and
/// a refreshed stat identity must replace the stored one or the hash would
/// describe a row the transaction is about to overwrite.
///
/// Avoids reading rows that will be replaced or removed: upserted paths are
/// overwritten in-memory, and deleted paths are excluded at the SQL level so
/// we don't pay to read rows we'll immediately discard.
pub(crate) fn compute_manifest_hash(
    connection: &Connection,
    upserts: &[revision::FileRecord],
    deletions: &[String],
    skipped: &[SkippedRecord],
) -> Result<String, rusqlite::Error> {
    let current_refs: BTreeMap<String, String> = {
        let exclude_count = upserts.len() + deletions.len() + skipped.len();
        if exclude_count == 0 {
            let rows: Vec<(String, String)> = connection
                .prepare("SELECT path, content_hash FROM files")?
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows.into_iter().collect()
        } else {
            let excluded = paths_json_array(
                upserts
                    .iter()
                    .map(|r| r.relative_path.as_str())
                    .chain(deletions.iter().map(String::as_str))
                    .chain(skipped.iter().map(|r| r.relative_path.as_str())),
            )?;
            let rows: Vec<(String, String)> = connection
                .prepare(&format!(
                    "SELECT path, content_hash FROM files \
                     WHERE path NOT IN ({PATH_SET_SUBQUERY})"
                ))?
                .query_map([excluded], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows.into_iter().collect()
        }
    };

    let mut current_refs = current_refs;
    for record in upserts {
        current_refs.insert(
            record.relative_path.clone(),
            record.identity.content_hash.clone(),
        );
    }
    for record in skipped {
        current_refs.insert(
            record.relative_path.clone(),
            record.identity.content_hash.clone(),
        );
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
