use super::*;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;

fn activate(root: &std::path::Path) {
    fs::create_dir_all(root.join(".planning").join("slugaudit")).expect("create activation dir");
}

#[test]
fn reports_real_counts_for_an_active_project() {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() {}\npub fn b() {}\n",
    )
    .expect("write fixture");

    let response = report(
        &Parameters(ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("report succeeds");

    assert_eq!(response.0.file_count, 1);
    assert!(
        response
            .0
            .languages
            .iter()
            .any(|entry| entry.language == "rust")
    );
    assert!(
        response
            .0
            .evidence_counts
            .iter()
            .any(|entry| entry.kind == "Structure")
    );
    assert_eq!(response.0.parser_failure_count, 0);
    assert!(
        response.0.parser_failure_files.is_empty(),
        "a cleanly-parsing project should have no parser failure files"
    );
    assert_eq!(response.0.open_finding_count, 0);
    // A single-file project with no imports has no dependency edges.
    assert!(
        response.0.import_resolution.is_empty(),
        "a single-file project has no import edges"
    );
    assert_eq!(response.0.diagnostic_count, 0);
}

/// `unsupported_language_unresolved_count` must count only the `Unresolved`
/// edges coming from a file whose language `graph::resolve` doesn't model
/// at all (here: `go`), not `Unresolved` edges from a supported language
/// with a genuinely broken import (here: `python`). Builds the DB rows
/// directly rather than through the real parse/resolve pipeline — a real
/// Go import requires the language pack's Go import extraction to behave a
/// specific way, which isn't what this test is pinning down; the pipeline
/// wiring itself (the per-edge resolver check reaching this exact query)
/// is what matters here.
#[test]
fn unsupported_language_unresolved_is_counted_separately_from_supported_language_unresolved() {
    use crate::store::open_read_write;

    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");

    connection
        .execute(
            "INSERT INTO revisions \
             (revision_id, manifest_hash, parser_pack_version, created_at_unix, is_current) \
             VALUES ('r1', 'h', '1.0.0', 0, 1)",
            [],
        )
        .expect("insert revision");

    for (path, language) in [("a.go", "go"), ("b.py", "python"), ("c.kt", "kotlin")] {
        connection
            .execute(
                "INSERT INTO files \
                 (path, file_kind, content, content_hash, hash_algorithm, byte_len, \
                  language, language_detected, parser_availability, parse_outcome, \
                  extraction_completeness, last_revision_id) \
                 VALUES (?1, 'indexed', '', 'h', 'blake3', 0, \
                         ?2, 1, 'Available', 'Succeeded', 'Full', 1)",
                rusqlite::params![path, language],
            )
            .expect("insert file");
    }
    let go_id: i64 = connection
        .query_row("SELECT id FROM files WHERE path = 'a.go'", [], |row| {
            row.get(0)
        })
        .expect("go file id");
    let py_id: i64 = connection
        .query_row("SELECT id FROM files WHERE path = 'b.py'", [], |row| {
            row.get(0)
        })
        .expect("python file id");
    let kotlin_id: i64 = connection
        .query_row("SELECT id FROM files WHERE path = 'c.kt'", [], |row| {
            row.get(0)
        })
        .expect("kotlin file id");

    connection
        .execute(
            "INSERT INTO dependency_edges (from_file_id, raw_import_text, resolution_kind) \
             VALUES (?1, 'import \"fmt\"', 'Unresolved')",
            [go_id],
        )
        .expect("insert go edge");
    connection
        .execute(
            "INSERT INTO dependency_edges (from_file_id, raw_import_text, resolution_kind) \
             VALUES (?1, 'import missing_module', 'Unresolved')",
            [py_id],
        )
        .expect("insert python edge");
    connection
        .execute(
            "INSERT INTO dependency_edges (from_file_id, raw_import_text, resolution_kind) \
             VALUES (?1, 'import kotlin.math.max', 'Unresolved')",
            [kotlin_id],
        )
        .expect("insert kotlin edge");

    let tx = connection.transaction().expect("begin transaction");
    let response = build_report(&tx, "r1".to_owned()).expect("build report");

    assert_eq!(
        response.unsupported_language_unresolved_count, 1,
        "only the go edge is from import syntax the resolver can't parse; \
         the kotlin edge (syntax parsed, no file match) must not count"
    );
    let total_unresolved: i64 = response
        .import_resolution
        .iter()
        .find(|entry| entry.kind == "Unresolved")
        .map_or(0, |entry| entry.count);
    assert_eq!(
        total_unresolved, 3,
        "all three edges must still count toward the general Unresolved bucket"
    );
}

#[test]
fn malformed_language_value_is_reported_instead_of_dropped() {
    use crate::store::open_read_write;

    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    connection
        .execute(
            "INSERT INTO revisions \
             (revision_id, manifest_hash, parser_pack_version, created_at_unix, is_current) \
             VALUES ('r1', 'h', '1.0.0', 0, 1)",
            [],
        )
        .expect("insert revision");
    connection
        .execute(
            "INSERT INTO files \
             (path, file_kind, content, content_hash, hash_algorithm, byte_len, language, \
              language_detected, parser_availability, parse_outcome, extraction_completeness, \
              last_revision_id) \
             VALUES ('broken.bin', 'indexed', '', 'h', 'blake3', 0, CAST(X'FF' AS BLOB), \
                     1, 'Unavailable', 'Failed', 'Unavailable', 1)",
            [],
        )
        .expect("insert malformed language");

    let tx = connection.transaction().expect("begin transaction");
    assert!(build_report(&tx, "r1".to_owned()).is_err());
}

#[test]
fn report_on_an_inactive_project_is_a_typed_error_not_a_panic() {
    let project = tempfile::tempdir().expect("project dir");
    let result = report(
        &Parameters(ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    );
    assert!(result.is_err());
}
