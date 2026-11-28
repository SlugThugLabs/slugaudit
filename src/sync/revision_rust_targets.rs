//! Named Cargo-target `main.rs` directory invalidation (`src/bin/<seg>/main.rs`,
//! `tests/<seg>/main.rs`, `benches/<seg>/main.rs`, `examples/<seg>/main.rs`).

use super::revision_rust_meta::is_rust_path;
use super::rust_super_paths::RustScopeView;
use crate::graph::resolver::parent_dir;
use rusqlite::{Transaction, params};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

pub(super) const NAMED_TARGET_DIRS: [&str; 4] = ["src/bin", "tests", "benches", "examples"];

pub(super) fn collect_named_target_dependents(
    tx: &Transaction<'_>,
    path_to_id: &HashMap<String, i64>,
    view: &RustScopeView<'_>,
    added_rust_paths: &[String],
    deleted_rust_paths: &[String],
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    let target_dir_prefixes: Vec<String> = added_rust_paths
        .iter()
        .chain(deleted_rust_paths.iter())
        .filter_map(|path| named_target_main_dir_prefix(path))
        .collect();
    collect_named_target_dir_rust_files(tx, path_to_id, view, &target_dir_prefixes, out)
}

fn collect_named_target_dir_rust_files(
    tx: &Transaction<'_>,
    path_to_id: &HashMap<String, i64>,
    view: &RustScopeView<'_>,
    target_dir_prefixes: &[String],
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    if target_dir_prefixes.is_empty() {
        return Ok(());
    }
    let mut is_rust_stmt = tx
        .prepare_cached("SELECT EXISTS(SELECT 1 FROM files WHERE id = ?1 AND language = 'rust')")?;
    for (path, &id) in path_to_id {
        if !out.contains(&id)
            && is_rust_path(path)
            && path_matches_named_target_prefixes(path, target_dir_prefixes, view)
            && is_rust_stmt.query_row(params![id], |row| row.get::<_, bool>(0))?
        {
            out.insert(id);
        }
    }
    Ok(())
}

fn path_matches_named_target_prefixes(
    path: &str,
    target_dir_prefixes: &[String],
    view: &RustScopeView<'_>,
) -> bool {
    if target_dir_prefixes
        .iter()
        .any(|prefix| path.starts_with(prefix.as_str()))
    {
        return true;
    }
    if view.rust_parents.is_empty() {
        return false;
    }
    let anchor = crate::graph::root_declaring_file(
        path,
        view.rust_parents,
        view.known_paths,
        view.workspace_crates,
    );
    target_dir_prefixes
        .iter()
        .any(|prefix| anchor.starts_with(prefix.as_str()))
}

fn named_target_main_dir_prefix(path: &str) -> Option<String> {
    let file_name = Path::new(path).file_name().and_then(|n| n.to_str())?;
    if file_name != "main.rs" {
        return None;
    }
    let dir = parent_dir(path);
    let (target_parent, seg) = dir.rsplit_once('/')?;
    if seg.is_empty() {
        return None;
    }
    let is_target_dir = NAMED_TARGET_DIRS.iter().any(|td| {
        target_parent == *td
            || target_parent
                .strip_suffix(td)
                .is_some_and(|prefix| prefix.ends_with('/'))
    });
    is_target_dir.then(|| format!("{dir}/"))
}
