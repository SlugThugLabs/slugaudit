//! The exclusive database lock `disable` holds while it removes the
//! activation directory, and the honest mapping of its failures.

use super::activation::ActivationError;
use super::database_path::database_path;
use super::root::ProjectRoot;
use crate::store::{StoreError, open_existing_read_write};
use rusqlite::Connection;

/// Acquires an exclusive database connection before removal. The connection
/// remains alive through `remove_dir_all`, so a concurrent SQLite session
/// cannot publish while the activation directory and its WAL sidecars are
/// being removed.
pub(super) fn acquire_database_lock(
    root: &ProjectRoot,
) -> Result<Option<Connection>, ActivationError> {
    let db_path = database_path(root);
    // `symlink_metadata`, not `try_exists`: a symlinked `project.db` must not
    // be followed. There is no database of ours to lock, and `disable`
    // unlinks the link itself.
    match db_path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => return Ok(None),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ActivationError::Io(error)),
    }
    // Never creates the file: if it vanishes after the check above, there is
    // nothing left to lock.
    let connection = match open_existing_read_write(&db_path) {
        Ok(connection) => connection,
        Err(StoreError::Missing) => return Ok(None),
        Err(error) => return Err(ActivationError::Database(error)),
    };
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
/// any other failure (an I/O error from the checkpoint, a corrupt page) is
/// reported as itself. It must never read as contention, because the retry
/// advice that accompanies contention is wrong for it.
fn lock_error(error: rusqlite::Error) -> ActivationError {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            ActivationError::DatabaseBusy(error)
        }
        _ => ActivationError::DatabaseOperation(error),
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
        assert!(
            matches!(io, ActivationError::DatabaseOperation(_)),
            "an I/O failure is not contention: {io:?}"
        );
        assert!(!io.to_string().contains("another connection"), "{io}");
        assert!(
            !io.to_string().contains("lock"),
            "an I/O failure must not read as a lock failure: {io}"
        );
    }

    /// The other half of the mapping: the database itself cannot be opened,
    /// so the failure is reported as a database operation (with the store's
    /// own reason), never as contention and never as a lock failure.
    #[cfg(unix)]
    #[test]
    fn an_unopenable_database_is_reported_as_a_database_error_not_contention() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temp dir");
        let root = ProjectRoot::resolve(directory.path()).expect("resolve root");
        super::super::activation::enable(&root).expect("enable");
        let db_path = database_path(&root);
        std::fs::write(&db_path, b"x").expect("write database file");
        std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o644))
            .expect("widen database permissions");

        let result = acquire_database_lock(&root);

        assert!(
            matches!(
                result,
                Err(ActivationError::Database(
                    crate::store::StoreError::Permissions(_)
                ))
            ),
            "an unopenable database is a database error, got {result:?}"
        );
    }

    #[test]
    fn a_reader_holding_a_wal_snapshot_makes_the_disable_lock_report_an_incomplete_checkpoint() {
        let directory = tempfile::tempdir().expect("temp dir");
        let root = ProjectRoot::resolve(directory.path()).expect("resolve root");
        super::super::activation::enable(&root).expect("enable");
        let db_path = database_path(&root);
        let writer = crate::store::open_read_write(&db_path).expect("open writer");
        writer
            .execute_batch("CREATE TABLE wal_probe(x); INSERT INTO wal_probe VALUES (1);")
            .expect("leave frames in the WAL");
        let reader = crate::store::open_read_write(&db_path).expect("open reader");
        reader.execute_batch("BEGIN").expect("begin read");
        let _: i64 = reader
            .query_row("SELECT count(*) FROM wal_probe", [], |row| row.get(0))
            .expect("pin a read snapshot on the WAL");

        let result = acquire_database_lock(&root);

        assert!(
            matches!(result, Err(ActivationError::CheckpointIncomplete)),
            "a pinned WAL snapshot must refuse the disable lock, got {result:?}"
        );
    }
}
