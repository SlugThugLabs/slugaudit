//! Rust-specific import resolution: mapping `crate::`/`super::`/`self::`
//! use-paths onto real files in the project.
//!
//! What this module models: Cargo workspaces (each crate anchors its own
//! `crate::`), grouped/nested `use` trees (via `rust_use_tree`), `#[path]`
//! parent hops (via `rust_mod_parents`), glob imports (`use super::*;`),
//! non-`mod.rs` module trees, and trailing item names.

use super::reference::ImportReference;
use super::resolve::parent_dir;
use super::resolver::{LanguageResolver, Resolution, ResolutionKind, external, pick};
use super::rust_mod_parents::{RustModuleParents, resolve_super_with_parents};
use super::rust_paths::{
    resolve_rust_from_base, resolve_rust_module_file, rust_module_dir, strip_trailing_glob,
};
use super::rust_use_tree::parse_rust_import;
use std::collections::HashSet;

/// Rust-specific import resolver. Handles `crate::`/`super::`/`self::`
/// semantics that the generic resolver can't model.
pub(super) struct RustResolver;

impl LanguageResolver for RustResolver {
    fn supports(&self, language: &str) -> bool {
        language == "rust"
    }

    fn extract_reference(&self, raw: &str) -> Option<ImportReference> {
        self.extract_references(raw).into_iter().next()
    }

    fn extract_references(&self, raw: &str) -> Vec<ImportReference> {
        let Some(targets) = parse_rust_import(raw) else {
            return Vec::new();
        };
        targets
            .into_iter()
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
        known_paths: &HashSet<&str>,
    ) -> Resolution {
        resolve_with_parents(
            reference,
            importing_relative_path,
            known_paths,
            &RustModuleParents::new(),
        )
    }

    fn resolve_with_rust_parents(
        &self,
        reference: &ImportReference,
        importing_relative_path: &str,
        known_paths: &HashSet<&str>,
        rust_parents: &RustModuleParents,
    ) -> Resolution {
        resolve_with_parents(
            reference,
            importing_relative_path,
            known_paths,
            rust_parents,
        )
    }
}

/// Finds the `src` directory of the crate that owns `importing_relative_path`
/// by walking up to the nearest ancestor holding a `Cargo.toml`.
fn crate_src_dir(importing_relative_path: &str, known_paths: &HashSet<&str>) -> String {
    let mut dir = parent_dir(importing_relative_path);
    loop {
        let manifest = if dir.is_empty() {
            "Cargo.toml".to_owned()
        } else {
            format!("{dir}/Cargo.toml")
        };
        if known_paths.contains(manifest.as_str()) {
            return if dir.is_empty() {
                "src".to_owned()
            } else {
                format!("{dir}/src")
            };
        }
        if dir.is_empty() {
            return "src".to_owned();
        }
        dir = parent_dir(&dir);
    }
}

fn resolve_self(
    text: &str,
    importing_relative_path: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    let rest = match text.split_once("::") {
        Some((_, rest)) => rest,
        None => "",
    };
    let strict = rust_module_dir(importing_relative_path);
    let legacy = parent_dir(importing_relative_path);
    let from_strict = resolve_rust_from_base(rest, &strict, known_paths, Some("Low"));
    if from_strict.kind == ResolutionKind::Resolved {
        return from_strict;
    }
    let from_legacy = resolve_rust_from_base(rest, &legacy, known_paths, Some("Low"));
    if from_legacy.kind == ResolutionKind::Resolved {
        return from_legacy;
    }
    resolve_rust_module_file(&strict, known_paths, Some("Low"))
}

fn resolve_with_parents(
    reference: &ImportReference,
    importing_relative_path: &str,
    known_paths: &HashSet<&str>,
    rust_parents: &RustModuleParents,
) -> Resolution {
    let text = reference.text.trim();

    if text.starts_with("::") {
        return external();
    }

    if text == "crate" {
        let base = crate_src_dir(importing_relative_path, known_paths);
        return pick(
            &[format!("{base}/lib.rs"), format!("{base}/main.rs")],
            known_paths,
        );
    }
    if let Some(rest) = text.strip_prefix("crate::") {
        let base = crate_src_dir(importing_relative_path, known_paths);
        if strip_trailing_glob(rest).is_empty() {
            return pick(
                &[format!("{base}/lib.rs"), format!("{base}/main.rs")],
                known_paths,
            );
        }
        let resolved = resolve_rust_from_base(rest, &base, known_paths, None);
        if resolved.kind == ResolutionKind::Resolved {
            return resolved;
        }
        return resolve_rust_module_file(&base, known_paths, Some("Low"));
    }

    if text == "self" || text.starts_with("self::") {
        return resolve_self(text, importing_relative_path, known_paths);
    }

    if text == "super" || text.starts_with("super::") {
        return resolve_super_with_parents(
            text,
            importing_relative_path,
            known_paths,
            rust_parents,
        );
    }

    let local = resolve_rust_from_base(
        text,
        &rust_module_dir(importing_relative_path),
        known_paths,
        Some("Low"),
    );
    if local.kind == ResolutionKind::Resolved {
        return local;
    }
    external()
}

#[cfg(test)]
#[path = "resolve_rust_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "resolve_rust_path_attr_tests.rs"]
mod path_attr_tests;
