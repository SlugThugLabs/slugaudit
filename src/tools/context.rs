use crate::{parse, project, store, sync};
use rmcp::ErrorData;
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A project brought fully up to date, ready for a tool to query. Every
/// tool calls this first — there is no separate sync/rebuild entry point
/// for a human or an AI to remember to call.
pub struct SyncedProject {
    pub database_path: PathBuf,
    pub revision_id: String,
    #[allow(dead_code)]
    pub root: PathBuf,
}

/// Resolves the active project from `path`, publishes a fresh revision,
/// and returns where its database lives. If a sync is effectively already
/// current (nothing changed on disk), this still verifies that rather than
/// trusting a cached assumption.
///
/// # Errors
///
/// Returns an error if `path` isn't inside an active project, or if sync
/// itself fails.
pub fn ensure_synced(path: &str) -> Result<SyncedProject, ErrorData> {
    let root = project::find_project_root(Path::new(path))
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    let database_path = project::database_path(&root);

    let mut connection = store::open_read_write(&database_path)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    ensure_project_row(&connection, root.as_path())?;
    let report = sync::publish(&mut connection, root.as_path(), parse::PACK_VERSION)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    drop(connection);

    Ok(SyncedProject {
        database_path,
        revision_id: report.revision_id,
        root: root.as_path().to_path_buf(),
    })
}

/// Runs `f` inside one deferred read transaction that first pins the
/// revision `synced` claimed. Verification and every subsequent read share
/// that snapshot, so a concurrent publish cannot change what the tool sees
/// mid-response — the call either observes the expected revision entirely
/// or fails the revision check before any tool data is read.
///
/// # Errors
///
/// Returns an error if the connection can't be opened, the revision no
/// longer matches, or `f` itself fails.
pub fn with_verified_read<T>(
    synced: &SyncedProject,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, ErrorData>,
) -> Result<T, ErrorData> {
    let mut connection = store::open_read_only(&synced.database_path)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    verify_revision_matches(&tx, &synced.revision_id)?;
    let result = f(&tx)?;
    tx.commit()
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(result)
}

/// Write-side counterpart: one immediate transaction holds the revision
/// check, any lookups, validation, and the write together.
///
/// # Errors
///
/// Returns an error if the connection can't be opened, the revision no
/// longer matches, or `f` itself fails.
pub fn with_verified_write<T>(
    synced: &SyncedProject,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, ErrorData>,
) -> Result<T, ErrorData> {
    let mut connection = store::open_read_write(&synced.database_path)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    verify_revision_matches(&tx, &synced.revision_id)?;
    let result = f(&tx)?;
    tx.commit()
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(result)
}

fn ensure_project_row(connection: &Connection, root: &Path) -> Result<(), ErrorData> {
    let root_path = root.to_string_lossy();
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        });
    connection
        .execute(
            "INSERT OR IGNORE INTO project (\
                id, project_id, root_path, contract_version, schema_version, created_at_unix\
             ) VALUES (1, 'default', ?1, 1, 1, ?2)",
            rusqlite::params![root_path.as_ref(), created_at],
        )
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(())
}

fn verify_revision_matches(
    connection: &Connection,
    expected_revision_id: &str,
) -> Result<(), ErrorData> {
    let current: String = connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    if current != expected_revision_id {
        return Err(ErrorData::internal_error(
            format!(
                "revision changed concurrently (expected {expected_revision_id}, now {current}); retry the call"
            ),
            None,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod test_helpers {
    use super::*;

    pub fn open_verified_read_only(synced: &SyncedProject) -> Result<Connection, ErrorData> {
        let mut connection = store::open_read_only(&synced.database_path)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        verify_revision_matches(&tx, &synced.revision_id)?;
        tx.commit()
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        Ok(connection)
    }

    pub fn open_verified_read_write(synced: &SyncedProject) -> Result<Connection, ErrorData> {
        let mut connection = store::open_read_write(&synced.database_path)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        verify_revision_matches(&tx, &synced.revision_id)?;
        tx.commit()
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        Ok(connection)
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
