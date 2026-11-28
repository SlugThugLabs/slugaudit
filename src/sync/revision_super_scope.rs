//! Scoped Rust `super::` and `Low`-confidence dependent collection for
//! incremental edge invalidation.

use super::revision_edge_invalidation::{EdgeChangeScope, collect_ids_query};
use super::revision_edge_match::{low_conf_edge_affected, super_import_targets_any_changed_path};
use super::rust_super_paths::{ChangedRustPaths, RustScopeView};
use rusqlite::Transaction;
use std::collections::{BTreeSet, HashSet};

const PRIOR_REVISION_FILTER: &str = " AND f.last_revision_id < (SELECT max(id) FROM revisions)";
const LOW_CONF_RUST_FROM: &str = "FROM dependency_edges de JOIN files f ON de.from_file_id = f.id";
const LOW_CONF_RUST_WHERE: &str = "WHERE f.language = 'rust' AND de.confidence = 'Low'";
const SUPER_IMPORT_FROM_WHERE: &str = "FROM files f JOIN evidence e ON e.file_id = f.id \
     WHERE f.language = 'rust' AND e.kind = 'Import' AND e.payload LIKE '%super%'";

fn prior_revision_suffix(only_prior_revisions: bool) -> &'static str {
    if only_prior_revisions {
        PRIOR_REVISION_FILTER
    } else {
        ""
    }
}

pub(super) fn collect_rust_dependents(
    tx: &Transaction<'_>,
    only_prior_revisions: bool,
    any_rust_path_attr_changed: bool,
    scope: &EdgeChangeScope,
    view: &RustScopeView<'_>,
    upserted_ids: &HashSet<i64>,
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    let suffix = prior_revision_suffix(only_prior_revisions);
    if scope.cargo_toml_touched || any_rust_path_attr_changed {
        let low_conf_sql = format!(
            "SELECT DISTINCT de.from_file_id {LOW_CONF_RUST_FROM} {LOW_CONF_RUST_WHERE}{suffix}"
        );
        let super_sql = format!("SELECT DISTINCT f.id {SUPER_IMPORT_FROM_WHERE}{suffix}");
        collect_ids_query(tx, &low_conf_sql, out)?;
        return collect_ids_query(tx, &super_sql, out);
    }
    if scope.added_rust_paths.is_empty() && scope.deleted_rust_paths.is_empty() {
        return Ok(());
    }

    let changed = ChangedRustPaths::new(&scope.added_rust_paths, &scope.deleted_rust_paths);
    collect_scoped_low_conf_dependents(tx, suffix, &changed, view, upserted_ids, out)?;
    collect_scoped_super_dependents(tx, suffix, &changed, view, upserted_ids, out)
}

fn collect_matching_file_ids(
    tx: &Transaction<'_>,
    sql: &str,
    upserted_ids: &HashSet<i64>,
    out: &mut BTreeSet<i64>,
    mut should_include: impl FnMut(&rusqlite::Row<'_>) -> Result<bool, rusqlite::Error>,
) -> Result<(), rusqlite::Error> {
    let mut stmt = tx.prepare_cached(sql)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let file_id: i64 = row.get(0)?;
        if upserted_ids.contains(&file_id) || out.contains(&file_id) {
            continue;
        }
        if should_include(row)? {
            out.insert(file_id);
        }
    }
    Ok(())
}

fn collect_scoped_low_conf_dependents(
    tx: &Transaction<'_>,
    suffix: &str,
    changed: &ChangedRustPaths<'_>,
    view: &RustScopeView<'_>,
    upserted_ids: &HashSet<i64>,
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    let sql = format!(
        "SELECT de.from_file_id, f.path, t.path, de.raw_import_text \
         {LOW_CONF_RUST_FROM} LEFT JOIN files t ON de.to_file_id = t.id \
         {LOW_CONF_RUST_WHERE}{suffix}"
    );
    collect_matching_file_ids(tx, &sql, upserted_ids, out, |row| {
        let file_path: String = row.get(1)?;
        let target_path: Option<String> = row.get(2)?;
        let raw_import: String = row.get(3)?;
        Ok(low_conf_edge_affected(
            &file_path,
            target_path.as_deref(),
            &raw_import,
            changed,
            view,
        ))
    })
}

fn collect_scoped_super_dependents(
    tx: &Transaction<'_>,
    suffix: &str,
    changed: &ChangedRustPaths<'_>,
    view: &RustScopeView<'_>,
    upserted_ids: &HashSet<i64>,
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    let sql = format!(
        "SELECT f.id, f.path, json_extract(e.payload, '$.source'), \
                json_extract(e.payload, '$.inline_segments') \
         {SUPER_IMPORT_FROM_WHERE}{suffix}"
    );
    collect_matching_file_ids(tx, &sql, upserted_ids, out, |row| {
        let file_path: String = row.get(1)?;
        let Some(source) = row.get::<_, Option<String>>(2)? else {
            return Ok(false);
        };
        let seg_json: Option<String> = row.get(3)?;
        let inline_segments =
            super::revision_edges::decode_import_inline_segments(seg_json.as_deref());
        Ok(super_import_targets_any_changed_path(
            &file_path,
            &source,
            &inline_segments,
            changed,
            view,
        ))
    })
}
