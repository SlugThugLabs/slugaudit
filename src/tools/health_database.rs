//! The `health` tool's read of the project database: one read-only open,
//! every query it needs, and a single absent answer on any failure.

use super::health::DatabaseState;
use crate::store;
use rusqlite::OptionalExtension;

/// Reads the database state read-only, degrading gracefully when the
/// database doesn't exist yet (project enabled but never synced) or can't
/// be read — a health check must never fail on transient DB state.
///
/// A read failure is logged (unlike the never-synced case, which is an
/// expected, silent absence), so an operator looking at a payload full of
/// empty fields can tell "not synced yet" from "the database is unreadable"
/// (permissions, corruption, symlink, NFS).
pub(super) fn read_database(database_path: &std::path::Path) -> Option<DatabaseState> {
    if !database_path.exists() {
        return None;
    }
    let mut connection = match store::open_read_only(database_path) {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(
                database_id = %crate::util::redacted_path(database_path),
                error = %error,
                "health: failed to open the project database read-only; \
                 reporting empty database fields",
            );
            return None;
        }
    };
    match query_database_state(&mut connection) {
        Ok(state) => Some(state),
        Err(error) => {
            tracing::warn!(
                database_id = %crate::util::redacted_path(database_path),
                error = %error,
                "health: failed to read the project database; \
                 reporting empty database fields",
            );
            None
        }
    }
}

/// Runs every query a health read needs, propagating errors instead of
/// swallowing them. `read_database` turns any one failure into a single
/// absent state, so an unreadable table can't yield a partial answer.
fn query_database_state(connection: &mut rusqlite::Connection) -> rusqlite::Result<DatabaseState> {
    let (revision_id, parser_pack_version) = read_revision_fields(connection)?;
    Ok(DatabaseState {
        revision_id,
        parser_pack_version,
        file_count: count_files(connection)?,
        skipped_file_count: count_skipped_files(connection)?,
    })
}

/// The two revision columns `health` reports, from the single row the
/// schema guarantees (`idx_revisions_single_current`). `None` when nothing
/// has been published yet; a query error propagates to the caller.
pub(super) fn read_revision_fields(
    connection: &mut rusqlite::Connection,
) -> rusqlite::Result<(Option<String>, Option<String>)> {
    Ok(connection
        .query_row(
            "SELECT revision_id, parser_pack_version FROM revisions WHERE is_current = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .unwrap_or((None, None)))
}

/// `count(*)` over every row in `files`, including skipped rows.
pub(super) fn count_files(connection: &mut rusqlite::Connection) -> rusqlite::Result<i64> {
    connection.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
}

/// The subset of `count_files` whose content was never read.
fn count_skipped_files(connection: &mut rusqlite::Connection) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT count(*) FROM files WHERE file_kind = 'skipped'",
        [],
        |row| row.get(0),
    )
}
