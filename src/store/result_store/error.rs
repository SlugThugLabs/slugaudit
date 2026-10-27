//! Typed failures of the result store.
//!
//! The store knows nothing of tools or the wire. The tool layer (9.2) maps the
//! kinds to wire codes: `Unavailable` -> `result_store_unavailable`, `Full` ->
//! `result_store_full`, `TooLarge` -> `result_too_large`, `Expired` ->
//! `result_expired`, `Unknown` -> `unknown_cursor`.
//!
//! Each failure is logged once, in the constructor that decides it, so the
//! layers above can propagate with `?` without logging again.

use std::error::Error;
use std::io;

/// Why a result store operation failed.
#[derive(Debug, thiserror::Error)]
pub enum ResultStoreError {
    /// The store cannot hold or serve results: an I/O or SQLite resource
    /// failure (disk full, I/O error, cannot open), a pragma that did not
    /// take, or a platform without unlink-while-open semantics.
    #[error("the result store is unavailable: {context}")]
    Unavailable {
        /// What the store was doing.
        context: &'static str,
        /// The underlying cause.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },
    /// The new result does not fit and every byte is held by an active result.
    #[error("the result store is full: {active_count} active results hold the space")]
    Full {
        /// Results still protected by the grace period.
        active_count: usize,
    },
    /// The result alone is larger than the whole budget.
    #[error("the result is larger than the whole result store budget")]
    TooLarge,
    /// A valid cursor named a result that was reclaimed.
    #[error("the result was reclaimed and is no longer stored")]
    Expired,
    /// A malformed or foreign cursor, an ordinal past the end, or a result
    /// that is not complete.
    #[error("the cursor does not name a readable result of this process")]
    Unknown,
    /// The caller described a result the store cannot hold.
    #[error("invalid result definition: {reason}")]
    Invalid {
        /// Static explanation.
        reason: &'static str,
    },
    /// A file operation outside the open/write path failed (the startup sweep).
    #[error("result store file operation failed: {context}")]
    Io {
        /// What the store was doing.
        context: &'static str,
        /// The underlying cause.
        #[source]
        source: io::Error,
    },
    /// A SQLite failure that is not a resource failure.
    #[error("result store query failed: {context}")]
    Sqlite {
        /// What the store was doing.
        context: &'static str,
        /// The underlying cause.
        #[source]
        source: rusqlite::Error,
    },
    /// Stored or outgoing JSON metadata does not encode or decode.
    #[error("result store metadata is not valid JSON: {context}")]
    Json {
        /// What was being encoded or decoded.
        context: &'static str,
        /// The underlying cause.
        #[source]
        source: serde_json::Error,
    },
    /// Stored data does not decode.
    #[error("stored result is inconsistent: {context}")]
    Corrupt {
        /// What did not decode.
        context: &'static str,
    },
}

/// A pragma that did not take effect on a result connection.
#[derive(Debug, thiserror::Error)]
#[error("pragma {pragma} reads back {actual:?}, expected {expected:?}")]
pub(super) struct PragmaMismatch {
    pub(super) pragma: &'static str,
    pub(super) expected: &'static str,
    pub(super) actual: String,
}

/// The platform cannot keep an unlinked open file readable.
#[derive(Debug, thiserror::Error)]
#[error("this platform is not supported by the result store")]
pub(super) struct UnsupportedPlatform;

impl ResultStoreError {
    /// An `Unavailable`, logged once here with its cause.
    pub(super) fn unavailable(
        context: &'static str,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        tracing::warn!(context, error = %source, "result store unavailable");
        Self::Unavailable {
            context,
            source: Box::new(source),
        }
    }

    /// Maps a SQLite failure: resource failures become `Unavailable`, the
    /// rest `Sqlite`; both keep the source and are logged once.
    pub(super) fn from_sqlite(context: &'static str, source: rusqlite::Error) -> Self {
        if is_resource_failure(&source) {
            return Self::unavailable(context, source);
        }
        tracing::warn!(context, error = %source, "result store query failed");
        Self::Sqlite { context, source }
    }

    /// A JSON failure, logged once.
    pub(super) fn json(context: &'static str, source: serde_json::Error) -> Self {
        tracing::error!(context, error = %source, "result store metadata is not valid JSON");
        Self::Json { context, source }
    }

    /// Stored data that does not decode, logged once.
    pub(super) fn corrupt(context: &'static str) -> Self {
        tracing::error!(context, "stored result is inconsistent");
        Self::Corrupt { context }
    }
}

/// Disk-full, I/O, cannot-open and similar environment failures.
fn is_resource_failure(error: &rusqlite::Error) -> bool {
    use rusqlite::ErrorCode::{
        CannotOpen, DiskFull, NoLargeFileSupport, OutOfMemory, ReadOnly, SystemIoFailure,
    };
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                DiskFull | SystemIoFailure | CannotOpen | OutOfMemory | ReadOnly | NoLargeFileSupport
            )
    )
}
