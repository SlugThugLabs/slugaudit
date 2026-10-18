//! Python-specific resolver helpers.
//!
//! Extracted from `src/graph/resolver.rs` so Python's dot-prefix relative
//! imports (`from .pkg import foo`, `from .. import bar`) live next to
//! the language's `__init__.py` index-filename handling and the
//! python-tuned `GenericResolver` config factory.
//!
//! Python is a special case for relative imports because its syntax
//! (`.foo`, `..pkg.mod`) is its own thing — neither filesystem-relative
//! (`./`, `../`) nor module-path (`foo.bar.baz`) — and the standard
//! library's package marker (`__init__.py`) needs explicit handling.

use std::collections::HashSet;

use super::generic::GenericResolverConfig;
use super::path_helpers::{candidate_paths, starts_with_python_dot_prefix};
use super::types::{Resolution, ResolutionKind, unresolved};
use crate::graph::reference::ImportReference;

/// Resolves a Python-style relative import — leading dots tell how many
/// packages to walk up, the rest of the text is a dot-separated module
/// path that's rewritten into `/` separators and probed for files.
///
/// Specifically:
/// - `.bar` (one dot)         → `pkg/bar.py` (current package, one
///   level up from the file).
/// - `..bar` (two dots)       → `bar.py` (parent package).
/// - `..pkg.bar` (two+dots)   → `pkg/bar.py` (walked up two levels,
///   then down through the module path).
/// - `.` (just a dot)         → `__init__.py` in the current package,
///   i.e. the package marker for `from . import anything`.
///
/// Anything that doesn't match one of these patterns (e.g. typos or a
/// Python `from X import Y` where `X` happens to start with a `.`
/// due to a method/attribute) ends up `'Unresolved'` rather than
/// misclassifying as a real file.
pub(crate) fn resolve_python_relative(
    text: &str,
    config: &GenericResolverConfig,
    importing_relative_path: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    let dots = text.chars().take_while(|c| *c == '.').count();
    let remainder = &text[dots..];
    // `.bar` is "one level up" — `dots - 1` because one dot means the
    // importing file's *own* package. Two dots (`..bar`) means the
    // parent's parent, etc. Clamp at zero so a degenerate `.` (just a
    // dot, the package marker) doesn't walk above the project root.
    let levels_up = dots.saturating_sub(1);

    let mut base_dir = crate::graph::resolve::parent_dir(importing_relative_path);
    // `parent_dir` of the root is the root, so without this check extra
    // dots would silently stay at the root and resolve `..bar` from a
    // root-level file to the in-project `bar.py` (a false edge).
    let depth = base_dir.split('/').filter(|part| !part.is_empty()).count();
    if levels_up > depth {
        return unresolved();
    }
    for _ in 0..levels_up {
        base_dir = crate::graph::resolve::parent_dir(&base_dir);
    }

    let joined = if remainder.is_empty() {
        crate::graph::resolve::normalize_join(&base_dir, "")
    } else {
        crate::graph::resolve::normalize_join(&base_dir, &remainder.replace('.', "/"))
    };
    let Some(path) = joined else {
        return unresolved();
    };

    if remainder.is_empty() {
        // `from . import` → `__init__.py` in the current package.
        if let Some(index) = config.index_filename {
            let init_path = format!("{path}/{index}.py");
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
        candidate_paths(&path, config, known_paths)
    }
}

pub(crate) fn extract_python_reference(trimmed: &str) -> Option<ImportReference> {
    if trimmed == "." {
        return Some(ImportReference {
            text: ".".to_owned(),
        });
    }
    if starts_with_python_dot_prefix(trimmed) {
        return Some(ImportReference {
            text: trimmed.to_owned(),
        });
    }
    if let Some(after_from) = strip_keyword(trimmed, "from") {
        let module = after_from.split_whitespace().next()?;
        if !module.starts_with('"') && !module.starts_with('\'') {
            return Some(ImportReference {
                text: module.to_owned(),
            });
        }
        return None;
    }
    if let Some(after_import) = strip_keyword(trimmed, "import") {
        let module = after_import.split_whitespace().next()?;
        if !module.starts_with('"') && !module.starts_with('\'') {
            return Some(ImportReference {
                text: module.to_owned(),
            });
        }
        return None;
    }
    None
}

fn strip_keyword<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let after = text.strip_prefix(keyword)?;
    after
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
        .then_some(after)
}
