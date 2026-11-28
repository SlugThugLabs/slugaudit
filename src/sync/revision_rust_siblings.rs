//! Rust sibling module counterpart invalidation (`foo.rs` <-> `foo/{mod,lib,main}.rs`
//! and `mod.rs` <-> `lib.rs` <-> `main.rs`).

use super::rust_super_paths::strip_raw_prefix;
use crate::graph::resolver::parent_dir;
use crate::graph::{WorkspaceCrates, file_stem, join_dir_file};
use rusqlite::{Transaction, params};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

pub(super) const RUST_ROOT_FILES: [&str; 3] = ["mod.rs", "lib.rs", "main.rs"];

pub(super) fn collect_sibling_dependents(
    tx: &Transaction<'_>,
    path_to_id: &HashMap<String, i64>,
    workspace_crates: &WorkspaceCrates,
    added_rust_paths: &[String],
    deleted_rust_paths: &[String],
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    let mut sibling_ids = BTreeSet::new();
    for path in added_rust_paths.iter().chain(deleted_rust_paths.iter()) {
        for counterpart in rust_sibling_counterparts(path) {
            if let Some(&id) = path_to_id.get(&counterpart) {
                sibling_ids.insert(id);
            }
        }
    }
    collect_sibling_importers(tx, &sibling_ids, out)?;
    collect_deleted_lib_root_importers(tx, workspace_crates, deleted_rust_paths, out)
}

fn is_crate_lib_root(deleted_path: &str, workspace_crates: &WorkspaceCrates) -> bool {
    if Path::new(deleted_path).file_name().and_then(|n| n.to_str()) == Some("lib.rs") {
        return true;
    }
    workspace_crates.is_lib_root(deleted_path)
}

fn collect_deleted_lib_root_importers(
    tx: &Transaction<'_>,
    workspace_crates: &WorkspaceCrates,
    deleted_rust_paths: &[String],
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    let mut lib_dirs = BTreeSet::new();
    for path in deleted_rust_paths {
        if is_crate_lib_root(path, workspace_crates) {
            lib_dirs.insert(parent_dir(path));
        }
    }
    if lib_dirs.is_empty() {
        return Ok(());
    }
    if lib_dirs.contains("") {
        let mut stmt = tx.prepare_cached(
            "SELECT DISTINCT de.from_file_id \
             FROM dependency_edges de \
             JOIN files f ON f.id = de.from_file_id \
             WHERE f.language = 'rust' AND de.to_file_id IS NOT NULL",
        )?;
        for row in stmt.query_map([], |row| row.get::<_, i64>(0))? {
            out.insert(row?);
        }
        return Ok(());
    }
    let mut stmt = tx.prepare_cached(
        "SELECT DISTINCT de.from_file_id \
         FROM dependency_edges de \
         JOIN files f ON f.id = de.from_file_id \
         JOIN files t ON t.id = de.to_file_id \
         WHERE f.language = 'rust' AND instr(t.path, ?1) = 1",
    )?;
    for dir in lib_dirs {
        let prefix = format!("{dir}/");
        for row in stmt.query_map(params![prefix], |row| row.get::<_, i64>(0))? {
            out.insert(row?);
        }
    }
    Ok(())
}

fn collect_sibling_importers(
    tx: &Transaction<'_>,
    sibling_ids: &BTreeSet<i64>,
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    if sibling_ids.is_empty() {
        return Ok(());
    }
    let mut stmt = tx.prepare_cached(
        "SELECT DISTINCT de.from_file_id \
         FROM dependency_edges de \
         JOIN files f ON f.id = de.from_file_id \
         WHERE f.language = 'rust' AND de.to_file_id = ?1",
    )?;
    for &sib_id in sibling_ids {
        for row in stmt.query_map(params![sib_id], |row| row.get::<_, i64>(0))? {
            out.insert(row?);
        }
    }
    Ok(())
}

fn rust_sibling_counterparts(path: &str) -> Vec<String> {
    let p = Path::new(path);
    let Some(file_name) = p.file_name().and_then(|n| n.to_str()) else {
        return Vec::new();
    };
    let stem = file_stem(path);
    if stem.is_empty() {
        return Vec::new();
    }
    let parent = parent_dir(path);
    let mut out = Vec::new();
    if matches!(stem.as_str(), "mod" | "lib" | "main") {
        if !parent.is_empty() {
            out.push(format!("{parent}.rs"));
            for other in RUST_ROOT_FILES {
                if other != file_name {
                    out.push(join_dir_file(&parent, other));
                }
            }
        }
    } else {
        let clean_stem = strip_raw_prefix(&stem);
        for dir_stem in [stem.as_str(), clean_stem] {
            if !dir_stem.is_empty() {
                let mod_dir = join_dir_file(&parent, dir_stem);
                for root_file in RUST_ROOT_FILES {
                    let candidate = format!("{mod_dir}/{root_file}");
                    if !out.contains(&candidate) {
                        out.push(candidate);
                    }
                }
            }
        }
    }
    out
}
