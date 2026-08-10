//! Path-arithmetic helpers used across the language-specific and
//! generic resolvers.
//!
//! Extracted from `src/graph/resolver.rs` as part of the small-file-rule
//! cleanup. These are pure functions operating on project-relative
//! forward-slash paths; they don't know or care which language is being
//! resolved.
//!
//! `crate::graph::resolve` already owns the low-level path utilities
//! (`normalize_join`, `parent_dir`); what's here is resolver-level glue:
//! extension selection, candidate enumeration, and the small helpers
//! the language-specific resolvers need but don't want to redefine.

use std::collections::HashSet;

use super::generic::{GenericResolverConfig, Resolution, external, pick, unresolved};

/// Extracts a single-quoted or double-quoted string literal from `text`.
/// Used by JS/TS-style `import ... from 'path'` extraction.
///
/// Returns `None` if the string is malformed (unmatched quote, missing
/// closing quote) — that's the same "treat as unparseable" verdict
/// callers apply to anything `extract_reference` returns `None` for.
pub(crate) fn extract_quoted_string(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let start = bytes
        .iter()
        .position(|byte| *byte == b'\'' || *byte == b'"')?;
    let quote = bytes[start];
    let end = bytes[start + 1..].iter().position(|byte| *byte == quote)? + start + 1;
    Some(text[start + 1..end].to_owned())
}

/// Translates a module path (`foo.bar.baz`) into the filesystem path
/// the resolver will probe. Languages whose module path uses `/` as the
/// separator (`generic.module_separator == "/"`) get the raw `text`
/// back, since that's already a filesystem path. Languages that use `.`
/// (Python, Ruby, Java, etc.) get `.` → `/` substitution.
pub(crate) fn module_path_to_fs_path(text: &str, config: &GenericResolverConfig) -> String {
    if config.module_separator == "/" {
        text.to_owned()
    } else {
        text.replace(config.module_separator, "/")
    }
}

/// Enumerates the set of paths a resolved import could refer to and
/// feeds them to `pick`, which decides between Resolved/Unresolved
/// based on what's in `known_paths`.
///
/// The candidate list intentionally starts with the bare `path` (no
/// extension added), so an exact-match project path is preferred over
/// an `.ext`-appended one — the user's own file is the more specific
/// answer when it exists.
pub(crate) fn candidate_paths(
    path: &str,
    config: &GenericResolverConfig,
    known_paths: &HashSet<&str>,
) -> Resolution {
    let mut candidates = vec![path.to_owned()];

    for ext in &config.extensions {
        candidates.push(format!("{path}.{ext}"));
    }

    if let Some(index) = config.index_filename {
        for ext in &config.extensions {
            candidates.push(format!("{path}/{index}.{ext}"));
        }
        // Index file without an extension matches e.g. `index.mjs` on
        // some platforms and keeps the resolver from over-fitting on a
        // specific `__init__.py` style. Stripping an extension here is
        // purely an FS exercise — a project that has `pkg/index` will
        // match, those that don't will fall through to other candidates.
        candidates.push(format!("{path}/{index}"));
    }

    pick(&candidates, known_paths)
}

/// Helper for resolving relative-style imports (`./foo`, `../pkg/bar`)
/// when the import is described by a `base_dir` and the relative path
/// `text`. The candidate enumeration lives in [`candidate_paths`].
///
/// Kept as a free function rather than a method so [`super::generic`]
/// can call it without dragging the [`crate::graph::resolve`] helpers
/// into its imports.
pub(crate) fn resolve_relative_path(
    base_dir: &str,
    text: &str,
    config: &GenericResolverConfig,
    known_paths: &HashSet<&str>,
) -> Resolution {
    let path = crate::graph::resolve::normalize_join(base_dir, text);
    candidate_paths(&path, config, known_paths)
}

/// Whether a text looks like a Python-style relative import: starts
/// with `.` but not with `./` or `../` (those are JS-style filesystem
/// paths). Used by the generic `extract_reference` and the
/// Python-specific relative-resolution branch.
pub(crate) fn starts_with_python_dot_prefix(text: &str) -> bool {
    text.starts_with('.') && !text.starts_with("./") && !text.starts_with("../")
}

/// External-or-unresolved fallback for ambiguous inputs.
///
/// Reads `bare_names_are_external` from `config` so a resolver that
/// treats bare names as project-local (Python relative imports) ends
/// up unresolved, while one that treats bare names as packages
/// (JS/TS) ends up external.
#[allow(dead_code)]
pub(crate) fn external_or_unresolved(config: &GenericResolverConfig) -> Resolution {
    if config.bare_names_are_external {
        external()
    } else {
        unresolved()
    }
}
