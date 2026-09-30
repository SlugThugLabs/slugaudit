use super::*;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;
use std::path::Path;

fn activate(root: &Path) {
    fs::create_dir_all(root.join(".planning").join("slugaudit")).expect("create activation dir");
}

fn run(
    path: &Path,
    mechanic: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> LeadsResponse {
    leads(
        &Parameters(LeadsRequest {
            path: path.to_string_lossy().into_owned(),
            mechanic: mechanic.map(str::to_owned),
            limit,
            offset,
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("leads succeeds")
    .0
}

fn count_of(response: &LeadsResponse, mechanic: &str) -> i64 {
    response
        .mechanic_counts
        .iter()
        .find(|entry| entry.mechanic == mechanic)
        .unwrap_or_else(|| panic!("mechanic {mechanic} missing from counts"))
        .count
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

#[test]
fn counts_every_mechanic_without_returning_rows() {
    let project = rust_fixture();
    let response = run(project.path(), None, None, None);

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
    assert!(!response.truncated);
    assert_eq!(response.next_offset, None);
}

#[test]
fn returns_rows_for_the_requested_mechanic_only() {
    let project = rust_fixture();
    let response = run(project.path(), Some("unwrap_calls"), None, None);

    assert_eq!(response.rows.len(), 1);
    assert_eq!(response.rows[0].file, "lib.rs");
    assert!(response.rows[0].line > 0);
    assert_eq!(response.rows[0].detail, "x.unwrap");
    assert_eq!(count_of(&response, "credential_files"), 1);
    assert!(!response.truncated);
}

#[test]
fn pages_with_limit_and_reports_next_offset() {
    let project = rust_fixture();
    let response = run(project.path(), Some("unwrap_calls"), Some(0), None);

    assert!(response.rows.is_empty());
    assert!(response.truncated);
    assert_eq!(response.next_offset, Some(0));
}

#[test]
fn rejects_an_unknown_mechanic_id() {
    let project = rust_fixture();
    let error = leads(
        &Parameters(LeadsRequest {
            path: project.path().to_string_lossy().into_owned(),
            mechanic: Some("nonsense".to_owned()),
            limit: None,
            offset: None,
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .expect("unknown mechanic is rejected");

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

    let response = run(project.path(), None, None, None);

    // The call walker is Rust-only: the mechanic exists but is not measured
    // for this project, so it reports zero rather than disappearing.
    assert_eq!(count_of(&response, "unwrap_calls"), 0);
    assert_eq!(count_of(&response, "credential_files"), 0);
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

    let response = run(project.path(), Some("comment_markers"), None, None);

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

    let response = run(project.path(), Some("comment_markers"), None, None);

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

    let response = run(project.path(), Some("comment_markers"), None, None);

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
