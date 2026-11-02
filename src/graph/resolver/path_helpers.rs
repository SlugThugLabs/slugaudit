//! Path-arithmetic helpers used across the language-specific and
//! generic resolvers.
//!
//! These are pure functions operating on project-relative forward-slash paths;
//! they don't know or care which language is being resolved.
//!
//! `crate::graph::resolve` already owns the low-level path utilities
//! (`normalize_join`, `parent_dir`); what's here is resolver-level glue:
//! extension selection, candidate enumeration, and the small helpers
//! the language-specific resolvers need but don't want to redefine.

use std::collections::HashSet;

use super::generic::GenericResolverConfig;
use super::types::{Resolution, external, pick, unresolved};

/// Extracts a single-quoted or double-quoted string literal from `text`.
/// Used by JS/TS-style `import ... from 'path'` extraction.
///
/// Returns `None` if the string is malformed (unmatched quote, missing
/// closing quote) — that's the same "treat as unparseable" verdict
/// callers apply to anything `extract_reference` returns `None` for.
pub(crate) fn extract_quoted_string(text: &str) -> Option<String> {
    let (open, quote) = text
        .char_indices()
        .find(|(_, character)| matches!(character, '\'' | '"'))?;
    // Both quote characters are one byte, so `open + 1` is a char boundary.
    let rest = text.get(open + 1..)?;
    let end = rest.find(quote)?;
    rest.get(..end).map(str::to_owned)
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
        // Extensionless index file: matches only a project path that is
        // literally `pkg/index` (an exact lookup; `pkg/index.mjs` is covered
        // by the extension candidates above).
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
    let Some(path) = crate::graph::resolve::normalize_join(base_dir, text) else {
        return unresolved();
    };
    candidate_paths(&path, config, known_paths)
}

/// Strips `keyword` from the start of `text` when it is followed by
/// whitespace, returning the text after the keyword. `import_x` and
/// `importx` do not match `import`.
pub(crate) fn strip_keyword<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let after = text.strip_prefix(keyword)?;
    after
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
        .then_some(after)
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
/// Reads `bare_names_are_external` from `config`: a bare name (no
/// separator) is external when it is set and unresolved when it is not.
/// Called from the generic resolver for references without a separator.
pub(crate) fn external_or_unresolved(config: &GenericResolverConfig) -> Resolution {
    if config.bare_names_are_external {
        external()
    } else {
        unresolved()
    }
}
