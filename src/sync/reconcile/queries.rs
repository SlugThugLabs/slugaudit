//! The SQL the reconcile pipeline reads: stored content hashes for dirty
//! paths (to skip unchanged files) and the current parser pack version
//! (to pin the new revision against).

use rusqlite::{Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};

/// Queries the stored content hashes for the given paths.
pub(super) fn query_existing_hashes(
    connection: &Connection,
    paths: &HashSet<String>,
) -> Result<HashMap<String, String>, rusqlite::Error> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; paths.len()].join(", ");
    let sql = format!("SELECT path, content_hash FROM files WHERE path IN ({placeholders})");
    let params = rusqlite::params_from_iter(paths.iter());
    let mut stmt = connection.prepare(&sql)?;
    let rows = stmt.query_map(params, |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut result = HashMap::new();
    for row in rows {
        let (path, hash) = row?;
        result.insert(path, hash);
    }
    Ok(result)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_database_with_no_revision_reports_the_pack_this_build_parses_with() {
        let db_dir = tempfile::tempdir().expect("db dir");
        let connection =
            crate::store::open_read_write(&db_dir.path().join("project.db")).expect("open db");

        let version = query_current_parser_pack_version(&connection).expect("query version");

        assert_eq!(version, crate::parse::PACK_VERSION);
    }
}
