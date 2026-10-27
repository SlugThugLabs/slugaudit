//! Phase 9 golden responses for `finding_read`; see `golden_phase9_support`.
use super::golden_phase9_support::{check_case, path_of, rust_project};
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::{FindingReadRequest, FindingRequest, finding, finding_read};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::json;

#[test]
fn golden_phase9_finding_read_list_and_offset() {
    let _guard = crate::util::lock_or_recover(&crate::util::SESSION_TEST_LOCK, "session_test_lock");
    let project = rust_project();
    let manager = SourceSyncManager::default();
    for (n, title) in ["first", "second", "third"].iter().enumerate() {
        finding(
            &Parameters(FindingRequest {
                path: path_of(&project),
                file: format!("mod{n}.rs"),
                line_start: 1,
                line_end: 2,
                severity: "low".to_owned(),
                category: "test".to_owned(),
                title: (*title).to_owned(),
                description: format!("{title} finding description"),
            }),
            &NoopProgressSink,
            &manager,
        )
        .expect("finding is stored");
    }
    for (name, offset) in [("finding_read_list", 0), ("finding_read_offset1", 1)] {
        let response = finding_read(
            &Parameters(FindingReadRequest {
                path: path_of(&project),
                file: None,
                offset,
                expected_revision: None,
            }),
            &NoopProgressSink,
            &manager,
        )
        .expect("finding_read succeeds")
        .0;
        check_case(name, &project, &json!({ "offset": offset }), &response);
    }
}
