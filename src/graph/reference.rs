//! Extracts the literal module/path reference a language actually wrote
//! from the *raw import statement text* the language pack captures as
//! `ImportInfo.source`. That field is not a clean path — for every
//! language checked, it is the entire statement (`"from . import foo"`,
//! `"use crate::baz::qux;"`, `"import x from './utils';"`) — so each
//! language needs its own small, syntax-aware extraction before any
//! resolution can happen.

/// A module/path reference as written in source, stripped of its
/// surrounding statement syntax but not yet resolved to a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ImportReference {
    pub(super) text: String,
}

/// Returns `None` when `raw` doesn't match any pattern this parser
/// recognizes for `language` — callers treat that the same as an
/// unparseable import, not an error.
pub(super) fn extract(language: &str, raw: &str) -> Option<ImportReference> {
    match language {
        "python" => python(raw),
        "javascript" | "typescript" | "jsx" | "tsx" => quoted_literal(raw),
        "rust" => rust(raw),
        _ => None,
    }
}

/// `from X import ...` → `X` (`.`, `.bar`, `..pkg.mod`, or an absolute
/// module name). Plain `import X` / `import X as Y` → `X`.
fn python(raw: &str) -> Option<ImportReference> {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("from ") {
        let module = rest.split_whitespace().next()?;
        return Some(ImportReference {
            text: module.to_owned(),
        });
    }
    if let Some(rest) = trimmed.strip_prefix("import ") {
        let module = rest.split_whitespace().next()?;
        return Some(ImportReference {
            text: module.to_owned(),
        });
    }
    None
}

/// The quoted string literal in a JS/TS `import ... from '<path>'` or
/// `import '<path>'` statement.
fn quoted_literal(raw: &str) -> Option<ImportReference> {
    let bytes = raw.as_bytes();
    let start = bytes
        .iter()
        .position(|byte| *byte == b'\'' || *byte == b'"')?;
    let quote = bytes[start];
    let end = bytes[start + 1..].iter().position(|byte| *byte == quote)? + start + 1;
    Some(ImportReference {
        text: raw[start + 1..end].to_owned(),
    })
}

/// `use crate::a::b;`, `use super::c;`, `use std::collections::HashMap;`
/// → the path with the trailing `;` and any `as alias` / `{...}` group
/// dropped, keeping only the leading path segment chain.
fn rust(raw: &str) -> Option<ImportReference> {
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

#[cfg(test)]
#[path = "reference_tests.rs"]
mod tests;
