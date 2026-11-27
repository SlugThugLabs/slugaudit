//! `report` completeness envelope over hand-built `files` rows: parser-failure
//! rows and totals, and the kind / reason / completeness grouping.

use super::tests::{build_report, database_with_one_revision, insert_indexed_file};
use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

/// A text file that is fully indexed but has no parser must appear in the
/// report as `ContentOnly` — its content is present and readable — rather
/// than as an indistinguishable `NotAttempted` failure; a parse that really
/// ran and then failed must still read as `Failed`.
#[test]
fn parser_failure_rows_distinguish_content_only_from_a_failed_parse() -> TestResult {
    let (_dir, mut connection) = database_with_one_revision()?;
    insert_indexed_file(
        &connection,
        "notes.txt",
        "Unavailable",
        "NotAttempted",
        None,
        "ContentOnly",
    )?;
    insert_indexed_file(
        &connection,
        "broken.rs",
        "Available",
        "Failed",
        Some("boom"),
        "ContentOnly",
    )?;

    let tx = connection.transaction().check("begin transaction")?;
    let response = build_report(&tx, "r1".to_owned()).check("build report")?;

    let notes = response
        .parser_failure_files
        .iter()
        .find(|file| file.path == "notes.txt")
        .check("indexed-but-unparsed file is listed")?;
    ensure_eq(
        &notes.extraction_completeness,
        &"ContentOnly",
        "notes.extraction_completeness == \"ContentOnly\"",
    )?;
    ensure_eq(
        &notes.parser_availability,
        &"Unavailable",
        "notes.parser_availability == \"Unavailable\"",
    )?;
    ensure_eq(
        &notes.parse_outcome,
        &"NotAttempted",
        "notes.parse_outcome == \"NotAttempted\"",
    )?;

    let broken = response
        .parser_failure_files
        .iter()
        .find(|file| file.path == "broken.rs")
        .check("genuinely failed parse is listed")?;
    ensure_eq(
        &broken.parse_outcome,
        &"Failed",
        "broken.parse_outcome == \"Failed\"",
    )?;
    ensure_eq(
        &broken.parse_error_reason.as_deref(),
        &Some("boom"),
        "broken.parse_error_reason.as_deref() == Some(\"boom\")",
    )?;
    Ok(())
}

#[test]
fn parser_failure_count_equals_the_list_when_under_the_cap() -> TestResult {
    let (_dir, mut connection) = database_with_one_revision()?;
    for name in ["a.txt", "b.txt", "c.txt"] {
        insert_indexed_file(
            &connection,
            name,
            "Unavailable",
            "NotAttempted",
            None,
            "ContentOnly",
        )?;
    }

    let tx = connection.transaction().check("begin transaction")?;
    let response = build_report(&tx, "r1".to_owned()).check("build report")?;

    ensure_eq(
        &response.parser_failure_files.len(),
        &3,
        "response.parser_failure_files.len() == 3",
    )?;
    ensure_eq(
        &response.parser_failure_count,
        &3,
        "response.parser_failure_count == 3",
    )?;
    Ok(())
}

#[test]
fn parser_failure_count_exceeds_the_capped_list_when_over_the_cap() -> TestResult {
    let (_dir, mut connection) = database_with_one_revision()?;
    let total = i64::try_from(MAX_PARSER_FAILURE_FILES).check("cap fits in i64")? + 5;
    for index in 0..total {
        insert_indexed_file(
            &connection,
            &format!("f{index:03}.txt"),
            "Unavailable",
            "NotAttempted",
            None,
            "ContentOnly",
        )?;
    }

    let tx = connection.transaction().check("begin transaction")?;
    let response = build_report(&tx, "r1".to_owned()).check("build report")?;

    ensure_eq(
        &response.parser_failure_files.len(),
        &MAX_PARSER_FAILURE_FILES,
        "response.parser_failure_files.len() == MAX_PARSER_FAILURE_FILES",
    )?;
    ensure_eq(
        &response.parser_failure_count,
        &total,
        "response.parser_failure_count == total",
    )?;
    Ok(())
}

/// A database with two indexed files (one carrying the evidence-cap marker)
/// and one skipped file.
fn database_with_envelope_rows() -> TestResult<(tempfile::TempDir, rusqlite::Connection)> {
    let (dir, connection) = database_with_one_revision()?;
    insert_indexed_file(
        &connection,
        "notes.txt",
        "Unavailable",
        "NotAttempted",
        None,
        "ContentOnly",
    )?;
    insert_indexed_file(
        &connection,
        "capped.rs",
        "Available",
        "Succeeded",
        None,
        "Full",
    )?;
    let capped_id: i64 = connection
        .query_row("SELECT id FROM files WHERE path = 'capped.rs'", [], |row| {
            row.get(0)
        })
        .check("capped.rs id")?;
    connection
        .execute(
            "INSERT INTO evidence \
             (file_id, key, kind, origin, span_availability, payload) \
             VALUES (?1, 'evidence:truncated', 'Diagnostic', 'SourceContent', \
                     'DerivedEvidence', '{}')",
            [capped_id],
        )
        .check("insert the evidence-cap marker")?;
    connection
        .execute(
            "INSERT INTO files \
             (path, file_kind, skip_reason, content, content_hash, hash_algorithm, byte_len, \
              language_detected, parser_availability, parse_outcome, extraction_completeness, \
              last_revision_id) \
             VALUES ('huge.rs', 'skipped', 'oversize', NULL, 'stat', 'stat-identity', 100000, \
                     0, 'Unavailable', 'NotAttempted', 'Unavailable', 1)",
            [],
        )
        .check("insert skipped row")?;
    Ok((dir, connection))
}

/// The envelope counts kinds, skip reasons, extraction completeness, and the
/// two evidence gaps independently. Asserted over hand-built rows so each
/// bucket is unambiguous: a `ContentOnly` row (text indexed, nothing parsed)
/// must show up as `ContentOnly` and not as a parser failure, a skipped row
/// must be counted as skipped rather than as either, and a row whose evidence
/// was cut short by a cap must not read as a complete one.
#[test]
fn completeness_envelope_separates_kinds_reasons_and_completeness() -> TestResult {
    let (_dir, mut connection) = database_with_envelope_rows()?;
    let tx = connection.transaction().check("begin transaction")?;
    let response = build_report(&tx, "r1".to_owned()).check("build report")?;

    let kind_count = |kind: &str| {
        response
            .completeness
            .file_kind_counts
            .iter()
            .find(|entry| entry.kind == kind)
            .map(|entry| entry.count)
    };
    ensure_eq(
        &kind_count("indexed"),
        &Some(2),
        "kind_count(\"indexed\") == Some(2)",
    )?;
    ensure_eq(
        &kind_count("skipped"),
        &Some(1),
        "kind_count(\"skipped\") == Some(1)",
    )?;
    ensure_eq(
        &response.completeness.skipped_file_count,
        &1,
        "response.completeness.skipped_file_count == 1",
    )?;
    ensure(
        response.completeness.index_partial,
        "response.completeness.index_partial",
    )?;
    ensure_eq(
        &response.completeness.skipped_by_reason.len(),
        &1,
        "response.completeness.skipped_by_reason.len() == 1",
    )?;
    let skipped = response
        .completeness
        .skipped_by_reason
        .first()
        .check("response.completeness.skipped_by_reason[0]")?;
    ensure_eq(
        &skipped.reason,
        &"oversize",
        "response.completeness.skipped_by_reason[0].reason == \"oversize\"",
    )?;
    ensure_eq(
        &skipped.count,
        &1,
        "response.completeness.skipped_by_reason[0].count == 1",
    )?;
    let completeness_for = |value: &str| {
        response
            .completeness
            .extraction_completeness_counts
            .iter()
            .find(|entry| entry.extraction_completeness == value)
            .map(|entry| entry.count)
    };
    ensure_eq(
        &completeness_for("ContentOnly"),
        &Some(1),
        "completeness_for(\"ContentOnly\") == Some(1)",
    )?;
    ensure_eq(
        &completeness_for("Full"),
        &Some(1),
        "completeness_for(\"Full\") == Some(1)",
    )?;
    ensure_eq(
        &completeness_for("Unavailable"),
        &Some(1),
        "completeness_for(\"Unavailable\") == Some(1)",
    )?;
    ensure_eq(
        &response.completeness.indexed_files_without_evidence,
        &1,
        "notes.txt has content but no structural evidence; capped.rs has evidence and the \
         skipped row is not an indexed gap",
    )?;
    ensure_eq(
        &response.completeness.evidence_capped_file_count,
        &1,
        "capped.rs carries the truncation marker",
    )?;
    ensure_eq(
        &response.parser_failure_count,
        &1,
        "a skipped file is never parsed, so it is not a parser failure",
    )?;
    Ok(())
}

/// A skipped row with no `skip_reason` breaks the index invariant; the report
/// fails naming the column instead of counting it under an invented label.
#[test]
fn a_skipped_row_without_a_skip_reason_fails_the_report_naming_the_column() -> TestResult {
    let (_dir, mut connection) = database_with_one_revision()?;
    connection
        .execute(
            "INSERT INTO files \
             (path, file_kind, skip_reason, content, content_hash, hash_algorithm, byte_len, \
              language_detected, parser_availability, parse_outcome, extraction_completeness, \
              last_revision_id) \
             VALUES ('huge.rs', 'skipped', NULL, NULL, 'stat', 'stat-identity', 100000, \
                     0, 'Unavailable', 'NotAttempted', 'Unavailable', 1)",
            [],
        )
        .check("insert skipped row without a reason")?;

    let tx = connection.transaction().check("begin transaction")?;
    let error = build_report(&tx, "r1".to_owned()).check_err("a NULL skip_reason must fail")?;

    let rusqlite::Error::InvalidColumnType(_, column, _) = &error else {
        return Err(format!(
            "expected InvalidColumnType naming the column: {error:?}"
        ))
        .check("unexpected outcome");
    };
    ensure_eq(&column, &"skip_reason", "column == \"skip_reason\"")?;
    Ok(())
}

/// A file the walk discovers but deliberately does not index must be
/// visible rather than absent: a `skipped` row with its reason and no
/// content, plus a response that says the index is partial.
#[test]
fn a_discovered_but_skipped_file_is_visible_as_a_skipped_row_and_a_partial_index() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    std::fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .check("create activation dir")?;
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
    let _config = crate::tools::test_support::use_global_config(r#"{"max_file_bytes": 50000}"#)?;
    std::fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    std::fs::write(project.path().join("huge.rs"), vec![b'x'; 100_000])
        .check("write oversize file")?;

    let response = report(
        &rmcp::handler::server::wrapper::Parameters(ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("report succeeds")?;

    ensure_eq(
        &response.0.file_count,
        &2,
        "both discovered files have a row, indexed or not",
    )?;
    let envelope = &response.0.completeness;
    ensure(
        envelope.index_partial,
        "a skipped file makes the index partial",
    )?;
    ensure_eq(
        &envelope.skipped_file_count,
        &1,
        "envelope.skipped_file_count == 1",
    )?;
    ensure_eq(
        &envelope.skipped_by_reason.len(),
        &1,
        "envelope.skipped_by_reason.len() == 1",
    )?;
    let skipped = envelope
        .skipped_by_reason
        .first()
        .check("envelope.skipped_by_reason[0]")?;
    ensure_eq(
        &skipped.reason,
        &"oversize",
        "envelope.skipped_by_reason[0].reason == \"oversize\"",
    )?;
    ensure_eq(
        &skipped.count,
        &1,
        "envelope.skipped_by_reason[0].count == 1",
    )?;
    ensure_eq(
        &envelope
            .file_kind_counts
            .iter()
            .find(|entry| entry.kind == "indexed")
            .map(|entry| entry.count),
        &Some(1),
        "indexed count == Some(1)",
    )?;

    ensure_skipped_row_is_stored(project.path())
}

/// The published database holds an existence row for `huge.rs`, with its
/// reason and no content.
fn ensure_skipped_row_is_stored(project: &std::path::Path) -> TestResult {
    let database = project
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let connection =
        crate::store::open_existing(&database).check("open the published database read-only")?;
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
        .check("the skipped file must have an existence row")?;
    ensure_eq(&kind, &"skipped", "kind == \"skipped\"")?;
    ensure_eq(
        &reason.as_deref(),
        &Some("oversize"),
        "reason.as_deref() == Some(\"oversize\")",
    )?;
    ensure_eq(&content, &None, "no content is retained for a skipped file")?;
    ensure_eq(
        &completeness,
        &"Unavailable",
        "completeness == \"Unavailable\"",
    )?;
    ensure_eq(&outcome, &"NotAttempted", "outcome == \"NotAttempted\"")?;
    ensure_eq(
        &algorithm,
        &"stat-identity",
        "the skipped row's identity is stat-derived, never a content hash",
    )?;
    Ok(())
}
