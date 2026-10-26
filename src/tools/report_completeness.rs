//! The completeness block of `report`: what the index does *not* contain.
//!
//! `report` used to present bare counts — `file_count` was
//! `count(*) FROM files` — with nothing saying whether a number was the whole
//! truth. A file discovered but skipped (oversize, unreadable) had no row at
//! all, so it was absent from every count; a file whose text was indexed but
//! never parsed was indistinguishable from one whose parser failed. The tools
//! contract is that every number is quotable as-is, so each of those gaps is
//! now a count in this block.
//!
//! Facts only, in the same register as the rest of `report`: no scores, no
//! severity, no ordering by importance. `skipped_by_reason` is ordered by the
//! reason code and the other breakdowns by their value text, so the ordering
//! carries no judgment a reader could mistake for a ranking.

use rusqlite::Transaction;

pub use super::report::{
    CompletenessCount, CompletenessEnvelope, FileClassCount, FileKindCount, SkipReasonCount,
};

/// Builds the envelope from the current revision's `files` table.
///
/// `unrepresentable_path_count` is the persisted count of files whose path
/// cannot be stored in `files` at all (a non-UTF-8 name); it is passed in
/// because the index has no `files` row that could carry it, and it makes
/// `index_partial` truthful when such a file exists.
///
/// # Errors
///
/// Returns a database error if any of the grouped counts fails.
pub(super) fn build(
    connection: &Transaction<'_>,
    unrepresentable_path_count: i64,
) -> Result<CompletenessEnvelope, rusqlite::Error> {
    let grouped = read_grouped_counts(connection)?;
    let skipped_file_count = grouped
        .file_kind_counts
        .iter()
        .find(|entry| entry.kind == "skipped")
        .map_or(0, |entry| entry.count);
    let (evidence_capped_file_count, indexed_files_without_evidence) = read_gap_counts(connection)?;
    let prose_files_not_parsed: i64 = connection.query_row(
        "SELECT count(*) FROM files WHERE file_kind = 'indexed' AND language = ?1",
        [crate::parse::PROSE_LANGUAGE],
        |row| row.get(0),
    )?;

    Ok(CompletenessEnvelope {
        file_kind_counts: grouped.file_kind_counts,
        file_class_counts: grouped.file_class_counts,
        skipped_file_count,
        skipped_by_reason: grouped.skipped_by_reason,
        extraction_completeness_counts: grouped.extraction_completeness_counts,
        index_partial: skipped_file_count > 0 || unrepresentable_path_count > 0,
        unrepresentable_path_count,
        evidence_capped_file_count,
        indexed_files_without_evidence,
        prose_files_not_parsed,
    })
}

/// Every grouped breakdown, read in the historical order.
struct GroupedCounts {
    file_kind_counts: Vec<FileKindCount>,
    file_class_counts: Vec<FileClassCount>,
    skipped_by_reason: Vec<SkipReasonCount>,
    extraction_completeness_counts: Vec<CompletenessCount>,
}

fn read_grouped_counts(connection: &Transaction<'_>) -> Result<GroupedCounts, rusqlite::Error> {
    Ok(GroupedCounts {
        file_kind_counts: grouped_counts(
            connection,
            "SELECT file_kind, count(*) FROM files GROUP BY file_kind ORDER BY file_kind",
            |kind, count| FileKindCount { kind, count },
        )?,
        file_class_counts: grouped_counts(
            connection,
            "SELECT file_class, count(*) FROM files GROUP BY file_class ORDER BY file_class",
            |class, count| FileClassCount { class, count },
        )?,
        skipped_by_reason: grouped_counts(
            connection,
            "SELECT skip_reason, count(*) FROM files WHERE file_kind = 'skipped' \
             GROUP BY skip_reason ORDER BY skip_reason",
            |reason, count| SkipReasonCount { reason, count },
        )?,
        extraction_completeness_counts: grouped_counts(
            connection,
            "SELECT extraction_completeness, count(*) FROM files \
             GROUP BY extraction_completeness ORDER BY extraction_completeness",
            |extraction_completeness, count| CompletenessCount {
                extraction_completeness,
                count,
            },
        )?,
    })
}

/// The two evidence-gap counts.
fn read_gap_counts(connection: &Transaction<'_>) -> Result<(i64, i64), rusqlite::Error> {
    // One `evidence:truncated` row per file (it is unique per file_id/key),
    // so a row count is the file count.
    let evidence_capped_file_count: i64 = connection.query_row(
        "SELECT count(*) FROM evidence WHERE key = 'evidence:truncated'",
        [],
        |row| row.get(0),
    )?;
    let indexed_files_without_evidence: i64 = connection.query_row(
        "SELECT count(*) FROM files f \
         WHERE f.file_kind = 'indexed' \
           AND NOT EXISTS (SELECT 1 FROM evidence e WHERE e.file_id = f.id)",
        [],
        |row| row.get(0),
    )?;
    Ok((evidence_capped_file_count, indexed_files_without_evidence))
}

/// Runs a `SELECT <value>, count(*) … GROUP BY <value>` and maps each row
/// through `make`. A NULL group value (e.g. a skipped row with no recorded
/// `skip_reason`) becomes the empty string rather than being dropped, so the
/// counts always sum to the rows they group.
fn grouped_counts<T>(
    connection: &Transaction<'_>,
    sql: &str,
    make: impl Fn(String, i64) -> T,
) -> Result<Vec<T>, rusqlite::Error> {
    let mut statement = connection.prepare(sql)?;
    statement
        .query_map([], |row| {
            let value: Option<String> = row.get(0)?;
            Ok(make(value.unwrap_or_default(), row.get(1)?))
        })?
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::tools::{ReportRequest, report};
    use rmcp::handler::server::wrapper::Parameters;
    use std::fs;

    fn activate(root: &std::path::Path) {
        fs::create_dir_all(root.join(".planning").join("slugaudit"))
            .expect("create activation dir");
    }

    /// A file the walk discovers but deliberately does not index must be
    /// visible rather than absent: a `skipped` row with its reason and no
    /// content, plus a response that says the index is partial. Needs the
    /// real publish path, so the oversize ceiling comes from the project's
    /// own config.
    #[test]
    fn a_discovered_but_skipped_file_is_visible_as_a_skipped_row_and_a_partial_index() {
        let project = tempfile::tempdir().expect("project dir");
        activate(project.path());
        fs::write(
            project
                .path()
                .join(".planning")
                .join("slugaudit")
                .join("config.json"),
            br#"{"max_file_bytes": 50000}"#,
        )
        .expect("write project config");
        fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
        fs::write(project.path().join("huge.rs"), vec![b'x'; 100_000])
            .expect("write oversize file");

        let response = report(
            &Parameters(ReportRequest {
                path: project.path().to_string_lossy().into_owned(),
            }),
            &crate::progress::NoopProgressSink,
            &crate::sync::SourceSyncManager::default(),
        )
        .expect("report succeeds");

        assert_eq!(
            response.0.file_count, 2,
            "both discovered files have a row, indexed or not"
        );
        let envelope = &response.0.completeness;
        assert!(
            envelope.index_partial,
            "a skipped file makes the index partial"
        );
        assert_eq!(envelope.skipped_file_count, 1);
        assert_eq!(envelope.skipped_by_reason.len(), 1);
        assert_eq!(envelope.skipped_by_reason[0].reason, "oversize");
        assert_eq!(envelope.skipped_by_reason[0].count, 1);
        assert_eq!(
            envelope
                .file_kind_counts
                .iter()
                .find(|entry| entry.kind == "indexed")
                .map(|entry| entry.count),
            Some(1)
        );

        let database = project
            .path()
            .join(".planning")
            .join("slugaudit")
            .join("project.db");
        let connection =
            crate::store::open_read_only(&database).expect("open the published database read-only");
        let (kind, reason, content, completeness, outcome, algorithm): (
            String,
            Option<String>,
            Option<String>,
            String,
            String,
            String,
        ) = connection
            .query_row(
                "SELECT file_kind, skip_reason, content, extraction_completeness, parse_outcome, \
                        hash_algorithm \
                 FROM files WHERE path = 'huge.rs'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .expect("the skipped file must have an existence row");
        assert_eq!(kind, "skipped");
        assert_eq!(reason.as_deref(), Some("oversize"));
        assert_eq!(content, None, "no content is retained for a skipped file");
        assert_eq!(completeness, "Unavailable");
        assert_eq!(outcome, "NotAttempted");
        assert_eq!(
            algorithm, "stat-identity",
            "the skipped row's identity is stat-derived, never a content hash"
        );
    }
}
