//! The `LoadFailed` retry rule of a publish diff: files whose grammar could
//! not be loaded last time are re-parsed even when their hash is unchanged.
use crate::sync::manifest::{ChangeStatus, FileChange};
use rusqlite::Connection;
use std::collections::HashSet;

/// Marks every still-present `LoadFailed` file as `Modified` so the publish
/// re-parses it even though its content hash is unchanged: its grammar could
/// not be fetched or loaded last time, and an unchanged hash would otherwise
/// keep that verdict forever. Only those rows are affected; the manifest hash
/// is computed from the current hashes and does not move.
pub(super) fn add_load_failed_retries(
    changes: &mut Vec<FileChange>,
    current: &[(&str, &str)],
    load_failed: &HashSet<String>,
) {
    if load_failed.is_empty() {
        return;
    }
    let already: HashSet<String> = changes
        .iter()
        .map(|change| change.relative_path.clone())
        .collect();
    changes.extend(
        current
            .iter()
            .filter(|(path, _)| load_failed.contains(*path) && !already.contains(*path))
            .map(|(path, _)| FileChange {
                relative_path: (*path).to_owned(),
                status: ChangeStatus::Modified,
            }),
    );
    changes.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
}

pub(super) fn load_failed_paths(
    connection: &Connection,
) -> Result<HashSet<String>, rusqlite::Error> {
    let mut statement =
        connection.prepare("SELECT path FROM files WHERE parser_availability = 'LoadFailed'")?;
    statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect()
}
