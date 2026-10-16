//! The exclusive database lock `disable` holds while it removes the
//! activation directory, and the honest mapping of its failures.

use super::activation::ActivationError;
use super::database_path::database_path;
use super::root::ProjectRoot;
use crate::store::open_read_write;
use rusqlite::Connection;

/// Acquires an exclusive database connection before removal. The connection
/// remains alive through `remove_dir_all`, so a concurrent SQLite session
/// cannot publish while the activation directory and its WAL sidecars are
/// being removed.
pub(super) fn acquire_database_lock(
    root: &ProjectRoot,
) -> Result<Option<Connection>, ActivationError> {
    let db_path = database_path(root);
    if !db_path.exists() {
        return Ok(None);
    }
    let connection = open_read_write(&db_path).map_err(ActivationError::Database)?;
    // EXCLUSIVE locking is held by this connection until it is dropped. It
    // therefore covers the database lock and removal as one guarded window.
    connection
        .pragma_update(None, "locking_mode", "EXCLUSIVE")
        .map_err(lock_error)?;
    connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            let busy: i64 = row.get(0)?;
            Ok(busy)
        })
        .map_err(lock_error)
        .and_then(|busy| {
            if busy == 0 {
                Ok(Some(connection))
            } else {
                Err(ActivationError::CheckpointIncomplete)
            }
        })
}

/// Only SQLite's busy/locked codes mean another connection is in the way;
/// any other failure is reported as itself rather than as contention.
fn lock_error(error: rusqlite::Error) -> ActivationError {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            ActivationError::DatabaseBusy(error)
        }
        _ => ActivationError::DatabaseLock(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlite_error(code: i32) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None)
    }

    #[test]
    fn only_busy_and_locked_sqlite_codes_are_reported_as_another_connection() {
        let busy = lock_error(sqlite_error(rusqlite::ffi::SQLITE_BUSY));
        let locked = lock_error(sqlite_error(rusqlite::ffi::SQLITE_LOCKED));
        let io = lock_error(sqlite_error(rusqlite::ffi::SQLITE_IOERR));

        assert!(matches!(busy, ActivationError::DatabaseBusy(_)), "{busy:?}");
        assert!(
            matches!(locked, ActivationError::DatabaseBusy(_)),
            "{locked:?}"
        );
        assert!(matches!(io, ActivationError::DatabaseLock(_)), "{io:?}");
        assert!(!io.to_string().contains("another connection"), "{io}");
    }
}
