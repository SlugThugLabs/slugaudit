//! Tests for the `structure` tool's limit and edge branches: the empty
//! query rejection, a file with no indexed content (binary), and the
//! long-match-text truncation path. Kept separate from `structure_tests.rs`
//! so each file stays under the source-size cap.

use super::*;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;

fn ask(
    project: &tempfile::TempDir,
    file: &str,
    query: &str,
) -> Result<StructureResponse, ErrorData> {
    structure(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: file.to_owned(),
            query: query.to_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

#[test]
fn an_empty_query_is_a_typed_error_not_a_silent_noop() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let result = ask(&project, "lib.rs", "   \n  ");
    let error = result.expect_err("a whitespace-only query must be rejected");
    assert!(error.message.contains("must not be empty"));
}

#[test]
fn a_file_without_indexed_content_is_a_typed_error() {
    let project = activated_project("logo.bin", b"\x89PNG\x00\x01\x02\x00\x00");
    let result = ask(&project, "logo.bin", "(identifier) @x");
    assert!(
        result.is_err(),
        "a binary file with no indexed source content must not be queried"
    );
}

/// Match text longer than `MAX_TEXT_BYTES` must be truncated at a UTF-8
/// boundary and flagged with `text_truncated`, never emitted in full and
/// never panicking on the boundary slice.
#[test]
fn long_match_text_is_truncated_and_flagged() {
    let mut content = String::new();
    content.push_str("let long = \"");
    for _ in 0..500 {
        content.push_str("abcdefghij");
    }
    content.push_str("\";\n");
    let project = activated_project("lib.rs", content.as_bytes());

    let response = ask(&project, "lib.rs", "(string_literal) @s").expect("query succeeds");
    let first = &response.matches[0];
    assert!(
        first.text_truncated,
        "a >2000-byte match must be flagged as truncated"
    );
    assert!(
        first.text.len() <= super::MAX_TEXT_BYTES,
        "truncated text must respect the byte cap"
    );
    assert!(
        first.text.is_char_boundary(first.text.len()),
        "truncation must land on a UTF-8 boundary"
    );
}
