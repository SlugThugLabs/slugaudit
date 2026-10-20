//! Stale-finding invalidation for the revision publish transaction.
//!
//! Split from [`super::revision`] to keep that file under the source-size
//! gate. The publish path calls [`invalidate_stale_findings`] inside its
//! transaction, so a finding can never be read as current against evidence
//! that has already changed.

/// How many touched paths one stale-finding invalidation binds.
///
/// `paths_json_array` serializes the given paths into ONE JSON SQL parameter,
/// so an unbounded set allocates and binds proportionally to the publish's
/// touched-file count — millions on a first publish of empty files. This
/// matches the reconcile stored-hash lookup's batch (4096 paths) and bounds
/// each execution without changing the statement's predicate.
const STALE_FINDING_PATH_BATCH: usize = 4096;

/// A finding is stale the moment its file's content hash no longer matches
/// the hash it was recorded against — including when the file is gone
/// entirely. This runs inside the publish transaction so a finding can
/// never be read as current against evidence that has already changed.
pub(super) fn invalidate_stale_findings(
    tx: &rusqlite::Transaction<'_>,
    touched_paths: &[&str],
) -> Result<(), rusqlite::Error> {
    invalidate_stale_findings_in_batches(tx, touched_paths, STALE_FINDING_PATH_BATCH)
}

/// Executes the same invalidation statement once per batch, preparing it
/// once and binding one chunk-sized JSON array per execution. Every touched
/// path still runs through the identical `WHERE`/`NOT EXISTS` predicate, and
/// the caller's transaction stays open across all batches.
pub(super) fn invalidate_stale_findings_in_batches(
    tx: &rusqlite::Transaction<'_>,
    touched_paths: &[&str],
    batch_size: usize,
) -> Result<(), rusqlite::Error> {
    if touched_paths.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "UPDATE findings SET status = 'stale' \
         WHERE status = 'current' AND path IN ({}) \
         AND NOT EXISTS ( \
             SELECT 1 FROM files \
             WHERE files.path = findings.path AND files.content_hash = findings.source_hash \
         )",
        super::manifest::PATH_SET_SUBQUERY
    );
    let mut statement = tx.prepare(&sql)?;
    for batch in touched_paths.chunks(batch_size) {
        let touched = super::manifest::paths_json_array(batch.iter().copied())?;
        statement.execute([touched])?;
    }
    Ok(())
}
