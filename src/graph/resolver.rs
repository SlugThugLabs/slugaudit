//! Language-agnostic import resolution framework.
//!
//! Most languages share common import patterns:
//! - Relative paths: `./foo`, `../bar`
//! - Module paths: `foo.bar.baz`
//! - Package names: `import foo` (third-party)
//!
//! The `GenericResolver` handles these common patterns. Languages with
//! unusual import semantics (e.g. Rust's `crate::`/`super::`/`self::`)
//! implement the `LanguageResolver` trait directly.

use std::collections::HashSet;
use std::sync::OnceLock;

use super::reference::ImportReference;

/// The outcome of resolving an import reference.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub kind: ResolutionKind,
    pub confidence: Option<&'static str>,
    pub to_relative_path: Option<String>,
}

/// Whether an import was resolved to a real file, identified as external,
/// or left unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionKind {
    Resolved,
    Unresolved,
    External,
}

impl ResolutionKind {
    pub fn as_sql_text(self) -> &'static str {
        match self {
            Self::Resolved => "Resolved",
            Self::Unresolved => "Unresolved",
            Self::External => "External",
        }
    }
}

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

/// Returns a resolution indicating the reference could not be resolved to
/// a project file.
pub fn unresolved() -> Resolution {
    Resolution {
        kind: ResolutionKind::Unresolved,
        confidence: None,
        to_relative_path: None,
    }
}

/// Returns a resolution indicating the reference is syntactically
/// identified as outside the project (standard library, third-party crate,
/// bare package name).
pub fn external() -> Resolution {
    Resolution {
        kind: ResolutionKind::External,
        confidence: None,
        to_relative_path: None,
    }
}

/// Picks the winning candidate out of a set of paths that would all
/// satisfy the reference: exactly one real match is `"High"` confidence,
/// more than one is `"Low"` (genuinely ambiguous — we still report the
/// first as a best guess rather than discarding the information).
pub fn pick(candidates: &[String], known_paths: &HashSet<&str>) -> Resolution {
    let matches: Vec<&String> = candidates
        .iter()
        .filter(|candidate| known_paths.contains(candidate.as_str()))
        .collect();
    match matches.len() {
        0 => unresolved(),
        1 => Resolution {
            kind: ResolutionKind::Resolved,
            confidence: Some("High"),
            to_relative_path: Some(matches[0].clone()),
        },
        _ => Resolution {
            kind: ResolutionKind::Resolved,
            confidence: Some("Low"),
            to_relative_path: Some(matches[0].clone()),
        },
    }
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
            // Common source extensions — the generic resolver tries all of
            // them and lets `known_paths` pick the real match. This covers
            // Python, JS/TS, Ruby, Go, and a few others out of the box.
            extensions: vec![
                "py", "js", "ts", "jsx", "tsx", "mjs", "cjs", "rb", "go",
                "rs", "java", "c", "cpp", "cc", "h", "hpp", "hh",
            ],
            index_filename: None,
            // `.` is the module separator for Python, Ruby, Java, etc.
            // JS/TS don't really use module paths (only relative `./`/`../`
            // and bare package names), so `.` works as a universal default.
            module_separator: ".",
            // `./` and `../` cover JS/TS relative imports. Python's `.`
            // prefix is handled by the fallback `text.starts_with('.')`
            // check in `resolve`, which routes to `resolve_python_relative`.
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
                // are handled by the `text.starts_with('.')` fallback in
                // `resolve`, which routes them to `resolve_python_relative`.
                // Including `.` as a prefix would incorrectly route them
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
        let trimmed = raw.trim();

        // Bare `.` is a Python-style relative import (current package).
        if trimmed == "." {
            return Some(ImportReference { text: ".".to_owned() });
        }

        // JS/TS relative: `./utils`, `../lib/helper`.
        if trimmed.starts_with("./") || trimmed.starts_with("../") {
            return Some(ImportReference { text: trimmed.to_owned() });
        }

        // Python relative: `.bar`, `..pkg.mod` — starts with `.` but not
        // `./` or `../` (which are JS/TS-style filesystem paths).
        if trimmed.starts_with('.') && !trimmed.starts_with("./") && !trimmed.starts_with("../") {
            return Some(ImportReference { text: trimmed.to_owned() });
        }

        // Python `from X import ...` → `X`.
        if let Some(rest) = trimmed.strip_prefix("from ") {
            let module = rest.split_whitespace().next()?;
            // Quoted module after `from` isn't valid Python — skip it so
            // we don't match Go's `import "fmt"` or similar.
            if !module.starts_with('"') && !module.starts_with('\'') {
                return Some(ImportReference {
                    text: module.to_owned(),
                });
            }
            return None;
        }

        // JS/TS `import ... from 'path'` — only extract quoted strings
        // when `from` is present to avoid matching Go's `import "fmt"`.
        if trimmed.contains("from") {
            if let Some(text) = extract_quoted_string(trimmed) {
                return Some(ImportReference { text });
            }
            // Has `from` but no quoted string → not a valid JS import.
            return None;
        }

        // Python `import X` / `import X as Y` → `X`.
        if let Some(rest) = trimmed.strip_prefix("import ") {
            let module = rest.split_whitespace().next()?;
            // Skip quoted imports (Go, etc.).
            if !module.starts_with('"') && !module.starts_with('\'') {
                return Some(ImportReference {
                    text: module.to_owned(),
                });
            }
        }

        // Bare package name (e.g. `react`, `os`, `numpy`). Only extract
        // if this resolver treats bare names as external (Python, JS,
        // etc.) — the resolution step will mark them as `External`.
        // Reject anything with spaces, quotes, or other statement syntax
        // to avoid matching full import statements like Go's `import "fmt"`.
        if self.config.bare_names_are_external
            && !trimmed.is_empty()
            && !trimmed.starts_with('.')
            && !trimmed.contains(self.config.module_separator)
            && !trimmed.contains(' ')
            && !trimmed.contains('"')
            && !trimmed.contains('\'')
        {
            return Some(ImportReference {
                text: trimmed.to_owned(),
            });
        }

        None
    }

    fn resolve(
        &self,
        reference: &ImportReference,
        importing_relative_path: &str,
        known_paths: &HashSet<&str>,
    ) -> Resolution {
        let text = reference.text.trim();

        // Check for relative import prefixes.
        for prefix in &self.config.relative_prefixes {
            if text.starts_with(prefix) {
                return self.resolve_relative(text, prefix, importing_relative_path, known_paths);
            }
        }

        // Python relative: starts with `.` but not `..` (handled above).
        if text.starts_with('.') {
            return self.resolve_python_relative(text, importing_relative_path, known_paths);
        }

        // Bare name with no separator — external package.
        if !text.contains(self.config.module_separator) {
            if self.config.bare_names_are_external {
                return external();
            }
            // Treat as module path.
            return self.resolve_module_path(text, importing_relative_path, known_paths);
        }

        // Module path like `foo.bar.baz`.
        self.resolve_module_path(text, importing_relative_path, known_paths)
    }
}

impl GenericResolver {
    fn resolve_relative(
        &self,
        text: &str,
        _prefix: &str,
        importing_relative_path: &str,
        known_paths: &HashSet<&str>,
    ) -> Resolution {
        let base_dir = super::resolve::parent_dir(importing_relative_path);
        let path = super::resolve::normalize_join(&base_dir, text);
        self.candidate_paths(&path, known_paths)
    }

    fn resolve_python_relative(
        &self,
        text: &str,
        importing_relative_path: &str,
        known_paths: &HashSet<&str>,
    ) -> Resolution {
        let dots = text.chars().take_while(|c| *c == '.').count();
        let remainder = &text[dots..];
        let levels_up = dots - 1;

        let mut base_dir = super::resolve::parent_dir(importing_relative_path);
        for _ in 0..levels_up {
            base_dir = super::resolve::parent_dir(&base_dir);
        }

        let path = if remainder.is_empty() {
            super::resolve::normalize_join(&base_dir, "")
        } else {
            super::resolve::normalize_join(&base_dir, &remainder.replace('.', "/"))
        };

        if remainder.is_empty() {
            // `from . import` → `__init__.py` in current package.
            if let Some(index) = self.config.index_filename {
                let init_path = format!("{}/{}.py", path, index);
                if known_paths.contains(init_path.as_str()) {
                    return Resolution {
                        kind: ResolutionKind::Resolved,
                        confidence: Some("High"),
                        to_relative_path: Some(init_path),
                    };
                }
            }
            unresolved()
        } else {
            self.candidate_paths(&path, known_paths)
        }
    }

    fn resolve_module_path(
        &self,
        text: &str,
        _importing_relative_path: &str,
        known_paths: &HashSet<&str>,
    ) -> Resolution {
        // Convert module path to filesystem path.
        let fs_path = if self.config.module_separator == "/" {
            text.to_owned()
        } else {
            text.replace(self.config.module_separator, "/")
        };

        self.candidate_paths(&fs_path, known_paths)
    }

    fn candidate_paths(&self, path: &str, known_paths: &HashSet<&str>) -> Resolution {
        let mut candidates = vec![path.to_owned()];

        // Try each extension.
        for ext in &self.config.extensions {
            candidates.push(format!("{}.{}", path, ext));
        }

        // Try index file in directory.
        if let Some(index) = self.config.index_filename {
            for ext in &self.config.extensions {
                candidates.push(format!("{}/{}.{}", path, index, ext));
            }
            // Also try index without extension.
            candidates.push(format!("{}/{}", path, index));
        }

        pick(&candidates, known_paths)
    }
}

/// Extracts a quoted string literal from text (for JS/TS imports).
fn extract_quoted_string(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let start = bytes
        .iter()
        .position(|byte| *byte == b'\'' || *byte == b'"')?;
    let quote = bytes[start];
    let end = bytes[start + 1..].iter().position(|byte| *byte == quote)? + start + 1;
    Some(text[start + 1..end].to_owned())
}

/// Registry of language-specific resolvers. Language-specific resolvers
/// (including configured `GenericResolver` instances for Python and JS)
/// are checked first; the catch-all generic resolver is the fallback for
/// any language without a specific resolver.
struct ResolverRegistry {
    specific_resolvers: Vec<Box<dyn LanguageResolver>>,
    fallback_resolver: GenericResolver,
}

impl ResolverRegistry {
    fn new() -> Self {
        Self {
            specific_resolvers: Vec::new(),
            fallback_resolver: GenericResolver::new(GenericResolverConfig::default()),
        }
    }

    fn register(&mut self, resolver: Box<dyn LanguageResolver>) {
        self.specific_resolvers.push(resolver);
    }

    fn get(&self, language: &str) -> &dyn LanguageResolver {
        // Try specific resolvers first (includes Python/JS generic resolvers).
        if let Some(resolver) = self
            .specific_resolvers
            .iter()
            .find(|r| r.supports(language))
        {
            return resolver.as_ref();
        }
        // Fall back to catch-all generic resolver.
        &self.fallback_resolver
    }
}

static REGISTRY: OnceLock<ResolverRegistry> = OnceLock::new();

fn registry() -> &'static ResolverRegistry {
    REGISTRY.get_or_init(|| {
        let mut reg = ResolverRegistry::new();
        // Register language-specific resolvers.
        // These take precedence over the catch-all generic resolver.
        reg.register(Box::new(super::resolve_rust::RustResolver));
        reg.register(Box::new(GenericResolver::python()));
        reg.register(Box::new(GenericResolver::js()));
        reg
    })
}

/// Returns the resolver for `language`. Falls back to the generic
/// resolver if no specific resolver supports the language.
pub fn get_resolver(language: &str) -> &'static dyn LanguageResolver {
    registry().get(language)
}

/// Returns true if a language-specific resolver (not the generic fallback)
/// supports `language`.
pub fn is_supported_language(language: &str) -> bool {
    registry()
        .specific_resolvers
        .iter()
        .any(|r| r.supports(language))
}

/// Resolves a single raw import source. Returns a `Resolution` — never
/// panics or returns an error.
pub fn resolve_one(
    language: &str,
    importing_relative_path: &str,
    raw: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    let resolver = get_resolver(language);

    let Some(reference) = resolver.extract_reference(raw) else {
        return unresolved();
    };

    resolver.resolve(&reference, importing_relative_path, known_paths)
}
