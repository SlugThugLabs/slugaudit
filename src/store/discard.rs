//! Discarding an unusable project database without deleting one that another
//! process has just repaired.
//!
//! Two sessions on one project can both see the same outdated or corrupt
//! database. The first discards it and creates a fresh one at the current
//! schema; if the second then deleted "its" copy it would destroy that fresh
//! database. [`discard_if_still_unusable`] therefore takes the database write
//! lock and looks again before it removes anything.

use super::connection::discard_corrupt_database;
use super::connection_guards::{BUSY_TIMEOUT, is_contention};
use super::migrations::CURRENT_SCHEMA_VERSION;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use thiserror::Error;

/// What [`discard_if_still_unusable`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum DiscardOutcome {
    /// The database (if any was left) was unusable and has been removed.
    Discarded,
    /// The database is now empty or at the current schema version, so another
    /// process already repaired it; nothing was removed.
    AlreadyCurrent,
}

#[derive(Debug, Error)]
/// Failures while discarding a database under the write lock.
pub enum DiscardError {
    /// The write lock could not be taken or the version could not be read for
    /// a reason other than corruption.
    #[error("could not lock the project database before discarding it: {0}")]
    Lock(#[source] rusqlite::Error),
    /// Another process kept the database locked for the whole busy timeout.
    #[error("the project database stayed locked by another process: {0}")]
    Busy(#[source] rusqlite::Error),
    /// Removing the database file or a sidecar failed.
    #[error("{0}")]
    Remove(#[source] std::io::Error),
}

/// Removes `path` and its sidecars only if, under the database write lock, it
/// is still corrupt or at a foreign schema version. A database that is empty
/// (just created, schema not applied yet) or already current is left alone.
///
/// # Errors
///
/// Returns [`DiscardError`] if the lock cannot be taken within the busy
/// timeout, the version cannot be read for a non-corruption reason, or a file
/// cannot be removed.
pub fn discard_if_still_unusable(path: &Path) -> Result<DiscardOutcome, DiscardError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let mut connection = match Connection::open_with_flags(path, flags) {
        Ok(connection) => connection,
        // The file is already gone: someone else discarded it.
        Err(error) if error.sqlite_error_code() == Some(rusqlite::ErrorCode::CannotOpen) => {
            return remove(path);
        }
        Err(error) => return Err(DiscardError::Lock(error)),
    };
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(DiscardError::Lock)?;
    let usable = match locked_version(&mut connection) {
        Ok(version) => version == 0 || version == CURRENT_SCHEMA_VERSION,
        Err(error) if super::is_rusqlite_corruption(&error) => false,
        Err(error) if is_contention(&error) => return Err(DiscardError::Busy(error)),
        Err(error) => return Err(DiscardError::Lock(error)),
    };
    if usable {
        return Ok(DiscardOutcome::AlreadyCurrent);
    }
    // Unlinking while the lock-holding connection is open is fine on unix and
    // keeps the lock held for the whole removal.
    let outcome = remove(path);
    drop(connection);
    outcome
}

fn locked_version(connection: &mut Connection) -> rusqlite::Result<i64> {
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.query_row("PRAGMA user_version", [], |row| row.get(0))
}

fn remove(path: &Path) -> Result<DiscardOutcome, DiscardError> {
    discard_corrupt_database(path).map_err(DiscardError::Remove)?;
    Ok(DiscardOutcome::Discarded)
}

#[cfg(test)]
#[path = "discard_tests.rs"]
mod tests;
