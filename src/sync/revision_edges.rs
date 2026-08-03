//! Resolves and persists `dependency_edges` rows for upserted files, as
//! part of the same publish transaction that writes `files`/`evidence`.
//! Kept separate from `revision.rs` so that module stays focused on the
//! file/evidence/findings write sequence rather than edge resolution.
use super::FileRecord;
use crate::model::EvidenceKind;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::HashSet;

/// Every path currently in `files`, once this transaction's deletions and
/// upserts have all landed — the universe [`crate::graph::resolve_imports`]
/// resolves against, so a same-revision addition resolves correctly.
pub(super) fn known_paths(tx: &Transaction<'_>) -> Result<Vec<String>, rusqlite::Error> {
    let mut statement = tx.prepare("SELECT path FROM files")?;
    statement.query_map([], |row| row.get(0))?.collect()
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
        tx.execute(
            "INSERT INTO dependency_edges (from_file_id, to_file_id, raw_import_text, resolution_kind, confidence) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![file_id, to_file_id, edge.raw_import_text, edge.resolution_kind, edge.confidence],
        )?;
    }
    Ok(())
}
