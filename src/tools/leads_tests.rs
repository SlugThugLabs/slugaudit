use super::*;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;
use std::path::Path;

fn activate(root: &Path) {
    fs::create_dir_all(root.join(".planning").join("slugaudit")).expect("create activation dir");
}

/// A request with everything at its default: no mechanic, no filter, no
/// limit, no pin, full rows. Tests mutate the fields they are about.
fn request(path: &Path, mechanic: Option<&str>) -> LeadsRequest {
    LeadsRequest {
        path: path.to_string_lossy().into_owned(),
        mechanic: mechanic.map(str::to_owned),
        pattern: None,
        limit: None,
        offset: None,
        expected_revision: None,
        mode: LeadMode::Full,
        page_bytes: None,
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

/// The full-mode rows of a response; panics on a compact page, which is what
/// a test asserting full rows should want.
fn full_rows(response: &LeadsResponse) -> &[LeadRow] {
    match &response.rows {
        LeadRows::Full(rows) => rows,
        _ => panic!("expected full rows"),
    }
}

/// The compact-mode rows of a response; the mirror of [`full_rows`].
fn compact_rows(response: &LeadsResponse) -> &[CompactLeadRow] {
    match &response.rows {
        LeadRows::Compact(rows) => rows,
        _ => panic!("expected compact rows"),
    }
}

/// The request a compact call makes: the same fields as [`request`] with
/// `mode` set, so a test can compare the two modes over one project.
fn compact_request(path: &Path, mechanic: Option<&str>) -> LeadsRequest {
    LeadsRequest {
        mode: LeadMode::Compact,
        ..request(path, mechanic)
    }
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
    assert_eq!(ids, expected);
    assert_eq!(count_of(&response, "unwrap_calls"), 1);
    assert_eq!(count_of(&response, "expect_calls"), 1);
    assert!(count_of(&response, "comment_markers") >= 1);
    assert_eq!(count_of(&response, "credential_files"), 1);
    assert!(
        full_rows(&response).is_empty(),
        "no rows without a mechanic"
    );
    assert!(response.complete);
    assert_eq!(response.next_offset, None);
    assert_eq!((response.total, response.files_total), (0, 0));
}

#[test]
fn returns_rows_for_the_requested_mechanic_only() {
    let project = rust_fixture();
    let response = run(request(project.path(), Some("unwrap_calls")));

    assert_eq!(full_rows(&response).len(), 1);
    assert_eq!(full_rows(&response)[0].file, "lib.rs");
    assert!(full_rows(&response)[0].line > 0);
    assert_eq!(full_rows(&response)[0].detail, "x.unwrap");
    assert_eq!(count_of(&response, "credential_files"), 1);
    assert!(response.complete);
    assert_eq!((response.returned, response.total), (1, 1));
    assert_eq!((response.files_total, response.files_covered), (1, 1));
    let pages = &response.page_file_counts;
    assert_eq!((pages.len(), pages[0].file.as_str()), (1, "lib.rs"));
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
    let covers = entry(&response, "unwrap_calls").covers;
    assert_eq!(covers, MechanicCoverage::RustOnly);
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

    assert_eq!((first.returned, first.total), (5, 12));
    assert_eq!(first.files_total, 3, "the denominator is files, not rows");
    assert_eq!(first.files_covered, 2, "5 rows reach mod0 and mod1");
    assert!(!first.complete && first.limited_by == PageLimit::Limit);
    assert_eq!(first.next_offset, Some(5));
    let (spent, budget) = (first.rows_bytes, first.row_budget_bytes);
    assert!(spent > 0 && budget >= spent, "{spent} of {budget}");

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
    assert_eq!(first.page_file_counts[1].count, 1, "mod1 split");
    assert_eq!((first.files_covered, first.files_total), (2, 2));

    let mut rest = request(project.path(), Some("unwrap_calls"));
    rest.offset = first.next_offset;
    let rest = run(rest);
    assert_eq!((rest.returned, rest.files_covered), (2, 2));
    assert!(rest.complete);
}

/// The byte budget replaces the deleted row cap: with no `limit`, rows stop
/// at the budget in both row modes, and the response says so rather than
/// reading as a small mechanic. One row is always returned, so an empty page
/// never means "budget too small".
#[test]
fn the_byte_budget_stops_the_page_in_both_modes_and_reports_it() {
    let project = unwrap_fixture(3, 4);
    for (mode, budget) in [(LeadMode::Full, 8), (LeadMode::Compact, 40)] {
        let limits = ResourceLimits {
            max_query_response_bytes: budget,
            ..ResourceLimits::default()
        };
        let mut request = request(project.path(), Some("unwrap_calls"));
        request.mode = mode;

        let response = run_with_limits(request, &limits);

        assert_eq!(response.returned, 1, "the first row is always returned");
        assert!(!response.complete);
        assert_eq!(response.limited_by, PageLimit::Bytes);
        assert_eq!(response.row_budget_bytes, budget);
        assert!(response.rows_bytes > budget, "{}", response.rows_bytes);
        assert_eq!(response.total, 12, "the total is still complete");
        assert_eq!(response.next_offset, Some(1));
    }
    // A byte-limited page cannot grow by raising `limit`; the schema's own
    // advice must name the knob that acted.
    let schema = schemars::schema_for!(PageLimit).to_value().to_string();
    assert!(
        schema.contains("page_bytes") && !schema.contains("Raise `limit`"),
        "{schema}"
    );
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
    let scoped_rows = full_rows(&scoped);
    let inside = scoped_rows.iter().all(|r| r.file.starts_with("src/auth/"));
    assert!(inside, "{scoped_rows:?}");

    // No `*`/`?` means a substring match, exactly as `structure` treats it.
    let mut substring = request(project.path(), Some("unwrap_calls"));
    substring.pattern = Some("token".to_owned());
    let substring = run(substring);
    assert_eq!(substring.total, 1);
    assert_eq!(full_rows(&substring)[0].file, "src/auth/token.rs");

    // A filter that matches nothing is a complete, empty enumeration — not
    // an error and not a silent fallback to the unfiltered set.
    let mut nothing = request(project.path(), None);
    nothing.pattern = Some("no/such/path".to_owned());
    let nothing = run(nothing);
    assert_eq!(count_of(&nothing, "unwrap_calls"), 0);
    assert!(nothing.complete);
    assert!(full_rows(&nothing).is_empty());
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
    assert_eq!(unfiltered.total, 2, "both absent a filter");

    let mut scoped = request(project.path(), Some("credential_files"));
    scoped.pattern = Some("sub".to_owned());
    let scoped = run(scoped);
    assert_eq!(scoped.total, 1, "only the pem is under sub/");
    assert_eq!(full_rows(&scoped)[0].file, "sub/client.pem");
}

#[test]
fn mechanic_counts_carry_the_file_denominator_and_what_each_mechanic_covers() {
    let project = unwrap_fixture(3, 2); // 6 unwraps across 3 files
    let response = run(request(project.path(), None));

    let unwrap = entry(&response, "unwrap_calls");
    assert_eq!(unwrap.count, 6);
    assert_eq!(unwrap.files, 3);
    assert_eq!(unwrap.covers, MechanicCoverage::RustOnly);
    let comment = entry(&response, "comment_markers").covers;
    let credential = entry(&response, "credential_files").covers;
    assert_eq!(comment, MechanicCoverage::AllLanguages);
    assert_eq!(credential, MechanicCoverage::AllLanguages);
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

    let added = project.path().join("mod2.rs");
    fs::write(added, b"pub fn e(x: Option<u8>) -> u8 { x.unwrap() }\n").expect("add a file");
    let error = try_run(resumed_page_request(&project, &first))
        .expect_err("a page pinned to a superseded revision is refused");
    let data = error.data.expect("the refusal carries a payload");
    assert_eq!(data["code"], "stale_revision");
    let remedy = data["remedy"].as_str().expect("remedy is a string");
    assert!(remedy.contains("offset 0"), "remedy restarts: {remedy}");
    assert!(error.message.contains(&first.revision_id), "refused");

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
fn comment_marker_detail_is_clipped_at_the_char_limit() {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    // Multi-byte padding proves the cut lands on a char boundary, not a byte
    // offset: 300 two-byte chars, truncated to the cap.
    let short = "y".repeat(120);
    let long = format!("// TODO: {}\n", "é".repeat(300));
    fs::write(project.path().join("long.rs"), long).expect("write long");
    let body = format!("// TODO: {short}\n");
    fs::write(project.path().join("short.rs"), body).expect("write short");

    let response = run(request(project.path(), Some("comment_markers")));
    let rows = full_rows(&response);

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].detail.chars().count(), MAX_DETAIL_CHARS);
    assert!(rows[0].detail_clipped, "clipped on a char boundary");
    assert!(!rows[1].detail_clipped);
    assert!(rows[1].detail.ends_with(&short), "{}", rows[1].detail);
}

#[test]
fn clip_detail_flags_only_past_the_char_limit() {
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

    assert_eq!(at_limit.detail.chars().count(), MAX_DETAIL_CHARS);
    assert!(!at_limit.detail_clipped, "exactly the cap is not clipped");
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
    let config = project.path().join(".planning/slugaudit/config.json");
    fs::write(config, br#"{"max_file_bytes": 500}"#).expect("write config");
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
    fs::write(project.path().join("big.pem"), vec![b'k'; 5_000]).expect("write big.pem");

    let counts = run(request(project.path(), None));
    let skipped = count_of(&counts, "credential_files");
    assert_eq!(skipped, 1, "skipped credentials are still counted");

    let rows = run(request(project.path(), Some("credential_files")));
    assert_eq!(full_rows(&rows).len(), 1);
    assert_eq!(full_rows(&rows)[0].file, "big.pem");
    assert_eq!(rows.files_total, 1);
}

/// The new switch is optional: a request JSON carrying only `path` still
/// deserializes, to `full`.
#[test]
fn a_request_without_mode_deserializes_to_full() {
    let request: LeadsRequest = serde_json::from_str(r#"{"path":"/tmp/anywhere"}"#)
        .expect("path-only request deserializes");
    assert_eq!(request.mode, LeadMode::Full);
    assert!(request.mechanic.is_none() && request.pattern.is_none());
}

/// The compact mode's whole point: a `mechanic`/`file`/`line` triple with no
/// per-row text — the serialized row carries exactly those three keys, no
/// `detail` — and, with no `mechanic`, that is the whole surface in one
/// response, every row tagged. The default mode keeps its text.
#[test]
fn compact_rows_are_mechanic_file_line_triples_without_detail() {
    let project = rust_fixture();
    let response = run(compact_request(project.path(), None));
    let rows = compact_rows(&response);
    assert!(rows.iter().any(|r| r.mechanic == "unwrap_calls"));
    assert!(rows.iter().any(|r| r.mechanic == "comment_markers"));
    let returned = i64::try_from(response.returned).expect("fits");
    assert_eq!(returned, response.total);
    let serialized = serde_json::to_value(rows).expect("rows serialize");
    let row = &serialized[0];
    assert_eq!(row["mechanic"], "unwrap_calls");
    assert_eq!(row["file"], "lib.rs");
    assert!(row["line"].as_i64().expect("line") > 0);
    assert_eq!(row.as_object().expect("object").len(), 3, "{row}");

    let full = run(request(project.path(), Some("unwrap_calls")));
    let text = serde_json::to_value(&full.rows).expect("rows serialize");
    assert_eq!(text[0]["detail"], "x.unwrap");
    assert!(text[0].get("mechanic").is_none());

    let mut paged = compact_request(project.path(), None);
    paged.limit = Some(1);
    let page = run(paged);
    assert_eq!(compact_rows(&page)[0].mechanic, rows[0].mechanic);
    assert_eq!(page.next_offset, Some(1));

    let mut scoped = compact_request(project.path(), None);
    scoped.pattern = Some("lib.rs".to_owned());
    let scoped = run(scoped);
    let files: Vec<&str> = compact_rows(&scoped)
        .iter()
        .map(|r| r.file.as_str())
        .collect();
    assert!(!files.is_empty() && files.iter().all(|f| *f == "lib.rs"));
}

/// Paging a compact sweep: the pages partition the rows and `files_covered`
/// stays cumulative rather than per-page.
#[test]
fn compact_paging_reports_a_cumulative_file_denominator() {
    let project = unwrap_fixture(2, 3); // 6 rows across 2 files
    let mut first = compact_request(project.path(), None);
    first.limit = Some(4);
    let first = run(first);
    assert_eq!((first.returned, first.total), (4, 6));
    assert_eq!((first.files_covered, first.files_total), (2, 2));
    assert_eq!((first.complete, first.next_offset), (false, Some(4)));

    let mut rest = compact_request(project.path(), None);
    rest.offset = first.next_offset;
    rest.expected_revision = Some(first.revision_id);
    let rest = run(rest);
    assert_eq!((rest.returned, rest.complete), (2, true));
    assert_eq!((rest.files_covered, rest.files_total), (2, 2));
}

/// The completeness fields mean the same thing in both modes: for one
/// mechanic they match field for field, the rows are the same places in the
/// same order, and only the bytes differ.
#[test]
fn both_modes_report_the_same_completeness_for_one_mechanic() {
    let project = unwrap_fixture(3, 4);
    let mut full_req = request(project.path(), Some("unwrap_calls"));
    full_req.limit = Some(5);
    let full = run(full_req);
    let mut compact_req = compact_request(project.path(), Some("unwrap_calls"));
    compact_req.limit = Some(5);
    let compact = run(compact_req);

    assert_eq!(compact.returned, full.returned);
    assert_eq!(compact.total, full.total);
    assert_eq!(compact.complete, full.complete);
    assert_eq!(compact.files_total, full.files_total);
    assert_eq!(compact.files_covered, full.files_covered);
    assert_eq!(compact.limited_by, full.limited_by);
    assert_eq!(compact.next_offset, full.next_offset);
    let crows = compact_rows(&compact);
    let cplaces: Vec<(&str, i64)> = crows.iter().map(|r| (r.file.as_str(), r.line)).collect();
    let frows = full_rows(&full);
    let fplaces: Vec<(&str, i64)> = frows.iter().map(|r| (r.file.as_str(), r.line)).collect();
    assert_eq!(cplaces, fplaces, "same rows, same order");
    let (cbytes, fbytes) = (compact.rows_bytes, full.rows_bytes);
    assert!(cbytes < fbytes, "{cbytes} vs {fbytes}");
}

/// Every caller-supplied string field is bounded before it reaches the
/// project: an over-cap value is refused with the message naming the field
/// and its cap, never truncated or passed through.
#[test]
fn leads_refuses_inputs_over_their_byte_caps() {
    let refused = |mutate: fn(&mut LeadsRequest)| {
        let mut request = request(Path::new("/tmp"), None);
        mutate(&mut request);
        try_run(request).expect_err("an over-cap input is refused")
    };
    let mut path = refused(|r| r.path = "a".repeat(MAX_PATH_BYTES + 1));
    assert_eq!(path.message, "path exceeds 32768 bytes");
    path = refused(|r| r.mechanic = Some("m".repeat(MAX_FILTER_BYTES + 1)));
    assert_eq!(path.message, "mechanic exceeds 8192 bytes");
    path = refused(|r| r.pattern = Some("p".repeat(MAX_FILTER_BYTES + 1)));
    assert_eq!(path.message, "pattern exceeds 8192 bytes");
    path = refused(|r| r.expected_revision = Some("r".repeat(MAX_REVISION_BYTES + 1)));
    assert_eq!(path.message, "expected_revision exceeds 256 bytes");
    assert_eq!(
        path.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{path:?}"
    );
}

#[path = "leads_page_tests.rs"]
mod leads_page_tests;
