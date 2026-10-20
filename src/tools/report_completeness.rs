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
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one `files.file_kind` value.
pub struct FileKindCount {
    /// One of `indexed`, `binary`, `skipped` — `files.file_kind`.
    pub kind: String,
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one `files.file_class` value.
pub struct FileClassCount {
    /// One of `authored`, `generated`, `vendored` — `files.file_class`.
    pub class: String,
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one recorded skip reason.
pub struct SkipReasonCount {
    /// A short stable code from `files.skip_reason`: `oversize` (over the
    /// per-file import ceiling), `unreadable` (the read failed) or
    /// `stat_failed` (the stat failed, so `files.byte_len` is NULL: size
    /// unknown, not 0). Empty when a skipped row carries no recorded reason.
    pub reason: String,
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one extraction-completeness value.
pub struct CompletenessCount {
    /// One of `Full`, `Partial`, `ContentOnly`, `Unavailable`. `ContentOnly`
    /// means the file's text is in `files.content` but no parser ran over it
    /// — the file is readable even though it has no structural evidence.
    /// `Unavailable` means no content is retained at all (binary, or skipped).
    pub extraction_completeness: String,
    pub count: i64,
}

/// Counts that say whether the rest of the response is the whole truth.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CompletenessEnvelope {
    /// Every `files` row by `file_kind`. `indexed` is the set with content and
    /// metadata — its structural evidence may still be missing, which
    /// `indexed_files_without_evidence` counts; `binary` was sampled and
    /// deliberately not parsed; `skipped` was discovered but never read, so
    /// it has no content.
    pub file_kind_counts: Vec<FileKindCount>,
    /// Every `files` row by `file_class` — where the code came from, derived
    /// from the path alone (`sync::classify`): `authored` (the project's own
    /// source), `generated` (a machine-written file: a `generated` path token
    /// or a codegen suffix such as `.pb.go` / `.g.dart` / `.min.js`), or
    /// `vendored` (a third-party tree: `vendor/`, `node_modules/`, …). A
    /// mechanic whose rows are dominated by one class can be summarised in a
    /// sentence instead of paged row by row. `vendored` subsumes `generated`
    /// when a path matches both. A path heuristic, not a verdict: it says
    /// where a file came from, never whether it matters.
    pub file_class_counts: Vec<FileClassCount>,
    /// `files` rows with `file_kind = 'skipped'`, i.e. files that exist in the
    /// project but are not in the index.
    pub skipped_file_count: i64,
    /// Those skipped rows grouped by `skip_reason`. This is the "why" paired
    /// with `index_partial`. A file with no representable `files.path` (a
    /// non-UTF-8 name) cannot appear here; it is counted by
    /// `unrepresentable_path_count` instead.
    pub skipped_by_reason: Vec<SkipReasonCount>,
    /// Every `files` row by `extraction_completeness`, so "text present but
    /// not parsed" (`ContentOnly`) is visible as itself instead of vanishing
    /// into a parser-failure count.
    pub extraction_completeness_counts: Vec<CompletenessCount>,
    /// True when the index does not cover every file discovery found for
    /// this report: an existence row is `skipped`, or a discovered file's
    /// path has no UTF-8 representation and therefore no `files` row at all.
    /// False means every discovered file is represented by a row here.
    pub index_partial: bool,
    /// Discovered files whose project-relative path is not valid UTF-8, so
    /// no `files` row can represent them and they cannot be stored, skipped
    /// or named in this report. Counted by a discovery walk run for this
    /// report — the index itself cannot hold the path — and included in
    /// `index_partial`.
    pub unrepresentable_path_count: i64,
    /// Files whose stored evidence was cut short by an evidence cap: each
    /// carries the `evidence:truncated` diagnostic appended by
    /// `sync::sample::apply_evidence_limits`. Their structural record is
    /// incomplete in a way this report cannot recover, so an agent counting
    /// `evidence_counts` for them is counting a floor, not a total.
    pub evidence_capped_file_count: i64,
    /// `indexed` rows with no evidence at all — content is present and
    /// readable, but the index holds nothing structural about them (a
    /// language with no loaded parser, or a file that parsed to nothing).
    /// Binary and skipped rows are excluded: no evidence is their expected
    /// state, not a gap.
    pub indexed_files_without_evidence: i64,
}

/// Builds the envelope from the current revision's `files` table.
///
/// `unrepresentable_path_count` is discovery's count of files whose path
/// cannot be stored in `files` at all (a non-UTF-8 name); it is passed in
/// because the index has no row that could carry it, and it makes
/// `index_partial` truthful when such a file exists.
///
/// # Errors
///
/// Returns a database error if any of the grouped counts fails.
pub(super) fn build(
    connection: &Transaction<'_>,
    unrepresentable_path_count: i64,
) -> Result<CompletenessEnvelope, rusqlite::Error> {
    let file_kind_counts = grouped_counts(
        connection,
        "SELECT file_kind, count(*) FROM files GROUP BY file_kind ORDER BY file_kind",
        |kind, count| FileKindCount { kind, count },
    )?;
    let file_class_counts = grouped_counts(
        connection,
        "SELECT file_class, count(*) FROM files GROUP BY file_class ORDER BY file_class",
        |class, count| FileClassCount { class, count },
    )?;
    let skipped_by_reason = grouped_counts(
        connection,
        "SELECT skip_reason, count(*) FROM files WHERE file_kind = 'skipped' \
         GROUP BY skip_reason ORDER BY skip_reason",
        |reason, count| SkipReasonCount { reason, count },
    )?;
    let extraction_completeness_counts = grouped_counts(
        connection,
        "SELECT extraction_completeness, count(*) FROM files \
         GROUP BY extraction_completeness ORDER BY extraction_completeness",
        |extraction_completeness, count| CompletenessCount {
            extraction_completeness,
            count,
        },
    )?;

    let skipped_file_count = file_kind_counts
        .iter()
        .find(|entry| entry.kind == "skipped")
        .map_or(0, |entry| entry.count);

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
    })
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
