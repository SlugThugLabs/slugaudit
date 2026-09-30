//! The `leads` mechanic catalogue, the SQL each family compiles to, and the
//! two reads that execute it.
//!
//! Split from `leads.rs` so that adding a mechanic stays a one-entry edit to a
//! declarative table, while the tool contract — request/response shapes,
//! paging, and the transaction envelope — lives in one place. Nothing here
//! draws a conclusion: an entry says which shape to look for and which string
//! to return, never whether the shape matters.

use super::leads::{LeadRow, MechanicCount};
use rmcp::ErrorData;
use rusqlite::{Transaction, params};

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
    pub(super) predicate: &'static str,
    pub(super) detail: &'static str,
}

/// Every mechanic, in declaration order. Call-site mechanics read `Call`
/// evidence, which the Rust call walker (`src/evidence/calls.rs`) is the only
/// producer of — they are empty for projects with no Rust files.
pub(super) const MECHANICS: &[Mechanic] = &[
    Mechanic {
        id: "unwrap_calls",
        family: Family::Evidence,
        predicate: "e.kind = 'Call' AND json_extract(e.payload, '$.callee_name') = 'unwrap'",
        detail: "json_extract(e.payload, '$.callee_text')",
    },
    Mechanic {
        id: "expect_calls",
        family: Family::Evidence,
        predicate: "e.kind = 'Call' AND json_extract(e.payload, '$.callee_name') = 'expect'",
        detail: "json_extract(e.payload, '$.callee_text')",
    },
    Mechanic {
        id: "panic_calls",
        family: Family::Evidence,
        predicate: "e.kind = 'Call' AND json_extract(e.payload, '$.callee_name') \
                    IN ('panic', 'unreachable')",
        detail: "json_extract(e.payload, '$.callee_text')",
    },
    Mechanic {
        id: "todo_calls",
        family: Family::Evidence,
        predicate: "e.kind = 'Call' AND json_extract(e.payload, '$.callee_name') \
                    IN ('todo', 'unimplemented')",
        detail: "json_extract(e.payload, '$.callee_text')",
    },
    Mechanic {
        id: "assert_calls",
        family: Family::Evidence,
        predicate: "e.kind = 'Call' AND json_extract(e.payload, '$.callee_name') \
                    IN ('assert', 'assert_eq', 'assert_ne', 'debug_assert', \
                        'debug_assert_eq', 'debug_assert_ne')",
        detail: "json_extract(e.payload, '$.callee_text')",
    },
    Mechanic {
        id: "diagnostics",
        family: Family::Evidence,
        predicate: "e.kind = 'Diagnostic'",
        detail: "json_extract(e.payload, '$.message')",
    },
    Mechanic {
        id: "comment_markers",
        family: Family::Evidence,
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
            ErrorData::invalid_params(format!("unknown mechanic `{id}`; valid ids: {valid}"), None)
        })
}

/// `SELECT count(*)` for a mechanic's family. Counts are always complete,
/// never paged, so they never carry `LIMIT`.
fn count_sql(mechanic: &Mechanic) -> String {
    match mechanic.family {
        Family::Evidence => {
            format!(
                "SELECT count(*) FROM evidence e WHERE {}",
                mechanic.predicate
            )
        }
        Family::Files => format!("SELECT count(*) FROM files WHERE {}", mechanic.predicate),
    }
}

/// The paged row query for a mechanic, returning `(path, line, detail)`.
/// `?1` is the limit and `?2` the offset, so both families page identically.
fn rows_sql(mechanic: &Mechanic) -> String {
    match mechanic.family {
        Family::Evidence => format!(
            "SELECT f.path, COALESCE(e.start_line, 0), COALESCE({}, '') \
             FROM evidence e JOIN files f ON e.file_id = f.id \
             WHERE {} ORDER BY f.path, e.start_line LIMIT ?1 OFFSET ?2",
            mechanic.detail, mechanic.predicate
        ),
        Family::Files => format!(
            "SELECT path, 0, '' FROM files WHERE {} ORDER BY path LIMIT ?1 OFFSET ?2",
            mechanic.predicate
        ),
    }
}

/// Counts for every mechanic, in `MECHANICS` order.
pub(super) fn count_all(tx: &Transaction<'_>) -> Result<Vec<MechanicCount>, ErrorData> {
    MECHANICS
        .iter()
        .map(|mechanic| {
            let count: i64 = tx
                .query_row(&count_sql(mechanic), [], |row| row.get(0))
                .map_err(|error| super::context::internal_error("counting leads", error))?;
            Ok(MechanicCount {
                mechanic: mechanic.id.to_owned(),
                count,
            })
        })
        .collect()
}

/// One page of rows for a single mechanic. `detail` is the stored text,
/// unclipped, so `detail_clipped` starts false; `leads` clips and flags.
pub(super) fn rows_for(
    tx: &Transaction<'_>,
    mechanic: &Mechanic,
    limit: usize,
    offset: usize,
) -> Result<Vec<LeadRow>, ErrorData> {
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let offset = i64::try_from(offset).unwrap_or(i64::MAX);
    let mut statement = tx
        .prepare(&rows_sql(mechanic))
        .map_err(|error| super::context::internal_error("preparing leads rows", error))?;
    let rows = statement
        .query_map(params![limit, offset], |row| {
            Ok(LeadRow {
                file: row.get(0)?,
                line: row.get(1)?,
                detail: row.get(2)?,
                detail_clipped: false,
            })
        })
        .map_err(|error| super::context::internal_error("reading leads rows", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| super::context::internal_error("reading leads rows", error))?;
    Ok(rows)
}
