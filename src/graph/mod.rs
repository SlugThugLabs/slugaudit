//! Resolves captured `Import` evidence into `dependency_edges` rows:
//! import → real project file, external dependency, or genuinely
//! unresolved. Reachable only from `sync::revision::publish_revision`,
//! the same single write path every other table goes through — this
//! module owns resolution logic, not persistence.
mod reference;
mod resolve;
mod resolve_rust;
pub(crate) mod resolver;

#[cfg(test)]
pub(crate) mod test_support;

use std::collections::HashSet;

/// True if a language-specific resolver (not the generic fallback) is
/// registered for `language`. The fallback additionally models relative
/// paths, module paths, and the `import`/`from`/`using`/`use`/`open`/
/// `#include`/quoted forms for every other language, so a `false` here
/// does not mean imports are ignored. Callers that need to tell "this
/// import's syntax isn't modeled" apart from "this import is genuinely
/// broken" check the resolver per edge instead of this list (see the
/// `report` tool).
#[must_use]
pub fn is_supported_language(language: &str) -> bool {
    resolver::is_supported_language(language)
}

/// One import evidence item's resolution outcome, ready to become a
/// `dependency_edges` row. `to_relative_path` is `None` unless
/// `resolution_kind` is `"Resolved"`.
pub struct ResolvedEdge {
    pub raw_import_text: String,
    pub resolution_kind: &'static str,
    pub confidence: Option<&'static str>,
    pub to_relative_path: Option<String>,
}

/// Resolves every raw import source captured for one file. `language` is
/// the file's detected language (as stored in `files.language`);
/// `import_sources` are the verbatim `source` strings from that file's
/// `Import` evidence payloads — the *entire raw statement text* the pack
/// captures, not a clean path (see `reference` for why that matters).
/// `known_paths` is every path that will exist in the project once this
/// revision commits, so a same-revision addition resolves correctly.
///
/// Every source produces exactly one `ResolvedEdge` — nothing is dropped
/// because it couldn't be parsed or resolved; it becomes `"Unresolved"`
/// instead, preserving the raw text for the AI to inspect.
#[must_use]
#[allow(clippy::implicit_hasher)]
pub fn resolve_imports(
    language: &str,
    importing_relative_path: &str,
    import_sources: &[String],
    known_paths: &HashSet<&str>,
) -> Vec<ResolvedEdge> {
    import_sources
        .iter()
        .map(|raw| resolve_one(language, importing_relative_path, raw, known_paths))
        .collect()
}

fn resolve_one(
    language: &str,
    importing_relative_path: &str,
    raw: &str,
    known_paths: &HashSet<&str>,
) -> ResolvedEdge {
    let resolution = resolver::resolve_one(language, importing_relative_path, raw, known_paths);
    ResolvedEdge {
        raw_import_text: raw.to_owned(),
        resolution_kind: resolution.kind.as_sql_text(),
        confidence: resolution.confidence,
        to_relative_path: resolution.to_relative_path,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
