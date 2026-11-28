//! Invalidation of `Resolved` Rust workspace dependency edges when newly added
//! `.rs` files introduce local modules that shadow workspace crate or alias
//! identifiers.

use super::revision_rust_path_stems::add_path_module_stems;
use super::rust_super_paths::strip_raw_prefix;
use crate::graph::WorkspaceCrates;
use rusqlite::Transaction;
use std::collections::{BTreeSet, HashSet};

/// Collects `from_file_id` values of Rust files holding `Resolved` dependency
/// edges whose unqualified root identifier matches a workspace crate or Cargo
/// dependency alias shadowed by `added_rust_paths`.
pub(super) fn collect_shadowed_workspace_dependents(
    tx: &Transaction<'_>,
    workspace_crates: &WorkspaceCrates,
    added_rust_paths: &[String],
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    if added_rust_paths.is_empty() || workspace_crates.is_empty() {
        return Ok(());
    }
    let mut added_stems = HashSet::new();
    for path in added_rust_paths {
        add_path_module_stems(path, &mut added_stems);
    }
    let shadowed_stems: HashSet<String> = added_stems
        .into_iter()
        .filter(|stem| workspace_crates.has_crate_or_alias_ident(stem))
        .collect();
    if shadowed_stems.is_empty() {
        return Ok(());
    }
    let mut stmt = tx.prepare_cached(
        "SELECT de.from_file_id, de.raw_import_text \
         FROM dependency_edges de \
         JOIN files f ON f.id = de.from_file_id \
         WHERE f.language = 'rust' AND de.resolution_kind = 'Resolved'",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let from_file_id: i64 = row.get(0)?;
        if out.contains(&from_file_id) {
            continue;
        }
        let raw_import: String = row.get(1)?;
        if !shadowed_stems
            .iter()
            .any(|stem| raw_import.contains(stem.as_str()))
        {
            continue;
        }
        if import_has_unqualified_root(&raw_import, &shadowed_stems) {
            out.insert(from_file_id);
        }
    }
    Ok(())
}

fn import_has_unqualified_root(raw_import: &str, stems: &HashSet<String>) -> bool {
    let Some(specs) = crate::graph::rust_use_tree::parse_rust_import(raw_import) else {
        return false;
    };
    for spec in specs {
        let trimmed = spec.trim();
        if trimmed.starts_with("::") {
            continue;
        }
        let Some(seg) = trimmed.split("::").next() else {
            continue;
        };
        if stems.contains(strip_raw_prefix(seg.trim())) {
            return true;
        }
    }
    false
}
