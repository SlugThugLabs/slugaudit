//! Turns an extracted [`ImportReference`] into a decision about whether it
//! points at a real file in this project. Every reference gets a verdict —
//! `Resolved` (a real file matched), `External` (syntactically identified
//! as outside the project: a bare package/module name, `std::`, a
//! third-party crate), or `Unresolved` (looked project-relative but no
//! candidate file existed, or this language/form isn't modeled at all).
//! Nothing is ever silently dropped: every import evidence item produces
//! exactly one edge row.
use super::reference::ImportReference;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResolutionKind {
    Resolved,
    Unresolved,
    External,
}

impl ResolutionKind {
    pub(super) fn as_sql_text(self) -> &'static str {
        match self {
            Self::Resolved => "Resolved",
            Self::Unresolved => "Unresolved",
            Self::External => "External",
        }
    }
}

pub(super) struct Resolution {
    pub(super) kind: ResolutionKind,
    pub(super) confidence: Option<&'static str>,
    pub(super) to_relative_path: Option<String>,
}

fn unresolved() -> Resolution {
    Resolution {
        kind: ResolutionKind::Unresolved,
        confidence: None,
        to_relative_path: None,
    }
}

fn external() -> Resolution {
    Resolution {
        kind: ResolutionKind::External,
        confidence: None,
        to_relative_path: None,
    }
}

/// Picks the winning candidate out of a set of paths that would all
/// satisfy the reference (e.g. a `.py` file vs. a `<name>/__init__.py`
/// package): exactly one real match is `"High"` confidence, more than one
/// is `"Low"` (genuinely ambiguous — we still report the first as a
/// best guess rather than discarding the information).
fn pick(candidates: &[String], known_paths: &HashSet<&str>) -> Resolution {
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

/// Joins path components with `/` and normalizes `..`/`.` segments,
/// working on project-relative forward-slash paths rather than the host
/// OS's `Path` semantics (a project's stored paths are always `/`-joined,
/// see `sync::discovery`).
fn normalize_join(base_dir: &str, relative: &str) -> String {
    let mut segments: Vec<&str> = if base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').collect()
    };
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

fn parent_dir(relative_path: &str) -> String {
    Path::new(relative_path)
        .parent()
        .map(Path::to_string_lossy)
        .map_or_else(String::new, std::borrow::Cow::into_owned)
}

pub(super) fn resolve(
    language: &str,
    reference: &ImportReference,
    importing_relative_path: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    match language {
        "python" => resolve_python(reference, importing_relative_path, known_paths),
        "javascript" | "typescript" | "jsx" | "tsx" => {
            resolve_js(reference, importing_relative_path, known_paths)
        }
        "rust" => resolve_rust(reference, importing_relative_path, known_paths),
        _ => unresolved(),
    }
}

/// `.` / `.bar` / `..pkg.mod`: leading dots count how many package levels
/// up from the importing file's own package (one dot = current package),
/// the remainder is a dotted module path joined onto that directory.
fn resolve_python(
    reference: &ImportReference,
    importing_relative_path: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    let text = &reference.text;
    if !text.starts_with('.') {
        return external();
    }
    let dots = text
        .chars()
        .take_while(|character| *character == '.')
        .count();
    let remainder = &text[dots..];
    let levels_up = dots - 1;
    let mut base_dir = parent_dir(importing_relative_path);
    for _ in 0..levels_up {
        base_dir = parent_dir(&base_dir);
    }

    let candidates = if remainder.is_empty() {
        vec![normalize_join(&base_dir, "__init__.py")]
    } else {
        let path = normalize_join(&base_dir, &remainder.replace('.', "/"));
        vec![format!("{path}.py"), format!("{path}/__init__.py")]
    };
    pick(&candidates, known_paths)
}

const JS_EXTENSIONS: [&str; 6] = ["ts", "tsx", "js", "jsx", "mjs", "cjs"];

/// `./utils`, `../lib/helper`: resolved relative to the importing file's
/// directory, trying the reference as-given, each common extension
/// appended, and each extension appended under an `index.` form (for a
/// directory-style module).
fn resolve_js(
    reference: &ImportReference,
    importing_relative_path: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    let text = &reference.text;
    if !(text.starts_with("./") || text.starts_with("../")) {
        return external();
    }
    let base_dir = parent_dir(importing_relative_path);
    let path = normalize_join(&base_dir, text);

    let mut candidates = vec![path.clone()];
    for extension in JS_EXTENSIONS {
        candidates.push(format!("{path}.{extension}"));
        candidates.push(format!("{path}/index.{extension}"));
    }
    pick(&candidates, known_paths)
}

/// Only `crate::`/`super::`/`self::` paths are project-internal; anything
/// else (`std::`, a third-party crate name) is external. `super::`/`self::`
/// resolution is a directory-based heuristic (parent/same directory of the
/// importing file) rather than a full module-tree walk, so it is always
/// `"Low"` confidence — it does not correctly model `mod.rs`-nested trees.
fn resolve_rust(
    reference: &ImportReference,
    importing_relative_path: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    let text = &reference.text;
    if let Some(rest) = text.strip_prefix("crate::") {
        return resolve_rust_path(rest, "src", known_paths, None);
    }
    if let Some(rest) = text.strip_prefix("self::") {
        let dir = parent_dir(importing_relative_path);
        return resolve_rust_path(rest, &dir, known_paths, Some("Low"));
    }
    if let Some(rest) = text.strip_prefix("super::") {
        let dir = parent_dir(&parent_dir(importing_relative_path));
        return resolve_rust_path(rest, &dir, known_paths, Some("Low"));
    }
    external()
}

/// A `use` path's trailing segments may name an item (a function, type, or
/// const) defined inside a module, not a further nested module — `use
/// crate::helper::greet;` targets `src/helper.rs`, where `greet` is an
/// item, not `src/helper/greet.rs`. With no semantic information about
/// which segments are modules vs. items, the full segment chain is tried
/// as a directory chain first, then progressively shorter prefixes (each
/// dropped trailing segment treated as an item name), stopping at the
/// first real file found.
fn resolve_rust_path(
    segments: &str,
    base_dir: &str,
    known_paths: &HashSet<&str>,
    forced_confidence: Option<&'static str>,
) -> Resolution {
    let parts: Vec<&str> = segments
        .split("::")
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return unresolved();
    }
    for take in (1..=parts.len()).rev() {
        let joined = normalize_join(base_dir, &parts[..take].join("/"));
        let candidates = vec![format!("{joined}.rs"), format!("{joined}/mod.rs")];
        let resolution = pick(&candidates, known_paths);
        if resolution.kind == ResolutionKind::Resolved {
            return match forced_confidence {
                Some(confidence) => Resolution {
                    confidence: Some(confidence),
                    ..resolution
                },
                None => resolution,
            };
        }
    }
    unresolved()
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
