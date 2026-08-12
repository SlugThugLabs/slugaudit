//! Core types and the generic import resolver.
//!
//! The `Resolution` / `ResolutionKind` data model, the `LanguageResolver`
//! trait, and the `GenericResolver` struct — the runtime entry point
//! every language-specific resolver ultimately uses. The language-
//! specific helpers (`super::python`, `super::js`) and the small path
//! helpers (`super::path_helpers`) handle the line-count split; this
//! file stays coherent by owning only the dispatcher logic and the
//! `Resolution` machinery.
// slugaudit-line-exception: approved-by=agent; reason=core types + language-trait dispatcher + GenericResolver impl are one cohesive runtime contract; the per-language helpers live next to their respective resolvers

use std::collections::HashSet;

use super::js::extract_js_reference;
use super::path_helpers::{
    candidate_paths, external_or_unresolved, extract_quoted_string, module_path_to_fs_path,
    resolve_relative_path, starts_with_python_dot_prefix,
};
use super::python::resolve_python_relative;
use crate::graph::reference::ImportReference;

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
        let trimmed = raw.trim();

        // Bare `.` is a Python-style relative import (current package).
        if trimmed == "." {
            return Some(ImportReference {
                text: ".".to_owned(),
            });
        }

        // JS/TS relative: `./utils`, `../lib/helper`.
        if trimmed.starts_with("./") || trimmed.starts_with("../") {
            return Some(ImportReference {
                text: trimmed.to_owned(),
            });
        }

        // Python relative: `.bar`, `..pkg.mod` — starts with `.` but not
        // `./` or `../` (which are JS/TS-style filesystem paths).
        if starts_with_python_dot_prefix(trimmed) {
            return Some(ImportReference {
                text: trimmed.to_owned(),
            });
        }

        // Python `from X import ...` → `X`. Accepts any single whitespace
        // token (space or tab) between `from` and the module, matching
        // Python's grammar. Caught initially by the proptest in
        // `super::proptest`, which generated `from\tmod import\tbar` and
        // surfaced that the prior `"from "` literal-space strip was too
        // strict; `"fromfoo"` (no separator) must still be rejected, so
        // we explicitly check that the character after `from` is whitespace.
        if let Some(after_from) = trimmed.strip_prefix("from")
            && after_from.chars().next().is_some_and(char::is_whitespace)
        {
            let module = after_from.split_whitespace().next()?;
            // Quoted module after `from` isn't valid Python — skip it so
            // we don't match Go's `import "fmt"` or similar.
            if !module.starts_with('"') && !module.starts_with('\'') {
                return Some(ImportReference {
                    text: module.to_owned(),
                });
            }
            return None;
        }

        // JS/TS `import ... from 'path'` — extracted via the dedicated
        // language helper so the quote-gating lives next to its matchers.
        // Token-gate on the literal `from` keyword, not a substring match:
        // a Python (and several other languages') bare-module import whose
        // name happens to contain `from` — `import from_util`,
        // `import foo_bar.from_baz`, etc. — is legal, must reach the
        // generic `import X` branch below, and the prior substring gate
        // was shadowing it (silently returning `None` instead of the
        // module path). The token check uses whitespace boundaries, which
        // is the same `split_whitespace().any(…)` shape the bare-name
        // candidate check below uses — consistent with how we treat
        // "does this look like a keyword or a token?" elsewhere.
        //
        // When `from` IS a real keyword but the statement has no quoted
        // path (`import x from broken`), extraction returns `None` and
        // we deliberately drop here; falling through would let the
        // generic `import X` branch lift the impossible `x` out as a
        // bare module name and call it External, which is worse than
        // honest `None` (it would mislabel unresolved syntax as a
        // third-party import).
        if trimmed.split_whitespace().any(|token| token == "from")
            && let Some(reference) = extract_js_reference(trimmed)
        {
            return Some(reference);
        }
        // If `from` was a real keyword but nothing was quoted, drop —
        // continuing would silently mislabel the next token as External.
        if trimmed.split_whitespace().any(|token| token == "from") {
            return None;
        }

        // Dart/Solidity-style quoted imports: `import '../util.dart';` or
        // `import "./Ownable.sol";`. Only explicit relative paths are
        // extracted — so Go's `import "fmt"` and bare-package imports
        // stay `Unresolved` exactly as before. Also lifts JS/TS side-effect
        // imports (`import './a.js';`) out of `Unresolved` into real
        // relative resolution, which the Python branch below never reached.
        if trimmed.starts_with("import")
            && let Some(quoted) = extract_quoted_string(trimmed)
        {
            // Relative paths resolve against the importing file.
            if quoted.starts_with("./") || quoted.starts_with("../") {
                return Some(ImportReference { text: quoted });
            }
            // Dart `package:`/`dart:` URIs are external by definition.
            if quoted.starts_with("package:") || quoted.starts_with("dart:") {
                return Some(ImportReference { text: quoted });
            }
            // Quoted non-relative imports (Go's `import "fmt"`) stay
            // unparsed so they remain Unresolved, not misresolved.
            return None;
        }

        // C# `using System.IO;` / julia `using LinearAlgebra` — strip the
        // keyword and trailing semicolon, skipping a `using static` marker.
        if let Some(after_using) = trimmed.strip_prefix("using")
            && after_using.chars().next().is_some_and(char::is_whitespace)
        {
            let mut tokens = after_using.split_whitespace();
            let mut module = tokens.next()?.trim_end_matches(';');
            if module == "static" {
                module = tokens.next()?.trim_end_matches(';');
            }
            return Some(ImportReference {
                text: module.to_owned(),
            });
        }

        // Perl/PHP `use strict;` / `use Foo\Bar;` — must come after the
        // `using` branch since "using…" starts with "use".
        if let Some(after_use) = trimmed.strip_prefix("use")
            && after_use.chars().next().is_some_and(char::is_whitespace)
        {
            let module = after_use.split_whitespace().next()?.trim_end_matches(';');
            // Perl `use My::Module;` and PHP `use Foo\Bar\Baz;` namespace
            // separators become module dots so the module-path resolver
            // can find `My/Module.pm` / `Foo/Bar/Baz.php` when the file
            // exists; bare names (`use strict;`) are untouched and stay
            // External.
            let module = module.replace("::", ".").replace('\\', ".");
            return Some(ImportReference {
                text: module.to_owned(),
            });
        }

        // OCaml `open Printf` / `open Util`.
        if let Some(after_open) = trimmed.strip_prefix("open")
            && after_open.chars().next().is_some_and(char::is_whitespace)
        {
            let module = after_open.split_whitespace().next()?;
            return Some(ImportReference {
                text: module.to_owned(),
            });
        }

        // C/C++ `#include "local.h"` / `#include <stdio.h>` — the path is
        // a filesystem path relative to the including file, so it is
        // prefixed with `./` to reach the relative-resolution branch.
        if let Some(rest) = trimmed.strip_prefix("#include") {
            let inner = rest.trim();
            let path = inner
                .strip_prefix('"')
                .and_then(|r| r.strip_suffix('"'))
                .or_else(|| inner.strip_prefix('<').and_then(|r| r.strip_suffix('>')));
            if let Some(path) = path {
                return Some(ImportReference {
                    text: format!("./{}", path.trim()),
                });
            }
            return None;
        }

        // Python `import X` / `import X as Y` → `X`. Accepts any single
        // whitespace token (space or tab) between `import` and the module,
        // matching the same fix applied to the `from` branch above. Without
        // this, `import\tos` returns None even though it's valid Python.
        if let Some(after_import) = trimmed.strip_prefix("import")
            && after_import.chars().next().is_some_and(char::is_whitespace)
        {
            let module = after_import.split_whitespace().next()?;
            // Skip quoted imports (Go, etc.).
            if !module.starts_with('"') && !module.starts_with('\'') {
                return Some(ImportReference {
                    text: module.to_owned(),
                });
            }
            return None;
        }

        // Bare package name (e.g. `react`, `os`, `numpy`). Only extract
        // if this resolver treats bare names as external (Python, JS,
        // etc.) — the resolution step will mark them as `External`.
        // Reject anything with spaces, quotes, or other statement
        // syntax to avoid matching full import statements like Go's
        // `import "fmt"`.
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
