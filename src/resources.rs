//! Static MCP resources: the query cookbook and the canonical schema.
//!
//! These are compile-time-embedded documents served over `resources/list` and
//! `resources/read`. They are deliberately *pull-based* — nothing here is
//! pushed into a connecting agent's context, so an agent that never needs the
//! recipes pays nothing for them, and one that does can read the exact
//! document instead of rediscovering the schema by trial.
//!
//! Content is `include_str!`-ed from files that already live in this
//! repository, so there is no second copy to drift: `examples/queries/` is the
//! cookbook and the payload-key reference, `src/store/schema.sql` is the
//! schema. Nothing here touches a project database or the filesystem at
//! runtime.

use rmcp::model::{Resource, ResourceContents};

/// One embedded document.
struct CatalogEntry {
    uri: &'static str,
    name: &'static str,
    description: &'static str,
    mime_type: &'static str,
    content: &'static str,
}

const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        uri: "slugaudit://queries/README.md",
        name: "query-cookbook",
        description: "Pre-built SQL and Tree-sitter recipes for querying the index.",
        mime_type: "text/markdown",
        content: include_str!("../examples/queries/README.md"),
    },
    CatalogEntry {
        uri: "slugaudit://queries/evidence_payload_keys.md",
        name: "evidence-payload-keys",
        description: "Every evidence.payload key, its JSON type, and the kind it belongs to.",
        mime_type: "text/markdown",
        content: include_str!("../examples/queries/evidence_payload_keys.md"),
    },
    CatalogEntry {
        uri: "slugaudit://queries/find_unhandled_panics.sql",
        name: "find-unhandled-panics",
        description: "Find unwrap/expect/panic call sites across the codebase.",
        mime_type: "application/sql",
        content: include_str!("../examples/queries/find_unhandled_panics.sql"),
    },
    CatalogEntry {
        uri: "slugaudit://queries/auth_attack_surface.sql",
        name: "auth-attack-surface",
        description: "Map functions, methods, and calls touching auth, tokens, or sessions.",
        mime_type: "application/sql",
        content: include_str!("../examples/queries/auth_attack_surface.sql"),
    },
    CatalogEntry {
        uri: "slugaudit://queries/syntax_diagnostics.sql",
        name: "syntax-diagnostics",
        description: "List syntax-error and parser diagnostics per file.",
        mime_type: "application/sql",
        content: include_str!("../examples/queries/syntax_diagnostics.sql"),
    },
    CatalogEntry {
        uri: "slugaudit://queries/rust_functions.scm",
        name: "rust-functions",
        description: "Tree-sitter pattern for Rust function declarations and bodies.",
        mime_type: "text/plain",
        content: include_str!("../examples/queries/rust_functions.scm"),
    },
    CatalogEntry {
        uri: "slugaudit://queries/blast_radius.sql",
        name: "blast-radius",
        description: "Per-file inbound/outbound dependency_edges counts and the transitive closure.",
        mime_type: "application/sql",
        content: include_str!("../examples/queries/blast_radius.sql"),
    },
    CatalogEntry {
        uri: "slugaudit://schema.sql",
        name: "database-schema",
        description: "Canonical SQLite schema of the project index: tables, columns, and constraints.",
        mime_type: "application/sql",
        content: include_str!("store/schema.sql"),
    },
];

/// Every resource this server exposes, for `resources/list`.
///
/// `list_resources` (`src/server.rs`) returns exactly this, so
/// `check_mcp_surface` snapshots the same list the wire carries and fails
/// on drift. Adding, removing, or re-describing a resource is therefore a
/// deliberate, recorded change.
pub fn list() -> Vec<Resource> {
    CATALOG
        .iter()
        .map(|entry| {
            Resource::new(entry.uri, entry.name)
                .with_description(entry.description)
                .with_mime_type(entry.mime_type)
        })
        .collect()
}

/// The document for `uri`, or `None` when this server does not serve it.
/// Exact matching only — every URI is fixed, so there are no templates.
pub(crate) fn read(uri: &str) -> Option<ResourceContents> {
    CATALOG.iter().find(|entry| entry.uri == uri).map(|entry| {
        ResourceContents::text(entry.content, entry.uri).with_mime_type(entry.mime_type)
    })
}

#[cfg(test)]
#[path = "resources_tests.rs"]
mod tests;
