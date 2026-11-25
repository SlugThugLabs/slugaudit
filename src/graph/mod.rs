//! Resolves captured `Import` evidence into `dependency_edges` rows:
//! import → real project file, external dependency, or genuinely
//! unresolved. Reachable only from `sync::revision::publish_revision`,
//! the same single write path every other table goes through — this
//! module owns resolution logic, not persistence.
mod reference;
mod resolve;
mod resolve_rust;
pub(crate) mod resolver;
pub(crate) mod rust_mod_parents;
mod rust_paths;
mod rust_use_tree;

#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use rust_mod_parents::{
    RustModuleParents, insert_rust_mod_parent, insert_rust_mod_parent_segments,
};
use std::collections::HashSet;

/// One import evidence item's resolution outcome, ready to become a
/// `dependency_edges` row. `to_relative_path` is `None` unless
/// `resolution_kind` is `"Resolved"`.
pub struct ResolvedEdge {
    /// Verbatim import statement text taken from the evidence payload.
    pub raw_import_text: String,
    /// Resolution outcome as stored in `dependency_edges.resolution_kind`
    /// (`Resolved`, `External`, or `Unresolved`).
    pub resolution_kind: &'static str,
    /// Resolver confidence tag (`High` or `Low`) when the resolver reports one.
    pub confidence: Option<&'static str>,
    /// Project-relative target path; `None` unless `resolution_kind` is
    /// `Resolved`.
    pub to_relative_path: Option<String>,
    /// Whether the statement syntax was not modeled by the language resolver.
    pub syntax_unmodeled: bool,
}

/// Resolves every raw import source captured for one file using empty Rust
/// `#[path]` parent metadata.
#[cfg(test)]
#[must_use]
pub(crate) fn resolve_imports(
    language: &str,
    importing_relative_path: &str,
    import_sources: &[String],
    known_paths: &HashSet<&str>,
) -> Vec<ResolvedEdge> {
    resolve_imports_with_rust_parents(
        language,
        importing_relative_path,
        import_sources,
        known_paths,
        &RustModuleParents::new(),
    )
}

/// Resolves every raw import source captured for one file, using `rust_parents`
/// to resolve `super::` imports inside `#[path = "..."]` files.
///
/// Grouped imports (such as Rust `use crate::{a, b};`) expand into one
/// deduplicated `ResolvedEdge` per distinct target; valid empty-brace `use`
/// statements emit zero edges, while unmodeled statements emit one
/// `"Unresolved"` edge with `syntax_unmodeled = true`.
#[must_use]
pub(crate) fn resolve_imports_with_rust_parents(
    language: &str,
    importing_relative_path: &str,
    import_sources: &[String],
    known_paths: &HashSet<&str>,
    rust_parents: &RustModuleParents,
) -> Vec<ResolvedEdge> {
    let mut edges = Vec::new();
    for raw in import_sources {
        let resolved = resolver::resolve_many(
            language,
            importing_relative_path,
            raw,
            known_paths,
            rust_parents,
        );
        for outcome in resolved {
            edges.push(ResolvedEdge {
                raw_import_text: raw.to_owned(),
                resolution_kind: outcome.resolution.kind.as_sql_text(),
                confidence: outcome.resolution.confidence,
                to_relative_path: outcome.resolution.to_relative_path,
                syntax_unmodeled: outcome.syntax_unmodeled,
            });
        }
    }
    edges
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
