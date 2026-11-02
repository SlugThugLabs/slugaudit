//! The session findings database as the finding tools use it. Findings live
//! in `findings-<id>.db` beside `project.db` (see `project::findings_session`), so
//! a write opens that file, and a read compares each finding's stored file
//! hash with the hash the freshly synced `project.db` holds for the file:
//! equal means `current`, anything else (edited, deleted, or no longer
//! readable) means `stale`.

use super::context::internal_error;
use crate::store;
use crate::sync::SyncedProject;
use rmcp::ErrorData;
use rusqlite::{OptionalExtension, Statement, Transaction, TransactionBehavior};
use std::path::{Path, PathBuf};

/// Runs `f` inside an immediate transaction on the findings database,
/// creating the file and its table on the first finding of the session.
pub(super) fn with_findings_write<T>(
    synced: &SyncedProject,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, ErrorData>,
) -> Result<T, ErrorData> {
    let path = synced.findings_path()?;
    let mut connection = store::open_findings(&path).map_err(|error| match error {
        // The directory is gone: `off` ran after the sync, so the project
        // is off.
        store::StoreError::Create(_) if path.parent().is_some_and(|dir| !dir.is_dir()) => {
            crate::sync::map_activation_error(&crate::project::ActivationError::NotActive)
        }
        error => internal_error("opening the findings database for write", error),
    })?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| internal_error("starting a findings write transaction", error))?;
    let result = f(&tx)?;
    tx.commit()
        .map_err(|error| internal_error("committing the findings write transaction", error))?;
    Ok(result)
}

/// This process's findings database for the project `tx` is reading.
pub(super) fn findings_path_of(tx: &Transaction<'_>) -> Result<PathBuf, ErrorData> {
    let database = tx.path().ok_or_else(|| {
        internal_error(
            "locating the findings database",
            "the project database has no file path",
        )
    })?;
    crate::sync::session_findings_for(Path::new(database))
}

/// Looks up the content hash `project.db` holds for a path, to decide
/// whether a finding is still current.
pub(super) struct HashLookup<'a>(Statement<'a>);

impl<'a> HashLookup<'a> {
    pub(super) fn new(project_tx: &'a Transaction<'_>) -> Result<Self, ErrorData> {
        project_tx
            .prepare("SELECT content_hash FROM files WHERE path = ?1")
            .map(Self)
            .map_err(|error| internal_error("preparing the file hash lookup", error))
    }

    /// True when `path` is indexed and still has `source_hash`.
    pub(super) fn is_current(&mut self, path: &str, source_hash: &str) -> Result<bool, ErrorData> {
        let current: Option<String> = self
            .0
            .query_row([path], |row| row.get(0))
            .optional()
            .map_err(|error| internal_error("reading a file hash", error))?;
        Ok(current.as_deref() == Some(source_hash))
    }
}

/// How many of this session's findings are still current against the index
/// `project_tx` reads; zero when no finding was ever written.
pub(super) fn count_current(
    project_tx: &Transaction<'_>,
    findings_path: &Path,
) -> Result<i64, ErrorData> {
    let Some(findings) = store::open_findings_existing(findings_path)
        .map_err(|error| internal_error("opening the findings database", error))?
    else {
        return Ok(0);
    };
    let mut lookup = HashLookup::new(project_tx)?;
    let mut statement = findings
        .prepare("SELECT path, source_hash FROM findings")
        .map_err(|error| internal_error("preparing the findings count", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| internal_error("executing the findings count", error))?;
    let mut current = 0i64;
    for row in rows {
        let (path, source_hash) =
            row.map_err(|error| internal_error("reading a finding for the count", error))?;
        if lookup.is_current(&path, &source_hash)? {
            current += 1;
        }
    }
    Ok(current)
}
