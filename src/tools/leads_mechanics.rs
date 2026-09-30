//! The `leads` mechanic catalogue: what each mechanic looks for, which family
//! of index rows it reads, and which languages it covers.
//!
//! Kept apart from `leads_query.rs` — the compiled statements and the reads
//! that execute them — so that adding a mechanic stays a one-entry edit to a
//! declarative table. Nothing here draws a conclusion: an entry says which
//! shape to look for, which SQL expressions name it, which string to return,
//! and what it was measured over, never whether the shape matters.

use super::leads::MechanicCoverage;
use crate::error_codes::{self, ErrorCode};
use rmcp::ErrorData;

/// Where a mechanic's rows come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    /// `evidence` joined to `files`; the predicate may use `e` and `f`.
    Evidence,
    /// `files` directly; the predicate may use an unaliased `path`.
    Files,
}

/// One mechanical observation surface.
///
/// `predicate` is a SQL fragment with no leading `WHERE`. `detail` is a SQL
/// expression for the row's one string. Adding a mechanic is a new entry
/// here — never a new table, and never a statement about what a match means.
pub(super) struct Mechanic {
    pub(super) id: &'static str,
    pub(super) family: Family,
    /// The languages this mechanic was actually measured over. See
    /// [`MechanicCoverage`] — this is the machine-readable half of "a zero
    /// here means unchecked, not absent".
    pub(super) covers: MechanicCoverage,
    pub(super) predicate: &'static str,
    pub(super) detail: &'static str,
}

/// Every mechanic, in declaration order. Call-site mechanics read `Call`
/// evidence, which the Rust call walker (`src/evidence/calls.rs`) is the only
/// producer of — they are empty for projects with no Rust files, which is why
/// they declare [`MechanicCoverage::RustOnly`].
pub(super) const MECHANICS: &[Mechanic] = &[
    Mechanic {
        id: "unwrap_calls",
        family: Family::Evidence,
        covers: MechanicCoverage::RustOnly,
        predicate: "e.kind = 'Call' AND json_extract(e.payload, '$.callee_name') = 'unwrap'",
        detail: "json_extract(e.payload, '$.callee_text')",
    },
    Mechanic {
        id: "expect_calls",
        family: Family::Evidence,
        covers: MechanicCoverage::RustOnly,
        predicate: "e.kind = 'Call' AND json_extract(e.payload, '$.callee_name') = 'expect'",
        detail: "json_extract(e.payload, '$.callee_text')",
    },
    Mechanic {
        id: "panic_calls",
        family: Family::Evidence,
        covers: MechanicCoverage::RustOnly,
        predicate: "e.kind = 'Call' AND json_extract(e.payload, '$.callee_name') \
                    IN ('panic', 'unreachable')",
        detail: "json_extract(e.payload, '$.callee_text')",
    },
    Mechanic {
        id: "todo_calls",
        family: Family::Evidence,
        covers: MechanicCoverage::RustOnly,
        predicate: "e.kind = 'Call' AND json_extract(e.payload, '$.callee_name') \
                    IN ('todo', 'unimplemented')",
        detail: "json_extract(e.payload, '$.callee_text')",
    },
    Mechanic {
        id: "assert_calls",
        family: Family::Evidence,
        covers: MechanicCoverage::RustOnly,
        predicate: "e.kind = 'Call' AND json_extract(e.payload, '$.callee_name') \
                    IN ('assert', 'assert_eq', 'assert_ne', 'debug_assert', \
                        'debug_assert_eq', 'debug_assert_ne')",
        detail: "json_extract(e.payload, '$.callee_text')",
    },
    Mechanic {
        id: "diagnostics",
        family: Family::Evidence,
        covers: MechanicCoverage::AllLanguages,
        predicate: "e.kind = 'Diagnostic'",
        detail: "json_extract(e.payload, '$.message')",
    },
    Mechanic {
        id: "comment_markers",
        family: Family::Evidence,
        covers: MechanicCoverage::AllLanguages,
        predicate: "e.kind = 'Comment' AND (\
                    json_extract(e.payload, '$.text') LIKE '%TODO%' \
                    OR json_extract(e.payload, '$.text') LIKE '%FIXME%' \
                    OR json_extract(e.payload, '$.text') LIKE '%HACK%' \
                    OR json_extract(e.payload, '$.text') LIKE '%XXX%')",
        detail: "json_extract(e.payload, '$.text')",
    },
    Mechanic {
        id: "credential_files",
        family: Family::Files,
        covers: MechanicCoverage::AllLanguages,
        predicate: super::CREDENTIAL_PATH_PREDICATE,
        detail: "''",
    },
];

/// Resolves the requested mechanic id, or reports every valid id.
pub(super) fn resolve_mechanic(
    requested: Option<&str>,
) -> Result<Option<&'static Mechanic>, ErrorData> {
    let Some(id) = requested else {
        return Ok(None);
    };
    MECHANICS
        .iter()
        .find(|mechanic| mechanic.id == id)
        .map(Some)
        .ok_or_else(|| {
            let valid = MECHANICS
                .iter()
                .map(|mechanic| mechanic.id)
                .collect::<Vec<_>>()
                .join(", ");
            ErrorData::invalid_params(
                format!("unknown mechanic `{id}`; valid ids: {valid}"),
                error_codes::payload(ErrorCode::UnknownMechanic),
            )
        })
}

impl Mechanic {
    /// The `FROM` clause: `files` alone for path-shaped mechanics, the
    /// `evidence`/`files` join for evidence-shaped ones.
    pub(super) fn read_from(&self) -> &'static str {
        match self.family {
            Family::Evidence => "evidence e JOIN files f ON e.file_id = f.id",
            Family::Files => "files",
        }
    }

    /// The project-relative path expression, used both as the returned
    /// `file` column and as the thing `pattern` is matched against.
    pub(super) fn path_expr(&self) -> &'static str {
        match self.family {
            Family::Evidence => "f.path",
            Family::Files => "path",
        }
    }

    pub(super) fn line_expr(&self) -> &'static str {
        match self.family {
            Family::Evidence => "COALESCE(e.start_line, 0)",
            Family::Files => "0",
        }
    }

    /// The stable row order every read shares: by file, then by line. A page
    /// boundary can split a file but never reorder the files, which is what
    /// makes `files_covered` cumulative for a sweep at one revision.
    pub(super) fn order_expr(&self) -> &'static str {
        match self.family {
            Family::Evidence => "f.path, e.start_line",
            Family::Files => "path",
        }
    }

    /// The column identifying the file a row belongs to — the key the
    /// per-mechanic distinct-file denominator counts.
    pub(super) fn file_key_expr(&self) -> &'static str {
        match self.family {
            Family::Evidence => "e.file_id",
            Family::Files => "path",
        }
    }

    pub(super) fn pattern_clause(&self) -> &'static str {
        match self.family {
            Family::Evidence => "f.path GLOB ?",
            Family::Files => "path GLOB ?",
        }
    }

    pub(super) fn detail_expr(&self) -> String {
        match self.family {
            Family::Evidence => format!("COALESCE({}, '')", self.detail),
            Family::Files => "''".to_owned(),
        }
    }

    /// The predicate with the path glob appended when one is present.
    ///
    /// The predicate is parenthesized before the `AND`: `credential_files`
    /// is an unparenthesized `OR` chain, and `A OR B AND glob` would bind
    /// the glob to the last term alone — a filter that silently keeps
    /// non-matching files.
    pub(super) fn where_clause(&self, matched: bool) -> String {
        if matched {
            format!(" WHERE ({}) AND {}", self.predicate, self.pattern_clause())
        } else {
            format!(" WHERE {}", self.predicate)
        }
    }
}
