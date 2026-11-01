//! The caller-facing record of a project database discard.
//!
//! A discard (corruption, schema or contract bump, root mismatch) deletes
//! the database file and rebuilds it from source. Nothing agent-authored is
//! lost: session findings live in their own `findings-<id>.db`, which a rebuild
//! never touches. The rebuild is automatic, so the call that triggered it
//! carries a [`DatabaseDiscard`] saying why.

use schemars::JsonSchema;
use serde::Serialize;

/// A project database was discarded and rebuilt from source during this
/// call. Present only on the call that discarded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DatabaseDiscard {
    /// Why the database was discarded.
    pub cause: String,
}

#[cfg(test)]
#[path = "discard_notice_tests.rs"]
mod tests;
