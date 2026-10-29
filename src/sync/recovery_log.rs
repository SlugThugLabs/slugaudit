//! The one WARN a database recovery (discard and rebuild) logs. Split from
//! `manager_orchestration.rs` to keep the workflow file under the size cap.

use super::discard_notice::DatabaseDiscard;
use crate::store::StoreError;
use std::path::Path;

/// One database recovery event: a database that failed to open, or one whose
/// metadata proves it belongs to another project root or another contract.
pub(super) enum DatabaseRecovery<'a> {
    /// The open failed for a reason that does not discard the database.
    OpenFailed { error: &'a StoreError },
    /// The open failed as corrupt or outdated; the database is discarded.
    OpenCorrupt {
        error: &'a StoreError,
        discard: &'a DatabaseDiscard,
        count_error: Option<&'a rusqlite::Error>,
    },
    RootMismatch {
        stored_root: &'a str,
        discard: &'a DatabaseDiscard,
        count_error: Option<&'a rusqlite::Error>,
    },
    IncompatibleVersion {
        contract_version: i64,
        schema_version: i64,
        discard: &'a DatabaseDiscard,
        count_error: Option<&'a rusqlite::Error>,
    },
}

/// The one WARN for a recovery event. Discarding variants also carry the
/// number of findings lost (`None` = unknown) and, when the count failed, the
/// source error of that failure.
pub(super) fn log_database_recovery(database_path: &Path, recovery: DatabaseRecovery<'_>) {
    match recovery {
        DatabaseRecovery::OpenFailed { error } => tracing::warn!(
            operation = "open_project_database",
            database_id = %crate::util::redacted_path(database_path),
            error_kind = store_error_kind(error),
            error = %error,
            "failed to open project database for sync",
        ),
        DatabaseRecovery::OpenCorrupt {
            error,
            discard,
            count_error,
        } => tracing::warn!(
            operation = "open_project_database",
            database_id = %crate::util::redacted_path(database_path),
            error_kind = store_error_kind(error),
            error = %error,
            findings_discarded = ?discard.findings_discarded,
            count_error = count_error.map(tracing::field::display),
            "database is corrupt or outdated; discarding and re-publishing from scratch",
        ),
        DatabaseRecovery::RootMismatch {
            stored_root,
            discard,
            count_error,
        } => tracing::warn!(
            operation = "verify_project_database",
            database_id = %crate::util::redacted_path(database_path),
            error_kind = "root_mismatch",
            stored_root_id = %crate::util::redacted_path(Path::new(stored_root)),
            findings_discarded = ?discard.findings_discarded,
            count_error = count_error.map(tracing::field::display),
            "project database is stale; discarding and rebuilding from source",
        ),
        DatabaseRecovery::IncompatibleVersion {
            contract_version,
            schema_version,
            discard,
            count_error,
        } => tracing::warn!(
            operation = "verify_project_database",
            database_id = %crate::util::redacted_path(database_path),
            error_kind = "incompatible_version",
            contract_version,
            schema_version,
            findings_discarded = ?discard.findings_discarded,
            count_error = count_error.map(tracing::field::display),
            "project database is stale; discarding and rebuilding from source",
        ),
    }
}

fn store_error_kind(error: &StoreError) -> &'static str {
    match error {
        StoreError::Open(_) => "open",
        StoreError::Configure(_) => "configure",
        StoreError::Busy(_) => "busy",
        StoreError::Migration(_) => "migration",
        StoreError::Symlink => "symlink",
        StoreError::Inspect(_) => "inspect",
        StoreError::WalUnavailable { .. } => "wal_unavailable",
        StoreError::Permissions(_) => "permissions",
        StoreError::NetworkFilesystem => "network_filesystem",
        StoreError::NetworkFilesystemCheck(_) => "network_filesystem_check",
    }
}
