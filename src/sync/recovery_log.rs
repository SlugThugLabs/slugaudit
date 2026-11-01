//! The one WARN a database recovery (discard and rebuild) logs. Split from
//! `manager_orchestration.rs` to keep the workflow file under the size cap.

use crate::store::StoreError;
use std::path::Path;

/// One database recovery event: a database that failed to open, or one whose
/// metadata proves it belongs to another project root or another contract.
pub(super) enum DatabaseRecovery<'a> {
    /// The open failed for a reason that does not discard the database.
    OpenFailed {
        error: &'a StoreError,
    },
    /// The open failed as corrupt or outdated; the database is discarded.
    OpenCorrupt {
        error: &'a StoreError,
    },
    RootMismatch {
        stored_root: &'a str,
    },
    IncompatibleVersion {
        contract_version: i64,
        schema_version: i64,
    },
}

/// The one WARN for a recovery event.
pub(super) fn log_database_recovery(database_path: &Path, recovery: DatabaseRecovery<'_>) {
    match recovery {
        DatabaseRecovery::OpenFailed { error } => tracing::warn!(
            operation = "open_project_database",
            path = %database_path.display(),
            error_kind = store_error_kind(error),
            error = %error,
            "failed to open project database for sync",
        ),
        DatabaseRecovery::OpenCorrupt { error } => tracing::warn!(
            operation = "open_project_database",
            path = %database_path.display(),
            error_kind = store_error_kind(error),
            error = %error,
            "database is corrupt or outdated; discarding and re-publishing from scratch",
        ),
        DatabaseRecovery::RootMismatch { stored_root } => tracing::warn!(
            operation = "verify_project_database",
            path = %database_path.display(),
            error_kind = "root_mismatch",
            stored_root = %stored_root,
            "project database is stale; discarding and rebuilding from source",
        ),
        DatabaseRecovery::IncompatibleVersion {
            contract_version,
            schema_version,
        } => tracing::warn!(
            operation = "verify_project_database",
            path = %database_path.display(),
            error_kind = "incompatible_version",
            contract_version,
            schema_version,
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
        StoreError::Missing => "missing",
        StoreError::Inspect(_) => "inspect",
        StoreError::Create(_) => "create",
        StoreError::WalUnavailable { .. } => "wal_unavailable",
    }
}
