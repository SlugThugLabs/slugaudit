//! Resolves and persists `dependency_edges` rows for upserted files, as
//! part of the same publish transaction that writes `files`/`evidence`.
//! Kept separate from `revision.rs` so that module stays focused on the
//! file/evidence/findings write sequence rather than edge resolution.
use super::FileRecord;
use crate::model::EvidenceKind;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::HashSet;

/// Every path a `files` row holds inside this transaction, once its
/// deletions and upserts have landed. The caller unions it with the
/// project-relative paths it walked on disk — never replaces them — to get
/// the universe [`crate::graph::resolve_imports`] resolves against, so a
/// same-revision addition resolves correctly.
///
/// The `files` table also holds one existence row per
/// discovered-but-skipped file (`file_kind = 'skipped'`), so the stored side
/// is a superset of the caller's walked set for every representable
/// path. The union is kept anyway: it is one set union, and the caller's
/// discovered set remains the transaction's authoritative statement of what
/// exists — a file skipped in this very transaction, or one whose path has
/// no representable key, must still resolve. Removing it would trade a
/// correctness guarantee for nothing.
pub(super) fn stored_paths(tx: &Transaction<'_>) -> Result<HashSet<String>, rusqlite::Error> {
    let mut statement = tx.prepare("SELECT path FROM files")?;
    let stored = statement.query_map([], |row| row.get::<_, String>(0))?;
    stored.collect()
}

/// Re-resolves stored `Unresolved` edges after a revision changed the path
/// set: an unchanged importer's edge can become `Resolved` when its target
/// appears, and must keep its raw text when the target disappears.
///
/// `syntax_unmodeled` edges are left alone — the resolver already declined
/// that syntax, and re-running it reaches the same verdict.
pub(super) fn re_resolve_unresolved(
    tx: &Transaction<'_>,
    known_paths: &HashSet<&str>,
) -> Result<(), rusqlite::Error> {
    let rows: Vec<(i64, String, String, String)> = {
        let mut statement = tx.prepare(
            "SELECT de.id, f.path, f.language, de.raw_import_text \
             FROM dependency_edges de JOIN files f ON de.from_file_id = f.id \
             WHERE de.resolution_kind = 'Unresolved' AND de.syntax_unmodeled = 0 \
               AND f.language IS NOT NULL",
        )?;
        statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<_, _>>()?
    };
    for (edge_id, from_path, language, raw) in rows {
        let mut edges = crate::graph::resolve_imports(
            &language,
            &from_path,
            std::slice::from_ref(&raw),
            known_paths,
        );
        let Some(edge) = edges.pop() else {
            continue;
        };
        let Some(target) = edge.to_relative_path else {
            continue;
        };
        let target_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                params![target],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(target_id) = target_id {
            tx.execute(
                "UPDATE dependency_edges SET to_file_id = ?1, resolution_kind = 'Resolved', \
                 confidence = ?2 WHERE id = ?3",
                params![target_id, edge.confidence, edge_id],
            )?;
        }
    }
    Ok(())
}

/// Re-resolves and replaces every outgoing edge for `file`: deletes what
/// was there (mirroring how evidence itself is delete-then-reinsert), then
/// resolves each `Import` evidence item's raw source and inserts one edge
/// row per source — never dropping an import because it didn't resolve,
/// only classifying it as `Unresolved`/`External`.
pub(super) fn resolve_and_store(
    tx: &Transaction<'_>,
    file: &FileRecord,
    file_id: i64,
    known_paths: &HashSet<&str>,
) -> Result<(), rusqlite::Error> {
    tx.execute(
        "DELETE FROM dependency_edges WHERE from_file_id = ?1",
        params![file_id],
    )?;

    let Some(language) = file.language.as_deref() else {
        return Ok(());
    };
    let sources: Vec<String> = file
        .evidence
        .iter()
        .filter(|item| item.kind == EvidenceKind::Import)
        .filter_map(|item| {
            item.payload
                .get("source")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .collect();
    if sources.is_empty() {
        return Ok(());
    }

    let edges = crate::graph::resolve_imports(language, &file.relative_path, &sources, known_paths);
    // C11: the "does this import's syntax even get extracted by the
    // resolver" verdict is cached on the edge at write time so `report`
    // counts it in SQL instead of re-running `extract_reference` over
    // every Unresolved edge on every call. The resolver is chosen by the
    // file's language.
    let resolver = crate::graph::resolver::get_resolver(language);
    for edge in edges {
        let to_file_id: Option<i64> = match &edge.to_relative_path {
            Some(path) => tx
                .query_row(
                    "SELECT id FROM files WHERE path = ?1",
                    params![path],
                    |row| row.get(0),
                )
                .optional()?,
            None => None,
        };
        let syntax_unmodeled = resolver.extract_reference(&edge.raw_import_text).is_none();
        tx.execute(
            "INSERT INTO dependency_edges (from_file_id, to_file_id, raw_import_text, resolution_kind, confidence, syntax_unmodeled) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                file_id,
                to_file_id,
                edge.raw_import_text,
                edge.resolution_kind,
                edge.confidence,
                syntax_unmodeled,
            ],
        )?;
    }
    Ok(())
}
