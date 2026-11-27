// slugaudit-line-exception: approved-by=split-judge; reason=transactional dependency_edges resolution and incremental invalidation scope across upserted and dependent files
//! Resolves and persists `dependency_edges` rows for upserted files and
//! incrementally affected dependents inside the commit transaction.

use super::FileRecord;
use super::manifest::{ChangeStatus, FileChange};
use super::revision::{RevisionError, check_deadline};
use super::revision_rust_meta::{
    RustModDecl, RustPathAttrTuple, extract_rust_mod_info, is_rust_path, load_file_rust_mod_decls,
    load_rust_module_parents, stored_rust_path_attrs,
};
use crate::graph::{RustModuleParents, WorkspaceCrates};
use crate::model::EvidencePayload;
use crate::util::Deadline;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{BTreeSet, HashMap, HashSet};

const PRIOR_REVISION_FILTER: &str = " AND f.last_revision_id < (SELECT max(id) FROM revisions)";

pub(super) fn is_cargo_toml_path(relative_path: &str) -> bool {
    relative_path == "Cargo.toml" || relative_path.ends_with("/Cargo.toml")
}

/// Incremental blast-radius classification for one revision's file-set changes.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct EdgeChangeScope {
    pub(super) rust_or_manifest_added_or_deleted: bool,
    pub(super) non_rust_added_or_deleted: bool,
    pub(super) cargo_toml_touched: bool,
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
            if matches!(change.status, ChangeStatus::Added | ChangeStatus::Deleted) {
                scope.record_added_or_deleted_path(&change.relative_path);
            }
        }
        scope
    }

    fn record_added_or_deleted_path(&mut self, path: &str) {
        if is_rust_path(path) || is_cargo_toml_path(path) {
            self.rust_or_manifest_added_or_deleted = true;
        } else {
            self.non_rust_added_or_deleted = true;
        }
    }

    pub(super) fn any_rust_or_manifest_added_or_deleted(self) -> bool {
        self.rust_or_manifest_added_or_deleted || self.cargo_toml_touched
    }
}

/// Derives [`EdgeChangeScope`] for `publish_revision` test inputs by checking
/// which `upserts` are newly added vs already stored in `files`.
#[cfg(test)]
pub(super) fn scope_for_test_inputs(
    tx: &Transaction<'_>,
    upserts: &[FileRecord],
    deletions: &[String],
    re_resolve_unresolved: bool,
) -> Result<EdgeChangeScope, rusqlite::Error> {
    let mut scope = EdgeChangeScope {
        cargo_toml_touched: upserts.iter().any(|f| is_cargo_toml_path(&f.relative_path))
            || deletions.iter().any(|p| is_cargo_toml_path(p)),
        ..EdgeChangeScope::default()
    };
    if !re_resolve_unresolved {
        return Ok(scope);
    }
    let mut exists_stmt =
        tx.prepare_cached("SELECT EXISTS(SELECT 1 FROM files WHERE path = ?1)")?;
    for file in upserts {
        let already_exists: bool =
            exists_stmt.query_row(params![file.relative_path], |row| row.get(0))?;
        if !already_exists {
            scope.record_added_or_deleted_path(&file.relative_path);
        }
    }
    for path in deletions {
        scope.record_added_or_deleted_path(path);
    }
    Ok(scope)
}

/// Lightweight per-file edge metadata captured during Pass 2 staging before
/// dropping the full [`FileRecord`].
#[derive(Debug, Clone)]
pub(super) struct StagedFileEdgeMeta {
    pub(super) seq: i64,
    pub(super) relative_path: String,
    pub(super) language: Option<String>,
    pub(super) import_sources: Vec<String>,
    pub(super) rust_mod_decls: Vec<RustModDecl>,
    pub(super) rust_path_attrs: Vec<RustPathAttrTuple>,
}

impl StagedFileEdgeMeta {
    pub(super) fn from_record(seq: i64, file: &FileRecord) -> Self {
        let import_sources = if file.language.is_some() {
            file.evidence
                .iter()
                .filter_map(|item| match &item.payload {
                    EvidencePayload::Import(import) => Some(import.source.clone()),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        let (rust_mod_decls, rust_path_attrs) = if file.language.as_deref() == Some("rust") {
            extract_rust_mod_info(&file.evidence)
        } else {
            (Vec::new(), Vec::new())
        };
        Self {
            seq,
            relative_path: file.relative_path.clone(),
            language: file.language.clone(),
            import_sources,
            rust_mod_decls,
            rust_path_attrs,
        }
    }

    pub(super) fn to_edge_input(
        &self,
        file_id: i64,
        rust_path_attr_changed: bool,
    ) -> UpsertedEdgeInput {
        UpsertedEdgeInput {
            file_id,
            relative_path: self.relative_path.clone(),
            language: self.language.clone(),
            import_sources: self.import_sources.clone(),
            rust_mod_decls: self.rust_mod_decls.clone(),
            rust_path_attr_changed,
        }
    }
}

/// Compares `meta.rust_path_attrs` against previously stored `#[path]` tuples
/// for `meta.relative_path` before replacing its `evidence` rows.
pub(super) fn did_rust_path_attr_change(
    tx: &Transaction<'_>,
    meta: &StagedFileEdgeMeta,
    full_rust_reresolve: bool,
) -> Result<bool, rusqlite::Error> {
    if !full_rust_reresolve && meta.language.as_deref() == Some("rust") {
        let old_tuples = stored_rust_path_attrs(tx, &meta.relative_path)?;
        Ok(old_tuples != meta.rust_path_attrs)
    } else {
        Ok(!meta.rust_path_attrs.is_empty())
    }
}

/// Per-file edge inputs resolved after all `files` and `evidence` rows land in
/// `project.db`.
#[derive(Debug, Clone)]
pub(super) struct UpsertedEdgeInput {
    pub(super) file_id: i64,
    pub(super) relative_path: String,
    pub(super) language: Option<String>,
    pub(super) import_sources: Vec<String>,
    pub(super) rust_mod_decls: Vec<RustModDecl>,
    pub(super) rust_path_attr_changed: bool,
}

/// Loads every `files` `(path, id)` pair once inside `tx`.
pub(super) fn load_path_to_id(
    tx: &Transaction<'_>,
) -> Result<HashMap<String, i64>, rusqlite::Error> {
    let mut statement = tx.prepare("SELECT path, id FROM files")?;
    statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect()
}

fn load_workspace_crates(
    tx: &Transaction<'_>,
    path_to_id: &HashMap<String, i64>,
    known_paths: &HashSet<&str>,
) -> Result<WorkspaceCrates, rusqlite::Error> {
    let mut cargo_ids: Vec<(&str, i64)> = path_to_id
        .iter()
        .filter(|(path, _)| is_cargo_toml_path(path))
        .map(|(path, &id)| (path.as_str(), id))
        .collect();
    if cargo_ids.is_empty() {
        return Ok(WorkspaceCrates::new());
    }
    cargo_ids.sort_by_key(|(a, _)| *a);
    let mut stmt = tx.prepare_cached("SELECT content FROM files WHERE id = ?1")?;
    let mut manifests = Vec::with_capacity(cargo_ids.len());
    for (path, id) in cargo_ids {
        if let Some(content) = stmt
            .query_row(params![id], |row| row.get::<_, Option<String>>(0))
            .optional()?
            .flatten()
        {
            manifests.push((path, content));
        }
    }
    Ok(crate::graph::build_workspace_crates(
        manifests
            .iter()
            .map(|(path, content)| (*path, content.as_str())),
        known_paths,
    ))
}

struct LazyGraphContext<'a> {
    path_to_id: &'a HashMap<String, i64>,
    known_paths: &'a HashSet<&'a str>,
    empty_parents: RustModuleParents,
    empty_crates: WorkspaceCrates,
    rust_parents: Option<RustModuleParents>,
    workspace_crates: Option<WorkspaceCrates>,
}

impl<'a> LazyGraphContext<'a> {
    fn new(
        path_to_id: &'a HashMap<String, i64>,
        known_paths: &'a HashSet<&'a str>,
        rust_parents: Option<RustModuleParents>,
    ) -> Self {
        Self {
            path_to_id,
            known_paths,
            empty_parents: RustModuleParents::new(),
            empty_crates: WorkspaceCrates::new(),
            rust_parents,
            workspace_crates: None,
        }
    }

    fn ensure_crates(&mut self, tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
        if self.workspace_crates.is_none() {
            self.workspace_crates = Some(load_workspace_crates(
                tx,
                self.path_to_id,
                self.known_paths,
            )?);
        }
        Ok(())
    }

    fn ensure_parents(&mut self, tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
        if self.rust_parents.is_none() {
            self.rust_parents = Some(load_rust_module_parents(tx)?);
        }
        Ok(())
    }

    fn non_rust_view(&self) -> EdgeContext<'_> {
        EdgeContext {
            path_to_id: self.path_to_id,
            known_paths: self.known_paths,
            rust_parents: &self.empty_parents,
            workspace_crates: &self.empty_crates,
        }
    }

    fn current_view(&self) -> EdgeContext<'_> {
        EdgeContext {
            path_to_id: self.path_to_id,
            known_paths: self.known_paths,
            rust_parents: self
                .rust_parents
                .as_ref()
                .map_or(&self.empty_parents, |p| p),
            workspace_crates: self
                .workspace_crates
                .as_ref()
                .map_or(&self.empty_crates, |c| c),
        }
    }
}

fn collect_initial_dependents(
    tx: &Transaction<'_>,
    any_rust_path_attr_changed: bool,
    scope: EdgeChangeScope,
) -> Result<(BTreeSet<i64>, Option<RustModuleParents>, bool), rusqlite::Error> {
    let mut affected = BTreeSet::new();
    let mut rust_parents: Option<RustModuleParents> = None;
    let mut affected_includes_rust = false;

    if scope.any_rust_or_manifest_added_or_deleted() {
        let parents = load_rust_module_parents(tx)?;
        collect_unresolved_importers(tx, true, &mut affected)?;
        collect_rust_ids(tx, parents.keys().map(String::as_str), None, &mut affected)?;
        collect_rust_dependents(tx, false, &mut affected)?;
        if scope.cargo_toml_touched {
            collect_ids_query(
                tx,
                "SELECT DISTINCT f.id FROM files f \
                 JOIN evidence e ON e.file_id = f.id \
                 WHERE f.language = 'rust' AND e.kind = 'Import'",
                &mut affected,
            )?;
        }
        rust_parents = Some(parents);
        affected_includes_rust = !affected.is_empty();
    } else {
        if scope.non_rust_added_or_deleted {
            collect_unresolved_importers(tx, false, &mut affected)?;
        }
        if any_rust_path_attr_changed {
            let before = affected.len();
            collect_rust_dependents(tx, true, &mut affected)?;
            if affected.len() > before {
                affected_includes_rust = true;
            }
        }
    }
    Ok((affected, rust_parents, affected_includes_rust))
}

/// Resolves and replaces outgoing edges for every upserted file, then
/// re-resolves affected dependent files in a single deduplicated pass.
pub(super) fn resolve_upserted_edges(
    tx: &Transaction<'_>,
    upserts: &[UpsertedEdgeInput],
    path_to_id: &HashMap<String, i64>,
    known_paths: &HashSet<&str>,
    scope: EdgeChangeScope,
    deleted_or_skipped_had_path_attr: bool,
    deadline: Option<&Deadline>,
) -> Result<(), RevisionError> {
    check_deadline(deadline)?;
    let has_rust_upserts = upserts
        .iter()
        .any(|file| file.language.as_deref() == Some("rust"));
    let any_rust_path_attr_changed =
        deleted_or_skipped_had_path_attr || upserts.iter().any(|file| file.rust_path_attr_changed);
    let (mut affected, initial_parents, mut affected_includes_rust) =
        collect_initial_dependents(tx, any_rust_path_attr_changed, scope)?;

    let mut graph_ctx = LazyGraphContext::new(path_to_id, known_paths, initial_parents);
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

    for file in upserts {
        affected.remove(&file.file_id);
    }

    if !affected.is_empty() {
        if affected_includes_rust {
            graph_ctx.ensure_parents(tx)?;
            graph_ctx.ensure_crates(tx)?;
        }
        let view = graph_ctx.current_view();
        re_resolve_ids(tx, &affected, &view, deadline)?;
    }
    Ok(())
}

fn resolve_one_upserted_file(
    tx: &Transaction<'_>,
    file: &UpsertedEdgeInput,
    scope: EdgeChangeScope,
    graph_ctx: &mut LazyGraphContext<'_>,
    affected: &mut BTreeSet<i64>,
    affected_includes_rust: &mut bool,
) -> Result<(), rusqlite::Error> {
    let Some(language) = file.language.as_deref() else {
        return delete_file_edges(tx, file.file_id);
    };
    if language != "rust" {
        let view = graph_ctx.non_rust_view();
        return replace_file_edges(
            tx,
            file.file_id,
            language,
            &file.relative_path,
            &file.import_sources,
            &[],
            &view,
        );
    }
    if *affected_includes_rust || file.rust_path_attr_changed || !file.import_sources.is_empty() {
        graph_ctx.ensure_parents(tx)?;
    }
    let view = graph_ctx.current_view();
    replace_file_edges(
        tx,
        file.file_id,
        language,
        &file.relative_path,
        &file.import_sources,
        &file.rust_mod_decls,
        &view,
    )?;
    if !scope.any_rust_or_manifest_added_or_deleted()
        && file.rust_path_attr_changed
        && !view.rust_parents.is_empty()
    {
        let before = affected.len();
        let declared = view
            .rust_parents
            .iter()
            .filter(|(_, parent)| parent.declaring_file == file.relative_path)
            .map(|(child_path, _)| child_path.as_str());
        collect_rust_ids(tx, declared, Some(file.file_id), affected)?;
        if affected.len() > before {
            *affected_includes_rust = true;
        }
    }
    Ok(())
}

fn collect_ids_query(
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

struct EdgeContext<'a> {
    path_to_id: &'a HashMap<String, i64>,
    known_paths: &'a HashSet<&'a str>,
    rust_parents: &'a RustModuleParents,
    workspace_crates: &'a WorkspaceCrates,
}

fn collect_rust_dependents(
    tx: &Transaction<'_>,
    only_prior_revisions: bool,
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    let suffix = if only_prior_revisions {
        PRIOR_REVISION_FILTER
    } else {
        ""
    };
    let super_sql = format!(
        "SELECT DISTINCT f.id FROM files f JOIN evidence e ON e.file_id = f.id \
         WHERE f.language = 'rust' AND e.kind = 'Import' AND e.payload LIKE '%super%'{suffix}"
    );
    let low_conf_sql = format!(
        "SELECT DISTINCT de.from_file_id FROM dependency_edges de \
         JOIN files f ON de.from_file_id = f.id \
         WHERE f.language = 'rust' AND de.confidence = 'Low'{suffix}"
    );
    collect_ids_query(tx, &super_sql, out)?;
    collect_ids_query(tx, &low_conf_sql, out)
}

fn collect_rust_ids<'a>(
    tx: &Transaction<'_>,
    paths: impl Iterator<Item = &'a str>,
    exclude_id: Option<i64>,
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    let mut lookup =
        tx.prepare_cached("SELECT id FROM files WHERE path = ?1 AND language = 'rust'")?;
    for path in paths {
        if let Some(id) = lookup
            .query_row(params![path], |row| row.get::<_, i64>(0))
            .optional()?
            && Some(id) != exclude_id
        {
            out.insert(id);
        }
    }
    Ok(())
}

fn re_resolve_ids(
    tx: &Transaction<'_>,
    file_ids: &BTreeSet<i64>,
    context: &EdgeContext<'_>,
    deadline: Option<&Deadline>,
) -> Result<(), RevisionError> {
    let mut file_stmt = tx.prepare_cached("SELECT path, language FROM files WHERE id = ?1")?;
    let mut src_stmt = tx.prepare_cached(
        "SELECT json_extract(payload, '$.source') FROM evidence WHERE file_id = ?1 AND kind = 'Import' ORDER BY id",
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
        let sources: Vec<String> = src_stmt
            .query_map(params![file_id], |row| row.get::<_, Option<String>>(0))?
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
    sources: &[String],
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
        crate::graph::resolve_imports_full(
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
            ));
        }
    }
    let mut insert_edge = tx.prepare_cached(
        "INSERT INTO dependency_edges (from_file_id, to_file_id, raw_import_text, resolution_kind, confidence, syntax_unmodeled) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for edge in edges {
        let to_id = edge
            .to_relative_path
            .as_deref()
            .and_then(|target_path| context.path_to_id.get(target_path).copied());
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
#[path = "revision_edges_tests.rs"]
mod tests;
