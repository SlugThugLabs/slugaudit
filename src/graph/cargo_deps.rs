//! Cargo dependency alias and `[workspace.dependencies]` inheritance parsing
//! and resolution for `Cargo.toml` manifests.

use super::cargo_workspace::{
    PackageTargets, WorkspaceRule, is_excluded_from_workspace, nearest_workspace_rule,
    non_empty_str,
};
use super::resolver::{KnownPaths, normalize_join};
use std::collections::{BTreeMap, HashMap};

/// One dependency entry declared in a package `Cargo.toml` that may rename a
/// workspace crate or resolve via `path` or `workspace = true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DepSpec {
    /// Normalized Rust crate identifier used in source (`-` replaced by `_`).
    pub(super) alias_ident: String,
    /// Optional explicit `package = "..."` target normalized with `-` -> `_`.
    pub(super) package_name: Option<String>,
    /// Optional relative `path = "..."` directory from the declaring manifest.
    pub(super) rel_path: Option<String>,
    /// True when `workspace = true` is set on the dependency entry.
    pub(super) workspace_inherited: bool,
}

/// One `[workspace.dependencies]` entry parsed from a workspace `Cargo.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkspaceDepSpec {
    /// Optional explicit `package = "..."` target normalized with `-` -> `_`.
    pub(super) package_name: Option<String>,
    /// Normalized repository-relative package directory for `path = "..."`.
    pub(super) resolved_pkg_dir: Option<String>,
}

/// Shared workspace tables used during second-pass dependency alias resolution.
pub(super) struct DepResolveContext<'a> {
    /// Parsed `[workspace]` rules across the repository.
    pub(super) workspaces: &'a [WorkspaceRule],
    /// Parsed package target tables keyed by package directory.
    pub(super) packages: &'a BTreeMap<String, PackageTargets>,
    /// Workspace crate library roots keyed by `[lib].name` or `[package].name`.
    pub(super) libs: &'a HashMap<String, String>,
    /// Workspace crate library roots keyed by normalized `[package].name`.
    pub(super) libs_by_pkg_name: &'a HashMap<String, String>,
    /// Known repository file paths used to verify resolved library entry files.
    pub(super) known_paths: &'a dyn KnownPaths,
}

/// Parses `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`, and
/// `[target.<cfg>.{dependencies,dev-dependencies,build-dependencies}]` entries
/// from a manifest `toml::Table`.
pub(super) fn parse_package_deps(table: &toml::Table) -> Vec<DepSpec> {
    let mut deps = Vec::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        collect_dep_table(table, section, &mut deps);
    }
    if let Some(targets) = table.get("target").and_then(toml::Value::as_table) {
        for target_val in targets.values() {
            let Some(target_table) = target_val.as_table() else {
                continue;
            };
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                collect_dep_table(target_table, section, &mut deps);
            }
        }
    }
    deps
}

/// Parses `[workspace.dependencies]` entries into [`WorkspaceDepSpec`] values
/// keyed by normalized dependency key (`-` replaced by `_`).
pub(super) fn parse_workspace_deps(
    ws_table: &toml::Table,
    ws_root_dir: &str,
) -> HashMap<String, WorkspaceDepSpec> {
    let Some(dep_table) = ws_table.get("dependencies").and_then(toml::Value::as_table) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for (dep_key, dep_val) in dep_table {
        if dep_key.is_empty() {
            continue;
        }
        let Some(entry) = dep_val.as_table() else {
            continue;
        };
        let package_name = non_empty_str(entry.get("package")).map(|s| s.replace('-', "_"));
        let resolved_pkg_dir =
            non_empty_str(entry.get("path")).and_then(|rel| normalize_dep_dir(ws_root_dir, rel));
        out.insert(
            dep_key.replace('-', "_"),
            WorkspaceDepSpec {
                package_name,
                resolved_pkg_dir,
            },
        );
    }
    out
}

/// Resolves a package's parsed [`DepSpec`] list into `alias_ident` -> `lib_path`
/// mappings verified against workspace exclusion rules and `known_paths`.
pub(super) fn resolve_dep_aliases(
    pkg_dir: &str,
    deps: &[DepSpec],
    ctx: &DepResolveContext<'_>,
) -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    let ws_deps = nearest_workspace_rule(pkg_dir, ctx.workspaces).map(|ws| &ws.workspace_deps);
    for spec in deps {
        if let Some(lib_path) = resolve_single_dep(pkg_dir, spec, ws_deps, ctx) {
            aliases.insert(spec.alias_ident.clone(), lib_path);
        }
    }
    aliases
}

fn collect_dep_table(parent: &toml::Table, section: &str, out: &mut Vec<DepSpec>) {
    let Some(dep_table) = parent.get(section).and_then(toml::Value::as_table) else {
        return;
    };
    for (dep_key, dep_val) in dep_table {
        if dep_key.is_empty() {
            continue;
        }
        let Some(entry) = dep_val.as_table() else {
            continue;
        };
        let package_name = non_empty_str(entry.get("package")).map(|s| s.replace('-', "_"));
        let rel_path = non_empty_str(entry.get("path")).map(str::to_owned);
        let workspace_inherited =
            entry.get("workspace").and_then(toml::Value::as_bool) == Some(true);
        if package_name.is_none() && rel_path.is_none() && !workspace_inherited {
            continue;
        }
        out.push(DepSpec {
            alias_ident: dep_key.replace('-', "_"),
            package_name,
            rel_path,
            workspace_inherited,
        });
    }
}

fn resolve_single_dep(
    pkg_dir: &str,
    spec: &DepSpec,
    ws_deps: Option<&HashMap<String, WorkspaceDepSpec>>,
    ctx: &DepResolveContext<'_>,
) -> Option<String> {
    let (ws_matched, ws_package_name, ws_pkg_dir) = if spec.workspace_inherited {
        let lookup_key = match spec.package_name.as_deref() {
            Some(name) => name,
            None => spec.alias_ident.as_str(),
        };
        match ws_deps.and_then(|map| map.get(lookup_key)) {
            Some(ws_spec) => (
                true,
                ws_spec.package_name.as_deref(),
                ws_spec.resolved_pkg_dir.as_deref(),
            ),
            None => (false, None, None),
        }
    } else {
        (false, None, None)
    };

    if let Some(rel) = spec.rel_path.as_deref() {
        let target_dir = normalize_dep_dir(pkg_dir, rel)?;
        return checked_pkg_lib(&target_dir, ctx);
    }

    if let Some(target_dir) = ws_pkg_dir {
        return checked_pkg_lib(target_dir, ctx);
    }

    let target_name = if ws_matched {
        ws_package_name
            .or(spec.package_name.as_deref())
            .or(Some(spec.alias_ident.as_str()))?
    } else {
        spec.package_name.as_deref()?
    };
    if let Some(lib_path) = ctx.libs_by_pkg_name.get(target_name) {
        return Some(lib_path.clone());
    }
    ctx.libs.get(target_name).cloned()
}

fn checked_pkg_lib(target_dir: &str, ctx: &DepResolveContext<'_>) -> Option<String> {
    if is_excluded_from_workspace(target_dir, ctx.workspaces) {
        return None;
    }
    let lib_path = ctx.packages.get(target_dir)?.lib_path.as_deref()?;
    ctx.known_paths
        .contains_path(lib_path)
        .then(|| lib_path.to_owned())
}

fn normalize_dep_dir(base_dir: &str, rel: &str) -> Option<String> {
    if rel.is_empty() || rel.starts_with('/') {
        return None;
    }
    normalize_join(base_dir, rel)
}
