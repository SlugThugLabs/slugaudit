// slugaudit-line-exception: approved-by=slugthuglabs; reason=transaction-scoped dependency_edges persistence and incremental re-resolution
//! Resolves and persists `dependency_edges` rows for upserted files, as
//! part of the same publish transaction that writes `files`/`evidence`.
//! Kept separate from `revision.rs` so that module stays focused on the
//! file/evidence/findings write sequence rather than edge resolution.
use super::FileRecord;
use super::revision::{RevisionError, check_deadline};
use crate::graph::RustModuleParents;
use crate::model::EvidenceKind;
use crate::util::Deadline;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{BTreeSet, HashSet};

const PRIOR_REVISION_FILTER: &str = " AND f.last_revision_id < (SELECT max(id) FROM revisions)";

/// Every path a `files` row holds inside this transaction, once its deletions
/// and upserts have landed.
pub(super) fn stored_paths(tx: &Transaction<'_>) -> Result<HashSet<String>, rusqlite::Error> {
    let mut statement = tx.prepare("SELECT path FROM files")?;
    let stored = statement.query_map([], |row| row.get::<_, String>(0))?;
    stored.collect()
}

/// Resolves and replaces outgoing edges for every upserted file, then
/// re-resolves all affected dependent files in a single deduplicated pass.
///
/// Loads `RustModuleParents` at most once per revision, accumulates affected
/// dependent `file_id`s into a single `BTreeSet<i64>` (both Rust dependents
/// across all upserted Rust files and `re_resolve_unresolved` targets), and
/// enforces `deadline` across both the upsert loop and dependent re-resolution.
pub(super) fn resolve_revision_edges(
    tx: &Transaction<'_>,
    upserts: &[FileRecord],
    upserted_ids: &[i64],
    known_paths: &HashSet<&str>,
    re_resolve_unresolved: bool,
    deadline: Option<&Deadline>,
) -> Result<(), RevisionError> {
    check_deadline(deadline)?;
    let has_rust_upserts = upserts
        .iter()
        .any(|file| file.language.as_deref() == Some("rust"));
    let mut affected = BTreeSet::new();
    let mut rust_parents: Option<RustModuleParents> = None;

    if re_resolve_unresolved {
        let parents = load_rust_module_parents(tx)?;
        collect_unresolved_importers(tx, &mut affected)?;
        collect_rust_ids(tx, parents.keys().map(String::as_str), None, &mut affected)?;
        collect_rust_dependents(tx, false, &mut affected)?;
        rust_parents = Some(parents);
    } else if has_rust_upserts {
        collect_rust_dependents(tx, true, &mut affected)?;
    }

    let empty_parents = RustModuleParents::new();
    for (file, file_id) in upserts.iter().zip(upserted_ids.iter().copied()) {
        check_deadline(deadline)?;
        let Some(language) = file.language.as_deref() else {
            delete_file_edges(tx, file_id)?;
            continue;
        };
        let sources: Vec<String> = file
            .evidence
            .iter()
            .filter(|item| item.kind == EvidenceKind::Import)
            .filter_map(|item| item.payload.get("source")?.as_str().map(str::to_owned))
            .collect();
        if language != "rust" {
            let context = EdgeContext {
                known_paths,
                rust_parents: &empty_parents,
            };
            replace_file_edges(
                tx,
                file_id,
                language,
                &file.relative_path,
                &sources,
                &context,
            )?;
            continue;
        }
        if rust_parents.is_none() && needs_rust_parents(file, &sources, &affected) {
            rust_parents = Some(load_rust_module_parents(tx)?);
        }
        let active_parents = rust_parents.as_ref().map_or(&empty_parents, |p| p);
        let context = EdgeContext {
            known_paths,
            rust_parents: active_parents,
        };
        replace_file_edges(
            tx,
            file_id,
            language,
            &file.relative_path,
            &sources,
            &context,
        )?;
        if !re_resolve_unresolved && !active_parents.is_empty() {
            let declared = active_parents
                .iter()
                .filter(|(_, parent)| parent.declaring_file == file.relative_path)
                .map(|(child_path, _)| child_path.as_str());
            collect_rust_ids(tx, declared, Some(file_id), &mut affected)?;
        }
    }

    if !affected.is_empty() {
        if rust_parents.is_none() && has_rust_upserts {
            rust_parents = Some(load_rust_module_parents(tx)?);
        }
        let context = EdgeContext {
            known_paths,
            rust_parents: rust_parents.as_ref().map_or(&empty_parents, |p| p),
        };
        re_resolve_ids(tx, &affected, &context, deadline)?;
    }
    Ok(())
}

fn collect_unresolved_importers(
    tx: &Transaction<'_>,
    out: &mut BTreeSet<i64>,
) -> Result<(), rusqlite::Error> {
    let mut statement = tx.prepare_cached(
        "SELECT DISTINCT de.from_file_id FROM dependency_edges de \
         JOIN files f ON de.from_file_id = f.id \
         WHERE de.resolution_kind = 'Unresolved' AND de.syntax_unmodeled = 0 \
           AND f.language IS NOT NULL",
    )?;
    for row in statement.query_map([], |row| row.get::<_, i64>(0))? {
        out.insert(row?);
    }
    Ok(())
}

struct EdgeContext<'a> {
    known_paths: &'a HashSet<&'a str>,
    rust_parents: &'a RustModuleParents,
}

fn needs_rust_parents(file: &FileRecord, sources: &[String], dependents: &BTreeSet<i64>) -> bool {
    !dependents.is_empty()
        || sources.iter().any(|source| source.contains("super"))
        || file.evidence.iter().any(|item| {
            item.kind == EvidenceKind::Structure
                && item
                    .payload
                    .get("rust_mod_path")
                    .is_some_and(|value| !value.is_null())
        })
}

fn load_rust_module_parents(tx: &Transaction<'_>) -> Result<RustModuleParents, rusqlite::Error> {
    let mut statement = tx.prepare_cached(
        "SELECT f.path, json_extract(e.payload, '$.rust_mod_path.path_attr'), \
                COALESCE(json_extract(e.payload, '$.rust_mod_path.inline_dir'), ''), \
                json_extract(e.payload, '$.rust_mod_path.inline_segments') \
         FROM evidence e JOIN files f ON f.id = e.file_id \
         WHERE f.language = 'rust' AND e.kind = 'Structure' \
           AND instr(e.payload, 'rust_mod_path') > 0 \
           AND json_extract(e.payload, '$.rust_mod_path.path_attr') IS NOT NULL \
         ORDER BY f.path, e.start_byte",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    let mut parents = RustModuleParents::new();
    for row in rows {
        let (declaring_file, path_attr, inline_dir, inline_segments_json): (
            String,
            String,
            String,
            Option<String>,
        ) = row?;
        if let Some(raw_segments) = inline_segments_json
            && let Ok(segments) = serde_json::from_str::<Vec<String>>(&raw_segments)
        {
            crate::graph::insert_rust_mod_parent_segments(
                &mut parents,
                &declaring_file,
                &path_attr,
                segments,
            );
        } else {
            crate::graph::insert_rust_mod_parent(
                &mut parents,
                &declaring_file,
                &path_attr,
                &inline_dir,
            );
        }
    }
    Ok(parents)
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
    for sql in [super_sql, low_conf_sql] {
        let mut statement = tx.prepare_cached(&sql)?;
        for row in statement.query_map([], |row| row.get::<_, i64>(0))? {
            out.insert(row?);
        }
    }
    Ok(())
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
        replace_file_edges(tx, file_id, &language, &path, &sources, context)?;
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
    context: &EdgeContext<'_>,
) -> Result<(), rusqlite::Error> {
    delete_file_edges(tx, file_id)?;
    if sources.is_empty() {
        return Ok(());
    }
    let edges = crate::graph::resolve_imports_with_rust_parents(
        language,
        relative_path,
        sources,
        context.known_paths,
        context.rust_parents,
    );
    let mut lookup_id = tx.prepare_cached("SELECT id FROM files WHERE path = ?1")?;
    let mut insert_edge = tx.prepare_cached(
        "INSERT INTO dependency_edges (from_file_id, to_file_id, raw_import_text, resolution_kind, confidence, syntax_unmodeled) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for edge in edges {
        let to_id = match edge.to_relative_path.as_deref() {
            Some(target_path) => lookup_id
                .query_row(params![target_path], |row| row.get::<_, i64>(0))
                .optional()?,
            None => None,
        };
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
