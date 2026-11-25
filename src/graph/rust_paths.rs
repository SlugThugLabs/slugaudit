//! Path-shape helpers for Rust's module tree: module directories, candidate
//! files, and the segment-shortening walk used by `resolve_rust`.
//!
//! These helpers are mutually recursive on the same `known_paths` contract
//! (module file → segment chain → module file), so they live together; the
//! resolver entry point and crate-anchor discovery stay in `resolve_rust`,
//! and `#[path]` parent tracking lives in `rust_mod_parents`.

use super::resolve::{normalize_join, parent_dir};
use super::resolver::{Resolution, ResolutionKind, pick, unresolved};
use std::collections::HashSet;
use std::path::Path;

/// File stems that mean "this file *is* its module" rather than "this file
/// *defines* a child module named after itself".
const RUST_MODULE_ROOT_STEMS: [&str; 3] = ["mod", "lib", "main"];

/// The file stem, or the empty string for a path with none (`..`, `/`).
fn file_stem(relative_path: &str) -> String {
    match Path::new(relative_path).file_stem() {
        Some(stem) => stem.to_string_lossy().into_owned(),
        None => String::new(),
    }
}

/// The directory that a Rust file's *own* module owns — i.e. where its
/// child modules live. `src/a/mod.rs` owns `src/a`; `src/a/b.rs` owns
/// `src/a/b`.
pub(super) fn rust_module_dir(relative_path: &str) -> String {
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

/// Strips a trailing Rust glob (`*` or `::*`) off `specifier`, returning
/// `Some(prefix)` when a glob suffix was present or `None` otherwise.
fn split_glob_suffix(specifier: &str) -> Option<&str> {
    let trimmed = specifier.trim();
    trimmed
        .strip_suffix("::*")
        .or_else(|| (trimmed == "*").then_some(""))
}

/// Returns whether `specifier` ends with a Rust glob (`*` or `::*`).
pub(super) fn has_glob_suffix(specifier: &str) -> bool {
    split_glob_suffix(specifier).is_some()
}

/// Strips a trailing glob (`::*`) off a use path, leaving the module path
/// it globs over.
pub(super) fn strip_trailing_glob(rest: &str) -> &str {
    let base = match split_glob_suffix(rest) {
        Some(prefix) => prefix,
        None => rest.trim(),
    };
    base.trim_end_matches(':')
}

/// The file that *defines* the module owning `dir` — `dir.rs` or
/// `dir/mod.rs` for an ordinary module, or the crate root (`lib.rs`/
/// `main.rs`) when `dir` is a crate's `src`.
pub(super) fn resolve_rust_module_file(
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
pub(super) fn resolve_rust_from_base(
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

/// Tries each candidate base directory in order and returns the first that
/// actually resolves, falling back to the base module's *own* file.
pub(super) fn first_resolved(
    rest: &str,
    bases: &[String],
    known_paths: &HashSet<&str>,
) -> Resolution {
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
        let Some(joined) = normalize_join(
            base_dir,
            &parts
                .iter()
                .take(take)
                .copied()
                .collect::<Vec<_>>()
                .join("/"),
        ) else {
            continue;
        };
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
