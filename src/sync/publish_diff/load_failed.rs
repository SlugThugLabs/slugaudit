//! The `LoadFailed` retry rule of a publish diff: files whose grammar could
//! not be loaded last time are re-parsed even when their hash is unchanged.
use crate::sync::manifest::{ChangeStatus, FileChange};
use crate::util::LockPoisoned;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

/// Marks every still-present `LoadFailed` file as `Modified` so the publish
/// re-parses it even though its content hash is unchanged: its grammar could
/// not be fetched or loaded last time, and an unchanged hash would otherwise
/// keep that verdict forever. Languages that suffered a permanent local
/// (non-fetch) load failure in this process are skipped so they do not trigger
/// infinite re-parse and revision churn on every tool call. Only those rows are
/// affected; the manifest hash is computed from the current hashes and does not
/// move.
pub(super) fn add_load_failed_retries(
    changes: &mut Vec<FileChange>,
    current: &[(&str, &str)],
    load_failed: &HashMap<String, Option<String>>,
) -> Result<(), LockPoisoned> {
    if load_failed.is_empty() {
        return Ok(());
    }
    let already: HashSet<&str> = changes
        .iter()
        .map(|change| change.relative_path.as_str())
        .collect();
    let mut retries = Vec::new();
    for (path, _) in current {
        let Some(language) = load_failed.get(*path) else {
            continue;
        };
        if already.contains(*path) {
            continue;
        }
        if let Some(lang) = language.as_deref()
            && crate::parse::fetch_gate::is_locally_failed(lang)?
        {
            continue;
        }
        retries.push(FileChange {
            relative_path: (*path).to_owned(),
            status: ChangeStatus::Modified,
        });
    }
    if !retries.is_empty() {
        changes.extend(retries);
        changes.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    }
    Ok(())
}

pub(super) fn load_failed_paths(
    connection: &Connection,
) -> Result<HashMap<String, Option<String>>, rusqlite::Error> {
    let mut statement = connection
        .prepare("SELECT path, language FROM files WHERE parser_availability = 'LoadFailed'")?;
    statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect()
}
