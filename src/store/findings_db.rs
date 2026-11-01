//! The session findings database (`findings-<id>.db`): opened with the same
//! contract as `project.db` (symlink refusal, owner-only creation, WAL, the
//! bounded busy wait), with its own tiny schema. It has no version: the file
//! lives for one SlugAudit process and is deleted when the next process first
//! touches the project (`project::begin_process_session`).

use super::connection::{StoreError, ensure_present};
use super::connection_guards::{
    configure, create_with_private_permissions_if_missing, open_flags, reject_symlink,
    resolve_parent_dir,
};
use rusqlite::Connection;
use std::path::Path;

const FINDINGS_DDL: &str = include_str!("findings_schema.sql");

/// Opens the findings database read-write, creating the file and its table
/// if needed.
///
/// # Errors
///
/// Returns an error if the file can't be created or opened, the path is a
/// symlink, pragmas can't be applied, or the table can't be created.
pub fn open_findings(path: &Path) -> Result<Connection, StoreError> {
    let resolved = resolve_parent_dir(path);
    let path = resolved.as_path();
    reject_symlink(path)?;
    create_with_private_permissions_if_missing(path)?;
    let connection =
        Connection::open_with_flags(path, open_flags(true)).map_err(StoreError::Open)?;
    configure(&connection)?;
    connection
        .execute_batch(FINDINGS_DDL)
        .map_err(StoreError::Configure)?;
    Ok(connection)
}

/// Opens the findings database read-only; `None` when no finding was ever
/// written in this session (the file does not exist).
///
/// # Errors
///
/// Returns an error if the path is a symlink or the file can't be opened.
pub fn open_findings_existing(path: &Path) -> Result<Option<Connection>, StoreError> {
    let resolved = resolve_parent_dir(path);
    match ensure_present(resolved.as_path()) {
        Ok(()) => super::connection::open_existing(path).map(Some),
        Err(StoreError::Missing) => Ok(None),
        Err(error) => Err(error),
    }
}
