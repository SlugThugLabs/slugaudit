//! Lazy transaction-scoped `files` path map, workspace crate graph, and Rust
//! module parent lookup shared across dependency edge resolution and
//! invalidation.

use super::revision_edge_invalidation::is_cargo_toml_path;
use super::revision_rust_meta::load_rust_module_parents;
use crate::graph::{RustModuleParents, WorkspaceCrates};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{HashMap, HashSet};

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

/// Loads workspace crate definitions from every stored `Cargo.toml` row in `tx`.
pub(super) fn load_workspace_crates(
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

/// Lazily initialized workspace-crate and Rust module-parent context for one
/// edge-resolution pass inside `tx`.
pub(super) struct LazyGraphContext<'a> {
    pub(super) path_to_id: &'a HashMap<String, i64>,
    pub(super) known_paths: &'a HashSet<&'a str>,
    empty_parents: RustModuleParents,
    empty_crates: WorkspaceCrates,
    rust_parents: Option<RustModuleParents>,
    workspace_crates: Option<WorkspaceCrates>,
}

impl<'a> LazyGraphContext<'a> {
    pub(super) fn new(
        path_to_id: &'a HashMap<String, i64>,
        known_paths: &'a HashSet<&'a str>,
    ) -> Self {
        Self {
            path_to_id,
            known_paths,
            empty_parents: RustModuleParents::new(),
            empty_crates: WorkspaceCrates::new(),
            rust_parents: None,
            workspace_crates: None,
        }
    }

    pub(super) fn ensure_crates(&mut self, tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
        if self.workspace_crates.is_none() {
            self.workspace_crates = Some(load_workspace_crates(
                tx,
                self.path_to_id,
                self.known_paths,
            )?);
        }
        Ok(())
    }

    pub(super) fn crates_view(&self) -> &WorkspaceCrates {
        self.workspace_crates
            .as_ref()
            .map_or(&self.empty_crates, |c| c)
    }

    pub(super) fn ensure_parents(&mut self, tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
        if self.rust_parents.is_none() {
            self.ensure_crates(tx)?;
            self.rust_parents = Some(load_rust_module_parents(
                tx,
                self.known_paths,
                self.crates_view(),
            )?);
        }
        Ok(())
    }

    pub(super) fn parents_view(&self) -> &RustModuleParents {
        self.rust_parents
            .as_ref()
            .map_or(&self.empty_parents, |p| p)
    }

    pub(super) fn empty_parents(&self) -> &RustModuleParents {
        &self.empty_parents
    }

    pub(super) fn empty_crates(&self) -> &WorkspaceCrates {
        &self.empty_crates
    }
}
