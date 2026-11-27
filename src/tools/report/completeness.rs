//! Completeness-envelope query for `report`.

use super::sql::{count_where, grouped_column_counts};
use super::{
    CompletenessCount, CompletenessEnvelope, FileClassCount, FileKindCount, SkipReasonCount,
};
use rusqlite::Transaction;

pub(super) fn build_completeness(
    conn: &Transaction<'_>,
) -> Result<CompletenessEnvelope, rusqlite::Error> {
    let unrepresentable_path_count = count_where(conn, "unrepresentable_paths", None)?;
    let file_kind_counts =
        grouped_column_counts(conn, "files", "file_kind", None, |kind, count| {
            FileKindCount { kind, count }
        })?;
    let file_class_counts =
        grouped_column_counts(conn, "files", "file_class", None, |class, count| {
            FileClassCount { class, count }
        })?;
    let skipped_by_reason = grouped_column_counts(
        conn,
        "files",
        "skip_reason",
        Some("file_kind = 'skipped'"),
        |reason, count| SkipReasonCount { reason, count },
    )?;
    let extraction_completeness_counts = grouped_column_counts(
        conn,
        "files",
        "extraction_completeness",
        None,
        |extraction_completeness, count| CompletenessCount {
            extraction_completeness,
            count,
        },
    )?;
    let skipped_file_count = file_kind_counts
        .iter()
        .find(|entry| entry.kind == "skipped")
        .map_or(0, |entry| entry.count);
    let evidence_capped_file_count =
        count_where(conn, "evidence", Some("key = 'evidence:truncated'"))?;
    let indexed_files_without_evidence = count_where(
        conn,
        "files f",
        Some(
            "f.file_kind = 'indexed' AND NOT EXISTS (SELECT 1 FROM evidence e WHERE e.file_id = f.id)",
        ),
    )?;
    let prose_files_not_parsed: i64 = conn.query_row(
        "SELECT count(*) FROM files WHERE file_kind = 'indexed' AND language = ?1",
        [crate::parse::PROSE_LANGUAGE],
        |row| row.get(0),
    )?;
    let (oversize_for_parse_file_count, parse_timeout_file_count) = conn.query_row(
        "SELECT count(*) FILTER (WHERE parse_error_reason = ?1), \
                count(*) FILTER (WHERE parse_error_reason = ?2) \
         FROM files WHERE file_kind = 'indexed'",
        [
            crate::model::ParseOutcome::OVERSIZE_FOR_PARSE,
            crate::model::ParseOutcome::PARSE_TIMEOUT,
        ],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(CompletenessEnvelope {
        file_kind_counts,
        file_class_counts,
        skipped_file_count,
        skipped_by_reason,
        extraction_completeness_counts,
        index_partial: skipped_file_count > 0 || unrepresentable_path_count > 0,
        unrepresentable_path_count,
        evidence_capped_file_count,
        indexed_files_without_evidence,
        prose_files_not_parsed,
        oversize_for_parse_file_count,
        parse_timeout_file_count,
    })
}
