//! Path-shape helpers for Rust's module tree: module directories, candidate
//! files, and the segment-shortening walk used by `resolve_rust`.
//!
//! These helpers are mutually recursive on the same `known_paths` contract
//! (module file → segment chain → module file), so they live together; the
//! resolver entry point and crate-anchor discovery stay in `resolve_rust`.

use super::resolve::normalize_join;
use super::resolver::{Resolution, ResolutionKind, pick, unresolved};
use std::collections::HashSet;

/// Strips a trailing glob (`::*`) off a use path, leaving the module path
/// it globs over. `use super::*;` imports every item of the parent module,
/// so the edge it produces points at that module's own file — searching for
/// a file literally named `*.rs` finds nothing, which is what made glob
/// imports (the single most common form in real Rust code) never resolve.
pub(super) fn strip_trailing_glob(rest: &str) -> &str {
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
/// actually resolves, so a strict module-tree interpretation wins when it
/// is real and the looser directory heuristic still catches the rest.
///
/// Falls back to the base module's *own* file once no segment chain
/// matches: `use super::AnthropicClient;` names an item re-exported from
/// `client/mod.rs`, not a `client/AnthropicClient.rs` module, so the edge
/// belongs on the parent module's file.
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
        let Some(joined) = normalize_join(base_dir, &parts[..take].join("/")) else {
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
