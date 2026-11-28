//! Rust 2018-edition `External` root-ident matching for incremental dependent
//! invalidation when new `.rs` files or crate roots are added.

use super::revision_cargo_lib_stems::add_cargo_lib_stems;
use super::revision_rust_path_stems::add_path_module_stems_with_boundary;
use super::rust_super_paths::strip_raw_prefix;
use crate::graph::join_dir_file;
use rusqlite::Transaction;
use std::collections::{BTreeSet, HashMap, HashSet};

pub(super) fn collect_external_rust_dependents(
    tx: &Transaction<'_>,
    path_to_id: &HashMap<String, i64>,
    added_rust_paths: &[String],
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    let added_stems = collect_added_rust_stems(tx, path_to_id, added_rust_paths)?;
    if added_stems.is_empty() {
        return Ok(());
    }
    let mut stmt = tx.prepare_cached(
        "SELECT de.from_file_id, de.raw_import_text \
         FROM dependency_edges de \
         JOIN files f ON f.id = de.from_file_id \
         WHERE f.language = 'rust' AND de.resolution_kind = 'External'",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let from_file_id: i64 = row.get(0)?;
        if out.contains(&from_file_id) {
            continue;
        }
        let raw_import: String = row.get(1)?;
        if added_stems
            .iter()
            .any(|stem| raw_import.contains(stem.as_str()))
            && import_has_root_ident(&raw_import, &added_stems)
        {
            out.insert(from_file_id);
        }
    }
    Ok(())
}

fn import_has_root_ident(raw_import: &str, stems: &HashSet<String>) -> bool {
    let Some(specs) = crate::graph::rust_use_tree::parse_rust_import(raw_import) else {
        return false;
    };
    for spec in specs {
        let trimmed = spec.trim();
        let after_colons = trimmed.strip_prefix("::").map_or(trimmed, |rest| rest);
        let Some(seg) = after_colons.split("::").next() else {
            continue;
        };
        if stems.contains(strip_raw_prefix(seg.trim())) {
            return true;
        }
    }
    false
}

fn collect_added_rust_stems(
    tx: &Transaction<'_>,
    path_to_id: &HashMap<String, i64>,
    added_rust_paths: &[String],
) -> Result<HashSet<String>, rusqlite::Error> {
    let mut stems = HashSet::new();
    for path in added_rust_paths {
        add_path_module_stems_with_boundary(
            path,
            |dir| path_to_id.contains_key(&join_dir_file(dir, "Cargo.toml")),
            &mut stems,
        );
    }
    add_cargo_lib_stems(tx, path_to_id, added_rust_paths, &mut stems)?;
    Ok(stems)
}
