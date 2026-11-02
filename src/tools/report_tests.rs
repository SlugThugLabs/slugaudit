use super::*;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::fs;

/// The response for an index alone, with no session findings, no rebuild
/// notice and no mechanics.
pub(super) fn build_report(
    connection: &rusqlite::Transaction<'_>,
    revision_id: String,
) -> Result<ReportResponse, rusqlite::Error> {
    let sections = crate::tools::report_sections::read_sections(connection, &revision_id)?;
    Ok(sections.into_response(revision_id, 0, None, ReportMechanics::default()))
}

fn activate(root: &std::path::Path) -> TestResult {
    crate::project::activate_for_test(root)
}

#[test]
fn reports_credential_pattern_file_count() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    fs::write(project.path().join(".env"), b"API_KEY=secret\n").check("write .env")?;
    fs::create_dir_all(project.path().join("keys")).check("keys dir")?;
    fs::write(
        project.path().join("keys").join("cred.pem"),
        b"-----BEGIN PRIVATE KEY-----\n",
    )
    .check("write pem")?;
    // Ordinary source must never match the credential patterns.
    fs::write(project.path().join("pem_util.rs"), b"pub fn parse() {}\n")
        .check("write ordinary source")?;

    let response = report(
        &Parameters(ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("report succeeds")?;

    assert_eq!(response.0.file_count, 4);
    assert_eq!(
        response.0.credential_pattern_file_count, 2,
        ".env and keys/cred.pem match credential patterns; pem_util.rs and lib.rs must not"
    );
    Ok(())
}

#[test]
fn reports_real_counts_for_an_active_project() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() {}\npub fn b() {}\n",
    )
    .check("write fixture")?;

    let response = report(
        &Parameters(ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("report succeeds")?;

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
    Ok(())
}

/// `unsupported_language_unresolved_count` must count only the `Unresolved`
/// edges coming from import syntax the resolver can't parse at all (here:
/// `go`'s `import "fmt"`), not `Unresolved` edges from a modeled language
/// with a genuinely broken import (here: `python`). The verdict
/// is cached on the edge at write time (`syntax_unmodeled`), so this test
/// pins the *SQL count* over that cached column — the write-time
/// computation itself is covered by the `publish_edges` pipeline tests.
#[test]
fn unsupported_language_unresolved_is_counted_separately_from_supported_language_unresolved()
-> TestResult {
    use crate::store::open_read_write;

    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;

    connection
        .execute(
            "INSERT INTO revisions \
             (revision_id, manifest_hash, parser_pack_version, created_at_unix, is_current) \
             VALUES ('r1', 'h', '1.0.0', 0, 1)",
            [],
        )
        .check("insert revision")?;

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
            .check("insert file")?;
    }
    let go_id: i64 = connection
        .query_row("SELECT id FROM files WHERE path = 'a.go'", [], |row| {
            row.get(0)
        })
        .check("go file id")?;
    let py_id: i64 = connection
        .query_row("SELECT id FROM files WHERE path = 'b.py'", [], |row| {
            row.get(0)
        })
        .check("python file id")?;
    let kotlin_id: i64 = connection
        .query_row("SELECT id FROM files WHERE path = 'c.kt'", [], |row| {
            row.get(0)
        })
        .check("kotlin file id")?;

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
        .check("insert go edge")?;
    connection
        .execute(
            "INSERT INTO dependency_edges (from_file_id, raw_import_text, resolution_kind, syntax_unmodeled) \
             VALUES (?1, 'import missing_module', 'Unresolved', 0)",
            [py_id],
        )
        .check("insert python edge")?;
    connection
        .execute(
            "INSERT INTO dependency_edges (from_file_id, raw_import_text, resolution_kind, syntax_unmodeled) \
             VALUES (?1, 'import kotlin.math.max', 'Unresolved', 0)",
            [kotlin_id],
        )
        .check("insert kotlin edge")?;

    let tx = connection.transaction().check("begin transaction")?;
    let response = build_report(&tx, "r1".to_owned()).check("build report")?;

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
    Ok(())
}

/// Inbound and outbound edge totals are arithmetic over
/// `dependency_edges`: `report` carries them so the zero-decision entry
/// point does not need SQL for the totals, and an edge that lands on no
/// project file still counts outbound.
#[test]
fn dependency_edge_totals_count_edges_and_files_in_both_directions() -> TestResult {
    let (_dir, mut connection) = database_with_one_revision()?;
    for path in ["src/lib.rs", "src/main.rs"] {
        insert_indexed_file(&connection, path, "Available", "Succeeded", None, "Full")?;
    }
    connection
        .execute_batch(
            "INSERT INTO dependency_edges (from_file_id, to_file_id, raw_import_text, resolution_kind) \
             SELECT (SELECT id FROM files WHERE path = 'src/lib.rs'), \
                    (SELECT id FROM files WHERE path = 'src/main.rs'), 'import x', 'Resolved'; \
             INSERT INTO dependency_edges (from_file_id, to_file_id, raw_import_text, resolution_kind) \
             SELECT (SELECT id FROM files WHERE path = 'src/lib.rs'), NULL, 'import ext', 'External';",
        )
        .check("insert one resolved and one targetless edge")?;

    let tx = connection.transaction().check("begin transaction")?;
    let response = build_report(&tx, "r1".to_owned()).check("build report")?;

    let totals = response.dependency_edges;
    assert_eq!(totals.outbound_edge_count, 2, "both edges leave lib.rs");
    assert_eq!(totals.inbound_edge_count, 1, "only the resolved edge lands");
    assert_eq!(totals.outbound_file_count, 1, "one file imports anything");
    assert_eq!(totals.inbound_file_count, 1, "one file is imported");
    Ok(())
}

/// The envelope carries where files came from as its own breakdown, so an
/// agent looking at a mechanic dominated by third-party or machine-written
/// rows can read the class counts off the report instead of re-deriving the
/// classification in SQL.
#[test]
fn the_completeness_envelope_groups_files_by_class() -> TestResult {
    let (_dir, mut connection) = database_with_one_revision()?;
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
            .check("insert file")?;
    }

    let tx = connection.transaction().check("begin transaction")?;
    let response = build_report(&tx, "r1".to_owned()).check("build report")?;

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
    Ok(())
}

#[test]
fn malformed_language_value_is_reported_instead_of_dropped() -> TestResult {
    use crate::store::open_read_write;

    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    connection
        .execute_batch(
            "INSERT INTO revisions (revision_id, manifest_hash, parser_pack_version, created_at_unix, is_current) VALUES ('r1', 'h', '1.0.0', 0, 1);\
             INSERT INTO files (path, file_kind, content, content_hash, hash_algorithm, byte_len, language, language_detected, parser_availability, parse_outcome, extraction_completeness, last_revision_id) \
             VALUES ('broken.bin', 'indexed', '', 'h', 'blake3', 0, CAST(X'FF' AS BLOB), 1, 'Unavailable', 'Failed', 'Unavailable', 1);"
        )
        .check("insert revision and file")?;

    let tx = connection.transaction().check("begin transaction")?;
    let error = build_report(&tx, "r1".to_owned()).check_err("malformed language")?;
    let rusqlite::Error::InvalidColumnType(_, column, _) = &error else {
        return Err(format!(
            "the malformed language column is named, not dropped: {error:?}"
        ))
        .check("unexpected outcome");
    };
    assert_eq!(column, "language");
    Ok(())
}

#[test]
fn report_on_an_inactive_project_is_a_typed_error_not_a_panic() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    let req = ReportRequest {
        path: project.path().to_string_lossy().into_owned(),
    };
    let err = report(
        &Parameters(req),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .check("report on inactive project must return ErrorData")?;
    assert!(
        err.message.contains("not enabled")
            || err.message.contains("inactive")
            || err.message.contains("activate")
    );
    Ok(())
}

#[test]
fn report_rejects_a_path_over_the_byte_cap() -> TestResult {
    let request = Parameters(ReportRequest {
        path: "a".repeat(MAX_PATH_BYTES + 1),
    });

    let error = report(
        &request,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .check("an over-cap path is refused")?;

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{error:?}"
    );
    assert_eq!(error.message, "path exceeds 32768 bytes");
    Ok(())
}

/// A section query that fails (here: a `files.language` value that is not
/// text) fails the whole report with an internal error; the report is never
/// returned with a section silently missing.
#[test]
fn report_maps_a_build_failure_to_an_internal_error() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let path = project.path().to_string_lossy().into_owned();
    let manager = crate::sync::SourceSyncManager::default();
    manager
        .ensure_current(&path, &crate::progress::NoopProgressSink)
        .check("initial sync")?;
    let database = project.path().join(".planning/slugaudit/project.db");
    let connection = crate::store::open_read_write(&database).check("open the database")?;
    connection
        .execute("UPDATE files SET language = CAST(X'FF' AS BLOB)", [])
        .check("malform the language column")?;
    drop(connection);

    let error = report(
        &Parameters(ReportRequest { path }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .err()
    .check("a failed section query fails the report")?;

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INTERNAL_ERROR,
        "{error:?}"
    );
    assert!(
        error.message.starts_with("building the report:")
            && error.message.contains("language")
            && !error.message.contains("retry the call")
            && error.data.is_none(),
        "the operation and cause must survive, with no retry hint and no caller-input code: {}",
        error.message
    );
    Ok(())
}

/// A discovered file whose name is not valid UTF-8 cannot have a `files`
/// row. Sync persists the set during its discovery walk, so the report
/// counts it from the index without walking, and adding or removing such a
/// file updates the count on the next sync — including when the file set is
/// otherwise unchanged and no new revision is published.
#[cfg(unix)]
#[test]
fn non_utf8_paths_are_counted_from_sync_state_as_the_tree_changes() -> TestResult {
    use std::os::unix::ffi::OsStringExt;

    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let report_state = || -> TestResult<_> {
        let response = report(
            &Parameters(ReportRequest {
                path: project.path().to_string_lossy().into_owned(),
            }),
            &crate::progress::NoopProgressSink,
            &crate::sync::SourceSyncManager::default(),
        )
        .check("report succeeds")?;
        Ok((
            response.0.completeness.unrepresentable_path_count,
            response.0.completeness.index_partial,
        ))
    };

    assert_eq!(report_state()?, (0, false), "nothing is unrepresentable");
    let bad = project
        .path()
        .join(std::ffi::OsString::from_vec(b"bad\xff.rs".to_vec()));
    fs::write(&bad, b"fn b() {}\n").check("write non-utf8 file")?;
    assert_eq!(report_state()?, (1, true), "the dropped path is counted");
    fs::remove_file(&bad).check("remove non-utf8 file")?;
    assert_eq!(report_state()?, (0, false), "removal updates the count");
    Ok(())
}

/// A project DB with one revision and no files, ready for direct `files`
/// inserts — the report tests below assert SQL over hand-built rows rather
/// than running the full sync pipeline.
pub(super) fn database_with_one_revision() -> TestResult<(tempfile::TempDir, rusqlite::Connection)>
{
    let db_dir = tempfile::tempdir().check("db dir")?;
    let connection =
        crate::store::open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    connection
        .execute(
            "INSERT INTO revisions \
             (revision_id, manifest_hash, parser_pack_version, created_at_unix, is_current) \
             VALUES ('r1', 'h', '1.0.0', 0, 1)",
            [],
        )
        .check("insert revision")?;
    Ok((db_dir, connection))
}

/// Inserts one indexed text file with the given parser columns, so a test
/// can reproduce a specific failure shape without a real parser.
pub(super) fn insert_indexed_file(
    connection: &rusqlite::Connection,
    path: &str,
    availability: &str,
    outcome: &str,
    error_reason: Option<&str>,
    completeness: &str,
) -> TestResult {
    connection
        .execute(
            "INSERT INTO files \
             (path, file_kind, content, content_hash, hash_algorithm, byte_len, language, \
              language_detected, parser_availability, parse_outcome, parse_error_reason, \
              extraction_completeness, last_revision_id) \
             VALUES (?1, 'indexed', '', 'h', 'blake3', 0, NULL, 0, ?2, ?3, ?4, ?5, 1)",
            rusqlite::params![path, availability, outcome, error_reason, completeness],
        )
        .check("insert file")?;
    Ok(())
}
