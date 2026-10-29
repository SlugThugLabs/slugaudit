//! The caller-facing record of a project database discard.
//!
//! A discard (corruption, schema or contract bump, root mismatch) deletes
//! the database file, and with it agent-authored findings that cannot be
//! re-derived from source. The rebuild is automatic, so the call that
//! triggered it carries a [`DatabaseDiscard`] saying how many findings were
//! lost. The count is taken before the file is removed; when the database
//! cannot be read it is reported as unknown (`null`), never as zero.

use rusqlite::{Connection, OpenFlags};
use schemars::JsonSchema;
use serde::Serialize;
use std::path::Path;

/// A project database was discarded and rebuilt from source during this
/// call. Present only on the call that discarded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DatabaseDiscard {
    /// Rows in `findings` (any status) that were in the discarded database.
    /// `null` means unknown: the database was too damaged to count. Zero
    /// means the database was readable and held no findings.
    pub findings_discarded: Option<u64>,
    /// Why the database was discarded.
    pub cause: String,
}

impl DatabaseDiscard {
    /// Builds the notice from a pre-discard count. A failed count becomes
    /// `findings_discarded: null`; the count error is returned beside the
    /// notice so the caller can log it with the discard.
    #[must_use]
    pub(crate) fn assess(
        count: Result<u64, rusqlite::Error>,
        cause: String,
    ) -> (Self, Option<rusqlite::Error>) {
        match count {
            Ok(findings) => (
                Self {
                    findings_discarded: Some(findings),
                    cause,
                },
                None,
            ),
            Err(error) => (
                Self {
                    findings_discarded: None,
                    cause,
                },
                Some(error),
            ),
        }
    }
}

impl DatabaseDiscard {
    /// [`DatabaseDiscard::assess`] on the findings of the database file at
    /// `path`, counted before the file is removed.
    #[must_use]
    pub(crate) fn assess_file(path: &Path, cause: String) -> (Self, Option<rusqlite::Error>) {
        Self::assess(count_findings_at(path), cause)
    }
}

/// Counts every row in `findings` on an open connection.
///
/// # Errors
///
/// Returns the SQLite error if the table cannot be read.
pub(crate) fn count_findings(connection: &Connection) -> Result<u64, rusqlite::Error> {
    connection.query_row("SELECT COUNT(*) FROM findings", [], |row| row.get(0))
}

/// Counts findings in the database file at `path` through a raw read-only
/// connection that skips the schema checks `store::open_read_only` makes, so
/// a database with a foreign schema version is still counted.
///
/// # Errors
///
/// Returns the SQLite error if the file cannot be opened or read as a
/// database with a `findings` table.
pub(crate) fn count_findings_at(path: &Path) -> Result<u64, rusqlite::Error> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    count_findings(&Connection::open_with_flags(path, flags)?)
}

#[cfg(test)]
#[path = "discard_notice_tests.rs"]
mod tests;
