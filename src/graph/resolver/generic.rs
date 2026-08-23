//! The `LanguageResolver` trait, its configuration, and the generic
//! resolver implementation that most languages use.
//!
//! The outcome data model (`Resolution`/`ResolutionKind`/`pick`) lives in
//! [`super::types`]; the raw import-statement parser lives in
//! [`super::extract`]. This module owns the dispatcher contract and the
//! resolution step — the two halves of the runtime API.
// slugaudit-line-exception: approved-by=agent; reason=trait + config + resolver struct + resolve() are one cohesive dispatcher contract; the outcome model (types.rs) and raw-text extraction (extract.rs) are already split out

use std::collections::HashSet;

use super::extract::extract_reference;
use super::path_helpers::{
    candidate_paths, external_or_unresolved, module_path_to_fs_path, resolve_relative_path,
    starts_with_python_dot_prefix,
};
use super::python::resolve_python_relative;
use crate::graph::reference::ImportReference;
use crate::graph::resolver::types::{Resolution, external};

/// A language-specific import resolver.
///
/// Most languages are handled by the built-in `GenericResolver`. Implement
/// this trait only for languages with unusual import semantics that the
/// generic resolver can't handle (e.g. Rust's `crate::`/`super::`/`self::`).
pub trait LanguageResolver: Send + Sync {
    /// Returns true if this resolver handles the given language name.
    fn supports(&self, language: &str) -> bool;

    /// Extract the module/path reference from raw import statement text.
    ///
    /// Returns `None` if the text doesn't match any known import pattern
    /// for this language — callers treat that the same as an unparseable
    /// import, not an error.
    fn extract_reference(&self, raw: &str) -> Option<ImportReference>;

    /// Resolve a reference to a project file.
    ///
    /// Returns `Resolution::External` for third-party/standard library
    /// imports, `Resolution::Resolved` with a file path for project-internal
    /// imports, or `Resolution::Unresolved` when no candidate file exists.
    fn resolve(
        &self,
        reference: &ImportReference,
        importing_relative_path: &str,
        known_paths: &HashSet<&str>,
    ) -> Resolution;
}

/// Configuration for the generic import resolver.
///
/// Most languages can be resolved with these settings. Languages with
/// unusual import semantics (Rust, etc.) implement `LanguageResolver`
/// directly instead of using this config.
#[derive(Debug, Clone)]
pub struct GenericResolverConfig {
    /// File extensions to try when resolving a module path.
    /// E.g. `["py"]` for Python, `["js", "ts", "jsx", "tsx"]` for JS/TS.
    pub extensions: Vec<&'static str>,

    /// Filename to try when a path resolves to a directory.
    /// E.g. `"__init__"` for Python packages, `"index"` for Node modules.
    pub index_filename: Option<&'static str>,

    /// Path separator in module paths. `"."` for Python (`foo.bar`),
    /// `"/"` for languages that use filesystem paths directly.
    pub module_separator: &'static str,

    /// Prefixes that indicate a relative import.
    /// E.g. `["./", "../"]` for JS/TS, `["."]` for Python (`from . import`).
    pub relative_prefixes: Vec<&'static str>,

    /// Whether bare names (no prefix, no separator) are treated as
    /// external (third-party packages) rather than project-relative.
    /// True for JS/TS/Go, false for Python relative imports.
    pub bare_names_are_external: bool,
}

impl Default for GenericResolverConfig {
    fn default() -> Self {
        Self {
            // Common source extensions — the generic resolver tries all
            // of them and lets `known_paths` pick the real match. This
            // covers Python, JS/TS, Ruby, Go, and a few others out of
            // the box.
            extensions: vec![
                "py", "js", "ts", "jsx", "tsx", "mjs", "cjs", "rb", "go", "rs", "java", "c", "cpp",
                "cc", "h", "hpp", "hh", "kt", "kts", "swift", "cs", "dart", "scala", "sc", "hs",
                "lua", "php", "pl", "pm", "ex", "exs", "ml", "sol", "jl", "erl", "hrl",
            ],
            index_filename: None,
            // `.` is the module separator for Python, Ruby, Java, etc.
            // JS/TS don't really use module paths (only relative `./`/`../`
            // and bare package names), so `.` works as a universal default.
            module_separator: ".",
            // `./` and `../` cover JS/TS relative imports. Python's `.`
            // prefix is handled by `starts_with_python_dot_prefix` in
            // `path_helpers`, which routes it to
            // `super::python::resolve_python_relative`
            // instead of being a member of this list.
            relative_prefixes: vec!["./", "../"],
            bare_names_are_external: true,
        }
    }
}

/// Generic import resolver that handles common patterns across most
/// languages: relative paths, module paths, and package names.
///
/// Use this for languages whose import semantics fit the common patterns.
/// For languages with unusual import syntax (Rust's `crate::`, etc.),
/// implement `LanguageResolver` directly.
///
/// A `GenericResolver` can be configured for specific languages (via
/// `python()`, `js()`, etc.) or used as a catch-all fallback (via `new()`
/// with a custom config or `Default`). When `languages` is non-empty,
/// `supports` only returns true for those languages; when empty, it
/// returns true for any language (fallback mode).
pub struct GenericResolver {
    config: GenericResolverConfig,
    languages: Vec<&'static str>,
}

impl GenericResolver {
    pub fn new(config: GenericResolverConfig) -> Self {
        Self {
            config,
            languages: Vec::new(),
        }
    }

    pub fn python() -> Self {
        Self {
            config: GenericResolverConfig {
                extensions: vec!["py"],
                index_filename: Some("__init__"),
                module_separator: ".",
                // Don't include `.` here — Python's `.bar` style imports
                // are recognized by `starts_with_python_dot_prefix` in
                // `super::path_helpers` and routed to
                // `super::python::resolve_python_relative`. Including
                // `.` as a prefix here would incorrectly route them
                // through `resolve_relative`, which treats `.bar` as a
                // literal filename instead of a module path.
                relative_prefixes: vec![],
                bare_names_are_external: true,
            },
            languages: vec!["python", "python3"],
        }
    }

    pub fn js() -> Self {
        Self {
            config: GenericResolverConfig {
                extensions: vec!["js", "ts", "jsx", "tsx", "mjs", "cjs"],
                index_filename: Some("index"),
                module_separator: "/",
                relative_prefixes: vec!["./", "../"],
                bare_names_are_external: true,
            },
            languages: vec!["javascript", "typescript", "jsx", "tsx"],
        }
    }
}

impl LanguageResolver for GenericResolver {
    fn supports(&self, language: &str) -> bool {
        // Empty languages list means "supports everything" (fallback mode).
        self.languages.is_empty() || self.languages.contains(&language)
    }

    fn extract_reference(&self, raw: &str) -> Option<ImportReference> {
        extract_reference(&self.config, raw)
    }

    fn resolve(
        &self,
        reference: &ImportReference,
        importing_relative_path: &str,
        known_paths: &HashSet<&str>,
    ) -> Resolution {
        let text = reference.text.trim();

        // Check for relative import prefixes (JS/TS-style). The actual
        // `prefix` value isn't forwarded to `resolve_relative_path` —
        // the helper does its own `normalize_join` against `base_dir`,
        // and `prefix` only matters for the membership test.
        if self
            .config
            .relative_prefixes
            .iter()
            .any(|prefix| text.starts_with(prefix))
        {
            let base_dir = crate::graph::resolve::parent_dir(importing_relative_path);
            return resolve_relative_path(&base_dir, text, &self.config, known_paths);
        }

        // Python relative: starts with `.` but not `../` (handled
        // above) and not `./` (also handled above).
        if starts_with_python_dot_prefix(text) {
            return resolve_python_relative(
                text,
                &self.config,
                importing_relative_path,
                known_paths,
            );
        }

        // Dart `package:` / `dart:` URIs are external by definition
        // (pub.dev packages and the Dart built-in library).
        if text.starts_with("package:") || text.starts_with("dart:") {
            return external();
        }

        // Bare name with no separator — external package.
        if !text.contains(self.config.module_separator) {
            return external_or_unresolved(&self.config);
        }

        // Module path like `foo.bar.baz`.
        let fs_path = module_path_to_fs_path(text, &self.config);
        candidate_paths(&fs_path, &self.config, known_paths)
    }
}
