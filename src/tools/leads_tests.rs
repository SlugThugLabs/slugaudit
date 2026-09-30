use super::*;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;
use std::path::Path;

fn activate(root: &Path) {
    fs::create_dir_all(root.join(".planning").join("slugaudit")).expect("create activation dir");
}

/// A request with everything at its default: no mechanic, no filter, no
/// limit, no pin. Tests mutate the fields they are about.
fn request(path: &Path, mechanic: Option<&str>) -> LeadsRequest {
    LeadsRequest {
        path: path.to_string_lossy().into_owned(),
        mechanic: mechanic.map(str::to_owned),
        pattern: None,
        limit: None,
        offset: None,
        expected_revision: None,
    }
}

fn try_run(request: LeadsRequest) -> Result<LeadsResponse, ErrorData> {
    leads_with_limits(
        &Parameters(request),
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

fn run(request: LeadsRequest) -> LeadsResponse {
    try_run(request).expect("leads succeeds")
}

fn run_with_limits(request: LeadsRequest, limits: &ResourceLimits) -> LeadsResponse {
    leads_with_limits(
        &Parameters(request),
        limits,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("leads succeeds")
    .0
}

fn entry<'a>(response: &'a LeadsResponse, mechanic: &str) -> &'a MechanicCount {
    response
        .mechanic_counts
        .iter()
        .find(|entry| entry.mechanic == mechanic)
        .unwrap_or_else(|| panic!("mechanic {mechanic} missing from counts"))
}

fn count_of(response: &LeadsResponse, mechanic: &str) -> i64 {
    entry(response, mechanic).count
}

/// A Rust project with one `unwrap`, one `expect`, one TODO comment, and one
/// credential-shaped file.
fn rust_fixture() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    fs::write(
        project.path().join("lib.rs"),
        b"// TODO: revisit\npub fn a(x: Option<u8>) -> u8 {\n    let _ = x.unwrap();\n    x.expect(\"nope\")\n}\n",
    )
    .expect("write rust source");
    fs::write(project.path().join(".env"), b"API_KEY=secret\n").expect("write .env");
    project
}

/// A Rust project with `per_file` `unwrap` calls in each of `files` files —
/// enough rows to page, with a distinct-file denominator above one.
fn unwrap_fixture(files: usize, per_file: usize) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    for file in 0..files {
        let body: String = (0..per_file)
            .map(|n| format!("pub fn f{n}(x: Option<u8>) -> u8 {{ x.unwrap() }}\n"))
            .collect();
        fs::write(project.path().join(format!("mod{file}.rs")), body).expect("write rust source");
    }
    project
}

/// Two directories, so a `pattern` has something to exclude.
fn two_dir_fixture() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    for (relative, body) in [
        (
            "src/auth/login.rs",
            "pub fn a(x: Option<u8>) -> u8 { x.unwrap() }\n",
        ),
        (
            "src/auth/token.rs",
            "pub fn b(x: Option<u8>) -> u8 { x.unwrap() }\n",
        ),
        (
            "src/db/pool.rs",
            "pub fn c(x: Option<u8>) -> u8 { x.unwrap() }\npub fn d(x: Option<u8>) -> u8 { x.unwrap() }\n",
        ),
    ] {
        let path = project.path().join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent dirs");
        fs::write(path, body).expect("write rust source");
    }
    project
}

#[test]
fn counts_every_mechanic_without_returning_rows() {
    let project = rust_fixture();
    let response = run(request(project.path(), None));

    let ids: Vec<&str> = response
        .mechanic_counts
        .iter()
        .map(|entry| entry.mechanic.as_str())
        .collect();
    assert_eq!(
        ids,
        vec![
            "unwrap_calls",
            "expect_calls",
            "panic_calls",
            "todo_calls",
            "assert_calls",
            "diagnostics",
            "comment_markers",
            "credential_files",
        ]
    );
    assert_eq!(count_of(&response, "unwrap_calls"), 1);
    assert_eq!(count_of(&response, "expect_calls"), 1);
    assert!(count_of(&response, "comment_markers") >= 1);
    assert_eq!(count_of(&response, "credential_files"), 1);
    assert!(response.rows.is_empty(), "counts-only call returns no rows");
    assert!(response.complete);
    assert_eq!(response.next_offset, None);
    assert_eq!(response.total, 0, "no mechanic was requested");
    assert_eq!(response.files_total, 0);
}

#[test]
fn returns_rows_for_the_requested_mechanic_only() {
    let project = rust_fixture();
    let response = run(request(project.path(), Some("unwrap_calls")));

    assert_eq!(response.rows.len(), 1);
    assert_eq!(response.rows[0].file, "lib.rs");
    assert!(response.rows[0].line > 0);
    assert_eq!(response.rows[0].detail, "x.unwrap");
    assert_eq!(count_of(&response, "credential_files"), 1);
    assert!(response.complete);
    assert_eq!(response.returned, 1);
    assert_eq!(response.total, 1);
    assert_eq!(response.files_total, 1);
    assert_eq!(response.files_covered, 1);
    assert_eq!(response.page_file_counts.len(), 1);
    assert_eq!(response.page_file_counts[0].file, "lib.rs");
    assert_eq!(response.limited_by, PageLimit::None);
}

#[test]
fn a_zero_limit_is_clamped_to_one_row() {
    let project = rust_fixture();
    let mut request = request(project.path(), Some("unwrap_calls"));
    request.limit = Some(0);
    let response = run(request);

    assert_eq!(response.returned, 1, "a zero-row page could not progress");
    assert_eq!(response.next_offset, None);
    assert!(response.complete);
}

#[test]
fn rejects_an_unknown_mechanic_id() {
    let project = rust_fixture();
    let error = try_run(request(project.path(), Some("nonsense")))
        .expect_err("unknown mechanic is rejected");

    let message = error.message.to_string();
    assert!(message.contains("unknown mechanic"), "{message}");
    assert!(message.contains("unwrap_calls"), "{message}");
}

#[test]
fn call_mechanics_are_zero_for_a_project_without_rust_files() {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    fs::write(
        project.path().join("app.py"),
        b"def a(x):\n    return x.unwrap()\n",
    )
    .expect("write python source");

    let response = run(request(project.path(), None));

    // The call walker is Rust-only: the mechanic exists but is not measured
    // for this project, so it reports zero rather than disappearing — and
    // `covers` says which of the two a zero is.
    assert_eq!(count_of(&response, "unwrap_calls"), 0);
    assert_eq!(
        entry(&response, "unwrap_calls").covers,
        MechanicCoverage::RustOnly
    );
    assert_eq!(count_of(&response, "credential_files"), 0);
}

/// The phase-3 contract: a mechanic with more rows than a page reports
/// `complete: false` with `returned`/`total`, and the denominator is
/// distinct files — the number a sweep can actually exhaust — not rows.
#[test]
fn a_page_reports_returned_total_complete_and_the_distinct_file_denominator() {
    let project = unwrap_fixture(3, 4); // 12 rows across 3 files
    let mut first = request(project.path(), Some("unwrap_calls"));
    first.limit = Some(5);
    let first = run(first);

    assert_eq!(first.returned, 5);
    assert_eq!(first.total, 12);
    assert!(!first.complete);
    assert_eq!(first.files_total, 3, "the denominator is files, not rows");
    assert_eq!(first.files_covered, 2, "5 rows reach mod0 and mod1");
    assert_eq!(first.limited_by, PageLimit::Limit);
    assert_eq!(first.next_offset, Some(5));
    assert!(
        first.rows_bytes > 0 && first.row_budget_bytes >= first.rows_bytes,
        "the page reports the budget it spent against: {} of {}",
        first.rows_bytes,
        first.row_budget_bytes
    );

    let mut last = request(project.path(), Some("unwrap_calls"));
    last.offset = Some(11);
    let last = run(last);
    assert_eq!(last.returned, 1);
    assert!(last.complete);
    assert_eq!(last.next_offset, None);
    assert_eq!(last.files_covered, last.files_total);
    assert_eq!(last.limited_by, PageLimit::None);
}

/// A file split across a page boundary is counted once, and the file count
/// is cumulative for the sweep — offset 0's `files_covered` plus the rows
/// still to come equal `files_total`, not `files_total` per page.
#[test]
fn files_covered_counts_a_file_split_across_pages_once() {
    let project = unwrap_fixture(2, 3); // 6 rows across 2 files
    let mut first = request(project.path(), Some("unwrap_calls"));
    first.limit = Some(4); // mod0's 3 rows, then mod1's first
    let first = run(first);

    assert_eq!(first.page_file_counts.len(), 2);
    assert_eq!(first.page_file_counts[0].count, 3);
    assert_eq!(
        first.page_file_counts[1].count, 1,
        "mod1 is split, not doubled"
    );
    assert_eq!(first.files_covered, 2);
    assert_eq!(first.files_total, 2);

    let mut rest = request(project.path(), Some("unwrap_calls"));
    rest.offset = first.next_offset;
    let rest = run(rest);
    assert_eq!(rest.returned, 2);
    assert!(rest.complete);
    assert_eq!(rest.files_covered, 2);
}

/// The byte budget replaces the deleted row cap: with no `limit`, rows stop
/// at the budget, and the response says so rather than reading as a small
/// mechanic. One row is always returned, so an empty page never means
/// "budget too small".
#[test]
fn the_byte_budget_stops_the_page_and_reports_it() {
    let project = unwrap_fixture(3, 4);
    let limits = ResourceLimits {
        max_query_response_bytes: 8,
        ..ResourceLimits::default()
    };
    let response = run_with_limits(request(project.path(), Some("unwrap_calls")), &limits);

    assert_eq!(response.returned, 1, "the first row is always returned");
    assert!(!response.complete);
    assert_eq!(response.limited_by, PageLimit::Bytes);
    assert_eq!(response.row_budget_bytes, 8);
    assert!(
        response.rows_bytes > 8,
        "one row alone can exceed the budget: {}",
        response.rows_bytes
    );
    assert_eq!(response.total, 12, "the total is still complete");
    assert_eq!(response.next_offset, Some(1));
}

#[test]
fn a_pattern_narrows_counts_and_rows_to_matching_paths() {
    let project = two_dir_fixture();
    let unfiltered = run(request(project.path(), Some("unwrap_calls")));
    assert_eq!(unfiltered.total, 4);
    assert_eq!(unfiltered.files_total, 3);

    let mut scoped = request(project.path(), Some("unwrap_calls"));
    scoped.pattern = Some("src/auth/*".to_owned());
    let scoped = run(scoped);
    assert_eq!(scoped.total, 2, "only src/auth rows");
    assert_eq!(scoped.files_total, 2);
    assert!(
        scoped
            .rows
            .iter()
            .all(|row| row.file.starts_with("src/auth/")),
        "every row is inside the filter: {:?}",
        scoped.rows.iter().map(|row| &row.file).collect::<Vec<_>>()
    );

    // No `*`/`?` means a substring match, exactly as `structure` treats it.
    let mut substring = request(project.path(), Some("unwrap_calls"));
    substring.pattern = Some("token".to_owned());
    let substring = run(substring);
    assert_eq!(substring.total, 1);
    assert_eq!(substring.rows[0].file, "src/auth/token.rs");

    // A filter that matches nothing is a complete, empty enumeration — not
    // an error and not a silent fallback to the unfiltered set.
    let mut nothing = request(project.path(), None);
    nothing.pattern = Some("no/such/path".to_owned());
    let nothing = run(nothing);
    assert_eq!(count_of(&nothing, "unwrap_calls"), 0);
    assert!(nothing.complete);
    assert!(nothing.rows.is_empty());
}

/// `credential_files`' predicate is an unparenthesized `OR` chain, so the
/// glob has to be AND-ed against the whole thing: binding it to the last
/// term alone would keep every non-matching credential path.
#[test]
fn a_pattern_filters_a_credential_predicate_that_is_an_or_chain() {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    fs::write(project.path().join(".env"), b"API_KEY=secret\n").expect("write .env");
    fs::create_dir_all(project.path().join("sub")).expect("create dir");
    fs::write(project.path().join("sub").join("client.pem"), b"key\n").expect("write pem");

    let unfiltered = run(request(project.path(), Some("credential_files")));
    assert_eq!(
        unfiltered.total, 2,
        "both credential paths are absent a filter"
    );

    let mut scoped = request(project.path(), Some("credential_files"));
    scoped.pattern = Some("sub".to_owned());
    let scoped = run(scoped);
    assert_eq!(scoped.total, 1, "only the pem is under sub/");
    assert_eq!(scoped.rows[0].file, "sub/client.pem");
}

#[test]
fn mechanic_counts_carry_the_file_denominator_and_what_each_mechanic_covers() {
    let project = unwrap_fixture(3, 2); // 6 unwraps across 3 files
    let response = run(request(project.path(), None));

    let unwrap = entry(&response, "unwrap_calls");
    assert_eq!(unwrap.count, 6);
    assert_eq!(unwrap.files, 3);
    assert_eq!(unwrap.covers, MechanicCoverage::RustOnly);
    assert_eq!(
        entry(&response, "comment_markers").covers,
        MechanicCoverage::AllLanguages
    );
    assert_eq!(
        entry(&response, "credential_files").covers,
        MechanicCoverage::AllLanguages
    );
}

/// A resumed page that pins the revision it came from is refused — with the
/// `stale_revision` code and a remedy that restarts the sweep rather than
/// retrying the same offset — instead of silently skipping or repeating
/// rows from a different ordering.
#[test]
fn a_stale_expected_revision_refuses_a_resumed_leads_page() {
    let project = unwrap_fixture(2, 2); // 4 rows across 2 files
    let mut first = request(project.path(), Some("unwrap_calls"));
    first.limit = Some(2);
    let first = run(first);
    assert_eq!(first.next_offset, Some(2));

    let accepted = run(resumed_page_request(&project, &first));
    assert!(accepted.complete, "an unchanged revision accepts the pin");
    assert_eq!(accepted.revision_id, first.revision_id);

    fs::write(
        project.path().join("mod2.rs"),
        b"pub fn e(x: Option<u8>) -> u8 { x.unwrap() }\n",
    )
    .expect("add a file mid-sweep");
    let error = try_run(resumed_page_request(&project, &first))
        .expect_err("a page pinned to a superseded revision is refused");
    let data = error.data.expect("the refusal carries a payload");
    assert_eq!(data["code"], "stale_revision");
    let remedy = data["remedy"].as_str().expect("remedy is a string");
    assert!(
        remedy.contains("offset 0"),
        "the remedy restarts the sweep: {remedy}"
    );
    assert!(
        error.message.contains(&first.revision_id),
        "the message names the pin it refused: {}",
        error.message
    );

    // Unpinned, the sweep simply continues against the new revision.
    let mut fresh = request(project.path(), Some("unwrap_calls"));
    fresh.offset = first.next_offset;
    let fresh = run(fresh);
    assert_ne!(fresh.revision_id, first.revision_id);
    assert_eq!(fresh.total, 5);
}

/// The follow-on call a caller makes after a page: same mechanic, resuming
/// at `next_offset`, pinned to the revision the page came from.
fn resumed_page_request(project: &tempfile::TempDir, first: &LeadsResponse) -> LeadsRequest {
    let mut request = request(project.path(), Some("unwrap_calls"));
    request.offset = first.next_offset;
    request.expected_revision = Some(first.revision_id.clone());
    request
}

#[test]
fn detail_text_is_capped_at_the_char_limit() {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    // Multi-byte padding proves the cut lands on a char boundary, not a byte
    // offset: 300 two-byte chars, truncated to the cap.
    fs::write(
        project.path().join("lib.rs"),
        format!("// TODO: {}\n", "é".repeat(300)),
    )
    .expect("write rust source");

    let response = run(request(project.path(), Some("comment_markers")));

    assert_eq!(response.rows.len(), 1);
    assert_eq!(response.rows[0].detail.chars().count(), MAX_DETAIL_CHARS);
}

#[test]
fn comment_marker_clip_is_reported_on_the_row() {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    fs::write(
        project.path().join("lib.rs"),
        format!("// TODO: {}\n", "x".repeat(300)),
    )
    .expect("write rust source");

    let response = run(request(project.path(), Some("comment_markers")));

    assert_eq!(response.rows.len(), 1);
    assert_eq!(response.rows[0].detail.chars().count(), MAX_DETAIL_CHARS);
    assert!(
        response.rows[0].detail_clipped,
        "a comment longer than the cap must report detail_clipped"
    );
}

#[test]
fn short_comment_detail_is_not_flagged_as_clipped() {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    let body = "y".repeat(120);
    fs::write(project.path().join("lib.rs"), format!("// TODO: {body}\n"))
        .expect("write rust source");

    let response = run(request(project.path(), Some("comment_markers")));

    assert_eq!(response.rows.len(), 1);
    assert!(!response.rows[0].detail_clipped);
    assert!(
        response.rows[0].detail.ends_with(&body),
        "unclipped detail must carry the whole stored text: {}",
        response.rows[0].detail
    );
}

#[test]
fn clip_detail_flags_only_past_the_char_limit() {
    let mut at_limit = LeadRow {
        file: "f.rs".to_owned(),
        line: 1,
        detail: "é".repeat(MAX_DETAIL_CHARS),
        detail_clipped: false,
    };

    clip_detail(&mut at_limit);

    assert_eq!(at_limit.detail.chars().count(), MAX_DETAIL_CHARS);
    assert!(!at_limit.detail_clipped, "exactly the cap is not clipped");

    let mut over_limit = LeadRow {
        file: "f.rs".to_owned(),
        line: 1,
        detail: "é".repeat(MAX_DETAIL_CHARS + 1),
        detail_clipped: false,
    };

    clip_detail(&mut over_limit);

    assert_eq!(over_limit.detail.chars().count(), MAX_DETAIL_CHARS);
    assert_eq!(over_limit.detail.len(), MAX_DETAIL_CHARS * 2);
    assert!(over_limit.detail_clipped, "one past the cap is clipped");
}

/// `credential_files` reads `files` directly, so a credential-shaped file the
/// index deliberately skipped is still something the agent must be told
/// about: no content was retained, but the path is exactly the signal this
/// mechanic exists to surface.
#[test]
fn credential_files_reports_a_skipped_credential_shaped_file() {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    fs::write(
        project
            .path()
            .join(".planning")
            .join("slugaudit")
            .join("config.json"),
        br#"{"max_file_bytes": 500}"#,
    )
    .expect("write config");
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
    fs::write(project.path().join("big.pem"), vec![b'k'; 5_000])
        .expect("write oversize credential-shaped file");

    let counts = run(request(project.path(), None));
    assert_eq!(
        count_of(&counts, "credential_files"),
        1,
        "a skipped credential-shaped file must still be counted, not hidden"
    );

    let rows = run(request(project.path(), Some("credential_files")));
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0].file, "big.pem");
    assert_eq!(rows.files_total, 1);
}
