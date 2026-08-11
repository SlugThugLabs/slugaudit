//! SQLite schema, connection management, and (later) typed repositories.
//! The store never reads project files; sync/tools own that.

mod connection;
mod migrations;
mod netfs;

#[cfg(test)]
#[path = "test_capture.rs"]
mod test_capture;

/// True when a `rusqlite` error reports a corrupt or non-database file.
/// `StoreError::is_corruption` and `MigrationError::is_corruption` both
/// delegate here so the two error types share one corruption definition
/// and can never drift apart.
pub(crate) fn is_rusqlite_corruption(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            )
    )
}

pub use connection::{StoreError, discard_corrupt_database, open_read_only, open_read_write};
pub use migrations::MigrationError;
