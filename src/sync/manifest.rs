//! Manifest comparison and aggregate-hash computation: what changed
//! between a stored revision and current disk state ([`compare`]), and the
//! single deterministic hash over a file set
//! ([`compute_manifest_hash`]) that makes a no-op publish detectable
//! without diffing every file.

use rusqlite::Connection;
use std::collections::BTreeMap;

use super::hash;
use super::revision;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Computes the aggregate manifest hash from a post-update file set:
/// existing DB state, updated with upserts, minus deletions.
///
/// Avoids reading rows that will be replaced or removed: upserted paths are
/// overwritten in-memory, and deleted paths are excluded at the SQL level so
/// we don't pay to read rows we'll immediately discard.
pub(crate) fn compute_manifest_hash(
    connection: &Connection,
    upserts: &[revision::FileRecord],
    deletions: &[String],
) -> Result<String, rusqlite::Error> {
    let current_refs: BTreeMap<String, String> = {
        let exclude_count = upserts.len() + deletions.len();
        if exclude_count == 0 {
            let rows: Vec<(String, String)> = connection
                .prepare("SELECT path, content_hash FROM files")?
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows.into_iter().collect()
        } else {
            let placeholders = vec!["?"; exclude_count].join(", ");
            let sql =
                format!("SELECT path, content_hash FROM files WHERE path NOT IN ({placeholders})");
            let params: Vec<&dyn rusqlite::ToSql> = upserts
                .iter()
                .map(|r| &r.relative_path as &dyn rusqlite::ToSql)
                .chain(deletions.iter().map(|p| p as &dyn rusqlite::ToSql))
                .collect();
            let rows: Vec<(String, String)> = connection
                .prepare(&sql)?
                .query_map(params.as_slice(), |row| {
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
