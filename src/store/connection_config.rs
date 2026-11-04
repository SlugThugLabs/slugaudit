//! Open-time configuration for the store: open flags, pragma configuration
//! and WAL enablement.
//!
//! The database is runtime scratch, deleted and rebuilt on any doubt. The
//! writer requires the WAL journal, so that is the one thing refused here.

use super::connection::StoreError;
use rusqlite::{Connection, OpenFlags};
use std::time::Duration;

pub(super) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const WAL_RETRY_INTERVAL: Duration = Duration::from_millis(20);

pub(super) fn open_flags(read_write: bool) -> OpenFlags {
    let mut flags = OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if read_write {
        flags |= OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
    } else {
        flags |= OpenFlags::SQLITE_OPEN_READ_ONLY;
    }
    flags
}

/// Applies the pragmas every read-write connection requires: the shared busy
/// timeout first (so every later statement that needs a lock waits for it),
/// then foreign keys on and WAL journal mode granted.
pub(super) fn configure(connection: &Connection) -> Result<(), StoreError> {
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::Configure)?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(StoreError::Configure)?;
    enable_wal(connection, BUSY_TIMEOUT)?;
    Ok(())
}

/// Switches the database to WAL, retrying for up to `budget` while another
/// process holds it. Converting a fresh database to WAL needs an exclusive
/// lock and SQLite reports `SQLITE_BUSY` for that statement immediately, even
/// with a busy handler installed, so two processes creating the same project
/// database at once would otherwise fail one of them. Once WAL is set it
/// persists in the file, so the retry only matters for the first open.
pub(super) fn enable_wal(connection: &Connection, budget: Duration) -> Result<(), StoreError> {
    let started = std::time::Instant::now();
    let granted: String = loop {
        match connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0)) {
            Ok(granted) => break granted,
            Err(error) if is_contention(&error) => {
                if started.elapsed() >= budget {
                    tracing::warn!(
                        target: "slugaudit::store",
                        budget_ms = budget.as_millis(),
                        "database stayed locked by another process while enabling WAL"
                    );
                    return Err(StoreError::Busy(error));
                }
                std::thread::sleep(WAL_RETRY_INTERVAL);
            }
            Err(error) => return Err(StoreError::Configure(error)),
        }
    };
    if !granted.eq_ignore_ascii_case("wal") {
        tracing::warn!(
            target: "slugaudit::store",
            granted = %granted,
            "SQLite did not grant WAL journal mode; refusing the connection"
        );
        return Err(StoreError::WalUnavailable { granted });
    }
    Ok(())
}

/// True for SQLite's busy/locked codes: another connection is in the way.
pub(super) fn is_contention(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}
