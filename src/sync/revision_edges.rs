// slugaudit-line-exception: approved-by=split-judge; reason=Resolves and persists dependency_edges for upserted and affected dependent files in one transactional pass, with invalidation, lazy graph context, Rust metadata loading, and tests already separated.
//! Resolves and persists `dependency_edges` rows for upserted files and
//! incrementally affected dependents inside the commit transaction.

use super::revision::{RevisionError, check_deadline};
use super::revision_edge_invalidation::{
    EdgeChangeScope, collect_declared_child_dependents, collect_initial_dependents,
};
use super::revision_graph_context::LazyGraphContext;
use super::revision_path_state::PriorPathAttrInvalidation;
use super::revision_rust_meta::{RustModDecl, load_file_rust_mod_decls};
use crate::graph::{ImportSource, ResolvedEdge, RustModuleParents, WorkspaceCrates};
use crate::util::Deadline;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Per-file edge inputs resolved after all `files` and `evidence` rows land in
/// `project.db`.
#[derive(Debug, Clone)]
pub(super) struct UpsertedEdgeInput {
    pub(super) file_id: i64,
    pub(super) relative_path: String,
    pub(super) language: Option<String>,
    pub(super) import_sources: Vec<ImportSource>,
    pub(super) rust_mod_decls: Vec<RustModDecl>,
    pub(super) rust_path_attr_changed: bool,
}

struct EdgeContext<'a> {
    path_to_id: &'a HashMap<String, i64>,
    known_paths: &'a HashSet<&'a str>,
    rust_parents: &'a RustModuleParents,
    workspace_crates: &'a WorkspaceCrates,
}

impl<'a> EdgeContext<'a> {
    fn from_lazy(graph_ctx: &'a LazyGraphContext<'_>, include_rust_views: bool) -> Self {
        let (rust_parents, workspace_crates) = if include_rust_views {
            (graph_ctx.parents_view(), graph_ctx.crates_view())
        } else {
            (graph_ctx.empty_parents(), graph_ctx.empty_crates())
        };
        Self {
            path_to_id: graph_ctx.path_to_id,
            known_paths: graph_ctx.known_paths,
            rust_parents,
            workspace_crates,
        }
    }
}

/// Resolves and replaces outgoing edges for every upserted file, then
/// re-resolves affected dependent files in a single deduplicated pass.
pub(super) fn resolve_upserted_edges(
    tx: &Transaction<'_>,
    upserts: &[UpsertedEdgeInput],
    path_to_id: &HashMap<String, i64>,
    known_paths: &HashSet<&str>,
    scope: &EdgeChangeScope,
    prior_path_attrs: &PriorPathAttrInvalidation,
    deadline: Option<&Deadline>,
) -> Result<(), RevisionError> {
    check_deadline(deadline)?;
    let has_rust_upserts = upserts
        .iter()
        .any(|file| file.language.as_deref() == Some("rust"));
    let any_rust_path_attr_changed = prior_path_attrs.deleted_or_skipped_had_path_attr
        || upserts.iter().any(|file| file.rust_path_attr_changed);

    let upserted_ids: HashSet<i64> = upserts.iter().map(|f| f.file_id).collect();
    let all_files_upserted = path_to_id.len() <= upserted_ids.len()
        && path_to_id.values().all(|id| upserted_ids.contains(id));

    let mut graph_ctx = LazyGraphContext::new(path_to_id, known_paths);
    let (mut affected, mut affected_includes_rust) = if all_files_upserted {
        (BTreeSet::new(), false)
    } else {
        collect_initial_dependents(
            tx,
            any_rust_path_attr_changed,
            scope,
            &prior_path_attrs.prior_child_paths,
            &upserted_ids,
            &mut graph_ctx,
        )?
    };

    if has_rust_upserts || affected_includes_rust {
        graph_ctx.ensure_crates(tx)?;
    }

    for file in upserts {
        check_deadline(deadline)?;
        resolve_one_upserted_file(
            tx,
            file,
            scope,
            &mut graph_ctx,
            &mut affected,
            &mut affected_includes_rust,
        )?;
    }

    if !affected.is_empty() {
        for file in upserts {
            affected.remove(&file.file_id);
        }
    }

    if !affected.is_empty() {
        if affected_includes_rust {
            graph_ctx.ensure_parents(tx)?;
        }
        let view = EdgeContext::from_lazy(&graph_ctx, true);
        re_resolve_ids(tx, &affected, &view, deadline)?;
    }
    Ok(())
}

fn resolve_one_upserted_file(
    tx: &Transaction<'_>,
    file: &UpsertedEdgeInput,
    scope: &EdgeChangeScope,
    graph_ctx: &mut LazyGraphContext<'_>,
    affected: &mut BTreeSet<i64>,
    affected_includes_rust: &mut bool,
) -> Result<(), rusqlite::Error> {
    let Some(language) = file.language.as_deref() else {
        return delete_file_edges(tx, file.file_id);
    };
    let is_rust = language == "rust";
    if is_rust
        && (*affected_includes_rust
            || file.rust_path_attr_changed
            || !file.import_sources.is_empty())
    {
        graph_ctx.ensure_parents(tx)?;
    }
    let empty_decls: &[RustModDecl] = &[];
    let view = EdgeContext::from_lazy(graph_ctx, is_rust);
    let rust_mod_decls = if is_rust {
        file.rust_mod_decls.as_slice()
    } else {
        empty_decls
    };
    replace_file_edges(
        tx,
        file.file_id,
        language,
        &file.relative_path,
        &file.import_sources,
        rust_mod_decls,
        &view,
    )?;
    if is_rust
        && !scope.any_rust_or_manifest_added_or_deleted()
        && file.rust_path_attr_changed
        && !view.rust_parents.is_empty()
        && collect_declared_child_dependents(
            tx,
            file.file_id,
            &file.relative_path,
            view.known_paths,
            view.rust_parents,
            view.workspace_crates,
            affected,
        )?
    {
        *affected_includes_rust = true;
    }
    Ok(())
}

pub(super) fn decode_import_inline_segments(seg_json: Option<&str>) -> Vec<String> {
    let Some(raw) = seg_json else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<String>>(raw) {
        Ok(segments) => segments,
        Err(error) => {
            tracing::warn!(%error, "ignoring malformed Import inline_segments JSON");
            Vec::new()
        }
    }
}

fn re_resolve_ids(
    tx: &Transaction<'_>,
    file_ids: &BTreeSet<i64>,
    context: &EdgeContext<'_>,
    deadline: Option<&Deadline>,
) -> Result<(), RevisionError> {
    let mut file_stmt = tx.prepare_cached("SELECT path, language FROM files WHERE id = ?1")?;
    let mut src_stmt = tx.prepare_cached(
        "SELECT json_extract(payload, '$.source'), json_extract(payload, '$.inline_segments') \
         FROM evidence WHERE file_id = ?1 AND kind = 'Import' ORDER BY id",
    )?;
    for &file_id in file_ids {
        check_deadline(deadline)?;
        let Some((path, Some(language))) = file_stmt
            .query_row(params![file_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .optional()?
        else {
            continue;
        };
        let sources: Vec<ImportSource> = src_stmt
            .query_map(params![file_id], |row| {
                let source: Option<String> = row.get(0)?;
                let seg_json: Option<String> = row.get(1)?;
                Ok(source.map(|source| ImportSource {
                    source,
                    inline_segments: decode_import_inline_segments(seg_json.as_deref()),
                }))
            })?
            .filter_map(Result::transpose)
            .collect::<Result<_, _>>()?;
        let rust_mod_decls = if language == "rust" {
            load_file_rust_mod_decls(tx, file_id)?
        } else {
            Vec::new()
        };
        replace_file_edges(
            tx,
            file_id,
            &language,
            &path,
            &sources,
            &rust_mod_decls,
            context,
        )?;
    }
    Ok(())
}

fn delete_file_edges(tx: &Transaction<'_>, file_id: i64) -> Result<(), rusqlite::Error> {
    tx.prepare_cached("DELETE FROM dependency_edges WHERE from_file_id = ?1")?
        .execute(params![file_id])?;
    Ok(())
}

fn replace_file_edges(
    tx: &Transaction<'_>,
    file_id: i64,
    language: &str,
    relative_path: &str,
    sources: &[ImportSource],
    rust_mod_decls: &[RustModDecl],
    context: &EdgeContext<'_>,
) -> Result<(), rusqlite::Error> {
    delete_file_edges(tx, file_id)?;
    if sources.is_empty() && rust_mod_decls.is_empty() {
        return Ok(());
    }
    let mut edges = if sources.is_empty() {
        Vec::new()
    } else {
        crate::graph::resolve_imports_scoped(
            language,
            relative_path,
            sources,
            context.known_paths,
            context.rust_parents,
            context.workspace_crates,
        )
    };
    if language == "rust" {
        for decl in rust_mod_decls {
            edges.push(crate::graph::resolve_rust_mod_declaration(
                relative_path,
                &decl.name,
                decl.path_attr.as_deref(),
                &decl.inline_segments,
                context.known_paths,
                context.workspace_crates,
            ));
        }
    }
    insert_resolved_edges(tx, file_id, &edges, context.path_to_id)
}

fn insert_resolved_edges(
    tx: &Transaction<'_>,
    file_id: i64,
    edges: &[ResolvedEdge],
    path_to_id: &HashMap<String, i64>,
) -> Result<(), rusqlite::Error> {
    let mut insert_edge = tx.prepare_cached(
        "INSERT INTO dependency_edges (from_file_id, to_file_id, raw_import_text, resolution_kind, confidence, syntax_unmodeled) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for edge in edges {
        let to_id = edge
            .to_relative_path
            .as_deref()
            .and_then(|target_path| path_to_id.get(target_path).copied());
        insert_edge.execute(params![
            file_id,
            to_id,
            edge.raw_import_text,
            edge.resolution_kind,
            edge.confidence,
            edge.syntax_unmodeled,
        ])?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "revision_test_fixtures.rs"]
mod revision_test_fixtures;

#[cfg(test)]
#[path = "edge_test_harness.rs"]
mod edge_test_harness;

#[cfg(test)]
#[path = "revision_edges_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "revision_edges_parity_tests.rs"]
mod parity_tests;

#[cfg(test)]
#[path = "revision_edges_corner_tests.rs"]
mod corner_tests;
