use super::*;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;

fn activate(root: &std::path::Path) {
    fs::create_dir_all(root.join(".planning").join("slugaudit")).expect("create activation dir");
}

#[test]
fn reports_credential_pattern_file_count() {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
    fs::write(project.path().join(".env"), b"API_KEY=secret\n").expect("write .env");
    fs::create_dir_all(project.path().join("keys")).expect("keys dir");
    fs::write(
        project.path().join("keys").join("cred.pem"),
        b"-----BEGIN PRIVATE KEY-----\n",
    )
    .expect("write pem");
    // Ordinary source must never match the credential patterns.
    fs::write(project.path().join("pem_util.rs"), b"pub fn parse() {}\n")
        .expect("write ordinary source");

    let response = report(
        &Parameters(ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("report succeeds");

    assert_eq!(response.0.file_count, 4);
    assert_eq!(
        response.0.credential_pattern_file_count, 2,
        ".env and keys/cred.pem match credential patterns; pem_util.rs and lib.rs must not"
    );
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
/// edges coming from import syntax the resolver can't parse at all (here:
/// `go`'s `import "fmt"`), not `Unresolved` edges from a modeled language
/// with a genuinely broken import (here: `python`). Since C11 the verdict
/// is cached on the edge at write time (`syntax_unmodeled`), so this test
/// pins the *SQL count* over that cached column — the write-time
/// computation itself is covered by the publish_edges pipeline tests.
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

    // syntax_unmodeled mirrors what `revision_edges::resolve_and_store`
    // would cache at write time for each raw import text: Go's quoted
    // `import "fmt"` yields no reference (1), while the modeled python
    // and kotlin forms yield a reference that just doesn't match a file
    // (0).
    connection
        .execute(
            "INSERT INTO dependency_edges (from_file_id, raw_import_text, resolution_kind, syntax_unmodeled) \
             VALUES (?1, 'import \"fmt\"', 'Unresolved', 1)",
            [go_id],
        )
        .expect("insert go edge");
    connection
        .execute(
            "INSERT INTO dependency_edges (from_file_id, raw_import_text, resolution_kind, syntax_unmodeled) \
             VALUES (?1, 'import missing_module', 'Unresolved', 0)",
            [py_id],
        )
        .expect("insert python edge");
    connection
        .execute(
            "INSERT INTO dependency_edges (from_file_id, raw_import_text, resolution_kind, syntax_unmodeled) \
             VALUES (?1, 'import kotlin.math.max', 'Unresolved', 0)",
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

/// The envelope carries where files came from as its own breakdown, so an
/// agent looking at a mechanic dominated by third-party or machine-written
/// rows can read the class counts off the report instead of re-deriving the
/// classification in SQL.
#[test]
fn the_completeness_envelope_groups_files_by_class() {
    let (_dir, mut connection) = database_with_one_revision();
    for (path, class) in [
        ("src/lib.rs", "authored"),
        ("src/generated/api.rs", "generated"),
        ("vendor/dep/lib.rs", "vendored"),
    ] {
        connection
            .execute(
                "INSERT INTO files \
                 (path, file_kind, file_class, content, content_hash, hash_algorithm, byte_len, \
                  language_detected, parser_availability, parse_outcome, extraction_completeness, \
                  last_revision_id) \
                 VALUES (?1, 'indexed', ?2, '', 'h', 'blake3', 0, 0, 'Available', 'Succeeded', \
                         'Full', 1)",
                rusqlite::params![path, class],
            )
            .expect("insert file");
    }

    let tx = connection.transaction().expect("begin transaction");
    let response = build_report(&tx, "r1".to_owned()).expect("build report");

    let class_count = |class: &str| {
        response
            .completeness
            .file_class_counts
            .iter()
            .find(|entry| entry.class == class)
            .map(|entry| entry.count)
    };
    assert_eq!(class_count("authored"), Some(1));
    assert_eq!(class_count("generated"), Some(1));
    assert_eq!(class_count("vendored"), Some(1));
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

/// A project DB with one revision and no files, ready for direct `files`
/// inserts — the report tests below assert SQL over hand-built rows rather
/// than running the full sync pipeline.
fn database_with_one_revision() -> (tempfile::TempDir, rusqlite::Connection) {
    let db_dir = tempfile::tempdir().expect("db dir");
    let connection =
        crate::store::open_read_write(&db_dir.path().join("project.db")).expect("open db");
    connection
        .execute(
            "INSERT INTO revisions \
             (revision_id, manifest_hash, parser_pack_version, created_at_unix, is_current) \
             VALUES ('r1', 'h', '1.0.0', 0, 1)",
            [],
        )
        .expect("insert revision");
    (db_dir, connection)
}

/// Inserts one indexed text file with the given parser columns, so a test
/// can reproduce a specific failure shape without a real parser.
fn insert_indexed_file(
    connection: &rusqlite::Connection,
    path: &str,
    availability: &str,
    outcome: &str,
    error_reason: Option<&str>,
    completeness: &str,
) {
    connection
        .execute(
            "INSERT INTO files \
             (path, file_kind, content, content_hash, hash_algorithm, byte_len, language, \
              language_detected, parser_availability, parse_outcome, parse_error_reason, \
              extraction_completeness, last_revision_id) \
             VALUES (?1, 'indexed', '', 'h', 'blake3', 0, NULL, 0, ?2, ?3, ?4, ?5, 1)",
            rusqlite::params![path, availability, outcome, error_reason, completeness],
        )
        .expect("insert file");
}

/// A text file that is fully indexed but has no parser must appear in the
/// report as `ContentOnly` — its content is present and readable — rather
/// than as an indistinguishable `NotAttempted` failure; a parse that really
/// ran and then failed must still read as `Failed`.
#[test]
fn parser_failure_rows_distinguish_content_only_from_a_failed_parse() {
    let (_dir, mut connection) = database_with_one_revision();
    insert_indexed_file(
        &connection,
        "notes.txt",
        "Unavailable",
        "NotAttempted",
        None,
        "ContentOnly",
    );
    insert_indexed_file(
        &connection,
        "broken.rs",
        "Available",
        "Failed",
        Some("boom"),
        "ContentOnly",
    );

    let tx = connection.transaction().expect("begin transaction");
    let response = build_report(&tx, "r1".to_owned()).expect("build report");

    let notes = response
        .parser_failure_files
        .iter()
        .find(|file| file.path == "notes.txt")
        .expect("indexed-but-unparsed file is listed");
    assert_eq!(notes.extraction_completeness, "ContentOnly");
    assert_eq!(notes.parser_availability, "Unavailable");
    assert_eq!(notes.parse_outcome, "NotAttempted");

    let broken = response
        .parser_failure_files
        .iter()
        .find(|file| file.path == "broken.rs")
        .expect("genuinely failed parse is listed");
    assert_eq!(broken.parse_outcome, "Failed");
    assert_eq!(broken.parse_error_reason.as_deref(), Some("boom"));
}

#[test]
fn parser_failure_files_total_equals_the_list_when_under_the_cap() {
    let (_dir, mut connection) = database_with_one_revision();
    for name in ["a.txt", "b.txt", "c.txt"] {
        insert_indexed_file(
            &connection,
            name,
            "Unavailable",
            "NotAttempted",
            None,
            "ContentOnly",
        );
    }

    let tx = connection.transaction().expect("begin transaction");
    let response = build_report(&tx, "r1".to_owned()).expect("build report");

    assert_eq!(response.parser_failure_files.len(), 3);
    assert_eq!(response.parser_failure_files_total, 3);
}

#[test]
fn parser_failure_files_total_exceeds_the_capped_list_when_over_the_cap() {
    let (_dir, mut connection) = database_with_one_revision();
    let total = MAX_PARSER_FAILURE_FILES + 5;
    for index in 0..total {
        insert_indexed_file(
            &connection,
            &format!("f{index:03}.txt"),
            "Unavailable",
            "NotAttempted",
            None,
            "ContentOnly",
        );
    }

    let tx = connection.transaction().expect("begin transaction");
    let response = build_report(&tx, "r1".to_owned()).expect("build report");

    assert_eq!(
        response.parser_failure_files.len(),
        MAX_PARSER_FAILURE_FILES
    );
    assert_eq!(response.parser_failure_files_total, total as i64);
}

/// A file the walk discovers but deliberately does not index must be visible
/// rather than absent: a `skipped` row with its reason and no content, plus a
/// response that says the index is partial. Needs the real publish path, so
/// the oversize ceiling comes from the project's own config.
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
    fs::write(project.path().join("huge.rs"), vec![b'x'; 100_000]).expect("write oversize file");

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

/// The envelope counts kinds, skip reasons, extraction completeness, and the
/// two evidence gaps independently. Asserted over hand-built rows so each
/// bucket is unambiguous: a `ContentOnly` row (text indexed, nothing parsed)
/// must show up as `ContentOnly` and not as a parser failure, a skipped row
/// must be counted as skipped rather than as either, and a row whose evidence
/// was cut short by a cap must not read as a complete one.
#[test]
fn completeness_envelope_separates_kinds_reasons_and_completeness() {
    let (_dir, mut connection) = database_with_one_revision();
    insert_indexed_file(
        &connection,
        "notes.txt",
        "Unavailable",
        "NotAttempted",
        None,
        "ContentOnly",
    );
    insert_indexed_file(
        &connection,
        "capped.rs",
        "Available",
        "Succeeded",
        None,
        "Full",
    );
    let capped_id: i64 = connection
        .query_row("SELECT id FROM files WHERE path = 'capped.rs'", [], |row| {
            row.get(0)
        })
        .expect("capped.rs id");
    connection
        .execute(
            "INSERT INTO evidence \
             (file_id, key, kind, origin, span_availability, payload) \
             VALUES (?1, 'evidence:truncated', 'Diagnostic', 'SourceContent', \
                     'DerivedEvidence', '{}')",
            [capped_id],
        )
        .expect("insert the evidence-cap marker");
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
        .expect("insert skipped row");

    let tx = connection.transaction().expect("begin transaction");
    let response = build_report(&tx, "r1".to_owned()).expect("build report");

    let kind_count = |kind: &str| {
        response
            .completeness
            .file_kind_counts
            .iter()
            .find(|entry| entry.kind == kind)
            .map(|entry| entry.count)
    };
    assert_eq!(kind_count("indexed"), Some(2));
    assert_eq!(kind_count("skipped"), Some(1));
    assert_eq!(response.completeness.skipped_file_count, 1);
    assert!(response.completeness.index_partial);
    assert_eq!(response.completeness.skipped_by_reason.len(), 1);
    assert_eq!(
        response.completeness.skipped_by_reason[0].reason,
        "oversize"
    );
    assert_eq!(response.completeness.skipped_by_reason[0].count, 1);
    let completeness_for = |value: &str| {
        response
            .completeness
            .extraction_completeness_counts
            .iter()
            .find(|entry| entry.extraction_completeness == value)
            .map(|entry| entry.count)
    };
    assert_eq!(completeness_for("ContentOnly"), Some(1));
    assert_eq!(completeness_for("Full"), Some(1));
    assert_eq!(completeness_for("Unavailable"), Some(1));
    assert_eq!(
        response.completeness.indexed_files_without_evidence, 1,
        "notes.txt has content but no structural evidence; capped.rs has evidence and the \
         skipped row is not an indexed gap"
    );
    assert_eq!(
        response.completeness.evidence_capped_file_count, 1,
        "capped.rs carries the truncation marker"
    );
    assert_eq!(
        response.parser_failure_count, 1,
        "a skipped file is never parsed, so it is not a parser failure"
    );
}
