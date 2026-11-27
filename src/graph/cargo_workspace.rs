// slugaudit-line-exception: approved-by=split-judge; reason=Cargo.toml manifest and workspace member/exclude/target parsing via toml::Table
//! `Cargo.toml` manifest parsing for Cargo workspace sibling crates, custom
//! `[lib].path`, declared `[[bin]]` targets, and `[workspace]` `members` /
//! `exclude` globs.

use super::resolver::{KnownPaths, normalize_join, parent_dir};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::BuildHasher;

/// Per-package target paths declared in a `Cargo.toml` manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PackageTargets {
    /// Repo-relative library root path (`src/lib.rs` or custom `[lib].path`).
    pub(super) lib_path: Option<String>,
    /// Repo-relative binary root paths declared in `[[bin]]` tables.
    pub(super) bin_paths: Vec<String>,
}

/// Workspace-wide Cargo crate and target table built from discovered
/// `Cargo.toml` manifests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WorkspaceCrates {
    libs: HashMap<String, String>,
    packages: BTreeMap<String, PackageTargets>,
    member_globs: Vec<String>,
    exclude_globs: Vec<String>,
}

impl WorkspaceCrates {
    /// Creates an empty [`WorkspaceCrates`] table.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// True when no workspace library crates are registered.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.libs.is_empty() && self.packages.is_empty()
    }

    /// Looks up the repo-relative library entry path for a normalized crate
    /// identifier (`-` replaced by `_`).
    #[must_use]
    pub(crate) fn get(&self, crate_ident: &str) -> Option<&String> {
        self.libs.get(crate_ident)
    }

    /// Declared `[lib]` / `[[bin]]` target paths for the package rooted at
    /// `pkg_dir` (`""` for the repository root).
    pub(super) fn package_at(&self, pkg_dir: &str) -> Option<&PackageTargets> {
        self.packages.get(pkg_dir)
    }

    /// Nearest enclosing package `(pkg_dir, targets)` owning `file_path`.
    pub(super) fn enclosing_package(&self, file_path: &str) -> Option<(&str, &PackageTargets)> {
        let mut dir = parent_dir(file_path);
        loop {
            if let Some((pkg_dir, targets)) = self.packages.get_key_value(dir.as_str()) {
                return Some((pkg_dir.as_str(), targets));
            }
            if dir.is_empty() {
                return None;
            }
            dir = parent_dir(&dir);
        }
    }

    /// True when `file_path` is an explicit `[lib].path` or `[[bin]].path` root
    /// declared in any parsed `Cargo.toml`.
    pub(super) fn is_declared_crate_root(&self, file_path: &str) -> bool {
        self.packages.values().any(|pkg| {
            pkg.lib_path.as_deref() == Some(file_path)
                || pkg.bin_paths.iter().any(|bin| bin == file_path)
        })
    }
}

#[derive(Default)]
struct ParsedManifest {
    package_name: Option<String>,
    lib_name: Option<String>,
    lib_path: Option<String>,
    bin_paths: Vec<String>,
    workspace_members: Vec<String>,
    workspace_exclude: Vec<String>,
    has_workspace_table: bool,
}

/// Parses `[package].name` from `cargo_toml_content` via [`toml::Table`] and
/// normalizes `-` to `_`.
#[cfg(test)]
#[must_use]
fn parse_cargo_package_name(cargo_toml_content: &str) -> Option<String> {
    parse_manifest(cargo_toml_content).package_name
}

/// Builds a [`WorkspaceCrates`] table from `(cargo_toml_rel_path, content)`
/// pairs, keeping library entry files verified in `known_paths` and recording
/// custom `[lib].path`, `[[bin]]`, and `[workspace]` `members`/`exclude` rules.
#[must_use]
pub(crate) fn build_workspace_crates<'a, S: BuildHasher>(
    cargo_tomls: impl IntoIterator<Item = (&'a str, &'a str)>,
    known_paths: &HashSet<&str, S>,
) -> WorkspaceCrates {
    build_workspace_crates_dyn(cargo_tomls, known_paths)
}

fn build_workspace_crates_dyn<'a>(
    cargo_tomls: impl IntoIterator<Item = (&'a str, &'a str)>,
    known_paths: &dyn KnownPaths,
) -> WorkspaceCrates {
    let mut parsed_entries: Vec<(String, ParsedManifest)> = Vec::new();
    let mut member_globs: Vec<String> = Vec::new();
    let mut exclude_globs: Vec<String> = Vec::new();
    let mut workspace_roots: Vec<String> = Vec::new();

    for (manifest_rel_path, content) in cargo_tomls {
        if manifest_rel_path.starts_with('/') {
            continue;
        }
        let Some(clean_manifest) = normalize_join("", manifest_rel_path) else {
            continue;
        };
        let pkg_dir = parent_dir(&clean_manifest);
        let manifest = parse_manifest(content);
        if manifest.has_workspace_table {
            workspace_roots.push(pkg_dir.clone());
            for pat in &manifest.workspace_members {
                if let Some(joined) = normalize_pattern(&pkg_dir, pat) {
                    member_globs.push(joined);
                }
            }
            for pat in &manifest.workspace_exclude {
                if let Some(joined) = normalize_pattern(&pkg_dir, pat) {
                    exclude_globs.push(joined);
                }
            }
        }
        parsed_entries.push((pkg_dir, manifest));
    }

    let mut out = WorkspaceCrates {
        libs: HashMap::new(),
        packages: BTreeMap::new(),
        member_globs,
        exclude_globs,
    };

    for (pkg_dir, manifest) in parsed_entries {
        let resolved_lib = resolve_lib_path(&pkg_dir, manifest.lib_path.as_deref());
        let bin_paths = manifest
            .bin_paths
            .iter()
            .filter_map(|rel| resolve_rel_target(&pkg_dir, rel))
            .collect();

        out.packages.insert(
            pkg_dir.clone(),
            PackageTargets {
                lib_path: resolved_lib.clone(),
                bin_paths,
            },
        );

        if is_excluded_from_workspace(&pkg_dir, &workspace_roots, &out) {
            continue;
        }
        let Some(crate_ident) = manifest.lib_name.or(manifest.package_name) else {
            continue;
        };
        if let Some(entry_path) = resolved_lib
            && known_paths.contains_path(&entry_path)
        {
            out.libs.insert(crate_ident, entry_path);
        }
    }
    out
}

fn is_excluded_from_workspace(
    pkg_dir: &str,
    workspace_roots: &[String],
    ws: &WorkspaceCrates,
) -> bool {
    if ws.exclude_globs.iter().any(|pat| {
        pkg_dir == pat
            || (!pat.is_empty() && pkg_dir.starts_with(&format!("{pat}/")))
            || glob_matches(pat, pkg_dir)
    }) {
        return true;
    }
    if !ws.member_globs.is_empty() {
        let is_ws_root = workspace_roots.iter().any(|root| root == pkg_dir);
        if !is_ws_root && !ws.member_globs.iter().any(|pat| glob_matches(pat, pkg_dir)) {
            return true;
        }
    }
    false
}

fn resolve_lib_path(pkg_dir: &str, custom_lib: Option<&str>) -> Option<String> {
    match custom_lib {
        Some(custom) => resolve_rel_target(pkg_dir, custom),
        None => resolve_rel_target(pkg_dir, "src/lib.rs"),
    }
}

fn resolve_rel_target(pkg_dir: &str, rel_target: &str) -> Option<String> {
    if rel_target.is_empty() || rel_target.starts_with('/') {
        return None;
    }
    let joined = normalize_join(pkg_dir, rel_target)?;
    (!joined.is_empty()).then_some(joined)
}

fn normalize_pattern(pkg_dir: &str, pattern: &str) -> Option<String> {
    let trimmed = pattern.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return None;
    }
    normalize_join(pkg_dir, trimmed)
}

fn parse_manifest(content: &str) -> ParsedManifest {
    let Ok(table) = toml::from_str::<toml::Table>(content) else {
        return ParsedManifest::default();
    };
    let mut out = ParsedManifest::default();

    if let Some(pkg) = table.get("package").and_then(toml::Value::as_table) {
        out.package_name = non_empty_str(pkg.get("name")).map(|s| s.replace('-', "_"));
    }
    if let Some(lib) = table.get("lib").and_then(toml::Value::as_table) {
        out.lib_name = non_empty_str(lib.get("name")).map(|s| s.replace('-', "_"));
        out.lib_path = non_empty_str(lib.get("path")).map(str::to_owned);
    }
    if let Some(bins) = table.get("bin").and_then(toml::Value::as_array) {
        for entry in bins {
            let Some(bin_table) = entry.as_table() else {
                continue;
            };
            if let Some(explicit) = non_empty_str(bin_table.get("path")) {
                out.bin_paths.push(explicit.to_owned());
            } else if let Some(bin_name) = non_empty_str(bin_table.get("name")) {
                out.bin_paths.push(format!("src/bin/{bin_name}.rs"));
                out.bin_paths.push(format!("src/bin/{bin_name}/main.rs"));
            }
        }
    }
    if let Some(ws) = table.get("workspace").and_then(toml::Value::as_table) {
        out.has_workspace_table = true;
        out.workspace_members = string_array(ws.get("members"));
        out.workspace_exclude = string_array(ws.get("exclude"));
    }
    out
}

fn non_empty_str(val: Option<&toml::Value>) -> Option<&str> {
    val.and_then(toml::Value::as_str).filter(|s| !s.is_empty())
}

fn string_array(val: Option<&toml::Value>) -> Vec<String> {
    let Some(arr) = val.and_then(toml::Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(toml::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Iterative glob matcher supporting `*` (within a path segment), `**` (across
/// path segments), and `?` (single non-`/` byte).
fn glob_matches(pattern: &str, candidate: &str) -> bool {
    if pattern == candidate {
        return true;
    }
    let pat = pattern.as_bytes();
    let text = candidate.as_bytes();
    let (mut pat_idx, mut text_idx) = (0usize, 0usize);
    let mut star: Option<(usize, usize, bool)> = None;

    while text_idx < text.len() {
        let p_byte = pat.get(pat_idx).copied();
        let Some(&t_byte) = text.get(text_idx) else {
            break;
        };
        if p_byte == Some(b'*') {
            let double = pat_idx
                .checked_add(1)
                .is_some_and(|next| pat.get(next) == Some(&b'*'));
            let step = if double { 2 } else { 1 };
            let Some(mut next_pat) = pat_idx.checked_add(step) else {
                return false;
            };
            if double && pat.get(next_pat) == Some(&b'/') {
                let Some(after_slash) = next_pat.checked_add(1) else {
                    return false;
                };
                next_pat = after_slash;
            }
            pat_idx = next_pat;
            star = Some((pat_idx, text_idx, double));
            continue;
        }
        if p_byte == Some(t_byte) || (p_byte == Some(b'?') && t_byte != b'/') {
            let (Some(next_pat), Some(next_text)) =
                (pat_idx.checked_add(1), text_idx.checked_add(1))
            else {
                return false;
            };
            pat_idx = next_pat;
            text_idx = next_text;
            continue;
        }
        let Some((star_pat, star_text, double)) = star else {
            return false;
        };
        if !double && text.get(star_text) == Some(&b'/') {
            return false;
        }
        let Some(next_text) = star_text.checked_add(1) else {
            return false;
        };
        star = Some((star_pat, next_text, double));
        pat_idx = star_pat;
        text_idx = next_text;
    }

    while pat.get(pat_idx) == Some(&b'*') {
        let Some(next_pat) = pat_idx.checked_add(1) else {
            return false;
        };
        pat_idx = next_pat;
    }
    pat_idx == pat.len()
}

#[cfg(test)]
#[path = "cargo_workspace_tests.rs"]
mod tests;
