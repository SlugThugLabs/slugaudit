//! The entry an auditor registers with `mechanic add`. Entries live in
//! `session_mechanics` (memory only, gone when the process exits).
//!
//! This layer knows the shape only. Whether an entry's query compiles,
//! whether its id collides with a built-in or with another entry, and what it
//! counts are decided in `tools::mechanic_catalog`, so a bad entry is
//! reported there with its reason.

use serde::{Deserialize, Serialize};

/// One auditor-registered mechanic: a Tree-sitter query with exactly one
/// capture, run over the indexed files of `language`; every distinct node it
/// captures is one row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MechanicEntry {
    /// The id `leads` takes as `mechanic`; never a built-in's id.
    pub id: String,
    /// The grammar the query compiles against and the files it reads.
    pub language: String,
    /// Tree-sitter S-expression with exactly one capture.
    pub query: String,
    /// The author's own words for what the entry counts.
    pub note: String,
}
