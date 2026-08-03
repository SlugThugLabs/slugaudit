//! Turns an extracted [`ImportReference`] into a decision about whether it
//! points at a real file in this project. Every reference gets a verdict —
//! `Resolved` (a real file matched), `External` (syntactically identified
//! as outside the project: a bare package/module name, `std::`, a
//! third-party crate), or `Unresolved` (looked project-relative but no
//! candidate file existed, or this language/form isn't modeled at all).
//! Nothing is ever silently dropped: every import evidence item produces
//! exactly one edge row.
use super::reference::ImportReference;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

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

/// Tracks how many imports have been encountered per unsupported language
/// during this process lifetime, so the first encounter logs a warning and
/// the total is logged when it changes. This makes unsupported languages
/// visible (to stderr, where the AI host reads diagnostics) rather than
/// silently collapsing into `Unresolved` edges that are indistinguishable
/// from genuinely missing files. `OnceLock` is used rather than a plain
/// `Mutex` so the map is lazily allocated on first unknown-language
/// encounter — the common case (only supported languages) pays nothing.
static UNKNOWN_LANGUAGE_COUNTS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

/// Records an import from an unsupported language and emits a diagnostic
/// warning. The first import from a given language logs a warning; each
/// subsequent one from the same language updates the running tally so the
/// final logged count reflects the full scale of unsupported-language
/// imports in this process.
fn record_unsupported_language(language: &str) {
    let counts = UNKNOWN_LANGUAGE_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = counts.lock().unwrap();
    let entry = guard.entry(language.to_owned()).or_insert(0);
    *entry += 1;
    let count = *entry;
    if count == 1 {
        tracing::warn!(
            language,
            "unsupported language for import resolution; \
             imports from {} files will be recorded as Unresolved edges \
             (indistinguishable from genuinely missing files)",
            language,
        );
    } else {
        tracing::info!(language, count, "unsupported-language import");
    }
}

pub(super) fn unresolved() -> Resolution {
    Resolution {
        kind: ResolutionKind::Unresolved,
        confidence: None,
        to_relative_path: None,
    }
}

pub(super) fn external() -> Resolution {
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
pub(super) fn pick(candidates: &[String], known_paths: &HashSet<&str>) -> Resolution {
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
pub(super) fn normalize_join(base_dir: &str, relative: &str) -> String {
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

pub(super) fn parent_dir(relative_path: &str) -> String {
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
        "rust" => super::resolve_rust::resolve(reference, importing_relative_path, known_paths),
        other => {
            record_unsupported_language(other);
            unresolved()
        }
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

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
