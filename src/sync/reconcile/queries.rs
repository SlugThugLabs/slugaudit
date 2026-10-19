//! The SQL the reconcile pipeline reads: stored content hashes for dirty
//! paths (to skip unchanged files) and the current parser pack version
//! (to pin the new revision against).

use crate::sync::manifest::{PATH_SET_SUBQUERY, paths_json_array};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;

/// How many paths one stored-hash lookup binds.
///
/// `PATH_SET_SUBQUERY` binds the whole path set as ONE JSON parameter, which
/// stays far below SQLite's variable limit no matter the batch size; this
/// batch exists to bound the serialized array and the allocation a bulk
/// watcher event burst can materialize before the reconciler sees a deadline.
/// Callers check the budget between batches, so a pathological set fails
/// closed instead of building one unbounded array first.
pub(super) const MAX_PATH_LOOKUP_BATCH: usize = 4096;

/// Queries the stored content hashes for the given paths. Callers pass at
/// most [`MAX_PATH_LOOKUP_BATCH`] paths per call so the serialized JSON
/// parameter stays bounded.
pub(super) fn query_existing_hashes<'a>(
    connection: &Connection,
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<HashMap<String, String>, rusqlite::Error> {
    let paths: Vec<&str> = paths.into_iter().collect();
    if paths.is_empty() {
        return Ok(HashMap::new());
    }
    let wanted = paths_json_array(paths)?;
    let mut stmt = connection.prepare(&format!(
        "SELECT path, content_hash FROM files WHERE path IN ({PATH_SET_SUBQUERY})"
    ))?;
    let rows = stmt.query_map([wanted], |row| {
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
            .expect("40k-path lookup");

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
                .expect("batch lookup");

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
