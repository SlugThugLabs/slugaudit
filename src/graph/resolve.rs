//! Turns an extracted [`ImportReference`] into a decision about whether it
//! points at a real file in this project. Every reference gets a verdict —
//! `Resolved` (a real file matched), `External` (syntactically identified
//! as outside the project: a bare package/module name, `std::`, a
//! third-party crate), or `Unresolved` (looked project-relative but no
//! candidate file existed, or this language/form isn't modeled at all).
//! Nothing is ever silently dropped: every import evidence item produces
//! exactly one edge row.
use super::reference::ImportReference;
use std::collections::{HashSet, HashMap};
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
    let counts = UNKNOWN_LANGUAGE_COUNTS
        .get_or_init(|| Mutex::new(HashMap::new()));
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

/// File stems that mean "this file *is* its module" rather than "this file
/// *defines* a child module named after itself". For `src/a/mod.rs` the
/// module is `a`; for `src/a/b.rs` the module is `a::b`. Getting this
/// distinction right is what makes `super::`/`self::` resolve correctly in
/// ordinary (non-`mod.rs`) module trees.
const RUST_MODULE_ROOT_STEMS: [&str; 3] = ["mod", "lib", "main"];

fn file_stem(relative_path: &str) -> String {
    Path::new(relative_path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
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

/// Strips a trailing glob (`::*`) off a use path, leaving the module path
/// it globs over. `use super::*;` imports every item of the parent module,
/// so the edge it produces points at that module's own file — searching for
/// a file literally named `*.rs` finds nothing, which is what made glob
/// imports (the single most common form in real Rust code) never resolve.
fn strip_trailing_glob(rest: &str) -> &str {
    let trimmed = rest.trim();
    let without_star = trimmed.strip_suffix('*').unwrap_or(trimmed);
    without_star.trim_end_matches(':')
}

/// The file that *defines* the module owning `dir` — `dir.rs` or
/// `dir/mod.rs` for an ordinary module, or the crate root (`lib.rs`/
/// `main.rs`) when `dir` is a crate's `src`. The crate-root forms matter
/// because `use super::*;` in a top-level module (`src/foo.rs`) refers to
/// the crate root itself, which has no `src.rs`/`src/mod.rs`.
fn resolve_rust_module_file(
    dir: &str,
    known_paths: &HashSet<&str>,
    forced_confidence: Option<&'static str>,
) -> Resolution {
    if dir.is_empty() {
        return unresolved();
    }
    let candidates = vec![
        format!("{dir}.rs"),
        format!("{dir}/mod.rs"),
        format!("{dir}/lib.rs"),
        format!("{dir}/main.rs"),
    ];
    let resolution = pick(&candidates, known_paths);
    match (resolution.kind, forced_confidence) {
        (ResolutionKind::Resolved, Some(confidence)) => Resolution {
            confidence: Some(confidence),
            ..resolution
        },
        _ => resolution,
    }
}

/// Resolves the segment chain `rest` (which may be a glob) against
/// `base_dir`.
fn resolve_rust_from_base(
    rest: &str,
    base_dir: &str,
    known_paths: &HashSet<&str>,
    forced_confidence: Option<&'static str>,
) -> Resolution {
    let segments = strip_trailing_glob(rest);
    if segments.is_empty() {
        return resolve_rust_module_file(base_dir, known_paths, forced_confidence);
    }
    resolve_rust_path(segments, base_dir, known_paths, forced_confidence)
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
fn resolve_rust(
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
        // `super::super::…` walks up one module per repetition.
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

/// Tries each candidate base directory in order and returns the first that
/// actually resolves, so a strict module-tree interpretation wins when it
/// is real and the looser directory heuristic still catches the rest.
///
/// Falls back to the base module's *own* file once no segment chain
/// matches: `use super::AnthropicClient;` names an item re-exported from
/// `client/mod.rs`, not a `client/AnthropicClient.rs` module, so the edge
/// belongs on the parent module's file.
fn first_resolved(rest: &str, bases: &[String], known_paths: &HashSet<&str>) -> Resolution {
    for base in bases {
        let resolution = resolve_rust_from_base(rest, base, known_paths, Some("Low"));
        if resolution.kind == ResolutionKind::Resolved {
            return resolution;
        }
    }
    for base in bases {
        let resolution = resolve_rust_module_file(base, known_paths, Some("Low"));
        if resolution.kind == ResolutionKind::Resolved {
            return resolution;
        }
    }
    unresolved()
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
