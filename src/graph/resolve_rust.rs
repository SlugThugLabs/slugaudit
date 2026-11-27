// slugaudit-line-exception: approved-by=split-judge; reason=Rust import resolver covering crate/self/super/local/workspace paths and #[path] super parent traversal
//! Rust-specific import resolution: mapping `crate::`/`super::`/`self::`,
//! local 2018-edition module paths, and workspace sibling crate `use` paths
//! onto real files in the project.

use super::resolver::{
    KnownPaths, LanguageResolver, Resolution, ResolutionKind, external, parent_dir, pick,
    unresolved,
};
use super::rust_crate_anchor::crate_anchor;
use super::rust_mod_parents::{RustModuleParents, SuperParentWalk, walk_super_parents};
use super::rust_paths::{
    effective_rust_module_dir, first_resolved, resolve_rust_from_base, resolve_rust_module_file,
    resolve_workspace_crate_segments, strip_trailing_glob,
};
use super::rust_use_tree::parse_rust_import;
use super::{ImportReference, WorkspaceCrates};
use std::path::Path;

/// Rust-specific import resolver.
pub(super) struct RustResolver;

impl LanguageResolver for RustResolver {
    fn supports(&self, language: &str) -> bool {
        language == "rust"
    }

    fn extract_reference(&self, raw: &str) -> Option<ImportReference> {
        self.extract_references(raw).into_iter().next()
    }

    fn extract_references(&self, raw: &str) -> Vec<ImportReference> {
        parse_rust_import(raw)
            .into_iter()
            .flatten()
            .map(|text| ImportReference { text })
            .collect()
    }

    fn is_modeled_syntax(&self, raw: &str) -> bool {
        parse_rust_import(raw).is_some()
    }

    fn resolve(
        &self,
        reference: &ImportReference,
        importing_relative_path: &str,
        known_paths: &dyn KnownPaths,
    ) -> Resolution {
        resolve_full(
            reference,
            importing_relative_path,
            known_paths,
            &RustModuleParents::new(),
            &WorkspaceCrates::new(),
        )
    }

    fn resolve_with_rust_context(
        &self,
        reference: &ImportReference,
        importing_relative_path: &str,
        known_paths: &dyn KnownPaths,
        rust_parents: &RustModuleParents,
        workspace_crates: &WorkspaceCrates,
    ) -> Resolution {
        resolve_full(
            reference,
            importing_relative_path,
            known_paths,
            rust_parents,
            workspace_crates,
        )
    }
}

fn resolve_crate(
    rest: Option<&str>,
    importing_relative_path: &str,
    known_paths: &dyn KnownPaths,
    rust_parents: &RustModuleParents,
    workspace_crates: &WorkspaceCrates,
) -> Resolution {
    let anchor = crate_anchor(
        importing_relative_path,
        known_paths,
        rust_parents,
        workspace_crates,
    );
    let Some(segments) = rest.map(strip_trailing_glob).filter(|s| !s.is_empty()) else {
        return pick(&anchor.root_files, known_paths);
    };
    let resolved = resolve_rust_from_base(segments, &anchor.base_dir, known_paths, None);
    if resolved.kind == ResolutionKind::Resolved {
        return resolved;
    }
    let fallback = pick(&anchor.root_files, known_paths);
    match fallback.kind {
        ResolutionKind::Resolved => Resolution {
            confidence: Some("Low"),
            ..fallback
        },
        _ => fallback,
    }
}

fn resolve_self(
    text: &str,
    importing_relative_path: &str,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> Resolution {
    let rest = text.split_once("::").map_or("", |(_, after)| after);
    let strict = effective_rust_module_dir(importing_relative_path, known_paths, workspace_crates);
    for base in [&strict, &parent_dir(importing_relative_path)] {
        let candidate = resolve_rust_from_base(rest, base, known_paths, Some("Low"));
        if candidate.kind == ResolutionKind::Resolved {
            return candidate;
        }
    }
    resolve_rust_module_file(&strict, known_paths, Some("Low"))
}

fn resolve_super_with_parents(
    text: &str,
    importing_relative_path: &str,
    known_paths: &dyn KnownPaths,
    rust_parents: &RustModuleParents,
    workspace_crates: &WorkspaceCrates,
) -> Resolution {
    let Some((supers, rest)) = split_leading_supers(text) else {
        return unresolved();
    };
    match walk_super_parents(importing_relative_path, supers, rust_parents) {
        None => unresolved(),
        Some(SuperParentWalk::InParent {
            declaring_file,
            base_dir,
        }) => resolve_under_base_or_declaring(declaring_file, &base_dir, rest, known_paths),
        Some(SuperParentWalk::Remaining { file, supers }) => {
            resolve_super_in_module_tree(file, supers, rest, known_paths, workspace_crates)
        }
    }
}

fn resolve_super_in_module_tree(
    current_file: &str,
    remaining_supers: usize,
    rest: &str,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> Resolution {
    let mut strict = effective_rust_module_dir(current_file, known_paths, workspace_crates);
    let mut legacy = parent_dir(current_file);
    for _ in 0..remaining_supers {
        strict = parent_dir(&strict);
        legacy = parent_dir(&legacy);
    }
    let keep_legacy =
        !legacy.is_empty() && legacy != strict && !is_multi_target_boundary(&strict, &legacy);
    let mut bases = vec![strict];
    if keep_legacy {
        bases.push(legacy);
    }
    first_resolved(rest, &bases, known_paths)
}

fn is_multi_target_boundary(strict: &str, legacy: &str) -> bool {
    if Path::new(strict).ends_with("src") {
        return true;
    }
    let legacy_path = Path::new(legacy);
    ["tests", "benches", "examples", "src/bin"]
        .iter()
        .any(|dir| legacy_path.ends_with(dir))
}

fn split_leading_supers(text: &str) -> Option<(usize, &str)> {
    let mut count: usize = 1;
    let mut rest = text.split_once("::").map_or("", |(_, after)| after);
    while let Some(next) = rest.strip_prefix("super::") {
        rest = next;
        count = count.checked_add(1)?;
    }
    if rest == "super" {
        rest = "";
        count = count.checked_add(1)?;
    }
    Some((count, rest))
}

fn resolve_under_base_or_declaring(
    declaring_file: &str,
    base: &str,
    rest: &str,
    known_paths: &dyn KnownPaths,
) -> Resolution {
    if !strip_trailing_glob(rest).is_empty() {
        let sub = resolve_rust_from_base(rest, base, known_paths, None);
        if sub.kind == ResolutionKind::Resolved {
            return sub;
        }
    }
    resolved_declaring_file(declaring_file, rest, known_paths)
}

fn resolved_declaring_file(
    declaring_file: &str,
    rest: &str,
    known_paths: &dyn KnownPaths,
) -> Resolution {
    if !known_paths.contains_path(declaring_file) {
        return unresolved();
    }
    let exact_module = strip_trailing_glob(rest).is_empty();
    Resolution {
        kind: ResolutionKind::Resolved,
        confidence: Some(if exact_module { "High" } else { "Low" }),
        to_relative_path: Some(declaring_file.to_owned()),
    }
}

fn resolve_lib_entry(
    lib_entry: &str,
    confidence: &'static str,
    known_paths: &dyn KnownPaths,
) -> Resolution {
    if known_paths.contains_path(lib_entry) {
        Resolution {
            kind: ResolutionKind::Resolved,
            confidence: Some(confidence),
            to_relative_path: Some(lib_entry.to_owned()),
        }
    } else {
        unresolved()
    }
}

fn resolve_workspace_crate(
    specifier: &str,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> Option<Resolution> {
    if workspace_crates.is_empty() {
        return None;
    }
    let (first_seg, rest) = match specifier.split_once("::") {
        Some((first, after)) => (first.trim(), after.trim()),
        None => (specifier.trim(), ""),
    };
    let crate_ident = match first_seg.strip_prefix("r#") {
        Some(stripped) => stripped,
        None => first_seg,
    };
    let lib_entry = workspace_crates.get(crate_ident)?;
    let trimmed_rest = strip_trailing_glob(rest);
    if trimmed_rest.is_empty() {
        return Some(resolve_lib_entry(lib_entry, "High", known_paths));
    }
    let sub = resolve_workspace_crate_segments(trimmed_rest, &parent_dir(lib_entry), known_paths);
    if sub.kind == ResolutionKind::Resolved {
        return Some(sub);
    }
    Some(resolve_lib_entry(lib_entry, "Low", known_paths))
}

fn resolve_full(
    reference: &ImportReference,
    importing_relative_path: &str,
    known_paths: &dyn KnownPaths,
    rust_parents: &RustModuleParents,
    workspace_crates: &WorkspaceCrates,
) -> Resolution {
    let text = reference.text.trim();

    if let Some(global_rest) = text.strip_prefix("::") {
        return match resolve_workspace_crate(global_rest, known_paths, workspace_crates) {
            Some(ws_res) => ws_res,
            None => external(),
        };
    }

    if text == "crate" || text.starts_with("crate::") {
        return resolve_crate(
            text.strip_prefix("crate::"),
            importing_relative_path,
            known_paths,
            rust_parents,
            workspace_crates,
        );
    }

    if text == "self" || text.starts_with("self::") {
        return resolve_self(text, importing_relative_path, known_paths, workspace_crates);
    }

    if text == "super" || text.starts_with("super::") {
        return resolve_super_with_parents(
            text,
            importing_relative_path,
            known_paths,
            rust_parents,
            workspace_crates,
        );
    }

    let local = resolve_rust_from_base(
        text,
        &effective_rust_module_dir(importing_relative_path, known_paths, workspace_crates),
        known_paths,
        Some("Low"),
    );
    if local.kind == ResolutionKind::Resolved {
        return local;
    }
    match resolve_workspace_crate(text, known_paths, workspace_crates) {
        Some(ws_res) => ws_res,
        None => external(),
    }
}

#[cfg(test)]
#[path = "resolve_rust_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "resolve_rust_path_attr_tests.rs"]
mod path_attr_tests;
