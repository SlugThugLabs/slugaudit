//! Phase 9 golden responses for `finding_read`; see `golden_phase9_support`.
use super::golden_phase9_support::{check_case, path_of, rust_project};
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::{FindingReadRequest, FindingRequest, LegacyPaging, finding, finding_read};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::json;

#[test]
fn paged_finding_read_matches_the_golden_apart_from_paging_fields() {
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
    let read = |request: FindingReadRequest| {
        finding_read(&Parameters(request), &NoopProgressSink, &manager)
            .expect("finding_read succeeds")
            .0
    };
    let by_path = FindingReadRequest {
        path: Some(path_of(&project)),
        file: None,
        cursor: None,
        legacy_paging: LegacyPaging::default(),
    };
    let list = read(by_path);
    check_case("finding_read_list", &project, &json!({}), &list);

    // The old `offset: 1` case: a page budget that holds one finding makes
    // page one the first row, and the cursor page is the rest (rows two and
    // three), which is what the offset-1 fixture recorded.
    let config = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("config.json");
    std::fs::write(config, "{\"default_page_bytes\": 300}").expect("write project config");
    let first = read(FindingReadRequest {
        path: Some(path_of(&project)),
        file: None,
        cursor: None,
        legacy_paging: LegacyPaging::default(),
    });
    let token = first.page.cursor.clone().expect("page one leaves a cursor");
    let second = read(FindingReadRequest {
        path: None,
        file: None,
        cursor: Some(token),
        legacy_paging: LegacyPaging::default(),
    });
    check_case(
        "finding_read_offset1",
        &project,
        &json!({ "cursor": "page two" }),
        &second,
    );
}
