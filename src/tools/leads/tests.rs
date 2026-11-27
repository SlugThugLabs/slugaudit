use super::*;
use crate::tools::PageLimit;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::fs;
use std::path::Path;

pub(super) fn activate(root: &Path) -> TestResult {
    fs::create_dir_all(root.join(".planning").join("slugaudit")).check("create activation dir")?;
    crate::project::write_database_marker(&root.join(".planning").join("slugaudit"))?;
    Ok(())
}

/// A request with everything at its default: no mechanic, no filter, no
/// limit, no cursor, full rows. Tests mutate the fields they are about.
pub(super) fn request(path: &Path, mechanic: Option<&str>) -> LeadsRequest {
    LeadsRequest {
        path: Some(path.to_string_lossy().into_owned()),
        mechanic: mechanic.map(str::to_owned),
        pattern: None,
        limit: None,
        cursor: None,
        mode: LeadMode::Full,
        page_bytes: None,
        legacy_paging: LegacyPaging::default(),
    }
}

pub(super) fn try_run(request: LeadsRequest) -> Result<LeadsResponse, ErrorData> {
    try_run_on(&crate::sync::SourceSyncManager::default(), request)
}

/// One call on a manager the test keeps, so a cursor from one call can be
/// continued by the next (the result store belongs to the manager).
fn try_run_on(
    manager: &crate::sync::SourceSyncManager,
    request: LeadsRequest,
) -> Result<LeadsResponse, ErrorData> {
    leads_with_limits(
        &Parameters(request),
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .map(|Json(response)| response)
}

pub(super) fn run_on(
    manager: &crate::sync::SourceSyncManager,
    request: LeadsRequest,
) -> TestResult<LeadsResponse> {
    try_run_on(manager, request).check("leads succeeds")
}

/// The request that continues a page: the cursor and the page size that
/// applied, as a caller sends them (a continuation honours `page_bytes`).
pub(super) fn continuation(page: &LeadsResponse) -> TestResult<LeadsRequest> {
    Ok(LeadsRequest {
        path: None,
        page_bytes: Some(page.page.page_bytes),
        cursor: Some(page.page.cursor.clone().check("a cursor to continue")?),
        ..request(Path::new("/unused"), None)
    })
}

pub(super) fn run(request: LeadsRequest) -> TestResult<LeadsResponse> {
    try_run(request).check("leads succeeds")
}

pub(super) fn run_with_limits(
    request: LeadsRequest,
    limits: &ResourceLimits,
) -> TestResult<LeadsResponse> {
    Ok(leads_with_limits(
        &Parameters(request),
        limits,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("leads succeeds")?
    .0)
}

pub(super) fn entry<'a>(
    response: &'a LeadsResponse,
    mechanic: &str,
) -> TestResult<&'a MechanicCount> {
    response
        .mechanic_counts
        .iter()
        .find(|entry| entry.mechanic == mechanic)
        .check(&format!("mechanic {mechanic} missing from counts"))
}

pub(super) fn count_of(response: &LeadsResponse, mechanic: &str) -> TestResult<i64> {
    Ok(entry(response, mechanic)?.count)
}

/// The full-mode rows of a response; a compact page is an error, which is
/// what a test asserting full rows should want.
pub(super) fn full_rows(response: &LeadsResponse) -> TestResult<&[LeadRow]> {
    match &response.rows {
        LeadRows::Full(rows) => Ok(rows),
        LeadRows::Compact(_) => Err("expected full rows".to_owned()).check("the rows are full"),
    }
}

/// The compact-mode rows of a response; the mirror of [`full_rows`].
pub(super) fn compact_rows(response: &LeadsResponse) -> TestResult<&[CompactLeadRow]> {
    match &response.rows {
        LeadRows::Compact(rows) => Ok(rows),
        LeadRows::Full(_) => Err("expected compact rows".to_owned()).check("the rows are compact"),
    }
}

/// The request a compact call makes: the same fields as [`request`] with
/// `mode` set, so a test can compare the two modes over one project.
pub(super) fn compact_request(path: &Path, mechanic: Option<&str>) -> LeadsRequest {
    LeadsRequest {
        mode: LeadMode::Compact,
        ..request(path, mechanic)
    }
}

/// A Rust project with one `unwrap`, one `expect`, one TODO comment, and one
/// credential-shaped file.
pub(super) fn rust_fixture() -> TestResult<tempfile::TempDir> {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    fs::write(
        project.path().join("lib.rs"),
        b"// TODO: revisit\npub fn a(x: Option<u8>) -> u8 {\n    let _ = x.unwrap();\n    x.expect(\"nope\")\n}\n",
    )
    .check("write rust source")?;
    fs::write(project.path().join(".env"), b"API_KEY=secret\n").check("write .env")?;
    Ok(project)
}

/// A Rust project with `per_file` TODO comments in each of `files` files, each
/// comment on the odd line above a function (lines 1, 3, 5, ...) — enough
/// `comment_markers` rows to page, with a distinct-file denominator above one.
pub(super) fn marker_fixture(files: usize, per_file: usize) -> TestResult<tempfile::TempDir> {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    for file in 0..files {
        let body = (0..per_file)
            .map(|n| format!("// TODO: {n}\npub fn f{n}() {{}}\n"))
            .collect::<Vec<_>>()
            .concat();
        fs::write(project.path().join(format!("mod{file}.rs")), body).check("write rust source")?;
    }
    Ok(project)
}

/// Two directories, so a `pattern` has something to exclude.
fn two_dir_fixture() -> TestResult<tempfile::TempDir> {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    for (relative, body) in [
        ("src/auth/login.rs", "// TODO: a\npub fn a() {}\n"),
        ("src/auth/token.rs", "// TODO: b\npub fn b() {}\n"),
        (
            "src/db/pool.rs",
            "// TODO: c\npub fn c() {}\n// TODO: d\npub fn d() {}\n",
        ),
    ] {
        let path = project.path().join(relative);
        fs::create_dir_all(path.parent().check("parent")?).check("create parent dirs")?;
        fs::write(path, body).check("write rust source")?;
    }
    Ok(project)
}

#[test]
fn counts_every_mechanic_without_returning_rows() -> TestResult {
    let project = rust_fixture()?;
    let response = run(request(project.path(), None))?;

    let ids: Vec<&str> = response
        .mechanic_counts
        .iter()
        .map(|e| e.mechanic.as_str())
        .collect();
    let expected = [
        "unwrap_calls",
        "expect_calls",
        "panic_calls",
        "todo_calls",
        "assert_calls",
        "diagnostics",
        "comment_markers",
        "credential_files",
    ];
    ensure_eq(&ids, &expected, "ids == expected")?;
    ensure_eq(
        &count_of(&response, "unwrap_calls")?,
        &1,
        "count_of(&response, \"unwrap_calls\")? == 1",
    )?;
    ensure_eq(
        &entry(&response, "unwrap_calls")?.files,
        &1,
        "entry(&response, \"unwrap_calls\")?.files == 1",
    )?;
    ensure_eq(
        &entry(&response, "unwrap_calls")?.covers,
        &["rust"],
        "entry(&response, \"unwrap_calls\")?.covers == [\"rust\"]",
    )?;
    ensure_eq(
        &count_of(&response, "expect_calls")?,
        &1,
        "count_of(&response, \"expect_calls\")? == 1",
    )?;
    ensure(
        count_of(&response, "comment_markers")? >= 1,
        "count_of(&response, \"comment_markers\")? >= 1",
    )?;
    ensure_eq(
        &count_of(&response, "credential_files")?,
        &1,
        "count_of(&response, \"credential_files\")? == 1",
    )?;
    ensure(
        full_rows(&response)?.is_empty(),
        "full_rows(&response)?.is_empty()",
    )?;
    ensure(response.page.complete, "response.page.complete")?;
    ensure_eq(&response.page.cursor, &None, "response.page.cursor == None")?;
    ensure_eq(
        &(response.page.total, response.files_total),
        &(Some(0), 0),
        "(response.page.total, response.files_total) == (Some(0), 0)",
    )?;

    // The row the `unwrap_calls` count stands for: file, 1-based line, callee.
    let rows = run(request(project.path(), Some("unwrap_calls")))?;
    let rows = full_rows(&rows)?;
    ensure_eq(&rows.len(), &1, "rows.len() == 1")?;
    ensure_eq(
        &(
            rows.first().check("rows[0]")?.file.as_str(),
            rows.first().check("rows[0]")?.line,
            rows.first().check("rows[0]")?.detail.as_str(),
        ),
        &("lib.rs", 3, "x.unwrap"),
        "(rows[0].file.as_str(), rows[0].line, rows[0].detail.as_str()) == (\"lib.rs\", 3, \"x.unwrap\")",
    )?;
    Ok(())
}

#[test]
fn returns_rows_for_the_requested_mechanic_only() -> TestResult {
    let project = rust_fixture()?;
    let response = run(request(project.path(), Some("expect_calls")))?;

    ensure_eq(
        &full_rows(&response)?.len(),
        &1,
        "full_rows(&response)?.len() == 1",
    )?;
    ensure_eq(
        &full_rows(&response)?
            .first()
            .check("full_rows(&response)?[0]")?
            .file,
        &"lib.rs",
        "full_rows(&response)?[0].file == \"lib.rs\"",
    )?;
    ensure_eq(
        &full_rows(&response)?
            .first()
            .check("full_rows(&response)?[0]")?
            .line,
        &4,
        "full_rows(&response)?[0].line == 4",
    )?;
    ensure_eq(
        &full_rows(&response)?
            .first()
            .check("full_rows(&response)?[0]")?
            .detail,
        &"x.expect",
        "full_rows(&response)?[0].detail == \"x.expect\"",
    )?;
    ensure_eq(
        &count_of(&response, "credential_files")?,
        &1,
        "count_of(&response, \"credential_files\")? == 1",
    )?;
    ensure(response.page.complete, "response.page.complete")?;
    ensure_eq(
        &(response.page.returned, response.page.total),
        &(1, Some(1)),
        "(response.page.returned, response.page.total) == (1, Some(1))",
    )?;
    ensure_eq(
        &(response.files_total, response.files_covered),
        &(1, 1),
        "(response.files_total, response.files_covered) == (1, 1)",
    )?;
    let pages = &response.page_file_counts;
    ensure_eq(
        &(pages.len(), pages.first().check("pages[0]")?.file.as_str()),
        &(1, "lib.rs"),
        "(pages.len(), pages[0].file.as_str()) == (1, \"lib.rs\")",
    )?;
    ensure_eq(
        &response.page.limited_by,
        &PageLimit::None,
        "response.page.limited_by == PageLimit::None",
    )?;
    Ok(())
}

#[test]
fn a_zero_limit_is_clamped_to_one_row() -> TestResult {
    let project = rust_fixture()?;
    let mut request = request(project.path(), Some("expect_calls"));
    request.limit = Some(0);
    let response = run(request)?;

    ensure_eq(&response.page.returned, &1, "response.page.returned == 1")?;
    ensure_eq(&response.page.cursor, &None, "response.page.cursor == None")?;
    ensure(response.page.complete, "response.page.complete")?;
    Ok(())
}

#[test]
fn rejects_an_unknown_mechanic_id() -> TestResult {
    let project = rust_fixture()?;
    let error = try_run(request(project.path(), Some("nonsense")))
        .check_err("unknown mechanic is rejected")?;

    let message = error.message.to_string();
    ensure(
        message.contains("unknown mechanic"),
        "message.contains(\"unknown mechanic\")",
    )?;
    ensure(
        message.contains("expect_calls"),
        "message.contains(\"expect_calls\")",
    )?;
    Ok(())
}

/// The paging contract: a mechanic with more rows than a page reports
/// `complete: false` with `returned`/`total`, and the denominator is
/// distinct files — the number a sweep can actually exhaust — not rows.
#[test]
fn a_page_reports_returned_total_complete_and_the_distinct_file_denominator() -> TestResult {
    let project = marker_fixture(3, 4)?; // 12 rows across 3 files
    let manager = crate::sync::SourceSyncManager::default();
    let mut first = request(project.path(), Some("comment_markers"));
    first.limit = Some(5);
    let first = run_on(&manager, first)?;

    ensure_eq(
        &(first.page.returned, first.page.total),
        &(5, Some(12)),
        "(first.page.returned, first.page.total) == (5, Some(12))",
    )?;
    ensure_eq(&first.files_total, &3, "first.files_total == 3")?;
    ensure_eq(&first.files_covered, &2, "first.files_covered == 2")?;
    ensure(
        !first.page.complete && first.page.limited_by == PageLimit::Limit,
        "!first.page.complete && first.page.limited_by == PageLimit::Limit",
    )?;
    first
        .page
        .cursor
        .as_ref()
        .check("`first.page.cursor` is present")?;
    let (spent, budget) = (first.page.bytes_used, first.page.page_bytes);
    ensure(spent > 0 && budget >= spent, "spent > 0 && budget >= spent")?;

    let mut second = continuation(&first)?;
    second.limit = Some(5);
    let second = run_on(&manager, second)?;
    let mut third = continuation(&second)?;
    third.limit = Some(5);
    let last = run_on(&manager, third)?;
    ensure_eq(&last.page.returned, &2, "last.page.returned == 2")?;
    ensure(last.page.complete, "last.page.complete")?;
    ensure_eq(&last.page.cursor, &None, "last.page.cursor == None")?;
    ensure_eq(
        &last.files_covered,
        &last.files_total,
        "last.files_covered == last.files_total",
    )?;
    ensure_eq(
        &last.page.limited_by,
        &PageLimit::None,
        "last.page.limited_by == PageLimit::None",
    )?;
    Ok(())
}

/// A file split across a page boundary is counted once, and the file count
/// is cumulative for the sweep — the first page's `files_covered` plus the
/// rows still to come equal `files_total`, not `files_total` per page.
#[test]
fn files_covered_counts_a_file_split_across_pages_once() -> TestResult {
    let project = marker_fixture(2, 3)?; // 6 rows across 2 files
    let manager = crate::sync::SourceSyncManager::default();
    let mut first = request(project.path(), Some("comment_markers"));
    first.limit = Some(4); // mod0's 3 rows, then mod1's first
    let first = run_on(&manager, first)?;

    ensure_eq(
        &first.page_file_counts.len(),
        &2,
        "first.page_file_counts.len() == 2",
    )?;
    ensure_eq(
        &first
            .page_file_counts
            .first()
            .check("first.page_file_counts[0]")?
            .count,
        &3,
        "first.page_file_counts[0].count == 3",
    )?;
    ensure_eq(
        &first
            .page_file_counts
            .get(1)
            .check("first.page_file_counts[1]")?
            .count,
        &1,
        "first.page_file_counts[1].count == 1",
    )?;
    ensure_eq(
        &(first.files_covered, first.files_total),
        &(2, 2),
        "(first.files_covered, first.files_total) == (2, 2)",
    )?;

    let rest = run_on(&manager, continuation(&first)?)?;
    ensure_eq(
        &(rest.page.returned, rest.files_covered),
        &(2, 2),
        "(rest.page.returned, rest.files_covered) == (2, 2)",
    )?;
    ensure(rest.page.complete, "rest.page.complete")?;
    Ok(())
}

#[test]
fn a_pattern_narrows_counts_and_rows_to_matching_paths() -> TestResult {
    let project = two_dir_fixture()?;
    let unfiltered = run(request(project.path(), Some("comment_markers")))?;
    ensure_eq(
        &unfiltered.page.total,
        &Some(4),
        "unfiltered.page.total == Some(4)",
    )?;
    ensure_eq(&unfiltered.files_total, &3, "unfiltered.files_total == 3")?;

    let mut scoped = request(project.path(), Some("comment_markers"));
    scoped.pattern = Some("src/auth/*".to_owned());
    let scoped = run(scoped)?;
    ensure_eq(&scoped.page.total, &Some(2), "scoped.page.total == Some(2)")?;
    ensure_eq(&scoped.files_total, &2, "scoped.files_total == 2")?;
    let scoped_rows = full_rows(&scoped)?;
    let inside = scoped_rows.iter().all(|r| r.file.starts_with("src/auth/"));
    ensure(inside, "inside")?;

    // No `*`/`?` means a substring match, exactly as `structure` treats it.
    let mut substring = request(project.path(), Some("comment_markers"));
    substring.pattern = Some("token".to_owned());
    let substring = run(substring)?;
    ensure_eq(
        &substring.page.total,
        &Some(1),
        "substring.page.total == Some(1)",
    )?;
    ensure_eq(
        &full_rows(&substring)?
            .first()
            .check("full_rows(&substring)?[0]")?
            .file,
        &"src/auth/token.rs",
        "full_rows(&substring)?[0].file == \"src/auth/token.rs\"",
    )?;

    // A filter that matches nothing is a complete, empty enumeration — not
    // an error and not a silent fallback to the unfiltered set.
    let mut nothing = request(project.path(), None);
    nothing.pattern = Some("no/such/path".to_owned());
    let nothing = run(nothing)?;
    ensure_eq(
        &count_of(&nothing, "comment_markers")?,
        &0,
        "count_of(&nothing, \"comment_markers\")? == 0",
    )?;
    ensure(nothing.page.complete, "nothing.page.complete")?;
    ensure(
        full_rows(&nothing)?.is_empty(),
        "full_rows(&nothing)?.is_empty()",
    )?;
    Ok(())
}

/// `credential_files`' predicate is an unparenthesized `OR` chain, so the
/// glob has to be AND-ed against the whole thing: binding it to the last
/// term alone would keep every non-matching credential path.
#[test]
fn a_pattern_filters_a_credential_predicate_that_is_an_or_chain() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    fs::write(project.path().join(".env"), b"API_KEY=secret\n").check("write .env")?;
    fs::create_dir_all(project.path().join("sub")).check("create dir")?;
    fs::write(project.path().join("sub").join("client.pem"), b"key\n").check("write pem")?;

    let unfiltered = run(request(project.path(), Some("credential_files")))?;
    ensure_eq(
        &unfiltered.page.total,
        &Some(2),
        "unfiltered.page.total == Some(2)",
    )?;

    let mut scoped = request(project.path(), Some("credential_files"));
    scoped.pattern = Some("sub".to_owned());
    let scoped = run(scoped)?;
    ensure_eq(&scoped.page.total, &Some(1), "scoped.page.total == Some(1)")?;
    ensure_eq(
        &full_rows(&scoped)?
            .first()
            .check("full_rows(&scoped)?[0]")?
            .file,
        &"sub/client.pem",
        "full_rows(&scoped)?[0].file == \"sub/client.pem\"",
    )?;
    Ok(())
}

#[test]
fn compact_page_file_counts_merges_non_adjacent_files_in_first_appearance_order() -> TestResult {
    let counts =
        super::page::page_file_counts(["a.rs", "b.rs", "a.rs", "c.rs", "b.rs", "a.rs"].into_iter())
            .check("page_file_counts succeeds")?;
    let pairs: Vec<(&str, i64)> = counts.iter().map(|c| (c.file.as_str(), c.count)).collect();
    ensure_eq(
        &pairs,
        &[("a.rs", 3), ("b.rs", 2), ("c.rs", 1)],
        "non-adjacent files are merged without duplicates in first-appearance order",
    )?;
    Ok(())
}
