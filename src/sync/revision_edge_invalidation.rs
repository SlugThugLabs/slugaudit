// slugaudit-line-exception: approved-by=split-judge; reason=Cohesive single responsibility for revision dependency-edge invalidation, combining change-scope classification with affected-dependent ID collection while delegating per-rule collectors to sibling modules.
//! Incremental blast-radius classification and affected-dependent ID collection
//! for `dependency_edges` re-resolution inside a revision commit transaction.

use super::manifest::{ChangeStatus, FileChange};
use super::revision_graph_context::LazyGraphContext;
use super::revision_rust_dependents::collect_external_rust_dependents;
use super::revision_rust_meta::is_rust_path;
use super::revision_rust_shadowing::collect_shadowed_workspace_dependents;
use super::revision_rust_siblings::collect_sibling_dependents;
use super::revision_rust_targets::collect_named_target_dependents;
use super::revision_super_scope::collect_rust_dependents;
use super::rust_super_paths::RustScopeView;
use crate::graph::{RustModuleParents, WorkspaceCrates};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{BTreeSet, HashSet};

pub(super) fn is_cargo_toml_path(relative_path: &str) -> bool {
    relative_path == "Cargo.toml" || relative_path.ends_with("/Cargo.toml")
}

/// Incremental blast-radius classification for one revision's file-set changes.
#[derive(Debug, Clone, Default)]
pub(super) struct EdgeChangeScope {
    pub(super) rust_or_manifest_added_or_deleted: bool,
    pub(super) non_rust_added_or_deleted: bool,
    pub(super) cargo_toml_touched: bool,
    pub(super) added_rust_paths: Vec<String>,
    pub(super) deleted_rust_paths: Vec<String>,
}

impl EdgeChangeScope {
    pub(super) fn from_changes(changes: &[FileChange], cargo_toml_upserted: bool) -> Self {
        let mut scope = Self {
            cargo_toml_touched: cargo_toml_upserted,
            ..Self::default()
        };
        for change in changes {
            if is_cargo_toml_path(&change.relative_path) {
                scope.cargo_toml_touched = true;
            }
            match change.status {
                ChangeStatus::Added => scope.record_path(&change.relative_path, true),
                ChangeStatus::Deleted => scope.record_path(&change.relative_path, false),
                ChangeStatus::Modified => {}
            }
        }
        scope
    }

    pub(super) fn record_path(&mut self, path: &str, is_added: bool) {
        if is_rust_path(path) {
            self.rust_or_manifest_added_or_deleted = true;
            if is_added {
                self.added_rust_paths.push(path.to_owned());
            } else {
                self.deleted_rust_paths.push(path.to_owned());
            }
        } else if is_cargo_toml_path(path) {
            self.rust_or_manifest_added_or_deleted = true;
        } else {
            self.non_rust_added_or_deleted = true;
        }
    }

    pub(super) fn any_rust_or_manifest_added_or_deleted(&self) -> bool {
        self.rust_or_manifest_added_or_deleted || self.cargo_toml_touched
    }
}

fn scope_view<'a>(ctx: &'a LazyGraphContext<'_>) -> RustScopeView<'a> {
    RustScopeView {
        known_paths: ctx.known_paths,
        rust_parents: ctx.parents_view(),
        workspace_crates: ctx.crates_view(),
    }
}

pub(super) fn collect_initial_dependents(
    tx: &Transaction<'_>,
    path_attr: bool,
    scope: &EdgeChangeScope,
    prior_child_paths: &[String],
    upserted: &HashSet<i64>,
    ctx: &mut LazyGraphContext<'_>,
) -> Result<(BTreeSet<i64>, bool), rusqlite::Error> {
    let mut out = BTreeSet::new();
    let prior = prior_child_paths.iter().map(String::as_str);
    let mut affected_includes_rust =
        !prior_child_paths.is_empty() && collect_rust_ids(tx, prior, None, &mut out)?;

    if scope.any_rust_or_manifest_added_or_deleted() {
        collect_rust_manifest_dependents(tx, path_attr, scope, upserted, ctx, &mut out)?;
        affected_includes_rust |= !out.is_empty();
    } else {
        if scope.non_rust_added_or_deleted {
            collect_unresolved_importers(tx, false, &mut out)?;
        }
        if path_attr {
            ctx.ensure_parents(tx)?;
            let before = out.len();
            let view = scope_view(ctx);
            collect_rust_dependents(tx, true, true, scope, &view, upserted, &mut out)?;
            affected_includes_rust |= out.len() > before;
        }
    }
    Ok((out, affected_includes_rust))
}

fn collect_rust_manifest_dependents(
    tx: &Transaction<'_>,
    path_attr: bool,
    scope: &EdgeChangeScope,
    upserted: &HashSet<i64>,
    ctx: &mut LazyGraphContext<'_>,
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    ctx.ensure_parents(tx)?;
    collect_unresolved_importers(tx, true, out)?;
    let view = scope_view(ctx);
    collect_reaching_rust_ids(
        tx,
        ctx.known_paths,
        view.rust_parents,
        view.workspace_crates,
        None,
        |_| true,
        out,
    )?;
    let path_to_id = ctx.path_to_id;
    let (added, deleted) = (&scope.added_rust_paths, &scope.deleted_rust_paths);
    collect_rust_dependents(tx, false, path_attr, scope, &view, upserted, out)?;
    if !added.is_empty() && !scope.cargo_toml_touched {
        collect_external_rust_dependents(tx, path_to_id, added, out)?;
        collect_shadowed_workspace_dependents(tx, view.workspace_crates, added, out)?;
    }
    if !added.is_empty() || !deleted.is_empty() {
        collect_sibling_dependents(tx, path_to_id, view.workspace_crates, added, deleted, out)?;
        collect_named_target_dependents(tx, path_to_id, &view, added, deleted, out)?;
    }
    if scope.cargo_toml_touched {
        let sql = "SELECT DISTINCT f.id FROM files f JOIN evidence e ON e.file_id = f.id \
             WHERE f.language = 'rust' AND (e.kind = 'Import' \
             OR (e.kind = 'Structure' AND instr(e.payload, 'rust_mod_path') > 0))";
        collect_ids_query(tx, sql, out)?;
    }
    Ok(())
}

pub(super) fn collect_declared_child_dependents(
    tx: &Transaction<'_>,
    file_id: i64,
    relative_path: &str,
    known_paths: &HashSet<&str>,
    rust_parents: &RustModuleParents,
    workspace_crates: &WorkspaceCrates,
    affected: &mut BTreeSet<i64>,
) -> Result<bool, rusqlite::Error> {
    collect_reaching_rust_ids(
        tx,
        known_paths,
        rust_parents,
        workspace_crates,
        Some(file_id),
        |host| host == relative_path,
        affected,
    )
}

fn collect_reaching_rust_ids(
    tx: &Transaction<'_>,
    known_paths: &HashSet<&str>,
    rust_parents: &RustModuleParents,
    workspace_crates: &WorkspaceCrates,
    exclude_id: Option<i64>,
    matches_host: impl Fn(&str) -> bool,
    out: &mut BTreeSet<i64>,
) -> Result<bool, rusqlite::Error> {
    if rust_parents.is_empty() {
        return Ok(false);
    }
    let matching = known_paths.iter().copied().filter(|path| {
        is_rust_path(path)
            && crate::graph::reaches_declaring_host(
                path,
                rust_parents,
                known_paths,
                workspace_crates,
                &matches_host,
            )
    });
    collect_rust_ids(tx, matching, exclude_id, out)
}

pub(super) fn collect_ids_query(
    tx: &Transaction<'_>,
    sql: &str,
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    let mut statement = tx.prepare_cached(sql)?;
    for row in statement.query_map([], |row| row.get::<_, i64>(0))? {
        out.insert(row?);
    }
    Ok(())
}

fn collect_unresolved_importers(
    tx: &Transaction<'_>,
    include_rust: bool,
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    let rust_clause = if include_rust {
        ""
    } else {
        " AND f.language != 'rust'"
    };
    let sql = format!(
        "SELECT DISTINCT de.from_file_id FROM dependency_edges de \
         JOIN files f ON de.from_file_id = f.id \
         WHERE de.resolution_kind = 'Unresolved' AND de.syntax_unmodeled = 0 \
           AND f.language IS NOT NULL{rust_clause}"
    );
    collect_ids_query(tx, &sql, out)
}

fn collect_rust_ids<'a>(
    tx: &Transaction<'_>,
    paths: impl Iterator<Item = &'a str>,
    exclude_id: Option<i64>,
    out: &mut BTreeSet<i64>,
) -> Result<bool, rusqlite::Error> {
    let mut inserted = false;
    let mut lookup =
        tx.prepare_cached("SELECT id FROM files WHERE path = ?1 AND language = 'rust'")?;
    for path in paths {
        if let Some(id) = lookup
            .query_row(params![path], |row| row.get::<_, i64>(0))
            .optional()?
            && Some(id) != exclude_id
        {
            inserted |= out.insert(id);
        }
    }
    Ok(inserted)
}
