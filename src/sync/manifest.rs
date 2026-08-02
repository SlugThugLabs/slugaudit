use std::collections::BTreeMap;

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
