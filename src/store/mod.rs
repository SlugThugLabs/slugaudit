//! SQLite schema and connection management. Connections are raw `rusqlite`
//! handles; `sync` and `tools` own file discovery and query execution.

mod connection;
mod migrations;
mod result_store;
mod sql_functions;

#[cfg(test)]
pub(crate) use crate::util::test_capture;

/// True when a `rusqlite` error reports a corrupt or non-database file.
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

pub use connection::{
    StoreError, discard_corrupt_database, open_existing, open_existing_read_write, open_findings,
    open_findings_existing, open_read_write, remove_database_files,
};
pub use migrations::MigrationError;
pub use result_store::{
    Cell, ClassStats, Clock, Cursor, Materialize, NonFinite, PageRead, ResultClass, ResultHandle,
    ResultId, ResultMeta, ResultSpec, ResultStore, ResultStoreError, ResultStoreLimits,
    ResultWriter, StoreStats, StoredRow, SweepOutcome, Take,
};

#[cfg(test)]
pub(crate) use migrations::CURRENT_SCHEMA_VERSION;
