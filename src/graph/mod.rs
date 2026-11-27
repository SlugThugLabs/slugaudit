//! Resolves captured `Import` and Rust `mod` evidence into `dependency_edges`
//! rows: import → real project file, external dependency, or genuinely
//! unresolved. Reachable from `sync::revision::publish_revision`, the single
//! write path every table goes through — this module owns resolution logic,
//! not persistence.

mod cargo_workspace;
mod resolve_rust;
pub(crate) mod resolver;
mod rust_crate_anchor;
mod rust_mod_decl;
pub(crate) mod rust_mod_parents;
mod rust_paths;
pub(crate) mod rust_use_tree;

pub(crate) use cargo_workspace::{WorkspaceCrates, build_workspace_crates};
pub(crate) use rust_mod_decl::resolve_rust_mod_declaration;
pub(crate) use rust_mod_parents::{RustModuleParents, insert_rust_mod_parent_segments};
use std::collections::HashSet;
use std::hash::BuildHasher;

/// Shared fixture helper for `graph` tests.
#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::HashSet;

    /// Builds the `HashSet<&str>` of known project paths for `resolve_*` calls.
    pub(crate) fn paths<'a>(list: &[&'a str]) -> HashSet<&'a str> {
        list.iter().copied().collect()
    }
}

/// A module/path reference as written in source, stripped of its surrounding
/// statement syntax (`"./utils"`, `"crate::foo::bar"`, `".bar"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportReference {
    pub text: String,
}

/// One import or `mod` evidence item's resolution outcome, ready to become a
/// `dependency_edges` row. `to_relative_path` is `None` unless
/// `resolution_kind` is `"Resolved"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedEdge {
    /// Verbatim import or `mod` statement text taken from the evidence payload.
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
/// `#[path]` parent metadata and empty workspace crate mappings.
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
#[cfg(test)]
#[must_use]
pub(crate) fn resolve_imports_with_rust_parents<S: BuildHasher>(
    language: &str,
    importing_relative_path: &str,
    import_sources: &[String],
    known_paths: &HashSet<&str, S>,
    rust_parents: &RustModuleParents,
) -> Vec<ResolvedEdge> {
    resolve_imports_full(
        language,
        importing_relative_path,
        import_sources,
        known_paths,
        rust_parents,
        &WorkspaceCrates::new(),
    )
}

/// Resolves every raw import source captured for one file using `rust_parents`
/// and Cargo `workspace_crates`.
///
/// Grouped imports (such as Rust `use crate::{a, b};`) expand into one
/// deduplicated [`ResolvedEdge`] per distinct target; valid empty-brace `use`
/// statements emit zero edges, while unmodeled statements emit one
/// `"Unresolved"` edge with `syntax_unmodeled = true`.
#[must_use]
pub(crate) fn resolve_imports_full<S: BuildHasher>(
    language: &str,
    importing_relative_path: &str,
    raw_imports: &[String],
    known_paths: &HashSet<&str, S>,
    rust_parents: &RustModuleParents,
    workspace_crates: &WorkspaceCrates,
) -> Vec<ResolvedEdge> {
    resolve_imports_dyn(
        language,
        importing_relative_path,
        raw_imports,
        known_paths,
        rust_parents,
        workspace_crates,
    )
}

fn resolve_imports_dyn(
    language: &str,
    importing_relative_path: &str,
    raw_imports: &[String],
    known_paths: &dyn resolver::KnownPaths,
    rust_parents: &RustModuleParents,
    workspace_crates: &WorkspaceCrates,
) -> Vec<ResolvedEdge> {
    let mut edges = Vec::new();
    for raw in raw_imports {
        let resolved = resolver::resolve_many(
            language,
            importing_relative_path,
            raw,
            known_paths,
            rust_parents,
            workspace_crates,
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

#[cfg(test)]
#[path = "reference_tests.rs"]
mod reference_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;
