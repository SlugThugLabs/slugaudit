//! `leads`: mechanic denominators, detail clipping, credential files, compact rows and input caps.
use super::tests::{
    activate, compact_request, compact_rows, continuation, count_of, entry, full_rows,
    marker_fixture, request, run, run_on, rust_fixture, try_run,
};
use super::*;
use crate::tools::MAX_PATH_BYTES;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::fs;
use std::path::Path;

#[test]
fn mechanic_counts_carry_the_file_denominator_and_what_each_mechanic_covers() -> TestResult {
    let project = marker_fixture(3, 2)?; // 6 markers across 3 files
    let response = run(request(project.path(), None))?;

    let markers = entry(&response, "comment_markers")?;
    ensure_eq(&markers.count, &6, "markers.count == 6")?;
    ensure_eq(&markers.files, &3, "markers.files == 3")?;
    ensure_eq(&markers.covers, &["rust"], "markers.covers == [\"rust\"]")?;
    ensure_eq(
        &entry(&response, "expect_calls")?.covers,
        &["rust"],
        "entry(&response, \"expect_calls\")?.covers == [\"rust\"]",
    )?;
    ensure_eq(
        &entry(&response, "credential_files")?.covers,
        &["rust"],
        "entry(&response, \"credential_files\")?.covers == [\"rust\"]",
    )?;
    Ok(())
}

#[test]
fn clip_detail_flags_only_past_the_char_limit() -> TestResult {
    let row = |chars: usize| LeadRow {
        file: "f.rs".to_owned(),
        line: 1,
        detail: "é".repeat(chars),
        detail_clipped: false,
    };
    let mut at_limit = row(MAX_DETAIL_CHARS);
    let mut over_limit = row(MAX_DETAIL_CHARS + 1);

    clip_detail(&mut at_limit);
    clip_detail(&mut over_limit);

    ensure_eq(
        &at_limit.detail.chars().count(),
        &MAX_DETAIL_CHARS,
        "at_limit.detail.chars().count() == MAX_DETAIL_CHARS",
    )?;
    ensure(!at_limit.detail_clipped, "!at_limit.detail_clipped")?;
    ensure_eq(
        &over_limit.detail.len(),
        &(MAX_DETAIL_CHARS * 2),
        "over_limit.detail.len() == MAX_DETAIL_CHARS * 2",
    )?;
    ensure(over_limit.detail_clipped, "over_limit.detail_clipped")?;
    Ok(())
}

/// `credential_files` reads `files` directly, so a credential-shaped file the
/// index deliberately skipped is still something the agent must be told
/// about: no content was retained, but the path is exactly the signal this
/// mechanic exists to surface.
#[test]
fn credential_files_reports_a_skipped_credential_shaped_file() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    let _config = crate::tools::test_support::use_global_config(r#"{"max_file_bytes": 500}"#)?;
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    fs::write(project.path().join("big.pem"), vec![b'k'; 5_000]).check("write big.pem")?;

    let counts = run(request(project.path(), None))?;
    let skipped = count_of(&counts, "credential_files")?;
    ensure_eq(&skipped, &1, "skipped == 1")?;

    let rows = run(request(project.path(), Some("credential_files")))?;
    ensure_eq(&full_rows(&rows)?.len(), &1, "full_rows(&rows)?.len() == 1")?;
    ensure_eq(
        &full_rows(&rows)?
            .first()
            .check("full_rows(&rows)?[0]")?
            .file,
        &"big.pem",
        "full_rows(&rows)?[0].file == \"big.pem\"",
    )?;
    ensure_eq(&rows.files_total, &1, "rows.files_total == 1")?;
    Ok(())
}

/// The `mode` switch is optional: a request JSON carrying only `path` still
/// deserializes, to `full`.
#[test]
fn a_request_without_mode_deserializes_to_full() -> TestResult {
    let request: LeadsRequest = serde_json::from_str(r#"{"path":"/tmp/anywhere"}"#)
        .check("path-only request deserializes")?;
    ensure_eq(
        &request.mode,
        &LeadMode::Full,
        "request.mode == LeadMode::Full",
    )?;
    ensure(
        request.mechanic.is_none() && request.pattern.is_none(),
        "request.mechanic.is_none() && request.pattern.is_none()",
    )?;
    Ok(())
}

/// The compact mode's whole point: a `mechanic`/`file`/`line` triple with no
/// per-row text — the serialized row carries exactly those three keys, no
/// `detail` — and, with no `mechanic`, that is the whole surface in one
/// response, every row tagged. The default mode keeps its text.
#[test]
fn compact_rows_are_mechanic_file_line_triples_without_detail() -> TestResult {
    let project = rust_fixture()?;
    let response = run(compact_request(project.path(), None))?;
    let rows = compact_rows(&response)?;
    ensure(
        rows.iter().any(|r| r.mechanic == "expect_calls"),
        "rows.iter().any(|r| r.mechanic == \"expect_calls\")",
    )?;
    ensure(
        rows.iter().any(|r| r.mechanic == "comment_markers"),
        "rows.iter().any(|r| r.mechanic == \"comment_markers\")",
    )?;
    let returned = u64::try_from(response.page.returned).check("fits")?;
    ensure_eq(
        &Some(returned),
        &response.page.total,
        "Some(returned) == response.page.total",
    )?;
    let serialized = serde_json::to_value(rows).check("rows serialize")?;
    let row = serialized.get(0).check("serialized[0]")?;
    // Rows come in declaration order: the first mechanic with a row leads.
    ensure_eq(
        &row["mechanic"],
        &super::mechanics::MECHANICS
            .first()
            .check("MECHANICS[0]")?
            .id,
        "row[\"mechanic\"] == super::mechanics::MECHANICS[0].id",
    )?;
    ensure_eq(&row["file"], &"lib.rs", "row[\"file\"] == \"lib.rs\"")?;
    ensure(
        row["line"].as_i64().check("line")? > 0,
        "row[\"line\"].as_i64().check(\"line\")? > 0",
    )?;
    ensure_eq(
        &row.as_object().check("object")?.len(),
        &3,
        "row.as_object().check(\"object\")?.len() == 3",
    )?;

    let full = run(request(project.path(), Some("expect_calls")))?;
    let text = serde_json::to_value(&full.rows).check("rows serialize")?;
    ensure_eq(
        text.get(0)
            .check("text[0][\"detail\"]")?
            .get("detail")
            .check("text[0][\"detail\"]")?,
        &"x.expect",
        "text[0][\"detail\"] == \"x.expect\"",
    )?;
    ensure(
        text.get(0).check("text[0]")?.get("mechanic").is_none(),
        "text[0].get(\"mechanic\").is_none()",
    )?;

    let mut paged = compact_request(project.path(), None);
    paged.limit = Some(1);
    let page = run(paged)?;
    ensure_eq(
        &compact_rows(&page)?
            .first()
            .check("compact_rows(&page)?[0]")?
            .mechanic,
        &rows.first().check("rows[0]")?.mechanic,
        "compact_rows(&page)?[0].mechanic == rows[0].mechanic",
    )?;
    page.page
        .cursor
        .as_ref()
        .check("`page.page.cursor` is present")?;

    let mut scoped = compact_request(project.path(), None);
    scoped.pattern = Some("lib.rs".to_owned());
    let scoped = run(scoped)?;
    let files: Vec<&str> = compact_rows(&scoped)?
        .iter()
        .map(|r| r.file.as_str())
        .collect();
    ensure(
        !files.is_empty() && files.iter().all(|f| *f == "lib.rs"),
        "!files.is_empty() && files.iter().all(|f| *f == \"lib.rs\")",
    )?;
    Ok(())
}

/// Paging a compact sweep: the pages partition the rows and `files_covered`
/// stays cumulative rather than per-page.
#[test]
fn compact_paging_reports_a_cumulative_file_denominator() -> TestResult {
    let project = marker_fixture(2, 3)?; // 6 rows across 2 files
    let manager = crate::sync::SourceSyncManager::default();
    let mut first = compact_request(project.path(), None);
    first.limit = Some(4);
    let first = run_on(&manager, first)?;
    ensure_eq(
        &(first.page.returned, first.page.total),
        &(4, Some(6)),
        "(first.page.returned, first.page.total) == (4, Some(6))",
    )?;
    ensure_eq(
        &(first.files_covered, first.files_total),
        &(2, 2),
        "(first.files_covered, first.files_total) == (2, 2)",
    )?;
    ensure_eq(
        &(first.page.complete, first.page.cursor.is_some()),
        &(false, true),
        "(first.page.complete, first.page.cursor.is_some()) == (false, true)",
    )?;

    let rest = run_on(&manager, continuation(&first)?)?;
    ensure_eq(
        &(rest.page.returned, rest.page.complete),
        &(2, true),
        "(rest.page.returned, rest.page.complete) == (2, true)",
    )?;
    ensure_eq(
        &(rest.files_covered, rest.files_total),
        &(2, 2),
        "(rest.files_covered, rest.files_total) == (2, 2)",
    )?;
    Ok(())
}

/// The completeness fields mean the same thing in both modes: for one
/// mechanic they match field for field, the rows are the same places in the
/// same order, and only the bytes differ.
#[test]
fn both_modes_report_the_same_completeness_for_one_mechanic() -> TestResult {
    let project = marker_fixture(3, 4)?;
    let mut full_req = request(project.path(), Some("comment_markers"));
    full_req.limit = Some(5);
    let full = run(full_req)?;
    let mut compact_req = compact_request(project.path(), Some("comment_markers"));
    compact_req.limit = Some(5);
    let compact = run(compact_req)?;

    ensure_eq(
        &compact.page.returned,
        &full.page.returned,
        "compact.page.returned == full.page.returned",
    )?;
    ensure_eq(
        &compact.page.total,
        &full.page.total,
        "compact.page.total == full.page.total",
    )?;
    ensure_eq(
        &compact.page.complete,
        &full.page.complete,
        "compact.page.complete == full.page.complete",
    )?;
    ensure_eq(
        &compact.files_total,
        &full.files_total,
        "compact.files_total == full.files_total",
    )?;
    ensure_eq(
        &compact.files_covered,
        &full.files_covered,
        "compact.files_covered == full.files_covered",
    )?;
    ensure_eq(
        &compact.page.limited_by,
        &full.page.limited_by,
        "compact.page.limited_by == full.page.limited_by",
    )?;
    ensure_eq(
        &compact.page.cursor.is_some(),
        &full.page.cursor.is_some(),
        "compact.page.cursor.is_some() == full.page.cursor.is_some()",
    )?;
    let crows = compact_rows(&compact)?;
    let cplaces: Vec<(&str, i64)> = crows.iter().map(|r| (r.file.as_str(), r.line)).collect();
    let frows = full_rows(&full)?;
    let fplaces: Vec<(&str, i64)> = frows.iter().map(|r| (r.file.as_str(), r.line)).collect();
    ensure_eq(&cplaces, &fplaces, "cplaces == fplaces")?;
    let (cbytes, fbytes) = (compact.page.bytes_used, full.page.bytes_used);
    ensure(cbytes < fbytes, "cbytes < fbytes")?;
    Ok(())
}

/// Every caller-supplied string field is bounded before it reaches the
/// project: an over-cap value is refused with the message naming the field
/// and its cap, never truncated or passed through.
#[test]
fn leads_refuses_inputs_over_their_byte_caps() -> TestResult {
    let refused = |mutate: fn(&mut LeadsRequest)| -> TestResult<ErrorData> {
        let mut request = request(Path::new("/tmp"), None);
        mutate(&mut request);
        try_run(request).check_err("an over-cap input is refused")
    };
    let mut path = refused(|r| r.path = Some("a".repeat(MAX_PATH_BYTES + 1)))?;
    ensure_eq(
        &path.message,
        &"path exceeds 32768 bytes",
        "path.message == \"path exceeds 32768 bytes\"",
    )?;
    path = refused(|r| r.mechanic = Some("m".repeat(MAX_FILTER_BYTES + 1)))?;
    ensure_eq(
        &path.message,
        &"mechanic exceeds 8192 bytes",
        "path.message == \"mechanic exceeds 8192 bytes\"",
    )?;
    path = refused(|r| r.pattern = Some("p".repeat(MAX_FILTER_BYTES + 1)))?;
    ensure_eq(
        &path.message,
        &"pattern exceeds 8192 bytes",
        "path.message == \"pattern exceeds 8192 bytes\"",
    )?;
    path = refused(|r| r.cursor = Some("c".repeat(MAX_CURSOR_BYTES + 1)))?;
    ensure_eq(
        &path.message,
        &"cursor exceeds 1024 bytes",
        "path.message == \"cursor exceeds 1024 bytes\"",
    )?;
    ensure_eq(
        &path.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "path.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    Ok(())
}
