//! Path-shape helpers for Rust's module tree: module directories, candidate
//! files, and the segment-shortening walk used by `resolve_rust`.

use super::WorkspaceCrates;
use super::resolver::{
    KnownPaths, Resolution, ResolutionKind, normalize_join, parent_dir, pick, unresolved,
};
use super::rust_package_root::is_non_stem_crate_root;
use std::path::Path;

/// File stems that mean "this file *is* its module" rather than "this file
/// *defines* a child module named after itself".
const RUST_MODULE_ROOT_STEMS: [&str; 3] = ["mod", "lib", "main"];

/// Joins a relative directory path and a file or subdirectory name with `/`,
/// omitting the separator when `dir` is empty.
pub(crate) fn join_dir_file(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

/// The file stem, or the empty string for a path with none (`..`, `/`).
pub(crate) fn file_stem(relative_path: &str) -> String {
    match Path::new(relative_path).file_stem() {
        Some(stem) => stem.to_string_lossy().into_owned(),
        None => String::new(),
    }
}

/// Syntactic module directory based on stem alone (`mod.rs`, `lib.rs`, `main.rs`
/// own `parent_dir`; other `.rs` files own `<parent>/<stem>`).
pub(crate) fn rust_module_dir(relative_path: &str) -> String {
    let parent = parent_dir(relative_path);
    let stem = file_stem(relative_path);
    if RUST_MODULE_ROOT_STEMS.contains(&stem.as_str()) {
        parent
    } else {
        join_dir_file(&parent, &stem)
    }
}

/// Context-aware `rustc` module directory: crate-root files (`build.rs`,
/// `<pkg>/{src/bin,tests,benches,examples}/<name>.rs`, and custom `[lib].path` /
/// `[[bin]].path` roots) own `parent_dir(relative_path)`, while non-root
/// submodule files own `<parent>/<stem>`.
pub(super) fn effective_rust_module_dir(
    relative_path: &str,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> String {
    if is_non_stem_crate_root(relative_path, known_paths, workspace_crates) {
        parent_dir(relative_path)
    } else {
        rust_module_dir(relative_path)
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

/// Strips a trailing glob (`::*`) off a use path, leaving the module path
/// it globs over.
pub(crate) fn strip_trailing_glob(rest: &str) -> &str {
    let base = match split_glob_suffix(rest) {
        Some(prefix) => prefix,
        None => rest.trim(),
    };
    base.trim_end_matches(':')
}

/// The file that *defines* the module owning `dir` — `dir.rs` or
/// `dir/mod.rs` for an ordinary module, or the crate root (`lib.rs`/
/// `main.rs`, or a declared `[lib].path` / `[[bin]].path` whose `parent_dir`
/// equals `dir`).
pub(super) fn resolve_rust_module_file(
    dir: &str,
    known_paths: &dyn KnownPaths,
    forced_confidence: Option<&'static str>,
    workspace_crates: &WorkspaceCrates,
) -> Resolution {
    let mut candidates = if dir.is_empty() {
        Vec::new()
    } else {
        vec![
            format!("{dir}.rs"),
            format!("{dir}/mod.rs"),
            format!("{dir}/lib.rs"),
            format!("{dir}/main.rs"),
        ]
    };
    for root in workspace_crates.crate_roots_in_dir(dir) {
        if !candidates.iter().any(|candidate| candidate == root) {
            candidates.push(root.to_owned());
        }
    }
    if candidates.is_empty() {
        return unresolved();
    }
    let resolution = pick(&candidates, known_paths);
    match (resolution.kind, forced_confidence) {
        (ResolutionKind::Resolved, Some(confidence)) => Resolution {
            confidence: Some(confidence),
            ..resolution
        },
        _ => resolution,
    }
}

/// Resolves the segment chain `rest` (which may be a glob) against `base_dir`,
/// stamping `"High"` only on an unambiguous exact module match (`take == parts.len()`)
/// and `"Low"` whenever prefix shortening stopped at an ancestor module file.
pub(super) fn resolve_rust_from_base(
    rest: &str,
    base_dir: &str,
    known_paths: &dyn KnownPaths,
    forced_confidence: Option<&'static str>,
    workspace_crates: &WorkspaceCrates,
) -> Resolution {
    let segments = strip_trailing_glob(rest);
    if segments.is_empty() {
        return resolve_rust_module_file(
            base_dir,
            known_paths,
            forced_confidence,
            workspace_crates,
        );
    }
    resolve_rust_path_inner(
        segments,
        base_dir,
        known_paths,
        forced_confidence,
        Some("Low"),
    )
}

/// Resolves workspace-crate segment chain `segments` under `base_dir`, keeping
/// `"High"` confidence on exact module matches and `"Low"` when shortened to
/// an ancestor / re-exporting module.
pub(super) fn resolve_workspace_crate_segments(
    segments: &str,
    base_dir: &str,
    known_paths: &dyn KnownPaths,
) -> Resolution {
    resolve_rust_path_inner(segments, base_dir, known_paths, None, Some("Low"))
}

/// Tries each candidate base directory in priority order, checking both
/// child paths under `base` and `base`'s own module file before falling back
/// to the next base.
pub(super) fn first_resolved(
    rest: &str,
    bases: &[String],
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> Resolution {
    for base in bases {
        let resolution =
            resolve_rust_from_base(rest, base, known_paths, Some("Low"), workspace_crates);
        if resolution.kind == ResolutionKind::Resolved {
            return resolution;
        }
        let module_file =
            resolve_rust_module_file(base, known_paths, Some("Low"), workspace_crates);
        if module_file.kind == ResolutionKind::Resolved {
            return module_file;
        }
    }
    unresolved()
}

fn resolve_rust_path_inner(
    segments: &str,
    base_dir: &str,
    known_paths: &dyn KnownPaths,
    forced_confidence: Option<&'static str>,
    shortened_confidence: Option<&'static str>,
) -> Resolution {
    let parts: Vec<&str> = segments
        .split("::")
        .map(|part| match part.strip_prefix("r#") {
            Some(stripped) => stripped,
            None => part,
        })
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return unresolved();
    }
    for take in (1..=parts.len()).rev() {
        let Some(prefix) = parts.get(..take) else {
            continue;
        };
        let Some(joined) = normalize_join(base_dir, &prefix.join("/")) else {
            continue;
        };
        let candidates = vec![format!("{joined}.rs"), format!("{joined}/mod.rs")];
        let resolution = pick(&candidates, known_paths);
        if resolution.kind == ResolutionKind::Resolved {
            let override_conf = forced_confidence.or(if take < parts.len() {
                shortened_confidence
            } else {
                None
            });
            return match override_conf {
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
