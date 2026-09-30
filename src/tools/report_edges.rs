//! Inbound/outbound `dependency_edges` totals for `report`.
//!
//! Counts of edges that exist in the index, in the same register as the rest
//! of `report`: no ranking, no severity, no ordering by importance, and no
//! claim about which file matters. "Blast radius" is arithmetic here — how
//! many edges leave a file and how many land on one — and the per-file
//! traversal is the `slugaudit://queries/blast_radius.sql` recipe, not a
//! ninth tool. `report` carries the totals because it is the zero-decision
//! entry point: one `path`, no schema knowledge.

use rusqlite::Transaction;
use schemars::JsonSchema;
use serde::Serialize;

/// Edge totals in both directions, over the whole revision.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DependencyEdgeTotals {
    /// `count(*) FROM dependency_edges`: every edge, since every edge
    /// originates at a file (`from_file_id` is `NOT NULL`).
    pub outbound_edge_count: i64,
    /// Edges whose `to_file_id` names a row in `files` — the subset that
    /// lands on a project file. `Unresolved` and `External` edges carry no
    /// target, and `import_resolution` splits those two apart; this total
    /// groups them only by their absence of a target.
    pub inbound_edge_count: i64,
    /// Distinct files that are the source of at least one edge.
    pub outbound_file_count: i64,
    /// Distinct files that are the target of at least one edge.
    pub inbound_file_count: i64,
}

/// Builds the totals for the current revision.
///
/// # Errors
///
/// Returns a database error if any of the four counts fails.
pub(super) fn build(connection: &Transaction<'_>) -> Result<DependencyEdgeTotals, rusqlite::Error> {
    let outbound_edge_count: i64 =
        connection.query_row("SELECT count(*) FROM dependency_edges", [], |row| {
            row.get(0)
        })?;
    let inbound_edge_count: i64 = connection.query_row(
        "SELECT count(*) FROM dependency_edges WHERE to_file_id IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    let outbound_file_count: i64 = connection.query_row(
        "SELECT count(DISTINCT from_file_id) FROM dependency_edges",
        [],
        |row| row.get(0),
    )?;
    let inbound_file_count: i64 = connection.query_row(
        "SELECT count(DISTINCT to_file_id) FROM dependency_edges",
        [],
        |row| row.get(0),
    )?;
    Ok(DependencyEdgeTotals {
        outbound_edge_count,
        inbound_edge_count,
        outbound_file_count,
        inbound_file_count,
    })
}
