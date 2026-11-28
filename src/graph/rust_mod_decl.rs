//! Out-of-line Rust `mod <name>;` declaration path resolution (`#[path]`, `r#`
//! raw identifiers, `.rs` / `/mod.rs` module layouts, crate-root targets, and
//! ambiguous dual-file `Low` confidence tagging).

use super::resolver::{self, KnownPaths};
use super::rust_path_attr::{join_inline_segments, resolve_rust_path_attr_segments_in_context};
use super::rust_paths::effective_rust_module_dir;
use super::{ResolvedEdge, WorkspaceCrates};
use std::collections::HashSet;
use std::hash::BuildHasher;
use std::path::Path;

/// Resolves an out-of-line Rust `mod <name>;` declaration to its target file.
#[must_use]
pub(crate) fn resolve_rust_mod_declaration<S: BuildHasher>(
    declaring_relative_path: &str,
    mod_name: &str,
    path_attr: Option<&str>,
    inline_segments: &[String],
    known_paths: &HashSet<&str, S>,
    workspace_crates: &WorkspaceCrates,
) -> ResolvedEdge {
    resolve_rust_mod_declaration_dyn(
        declaring_relative_path,
        mod_name,
        path_attr,
        inline_segments,
        known_paths,
        workspace_crates,
    )
}

fn resolve_rust_mod_declaration_dyn(
    declaring_relative_path: &str,
    mod_name: &str,
    path_attr: Option<&str>,
    inline_segments: &[String],
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> ResolvedEdge {
    let candidates = match path_attr {
        Some(attr) => path_attr_candidates(
            declaring_relative_path,
            attr,
            inline_segments,
            known_paths,
            workspace_crates,
        ),
        None => named_mod_candidates(
            declaring_relative_path,
            mod_name,
            inline_segments,
            known_paths,
            workspace_crates,
        ),
    };
    let resolution = resolver::pick(&candidates, known_paths);
    ResolvedEdge {
        raw_import_text: format!("mod {mod_name};"),
        resolution_kind: resolution.kind.as_sql_text(),
        confidence: resolution.confidence,
        to_relative_path: resolution.to_relative_path,
        syntax_unmodeled: false,
    }
}

fn path_attr_candidates(
    declaring_relative_path: &str,
    attr: &str,
    inline_segments: &[String],
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> Vec<String> {
    let Some(child_path) = resolve_rust_path_attr_segments_in_context(
        declaring_relative_path,
        attr,
        inline_segments,
        known_paths,
        workspace_crates,
    ) else {
        return Vec::new();
    };
    if Path::new(attr).extension().is_some() {
        return vec![child_path];
    }
    vec![
        format!("{child_path}.rs"),
        format!("{child_path}/mod.rs"),
        child_path,
    ]
}

fn named_mod_candidates(
    declaring_relative_path: &str,
    mod_name: &str,
    inline_segments: &[String],
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> Vec<String> {
    let trimmed = mod_name.trim();
    let clean_name = match trimmed.strip_prefix("r#") {
        Some(stripped) => stripped,
        None => trimmed,
    };
    if clean_name.is_empty() || clean_name == "." || clean_name == ".." || clean_name.contains('/')
    {
        return Vec::new();
    }
    let mod_dir = effective_rust_module_dir(declaring_relative_path, known_paths, workspace_crates);
    let Some(base_dir) = join_inline_segments(&mod_dir, inline_segments.iter().map(String::as_str))
    else {
        return Vec::new();
    };
    let Some(joined) = resolver::normalize_join(&base_dir, clean_name) else {
        return Vec::new();
    };
    vec![format!("{joined}.rs"), format!("{joined}/mod.rs")]
}

#[cfg(test)]
#[path = "rust_mod_decl_tests.rs"]
mod tests;
