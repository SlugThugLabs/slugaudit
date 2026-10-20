//! Rust-specific import resolution: mapping `crate::`/`super::`/`self::`
//! use-paths onto real files in the project.
//!
//! Rust's module tree does not map one-to-one onto the filesystem, and we
//! deliberately resolve it without reading `mod` declarations — so every
//! `super::`/`self::` verdict is `"Low"` confidence. What this module does
//! model correctly: Cargo workspaces (each crate anchors its own `crate::`),
//! glob imports (`use super::*;` points at the globbed module's file),
//! non-`mod.rs` module trees, and trailing item names that live in a
//! module's own file rather than a nested one.
//!
//! The path-shape helpers (module files, segment shortening) live in
//! `super::rust_paths`; this file owns the import-form dispatch and the
//! workspace crate anchor.

use super::reference::ImportReference;
use super::resolve::parent_dir;
use super::resolver::{LanguageResolver, Resolution, external, pick};
use super::rust_paths::{first_resolved, resolve_rust_from_base, strip_trailing_glob};
use std::collections::HashSet;
use std::path::Path;

/// File stems that mean "this file *is* its module" rather than "this file
/// *defines* a child module named after itself". For `src/a/mod.rs` the
/// module is `a`; for `src/a/b.rs` the module is `a::b`. Getting this
/// distinction right is what makes `super::`/`self::` resolve correctly in
/// ordinary (non-`mod.rs`) module trees.
const RUST_MODULE_ROOT_STEMS: [&str; 3] = ["mod", "lib", "main"];

/// Rust-specific import resolver. Handles `crate::`/`super::`/`self::`
/// semantics that the generic resolver can't model.
pub(super) struct RustResolver;

impl LanguageResolver for RustResolver {
    fn supports(&self, language: &str) -> bool {
        language == "rust"
    }

    fn extract_reference(&self, raw: &str) -> Option<ImportReference> {
        let without_visibility = raw
            .trim()
            .trim_start_matches("pub(crate) ")
            .trim_start_matches("pub(super) ")
            .trim_start_matches("pub(self) ")
            .trim_start_matches("pub ");
        let rest = without_visibility.strip_prefix("use ")?;
        let end = rest.find([';', '{', ' ']).map_or(rest.len(), |index| index);
        let path = rest[..end].trim_end_matches("::");
        if path.is_empty() {
            None
        } else {
            Some(ImportReference {
                text: path.to_owned(),
            })
        }
    }

    fn resolve(
        &self,
        reference: &ImportReference,
        importing_relative_path: &str,
        known_paths: &HashSet<&str>,
    ) -> Resolution {
        resolve(reference, importing_relative_path, known_paths)
    }
}

/// The file stem, or the empty string for a path with none (`..`, `/`): such
/// a path owns no module, which is the correct answer, not a lost error.
fn file_stem(relative_path: &str) -> String {
    Path::new(relative_path)
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned())
}

/// The directory that a Rust file's *own* module owns — i.e. where its
/// child modules live. `src/a/mod.rs` owns `src/a`; `src/a/b.rs` owns
/// `src/a/b`.
fn rust_module_dir(relative_path: &str) -> String {
    let parent = parent_dir(relative_path);
    let stem = file_stem(relative_path);
    if RUST_MODULE_ROOT_STEMS.contains(&stem.as_str()) {
        parent
    } else if parent.is_empty() {
        stem
    } else {
        format!("{parent}/{stem}")
    }
}

/// Finds the `src` directory of the crate that owns `importing_relative_path`
/// by walking up to the nearest ancestor holding a `Cargo.toml`. A Cargo
/// workspace puts each crate under its own directory (`crates/api/src/…`),
/// so assuming a single top-level `src` makes every `crate::` import in
/// every non-root crate unresolvable. Falls back to `"src"` when no manifest
/// is indexed, which is the correct answer for a plain single-crate project.
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

/// Only `crate::`/`super::`/`self::` paths are project-internal; anything
/// else (`std::`, a third-party crate name) is external.
///
/// `crate::` is anchored at the owning crate's `src` (workspace-aware, see
/// [`crate_src_dir`]). `super::`/`self::` are resolved against the real
/// module tree first — `super::` from `src/a/b.rs` means module `a`, not
/// `src` — and fall back to the older parent-directory heuristic when the
/// strict interpretation finds nothing, so trees that genuinely are
/// `mod.rs`-shaped still resolve. Both remain `"Low"` confidence: without
/// reading `mod` declarations we cannot prove which interpretation the
/// compiler would pick.
fn resolve(
    reference: &ImportReference,
    importing_relative_path: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    let text = reference.text.trim();

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
        return resolve_rust_from_base(rest, &base, known_paths, None);
    }

    if text == "self" || text.starts_with("self::") {
        let rest = text.strip_prefix("self::").unwrap_or("");
        let strict = rust_module_dir(importing_relative_path);
        let legacy = parent_dir(importing_relative_path);
        return first_resolved(rest, &[strict, legacy], known_paths);
    }

    if text == "super" || text.starts_with("super::") {
        let mut rest = text.strip_prefix("super::").unwrap_or("");
        let mut strict = parent_dir(&rust_module_dir(importing_relative_path));
        let mut legacy = parent_dir(&parent_dir(importing_relative_path));
        while let Some(next) = rest.strip_prefix("super::") {
            rest = next;
            strict = parent_dir(&strict);
            legacy = parent_dir(&legacy);
        }
        if rest == "super" {
            rest = "";
            strict = parent_dir(&strict);
            legacy = parent_dir(&legacy);
        }
        return first_resolved(rest, &[strict, legacy], known_paths);
    }

    external()
}

#[cfg(test)]
#[path = "resolve_rust_tests.rs"]
mod tests;
