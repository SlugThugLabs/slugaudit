//! Resolves and persists `dependency_edges` rows for upserted files, as
//! part of the same publish transaction that writes `files`/`evidence`.
//! Kept separate from `revision.rs` so that module stays focused on the
//! file/evidence/findings write sequence rather than edge resolution.
use super::FileRecord;
use crate::model::EvidenceKind;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::HashSet;

/// Every path that will exist in the project once this transaction's
/// deletions and upserts have all landed — the universe
/// [`crate::graph::resolve_imports`] resolves against, so a same-revision
/// addition resolves correctly.
///
/// `discovered_paths` are the project-relative paths the caller walked on
/// disk (discovery for a full publish, the stat sweep for a reconcile).
/// They are unioned with the `files` rows, never replaced by them: a
/// discovered file that sampling *skipped* (oversize, unreadable) has no
/// `files` row, yet it exists, and resolving its importers against indexed
/// rows alone would write `Unresolved` — which an auditing agent reads as
/// *dead code*, a fabricated finding.
pub(super) fn known_paths(
    tx: &Transaction<'_>,
    discovered_paths: &HashSet<String>,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut paths: HashSet<String> = discovered_paths.clone();
    let mut statement = tx.prepare("SELECT path FROM files")?;
    let stored = statement.query_map([], |row| row.get::<_, String>(0))?;
    for path in stored {
        paths.insert(path?);
    }
    Ok(paths.into_iter().collect())
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
    // file's language exactly as `report` used to do per edge.
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
