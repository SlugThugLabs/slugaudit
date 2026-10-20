//! The `health` tool's read of the project database: one read-only open,
//! every query it needs, and an explicit outcome that keeps "never synced"
//! separate from "exists but could not be read".

use super::health::DatabaseState;
use crate::store;
use rusqlite::OptionalExtension;

/// The outcome of one read-only database inspection.
///
/// "Never synced" and "unreadable" are different facts: the first explains
/// an absent `database` group, while the second must never be read as it.
/// Both used to arrive as `None`, which let the phase report claim a cause
/// that was false.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum DatabaseRead {
    /// No database file: the project has never been synced.
    NeverSynced,
    /// The database exists but could not be inspected, opened, or queried.
    /// The failure is logged, so an operator can tell permissions,
    /// corruption, symlink, NFS, and schema drift apart; the health answer
    /// only needs to know that the read failed.
    Unreadable,
    /// The database was opened and every field was read.
    Read(DatabaseState),
}

/// Reads the database state read-only, degrading gracefully when the
/// database doesn't exist yet (project enabled but never synced) or can't
/// be read — a health check must never fail on transient DB state.
///
/// A read failure is logged (unlike the never-synced case, which is an
/// expected, silent absence) and returned as [`DatabaseRead::Unreadable`],
/// not folded into [`DatabaseRead::NeverSynced`].
pub(super) fn read_database(database_path: &std::path::Path) -> DatabaseRead {
    match database_path.try_exists() {
        Ok(true) => {}
        Ok(false) => return DatabaseRead::NeverSynced,
        Err(error) => {
            tracing::warn!(
                database_id = %crate::util::redacted_path(database_path),
                error = %error,
                "health: failed to inspect the project database path; \
                 reporting empty database fields",
            );
            return DatabaseRead::Unreadable;
        }
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
            return DatabaseRead::Unreadable;
        }
    };
    match query_database_state(&mut connection) {
        Ok(state) => DatabaseRead::Read(state),
        Err(error) => {
            tracing::warn!(
                database_id = %crate::util::redacted_path(database_path),
                error = %error,
                "health: failed to read the project database; \
                 reporting empty database fields",
            );
            DatabaseRead::Unreadable
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
