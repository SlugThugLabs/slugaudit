//! SQLite schema and connection management. Connections are deliberately
//! raw: `tools::context` owns the query contracts over them, so a typed
//! repository layer would add indirection without a second consumer.
//! The store never reads project files; sync/tools own that.

mod connection;
mod connection_guards;
mod migrations;
mod netfs;
mod sql_functions;

#[cfg(test)]
#[path = "test_capture.rs"]
pub(crate) mod test_capture;

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

/// The schema version a fresh database is brought to. Re-exported for tests
/// outside this module so they assert against the constant instead of a
/// literal that goes stale on every bump.
#[cfg(test)]
pub(crate) use migrations::CURRENT_SCHEMA_VERSION;
