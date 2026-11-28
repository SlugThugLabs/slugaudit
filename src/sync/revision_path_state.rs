//! Pre-mutation Rust `#[path]` invalidation tracking captured before
//! `apply_deletions`, `copy_staged_files_to_tx`, and `upsert_skipped_files`
//! mutate `evidence`.

use super::FileRecord;
use super::revision::{RevisionError, check_deadline};
use super::revision_edges::UpsertedEdgeInput;
use super::revision_graph_context::{load_path_to_id, load_workspace_crates};
use super::revision_rust_meta::{
    RustModDecl, RustPathAttrTuple, any_stored_rust_path_attr_exists, extract_rust_mod_info,
    is_rust_path, load_all_stored_rust_path_attrs, load_rust_module_parents,
};
use crate::graph::{ImportSource, RustModuleParents};
use crate::model::EvidencePayload;
use crate::util::Deadline;
use rusqlite::Transaction;
use std::collections::{BTreeSet, HashMap, HashSet};

/// Pre-mutation `#[path]` invalidation state captured before `apply_deletions`,
/// `copy_staged_files_to_tx`, and `upsert_skipped_files` alter `evidence`.
#[derive(Debug, Default)]
pub(super) struct PriorPathAttrInvalidation {
    pub(super) deleted_or_skipped_had_path_attr: bool,
    pub(super) prior_child_paths: Vec<String>,
}

/// Complete pre-mutation `#[path]` change and invalidation state for one commit
/// transaction.
#[derive(Debug, Default)]
pub(super) struct PriorRustPathState {
    pub(super) invalidation: PriorPathAttrInvalidation,
    pub(super) changed_staged_seqs: HashSet<i64>,
}

/// Lightweight per-file edge metadata captured during Pass 2 staging before
/// dropping the full [`FileRecord`].
#[derive(Debug, Clone)]
pub(super) struct StagedFileEdgeMeta {
    pub(super) seq: i64,
    pub(super) relative_path: String,
    pub(super) language: Option<String>,
    pub(super) import_sources: Vec<ImportSource>,
    pub(super) rust_mod_decls: Vec<RustModDecl>,
    pub(super) rust_path_attrs: Vec<RustPathAttrTuple>,
}

impl StagedFileEdgeMeta {
    pub(super) fn from_record(seq: i64, file: &FileRecord) -> Self {
        let import_sources = if file.language.is_some() {
            file.evidence
                .iter()
                .filter_map(|item| match &item.payload {
                    EvidencePayload::Import(import) => Some(ImportSource {
                        source: import.source.clone(),
                        inline_segments: import.inline_segments.clone(),
                    }),
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

/// Records prior `#[path]` host changes and child paths BEFORE
/// `apply_deletions`, `copy_staged_files_to_tx`, and `upsert_skipped_files`
/// mutate `evidence`.
pub(super) fn collect_prior_rust_path_state<'a>(
    tx: &Transaction<'_>,
    removed_or_skipped_paths: impl IntoIterator<Item = &'a str>,
    staged_metas: &[StagedFileEdgeMeta],
    deadline: Option<&Deadline>,
) -> Result<PriorRustPathState, RevisionError> {
    check_deadline(deadline)?;
    if !any_stored_rust_path_attr_exists(tx)? {
        return Ok(PriorRustPathState {
            invalidation: PriorPathAttrInvalidation::default(),
            changed_staged_seqs: staged_metas
                .iter()
                .filter(|m| m.language.as_deref() == Some("rust") && !m.rust_path_attrs.is_empty())
                .map(|m| m.seq)
                .collect(),
        });
    }

    let all_stored = load_all_stored_rust_path_attrs(tx)?;
    let mut deleted_or_skipped_had_path_attr = false;
    let mut changed_hosts = HashSet::new();
    for path in removed_or_skipped_paths {
        if is_rust_path(path) && all_stored.contains_key(path) {
            deleted_or_skipped_had_path_attr = true;
            changed_hosts.insert(path.to_owned());
        }
    }
    let mut changed_staged_seqs = HashSet::new();
    for meta in staged_metas {
        check_deadline(deadline)?;
        if meta.language.as_deref() != Some("rust") {
            continue;
        }
        let old_tuples = match all_stored.get(&meta.relative_path) {
            Some(tuples) => tuples.as_slice(),
            None => &[],
        };
        if old_tuples != meta.rust_path_attrs.as_slice() {
            changed_staged_seqs.insert(meta.seq);
            if !old_tuples.is_empty() {
                changed_hosts.insert(meta.relative_path.clone());
            }
        }
    }
    let prior_child_paths = if changed_hosts.is_empty() {
        Vec::new()
    } else {
        collect_prior_child_paths_for_hosts(tx, &all_stored, &changed_hosts)?
    };
    Ok(PriorRustPathState {
        invalidation: PriorPathAttrInvalidation {
            deleted_or_skipped_had_path_attr,
            prior_child_paths,
        },
        changed_staged_seqs,
    })
}

fn collect_prior_child_paths_for_hosts(
    tx: &Transaction<'_>,
    all_stored: &HashMap<String, Vec<RustPathAttrTuple>>,
    changed_hosts: &HashSet<String>,
) -> Result<Vec<String>, rusqlite::Error> {
    let pre_path_to_id = load_path_to_id(tx)?;
    let pre_known_paths: HashSet<&str> = pre_path_to_id.keys().map(String::as_str).collect();
    let pre_crates = load_workspace_crates(tx, &pre_path_to_id, &pre_known_paths)?;
    let pre_parents = load_rust_module_parents(tx, &pre_known_paths, &pre_crates)?;

    let mut children = BTreeSet::new();
    for path in pre_known_paths.iter().copied().filter(|p| is_rust_path(p)) {
        if crate::graph::reaches_declaring_host(
            path,
            &pre_parents,
            &pre_known_paths,
            &pre_crates,
            |host| changed_hosts.contains(host),
        ) {
            children.insert(path.to_owned());
        }
    }
    let mut direct_parents = RustModuleParents::new();
    for host in changed_hosts {
        if let Some(tuples) = all_stored.get(host) {
            for tuple in tuples {
                crate::graph::insert_rust_mod_parent_segments_in_context(
                    &mut direct_parents,
                    host,
                    &tuple.path_attr,
                    tuple.inline_segments.clone(),
                    &pre_known_paths,
                    &pre_crates,
                );
            }
        }
    }
    children.extend(direct_parents.into_keys());
    Ok(children.into_iter().collect())
}
