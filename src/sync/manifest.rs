//! Manifest comparison: how a stored revision differs from current disk
//! state ([`compare`]), plus the JSON path-set parameter shared by the
//! set-based queries in sync.

use std::collections::BTreeMap;

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
/// publish touches every file, so a per-path bind list could not index a
/// project with more than 32766 files. One parameter keeps the work
/// set-based and inside the caller's transaction.
pub(crate) const PATH_SET_SUBQUERY: &str = "SELECT value FROM json_each(?)";

/// Serializes `paths` into the JSON array [`PATH_SET_SUBQUERY`] binds.
///
/// The whole array becomes ONE SQL parameter, so every caller must pass a
/// bounded batch (4096 paths is the existing unit) rather than a set whose
/// size scales with the project: an unbounded path set allocates and binds
/// proportionally to it.
pub(crate) fn paths_json_array<'a>(
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<String, rusqlite::Error> {
    let paths: Vec<&str> = paths.into_iter().collect();
    serde_json::to_string(&paths).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
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
