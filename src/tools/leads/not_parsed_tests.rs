//! A file whose grammar did not run is visible in every `leads` response
//! (`files_not_parsed`) and in `report.completeness`, so a mechanic's count
//! is never quoted as complete while a file was left unparsed.

use super::*;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::fs;
use std::time::{Duration, Instant};

/// A 3 MiB Rust file (over the 2 MiB `max_parse_bytes` default) full of
/// `expect()` calls, beside one small parsed file with exactly one.
fn oversize_fixture() -> TestResult<tempfile::TempDir> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join(".planning").join("slugaudit")).check("activate")?;
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
    let line = "pub fn f(x: Option<u8>) -> u8 { x.expect() }\n";
    let big = line.repeat(
        (3 * 1024 * 1024usize)
            .checked_div(line.len())
            .check("line is not empty")?
            .checked_add(1)
            .check("repeat count does not overflow")?,
    );
    ensure(big.len() > 3 * 1024 * 1024, "big.len() > 3 * 1024 * 1024")?;
    fs::write(project.path().join("big.rs"), big).check("write oversize file")?;
    fs::write(project.path().join("small.rs"), line).check("write small file")?;
    Ok(project)
}

fn leads_request(project: &tempfile::TempDir, mechanic: Option<&str>) -> LeadsRequest {
    LeadsRequest {
        path: Some(project.path().to_string_lossy().into_owned()),
        mechanic: mechanic.map(str::to_owned),
        pattern: None,
        limit: None,
        cursor: None,
        mode: LeadMode::Full,
        page_bytes: None,
        legacy_paging: crate::tools::LegacyPaging::default(),
    }
}

fn run_leads(
    request: LeadsRequest,
    manager: &crate::sync::SourceSyncManager,
) -> TestResult<LeadsResponse> {
    Ok(leads(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .check("leads succeeds")?
    .0)
}

#[test]
fn an_oversize_file_indexes_fast_and_every_leads_response_counts_it_not_parsed() -> TestResult {
    let project = oversize_fixture()?;
    let manager = crate::sync::SourceSyncManager::default();

    let started = Instant::now();
    let counts = run_leads(leads_request(&project, None), &manager)?;
    let elapsed = started.elapsed();
    let rows = run_leads(leads_request(&project, Some("expect_calls")), &manager)?;
    let compact = run_leads(
        LeadsRequest {
            mode: LeadMode::Compact,
            ..leads_request(&project, None)
        },
        &manager,
    )?;

    ensure(
        elapsed < Duration::from_secs(2),
        "elapsed < Duration::from_secs(2)",
    )?;
    ensure_eq(&counts.files_not_parsed, &1, "counts.files_not_parsed == 1")?;
    ensure_eq(&rows.files_not_parsed, &1, "rows.files_not_parsed == 1")?;
    ensure_eq(
        &compact.files_not_parsed,
        &1,
        "compact.files_not_parsed == 1",
    )?;
    ensure_eq(&rows.page.total, &Some(1), "rows.page.total == Some(1)")?;
    let LeadRows::Full(rows) = &rows.rows else {
        return Err("full mode returns full rows".to_owned()).check("unexpected outcome");
    };
    ensure_eq(
        &rows.first().check("rows[0]")?.file,
        &"small.rs",
        "rows[0].file == \"small.rs\"",
    )?;
    Ok(())
}

#[test]
fn files_not_parsed_follows_the_request_pattern() -> TestResult {
    let project = oversize_fixture()?;
    let manager = crate::sync::SourceSyncManager::default();

    let narrowed = run_leads(
        LeadsRequest {
            pattern: Some("small".to_owned()),
            ..leads_request(&project, None)
        },
        &manager,
    )?;

    ensure_eq(
        &narrowed.files_not_parsed,
        &0,
        "narrowed.files_not_parsed == 0",
    )?;
    Ok(())
}

#[test]
fn report_completeness_counts_an_oversize_for_parse_file_and_lists_its_reason() -> TestResult {
    let project = oversize_fixture()?;

    let response = crate::tools::report(
        &Parameters(crate::tools::ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("report succeeds")?
    .0;

    ensure_eq(
        &response.completeness.oversize_for_parse_file_count,
        &1,
        "response.completeness.oversize_for_parse_file_count == 1",
    )?;
    ensure_eq(
        &response.completeness.parse_timeout_file_count,
        &0,
        "response.completeness.parse_timeout_file_count == 0",
    )?;
    let listed: Vec<(&str, Option<&str>)> = response
        .parser_failure_files
        .iter()
        .map(|file| (file.path.as_str(), file.parse_error_reason.as_deref()))
        .collect();
    ensure_eq(
        &listed,
        &[("big.rs", Some("oversize_for_parse"))],
        "listed == [(\"big.rs\", Some(\"oversize_for_parse\"))]",
    )?;
    Ok(())
}

#[test]
fn a_file_with_no_detected_language_is_not_counted_as_not_parsed() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join(".planning").join("slugaudit")).check("activate")?;
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
    fs::write(project.path().join(".gitignore"), "target/\n").check("write .gitignore")?;
    fs::write(project.path().join("small.rs"), "pub fn f() {}\n").check("write small file")?;
    let manager = crate::sync::SourceSyncManager::default();

    let response = run_leads(leads_request(&project, None), &manager)?;

    ensure_eq(
        &response.files_not_parsed,
        &0,
        "response.files_not_parsed == 0",
    )?;
    Ok(())
}

/// Two small files and one just under `max_parse_bytes` that no 200 ms budget
/// can parse, under a global config that sets `max_parse_wall_clock_ms`.
fn slow_parse_fixture() -> TestResult<(
    tempfile::TempDir,
    crate::tools::test_support::GlobalConfigGuard,
)> {
    let project = tempfile::tempdir().check("project dir")?;
    let config_dir = project.path().join(".planning").join("slugaudit");
    fs::create_dir_all(&config_dir).check("activate")?;
    crate::project::write_database_marker(&config_dir)?;
    let config =
        crate::tools::test_support::use_global_config(r#"{"max_parse_wall_clock_ms": 200}"#)?;
    let line = "pub fn f(x: Option<u8>) -> u8 { x.expect() }\n";
    let big = line.repeat(
        1_900_000usize
            .checked_div(line.len())
            .check("line is not empty")?,
    );
    ensure(big.len() < 2 * 1024 * 1024, "big.len() < 2 * 1024 * 1024")?;
    fs::write(project.path().join("slow.rs"), big).check("write slow file")?;
    fs::write(project.path().join("one.rs"), line).check("write small file")?;
    fs::write(project.path().join("two.rs"), line).check("write small file")?;
    Ok((project, config))
}

fn run_report(
    project: &tempfile::TempDir,
    manager: &crate::sync::SourceSyncManager,
) -> TestResult<crate::tools::ReportResponse> {
    Ok(crate::tools::report(
        &Parameters(crate::tools::ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .check("report succeeds")?
    .0)
}

#[test]
fn a_file_that_outlives_its_parse_allowance_is_counted_and_the_sync_goes_on() -> TestResult {
    let (project, _config) = slow_parse_fixture()?;
    let manager = crate::sync::SourceSyncManager::default();

    let counts = run_leads(leads_request(&project, None), &manager)?;
    let rows = run_leads(leads_request(&project, Some("expect_calls")), &manager)?;
    let first_report = run_report(&project, &manager)?;
    let second_report = run_report(&project, &manager)?;

    ensure_eq(&counts.files_not_parsed, &1, "counts.files_not_parsed == 1")?;
    ensure_eq(&rows.files_not_parsed, &1, "rows.files_not_parsed == 1")?;
    ensure_eq(
        &second_report.revision_id,
        &counts.page.revision_id,
        "second_report.revision_id == counts.page.revision_id",
    )?;
    ensure_eq(&rows.page.total, &Some(2), "rows.page.total == Some(2)")?;
    for report in [&first_report, &second_report] {
        ensure_eq(
            &report.completeness.parse_timeout_file_count,
            &1,
            "report.completeness.parse_timeout_file_count == 1",
        )?;
        ensure_eq(
            &report.completeness.oversize_for_parse_file_count,
            &0,
            "report.completeness.oversize_for_parse_file_count == 0",
        )?;
        let listed: Vec<(&str, Option<&str>)> = report
            .parser_failure_files
            .iter()
            .map(|file| (file.path.as_str(), file.parse_error_reason.as_deref()))
            .collect();
        ensure_eq(
            &listed,
            &[("slow.rs", Some("parse_timeout"))],
            "listed == [(\"slow.rs\", Some(\"parse_timeout\"))]",
        )?;
    }
    Ok(())
}

/// `PARSED_FILES` (what a mechanic reads) and `PARSER_FAILURE_PREDICATE`
/// (what `files_not_parsed` counts) are written apart. Over every combination
/// a parse can leave (an outcome other than `NotAttempted` needs an available
/// parser), no file is in both, and every indexed, non-prose file whose parse
/// ran is in exactly one.
#[test]
fn the_parsed_and_parser_failure_predicates_never_overlap_and_cover_every_parse() -> TestResult {
    let connection = rusqlite::Connection::open_in_memory().check("in-memory database")?;
    connection
        .execute_batch(
            "CREATE TABLE files (file_kind TEXT, language TEXT, parser_availability TEXT, \
             parse_outcome TEXT)",
        )
        .check("create files")?;
    for kind in ["indexed", "skipped"] {
        for language in [None, Some("rust"), Some("prose")] {
            for availability in ["Available", "Unavailable", "LoadFailed"] {
                for outcome in ["NotAttempted", "Succeeded", "SyntaxErrors", "Failed"] {
                    if outcome != "NotAttempted" && availability != "Available" {
                        continue;
                    }
                    connection
                        .execute(
                            "INSERT INTO files VALUES (?1, ?2, ?3, ?4)",
                            rusqlite::params![kind, language, availability, outcome],
                        )
                        .check("insert a files row")?;
                }
            }
        }
    }
    let count = |predicate: &str| -> TestResult<i64> {
        connection
            .query_row(
                &format!("SELECT count(*) FROM files WHERE {predicate}"),
                [],
                |row| row.get(0),
            )
            .check("count rows")
    };
    let parsed = crate::tools::sql_predicates::PARSED_FILES;
    let failed = crate::tools::PARSER_FAILURE_PREDICATE;

    ensure_eq(
        &count(&format!("({parsed}) AND ({failed})"))?,
        &0,
        "count(&format!(\"({parsed}) AND ({failed})\"))? == 0",
    )?;
    ensure_eq(
        &count(&format!(
            "file_kind = 'indexed' AND language IS NOT 'prose' AND language IS NOT NULL \
             AND parse_outcome != 'NotAttempted' AND NOT ({parsed}) AND NOT ({failed})"
        ))?,
        &0,
        "count(&format!( \"file_kind = 'indexed' AND language IS NOT 'prose' AND language IS NOT NULL \\ AND parse_outcome != 'NotAttempted' AND NOT ({parsed}) AND NOT ({failed})\" ))? == 0",
    )?;
    Ok(())
}

#[test]
fn report_and_leads_agree_on_unparsed_file_counts_including_unknown_extensions() -> TestResult {
    let project = oversize_fixture()?;
    fs::write(project.path().join("a.md"), "# docs\n").check("write md")?;
    fs::write(project.path().join(".gitignore"), "target/\n").check("write .gitignore")?;
    fs::write(project.path().join("c.zzunknown"), "mystery bytes\n").check("write unknown ext")?;
    let manager = crate::sync::SourceSyncManager::default();

    let report = run_report(&project, &manager)?;
    let counts = run_leads(leads_request(&project, None), &manager)?;

    ensure_eq(
        &report.parser_failure_count,
        &2,
        "report counts big.rs and c.zzunknown while excluding prose",
    )?;
    ensure_eq(
        &counts.files_not_parsed,
        &report.parser_failure_count,
        "leads.files_not_parsed must match report.parser_failure_count",
    )?;
    Ok(())
}

#[test]
fn custom_mechanic_stopped_at_zero_files_reports_empty_covers() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    let config_dir = project.path().join(".planning").join("slugaudit");
    fs::create_dir_all(&config_dir).check("activate")?;
    crate::project::write_database_marker(&config_dir)?;
    fs::write(project.path().join("app.py"), "eval(\"1\")\n").check("write app.py")?;
    let path = project.path().to_string_lossy().into_owned();
    let manager = crate::sync::SourceSyncManager::default();
    crate::tools::mechanic(
        &Parameters(crate::tools::MechanicRequest {
            path: Some(path.clone()),
            action: Some(crate::tools::MechanicAction::Add),
            cursor: None,
            legacy_paging: crate::tools::LegacyPaging::default(),
            id: Some("py_eval".to_owned()),
            language: Some("python".to_owned()),
            query: Some(
                r#"(call function: (identifier) @callee (#eq? @callee "eval"))"#.to_owned(),
            ),
            note: Some("eval calls".to_owned()),
            limit: None,
            page_bytes: None,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("add custom mechanic")?;

    let root = crate::tools::project_root(&path).check("project root")?;
    let mut limits = crate::tools::limits_for_root(&root).check("limits")?;
    limits.max_structure_execution_time = Duration::ZERO;
    let catalog =
        super::super::mechanic::catalog::load(manager.session_mechanics(), root.as_path())
            .check("load catalog")?;
    let response = super::leads_with_catalog(
        &leads_request(&project, Some("py_eval")),
        &limits,
        &catalog,
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("zero-budget leads returns partial response")?;

    ensure(
        matches!(
            response.page.completeness,
            Some(crate::tools::Completeness::Partial {
                files_scanned: 0,
                ..
            })
        ),
        format!(
            "expected 0 files scanned partial completeness: {:?}",
            response.page.completeness
        ),
    )?;
    let custom = response
        .mechanic_counts
        .iter()
        .find(|entry| entry.mechanic == "py_eval")
        .check("py_eval count entry present")?;
    ensure(
        custom.covers.is_empty(),
        format!(
            "expected empty covers when 0 files scanned, got {:?}",
            custom.covers
        ),
    )?;
    Ok(())
}
