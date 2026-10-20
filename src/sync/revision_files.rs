//! The per-file `files` / `evidence` writes for one publish, and the caps
//! that bound the evidence stored with each file.
//!
//! Split out of `revision.rs` (the same pattern as `revision_edges.rs` and
//! `revision_skipped.rs`) so that module reads as the transaction it is: the
//! input struct, the compare-and-swap baseline check, the write ordering, and
//! the current-pointer swap. How one indexed file's row and evidence are
//! written lives here.

use super::FileRecord;
use super::RevisionError;
use super::classify::classify;
use super::sample::truncation_evidence;
use crate::evidence::{self, EvidenceRow};
use crate::model::{EvidenceItem, ResourceLimits};
use rusqlite::params;

/// Upserts one indexed file's `files` row, then replaces its evidence.
///
/// The row is inserted or updated in place: a path that previously existed as
/// a skipped existence row becomes `indexed` again, and `skip_reason` is
/// cleared so a stale reason cannot outlive the skip it described. A file
/// that was `binary` and is now text (or the reverse) is re-classified the
/// same way — `file_kind` always describes the current sample.
///
/// # Errors
///
/// Returns [`RevisionError::InvalidParserRun`] if the record's parser columns
/// contradict each other, or a database error from any write.
pub(super) fn upsert_file(
    tx: &rusqlite::Transaction<'_>,
    file: &FileRecord,
    revision_db_id: i64,
) -> Result<i64, RevisionError> {
    validate_record(file)?;
    insert_file_row(tx, file, revision_db_id)?;
    let file_id = file_id_for_path(tx, &file.relative_path)?;
    replace_evidence(tx, file_id, file)?;
    Ok(file_id)
}

fn validate_record(file: &FileRecord) -> Result<(), RevisionError> {
    file.run
        .validate()
        .map_err(|reason| RevisionError::InvalidParserRun {
            path: file.relative_path.clone(),
            reason,
        })
}

fn insert_file_row(
    tx: &rusqlite::Transaction<'_>,
    file: &FileRecord,
    revision_db_id: i64,
) -> Result<(), RevisionError> {
    tx.execute(
        "INSERT INTO files (\
            path, file_kind, content, content_hash, hash_algorithm, byte_len, \
            modified_unix_seconds, language, language_detected, parser_availability, \
            parse_outcome, parse_error_reason, extraction_completeness, last_revision_id, \
            file_class\
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
         ON CONFLICT(path) DO UPDATE SET \
            file_kind = excluded.file_kind, \
            file_class = excluded.file_class, \
            skip_reason = NULL, \
            content = excluded.content, \
            content_hash = excluded.content_hash, \
            hash_algorithm = excluded.hash_algorithm, \
            byte_len = excluded.byte_len, \
            modified_unix_seconds = excluded.modified_unix_seconds, \
            language = excluded.language, \
            language_detected = excluded.language_detected, \
            parser_availability = excluded.parser_availability, \
            parse_outcome = excluded.parse_outcome, \
            parse_error_reason = excluded.parse_error_reason, \
            extraction_completeness = excluded.extraction_completeness, \
            last_revision_id = excluded.last_revision_id",
        params![
            file.relative_path,
            if file.is_binary { "binary" } else { "indexed" },
            file.content,
            file.identity.content_hash,
            file.identity.hash_algorithm,
            file.byte_len,
            file.modified_unix_seconds,
            file.language,
            file.language_detected,
            file.run.availability.as_sql_text(),
            file.run.outcome.as_sql_text(),
            file.run.error_reason(),
            file.run.completeness.as_sql_text(),
            revision_db_id,
            classify(&file.relative_path).as_sql_text(),
        ],
    )?;
    Ok(())
}

fn file_id_for_path(
    tx: &rusqlite::Transaction<'_>,
    relative_path: &str,
) -> Result<i64, rusqlite::Error> {
    tx.query_row(
        "SELECT id FROM files WHERE path = ?1",
        params![relative_path],
        |row| row.get(0),
    )
}

fn replace_evidence(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    file: &FileRecord,
) -> Result<(), rusqlite::Error> {
    tx.execute("DELETE FROM evidence WHERE file_id = ?1", params![file_id])?;
    for item in &file.evidence {
        insert_evidence(tx, file_id, &evidence::to_row(item))?;
    }
    Ok(())
}

/// Inserts one normalized evidence row for a file. The caller has already
/// deleted the file's previous evidence, so this is a plain insert.
fn insert_evidence(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    row: &EvidenceRow,
) -> Result<(), rusqlite::Error> {
    tx.execute(
        "INSERT INTO evidence (\
            file_id, key, kind, origin, span_availability, \
            start_byte, end_byte, start_line, start_column, end_line, end_column, payload\
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            file_id,
            row.key,
            row.kind,
            row.origin,
            row.span_availability,
            row.start_byte,
            row.end_byte,
            row.start_line,
            row.start_column,
            row.end_line,
            row.end_column,
            row.payload,
        ],
    )?;
    Ok(())
}

/// Reserved out of `max_payload_bytes_per_file` so the truncation marker
/// itself never pushes a file's evidence over the cap it is reporting on.
const TRUNCATION_MARKER_RESERVE_BYTES: usize = 512;

/// Enforces all three evidence caps in one pass: per-item count, per-item
/// payload bytes, and — the one the old truncate-then-retain version never
/// checked — cumulative payload bytes across every kept item. A file with
/// many small-but-not-tiny items (each under the per-item cap) could still
/// serialize to a multi-megabyte evidence blob; this is what actually
/// caught the 5,326,990-byte failure. Whenever anything is discarded, an
/// explicit `Diagnostic` evidence item records it — the AI sees a "this
/// file's evidence was truncated" signal instead of silently incomplete data.
///
/// Returns `true` when anything was discarded, so `to_file_record` can
/// downgrade a would-be `Full` run to `Partial` instead of storing a
/// completeness claim that contradicts the marker sitting next to it.
#[must_use]
pub(crate) fn apply_evidence_limits(
    items: &mut Vec<EvidenceItem>,
    limits: &ResourceLimits,
) -> bool {
    let original_len = items.len();
    let mut budget = EvidenceBudget::new(original_len, limits);
    for item in items.drain(..) {
        if matches!(budget.admit(item), Admission::Stop) {
            break;
        }
    }
    let kept_len = budget.kept.len();
    let discarded = budget.anything_discarded();
    *items = budget.kept;
    if discarded {
        items.push(truncation_evidence(
            original_len,
            kept_len,
            budget.oversized_dropped,
            budget.unmeasurable_dropped,
        ));
    }
    discarded
}

/// One pass of the three evidence caps; `apply_evidence_limits` drains the
/// source list into `kept`, stopping as soon as a cap is reached.
struct EvidenceBudget {
    max_items: usize,
    max_bytes: usize,
    max_payload_bytes_per_item: usize,
    kept: Vec<EvidenceItem>,
    cumulative_bytes: usize,
    oversized_dropped: usize,
    unmeasurable_dropped: usize,
    limit_truncated: bool,
}

/// What one item's cap check decided: keep it, drop it under its own
/// counter, or stop the pass because an item-count or cumulative-byte cap
/// was reached.
enum Admission {
    Keep,
    Drop,
    Stop,
}

impl EvidenceBudget {
    fn new(original_len: usize, limits: &ResourceLimits) -> Self {
        let cap = limits.evidence;
        Self {
            max_items: cap.max_items_per_file.saturating_sub(1),
            max_bytes: cap
                .max_payload_bytes_per_file
                .saturating_sub(TRUNCATION_MARKER_RESERVE_BYTES),
            max_payload_bytes_per_item: cap.max_payload_bytes_per_item,
            kept: Vec::with_capacity(original_len.min(cap.max_items_per_file)),
            cumulative_bytes: 0,
            oversized_dropped: 0,
            unmeasurable_dropped: 0,
            limit_truncated: false,
        }
    }

    fn admit(&mut self, item: EvidenceItem) -> Admission {
        if self.kept.len() >= self.max_items {
            self.limit_truncated = true;
            return Admission::Stop;
        }
        let payload_bytes = match serde_json::to_vec(&item.payload) {
            Ok(bytes) => bytes.len(),
            Err(error) => {
                // A `Value` serializes infallibly today; if that ever stops
                // being true the item is dropped under its own count, never
                // passed off as an over-the-cap item.
                tracing::warn!(
                    key = %item.key,
                    error = %error,
                    "evidence payload failed to serialize; item dropped",
                );
                self.unmeasurable_dropped += 1;
                return Admission::Drop;
            }
        };
        if payload_bytes > self.max_payload_bytes_per_item {
            self.oversized_dropped += 1;
            return Admission::Drop;
        }
        if self.cumulative_bytes.saturating_add(payload_bytes) > self.max_bytes {
            self.limit_truncated = true;
            return Admission::Stop;
        }
        self.cumulative_bytes += payload_bytes;
        self.kept.push(item);
        Admission::Keep
    }

    fn anything_discarded(&self) -> bool {
        self.limit_truncated || self.oversized_dropped > 0 || self.unmeasurable_dropped > 0
    }
}
