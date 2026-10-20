use super::{SyncedProject, internal_error, verify_revision_matches};
use crate::store;
use rmcp::ErrorData;
use rusqlite::{Transaction, TransactionBehavior};

pub(crate) fn with_verified_read<T>(
    synced: &SyncedProject,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, ErrorData>,
) -> Result<T, ErrorData> {
    let mut connection = store::open_read_only(&synced.database_path)
        .map_err(|error| internal_error("opening the project database", error))?;
    let tx = start_read_transaction(&mut connection)?;
    verify_revision_matches(&tx, &synced.revision_id)?;
    let result = f(&tx)?;
    tx.commit()
        .map_err(|error| internal_error("committing the read transaction", error))?;
    Ok(result)
}

/// Starts the read transaction `with_verified_read` runs in. Split out so
/// the start-failure mapping is testable with a connection that already
/// has a transaction open — the one state in which SQLite refuses a second
/// `BEGIN DEFERRED`.
pub(super) fn start_read_transaction(
    connection: &mut rusqlite::Connection,
) -> Result<Transaction<'_>, ErrorData> {
    connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(|error| internal_error("starting a read transaction", error))
}

pub(crate) fn with_verified_write<T>(
    synced: &SyncedProject,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, ErrorData>,
) -> Result<T, ErrorData> {
    let mut connection = store::open_read_write(&synced.database_path)
        .map_err(|error| internal_error("opening the project database for write", error))?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| internal_error("starting a write transaction", error))?;
    verify_revision_matches(&tx, &synced.revision_id)?;
    let result = f(&tx)?;
    tx.commit()
        .map_err(|error| internal_error("committing the write transaction", error))?;
    Ok(result)
}
