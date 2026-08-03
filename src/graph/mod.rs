//! Resolves captured `Import` evidence into `dependency_edges` rows:
//! import → real project file, external dependency, or genuinely
//! unresolved. Reachable only from `sync::revision::publish_revision`,
//! the same single write path every other table goes through — this
//! module owns resolution logic, not persistence.
mod reference;
mod resolve;

use std::collections::HashSet;

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
// Generalizing over `S: BuildHasher` here would ripple through resolve_one
// and every per-language resolver in `resolve.rs` for no real benefit —
// the only caller (`sync::revision_edges`) always builds a plain
// `HashSet<&str>` from a `Vec<String>` of file paths.
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
    let Some(reference) = reference::extract(language, raw) else {
        return ResolvedEdge {
            raw_import_text: raw.to_owned(),
            resolution_kind: "Unresolved",
            confidence: None,
            to_relative_path: None,
        };
    };
    let resolution = resolve::resolve(language, &reference, importing_relative_path, known_paths);
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
