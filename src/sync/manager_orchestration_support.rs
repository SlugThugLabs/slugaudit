//! Free functions behind [`super::manager`]'s orchestration module.
//!
//! The orchestration workflow itself lives in `manager_orchestration.rs`;
//! this module holds the pieces that do not touch the manager's private
//! state — the database-recovery log records (with their static error
//! categories) and the saturating sync-duration stamp. Keeping them here
//! leaves `ensure_current` readable in one file.

use crate::store::StoreError;
use std::path::Path;

/// One database recovery event: a database that failed to open, or one whose
/// metadata proves it belongs to another project root or another contract.
///
/// The recovery paths discard and rebuild, so the record exists to explain
/// why. It never carries the error's `Display` text: `StoreError` wraps
/// dependency errors whose wording is not a controlled diagnostic (rusqlite
/// can embed the database path), and a stored root is a raw absolute path.
pub(super) enum DatabaseRecovery<'a> {
    /// `operation = "open_project_database"`; `message` names the
    /// consequence (a corrupt/outdated discard, or a surfaced open failure).
    OpenFailed {
        error: &'a StoreError,
        message: &'static str,
    },
    /// `operation = "verify_project_database"`: the database was published
    /// for a different project root.
    RootMismatch { stored_root: &'a str },
    /// `operation = "verify_project_database"`: the database's
    /// contract/schema version is unsupported.
    IncompatibleVersion {
        contract_version: i64,
        schema_version: i64,
    },
}

/// Logs a database recovery warning with only static categories and redacted
/// ids. See [`DatabaseRecovery`] for why the error text is never logged.
pub(super) fn log_database_recovery(database_path: &Path, recovery: DatabaseRecovery<'_>) {
    match recovery {
        DatabaseRecovery::OpenFailed { error, message } => tracing::warn!(
            operation = "open_project_database",
            database_id = %crate::util::redacted_path(database_path),
            error_kind = store_error_kind(error),
            "{}",
            message,
        ),
        DatabaseRecovery::RootMismatch { stored_root } => tracing::warn!(
            operation = "verify_project_database",
            database_id = %crate::util::redacted_path(database_path),
            error_kind = "root_mismatch",
            stored_root_id = %crate::util::redacted_path(Path::new(stored_root)),
            "project database is stale; discarding and rebuilding from source",
        ),
        DatabaseRecovery::IncompatibleVersion {
            contract_version,
            schema_version,
        } => tracing::warn!(
            operation = "verify_project_database",
            database_id = %crate::util::redacted_path(database_path),
            error_kind = "incompatible_version",
            contract_version,
            schema_version,
            "project database is stale; discarding and rebuilding from source",
        ),
    }
}

/// The `StoreError` variant as a static, path-free category. `StoreError`
/// wraps dependency errors whose `Display` is not a controlled diagnostic
/// (rusqlite can embed the database path on non-Unix targets), so a database
/// failure is never formatted into a log line.
fn store_error_kind(error: &StoreError) -> &'static str {
    match error {
        StoreError::Open(_) => "open",
        StoreError::Configure(_) => "configure",
        StoreError::Migration(_) => "migration",
        StoreError::Symlink => "symlink",
        StoreError::Inspect(_) => "inspect",
        StoreError::WalUnavailable { .. } => "wal_unavailable",
        StoreError::Permissions(_) => "permissions",
        StoreError::NetworkFilesystem => "network_filesystem",
        StoreError::NetworkFilesystemCheck(_) => "network_filesystem_check",
    }
}

/// Milliseconds since `start`, for the `last_sync_duration_ms` stamp. A
/// `u128` count of milliseconds only exceeds `u64::MAX` after about 584
/// million years, so the saturation is a documented impossibility; it is
/// still logged with its cause rather than swallowed.
pub(super) fn elapsed_ms(start: std::time::Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or_else(|error| {
        tracing::warn!(error = %error, "sync duration exceeds u64 milliseconds; stamp saturates");
        u64::MAX
    })
}
