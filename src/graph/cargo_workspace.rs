// slugaudit-line-exception: approved-by=split-judge; reason=Cargo.toml manifest and workspace member/exclude/target parsing via toml::Table
//! `Cargo.toml` manifest parsing for Cargo workspace sibling crates, custom
//! `[lib].path`, declared `[[bin]]` targets, and `[workspace]` `members` /
//! `exclude` globs.

use super::cargo_deps::{
    DepResolveContext, DepSpec, WorkspaceDepSpec, parse_package_deps, parse_workspace_deps,
    resolve_dep_aliases,
};
use super::resolver::{KnownPaths, normalize_join, parent_dir};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::BuildHasher;

/// Per-package target paths and dependency aliases declared in a `Cargo.toml`
/// manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PackageTargets {
    /// Repo-relative library root path (`src/lib.rs` or custom `[lib].path`).
    pub(super) lib_path: Option<String>,
    /// Repo-relative binary root paths declared in `[[bin]]` tables.
    pub(super) bin_paths: Vec<String>,
    /// Repo-relative explicit target root paths declared in `[[test]]`,
    /// `[[example]]`, and `[[bench]]` tables.
    pub(super) extra_root_paths: Vec<String>,
    /// Per-package Cargo dependency aliases (`alias_ident -> lib_path`).
    pub(super) dep_aliases: HashMap<String, String>,
}

/// Parsed `[workspace]` rules and `[workspace.dependencies]` for one workspace
/// root manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct WorkspaceRule {
    /// Workspace root directory (`""` for the repository root).
    pub(super) root: String,
    /// Normalized `[workspace].members` patterns.
    pub(super) members: Vec<String>,
    /// Normalized `[workspace].exclude` patterns.
    pub(super) exclude: Vec<String>,
    /// Parsed `[workspace.dependencies]` specifications.
    pub(super) workspace_deps: HashMap<String, WorkspaceDepSpec>,
}

/// Workspace-wide Cargo crate and target table built from discovered
/// `Cargo.toml` manifests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WorkspaceCrates {
    libs: HashMap<String, String>,
    packages: BTreeMap<String, PackageTargets>,
    workspaces: Vec<WorkspaceRule>,
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
    #[cfg(test)]
    #[must_use]
    pub(crate) fn get(&self, crate_ident: &str) -> Option<&String> {
        self.libs.get(crate_ident)
    }

    /// Looks up the repo-relative library entry path for a normalized crate
    /// identifier (`-` replaced by `_`), checking per-package Cargo
    /// dependency aliases for `importing_relative_path` before falling back to
    /// workspace library crates.
    #[must_use]
    pub(crate) fn resolve_crate_ident(
        &self,
        importing_relative_path: &str,
        crate_ident: &str,
    ) -> Option<&String> {
        if let Some((_, pkg)) = self.enclosing_package(importing_relative_path)
            && let Some(alias_target) = pkg.dep_aliases.get(crate_ident)
        {
            return Some(alias_target);
        }
        self.libs.get(crate_ident)
    }

    /// True when `crate_ident` matches either a workspace library crate or a
    /// renamed Cargo dependency alias declared by any package.
    #[must_use]
    pub(crate) fn has_crate_or_alias_ident(&self, crate_ident: &str) -> bool {
        self.libs.contains_key(crate_ident)
            || self
                .packages
                .values()
                .any(|pkg| pkg.dep_aliases.contains_key(crate_ident))
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

    /// True when `file_path` is the `[lib]` root (`src/lib.rs` or custom
    /// `[lib].path`) of its enclosing package.
    pub(crate) fn is_lib_root(&self, file_path: &str) -> bool {
        self.enclosing_package(file_path)
            .is_some_and(|(_, targets)| targets.lib_path.as_deref() == Some(file_path))
    }

    /// True when `file_path` is an explicit `[lib].path`, `[[bin]].path`, or
    /// `[[test]]`/`[[example]]`/`[[bench]]` `path` root declared in any parsed
    /// `Cargo.toml`.
    pub(super) fn is_declared_crate_root(&self, file_path: &str) -> bool {
        self.packages.values().any(|pkg| {
            pkg.lib_path.as_deref() == Some(file_path)
                || pkg.bin_paths.iter().any(|bin| bin == file_path)
                || pkg.extra_root_paths.iter().any(|extra| extra == file_path)
        })
    }

    /// Declared `[lib].path`, `[[bin]].path`, `[[test]].path`,
    /// `[[example]].path`, and `[[bench]].path` crate-root files whose
    /// `parent_dir` equals `dir`, ordered with `[lib]` paths before `[[bin]]`
    /// and extra target paths.
    pub(super) fn crate_roots_in_dir(&self, dir: &str) -> Vec<&str> {
        let mut out = Vec::new();
        for pkg in self.packages.values() {
            if let Some(lib_path) = pkg.lib_path.as_deref()
                && parent_dir(lib_path) == dir
            {
                out.push(lib_path);
            }
        }
        for pkg in self.packages.values() {
            for bin_path in &pkg.bin_paths {
                if parent_dir(bin_path) == dir {
                    out.push(bin_path.as_str());
                }
            }
        }
        for pkg in self.packages.values() {
            for extra_path in &pkg.extra_root_paths {
                if parent_dir(extra_path) == dir {
                    out.push(extra_path.as_str());
                }
            }
        }
        out
    }
}

#[derive(Default)]
struct ParsedManifest {
    package_name: Option<String>,
    lib_name: Option<String>,
    lib_path: Option<String>,
    bin_paths: Vec<String>,
    extra_root_paths: Vec<String>,
    deps: Vec<DepSpec>,
    workspace_members: Vec<String>,
    workspace_exclude: Vec<String>,
    workspace_deps: HashMap<String, WorkspaceDepSpec>,
    has_workspace_table: bool,
}

/// Parses `[package].name` from `cargo_toml_content` via [`toml::Table`] and
/// normalizes `-` to `_`.
#[cfg(test)]
#[must_use]
fn parse_cargo_package_name(cargo_toml_content: &str) -> Option<String> {
    parse_manifest(cargo_toml_content, "").package_name
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
    let mut workspaces: Vec<WorkspaceRule> = Vec::new();

    for (manifest_rel_path, content) in cargo_tomls {
        if manifest_rel_path.starts_with('/') {
            continue;
        }
        let Some(clean_manifest) = normalize_join("", manifest_rel_path) else {
            continue;
        };
        let pkg_dir = parent_dir(&clean_manifest);
        let manifest = parse_manifest(content, &pkg_dir);
        if manifest.has_workspace_table {
            let members = manifest
                .workspace_members
                .iter()
                .filter_map(|pat| normalize_pattern(&pkg_dir, pat))
                .collect();
            let exclude = manifest
                .workspace_exclude
                .iter()
                .filter_map(|pat| normalize_pattern(&pkg_dir, pat))
                .collect();
            workspaces.push(WorkspaceRule {
                root: pkg_dir.clone(),
                members,
                exclude,
                workspace_deps: manifest.workspace_deps.clone(),
            });
        }
        parsed_entries.push((pkg_dir, manifest));
    }

    let mut out = WorkspaceCrates {
        libs: HashMap::new(),
        packages: BTreeMap::new(),
        workspaces,
    };
    let mut libs_by_pkg_name: HashMap<String, String> = HashMap::new();
    let mut pending_deps: Vec<(String, Vec<DepSpec>)> = Vec::new();

    for (pkg_dir, manifest) in parsed_entries {
        let resolved_lib = resolve_lib_path(&pkg_dir, manifest.lib_path.as_deref());
        let bin_paths = manifest
            .bin_paths
            .iter()
            .filter_map(|rel| resolve_rel_target(&pkg_dir, rel))
            .collect();
        let extra_root_paths = manifest
            .extra_root_paths
            .iter()
            .filter_map(|rel| resolve_rel_target(&pkg_dir, rel))
            .collect();

        out.packages.insert(
            pkg_dir.clone(),
            PackageTargets {
                lib_path: resolved_lib.clone(),
                bin_paths,
                extra_root_paths,
                dep_aliases: HashMap::new(),
            },
        );
        if !manifest.deps.is_empty() {
            pending_deps.push((pkg_dir.clone(), manifest.deps));
        }

        if is_excluded_from_workspace(&pkg_dir, &out.workspaces) {
            continue;
        }
        let Some(entry_path) = resolved_lib else {
            continue;
        };
        if !known_paths.contains_path(&entry_path) {
            continue;
        }
        if let Some(pkg_name) = manifest.package_name.as_deref() {
            libs_by_pkg_name.insert(pkg_name.to_owned(), entry_path.clone());
        }
        if let Some(crate_ident) = manifest.lib_name.or(manifest.package_name) {
            out.libs.insert(crate_ident, entry_path);
        }
    }

    let resolved_aliases = {
        let dep_ctx = DepResolveContext {
            workspaces: &out.workspaces,
            packages: &out.packages,
            libs: &out.libs,
            libs_by_pkg_name: &libs_by_pkg_name,
            known_paths,
        };
        pending_deps
            .into_iter()
            .filter_map(|(pkg_dir, deps)| {
                let aliases = resolve_dep_aliases(&pkg_dir, &deps, &dep_ctx);
                (!aliases.is_empty()).then_some((pkg_dir, aliases))
            })
            .collect::<Vec<_>>()
    };
    for (pkg_dir, aliases) in resolved_aliases {
        if let Some(pkg) = out.packages.get_mut(&pkg_dir) {
            pkg.dep_aliases = aliases;
        }
    }
    out
}

/// Returns the nearest enclosing [`WorkspaceRule`] for `pkg_dir`.
pub(super) fn nearest_workspace_rule<'a>(
    pkg_dir: &str,
    rules: &'a [WorkspaceRule],
) -> Option<&'a WorkspaceRule> {
    rules
        .iter()
        .filter(|rule| {
            rule.root.is_empty()
                || pkg_dir == rule.root
                || pkg_dir
                    .strip_prefix(rule.root.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|rule| rule.root.len())
}

/// True when `pkg_dir` is excluded by its nearest enclosing [`WorkspaceRule`].
pub(super) fn is_excluded_from_workspace(pkg_dir: &str, rules: &[WorkspaceRule]) -> bool {
    let Some(ws) = nearest_workspace_rule(pkg_dir, rules) else {
        return false;
    };
    if ws.exclude.iter().any(|pat| {
        pkg_dir == pat
            || (!pat.is_empty()
                && pkg_dir
                    .strip_prefix(pat.as_str())
                    .is_some_and(|rest| rest.starts_with('/')))
            || glob_matches(pat, pkg_dir)
    }) {
        return true;
    }
    if !ws.members.is_empty() {
        let is_ws_root = ws.root == pkg_dir;
        if !is_ws_root && !ws.members.iter().any(|pat| glob_matches(pat, pkg_dir)) {
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

fn parse_manifest(content: &str, pkg_dir: &str) -> ParsedManifest {
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
    for key in ["test", "example", "bench"] {
        collect_explicit_target_paths(&table, key, &mut out.extra_root_paths);
    }
    out.deps = parse_package_deps(&table);
    if let Some(ws) = table.get("workspace").and_then(toml::Value::as_table) {
        out.has_workspace_table = true;
        out.workspace_members = string_array(ws.get("members"));
        out.workspace_exclude = string_array(ws.get("exclude"));
        out.workspace_deps = parse_workspace_deps(ws, pkg_dir);
    }
    out
}

fn collect_explicit_target_paths(table: &toml::Table, key: &str, out: &mut Vec<String>) {
    let Some(entries) = table.get(key).and_then(toml::Value::as_array) else {
        return;
    };
    for entry in entries {
        if let Some(target_table) = entry.as_table()
            && let Some(explicit) = non_empty_str(target_table.get("path"))
        {
            out.push(explicit.to_owned());
        }
    }
}

/// Returns `Some(s)` when `val` is a non-empty TOML string.
pub(super) fn non_empty_str(val: Option<&toml::Value>) -> Option<&str> {
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

fn glob_matches(pattern: &str, candidate: &str) -> bool {
    crate::util::path_glob_matches(pattern, candidate)
}

#[cfg(test)]
#[path = "cargo_workspace_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workspace_import_resolution_tests.rs"]
mod import_resolution_tests;
