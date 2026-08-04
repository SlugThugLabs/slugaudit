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
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(|error| internal_error("starting a read transaction", error))?;
    verify_revision_matches(&tx, &synced.revision_id)?;
    let result = f(&tx)?;
    tx.commit()
        .map_err(|error| internal_error("committing the read transaction", error))?;
    Ok(result)
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
