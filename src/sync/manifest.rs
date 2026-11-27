//! Manifest comparison: how a stored revision differs from current disk
//! state ([`compare`]).

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
/// stored, returning only files that changed sorted by `relative_path`.
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
#[path = "manifest_tests.rs"]
mod tests;
