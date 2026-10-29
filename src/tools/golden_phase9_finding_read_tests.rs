//! Phase 9 golden responses for `finding_read`; see `golden_phase9_support`.
use super::golden_phase9_support::{check_case, fixture_dir, normalise, path_of, rust_project};
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::{FindingReadRequest, FindingRequest, LegacyPaging, finding, finding_read};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};

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

    // The old `offset: 1` case. A cursor continuation keeps page one's stored
    // page budget (300 bytes here), so its pages do not split the findings
    // the way the fixture's default-budget page did (two rows on one page).
    // The fixture's `page` block (page_bytes, returned, bytes_used) therefore
    // cannot match by design; the comparison is content-level: the findings
    // of every page after page one, concatenated, equal the fixture's
    // `findings` array element for element. Paging fields are not compared.
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
    let mut token = first.page.cursor.clone().expect("page one leaves a cursor");
    let mut findings: Vec<Value> = Vec::new();
    loop {
        let page = read(FindingReadRequest {
            path: None,
            file: None,
            cursor: Some(token),
            legacy_paging: LegacyPaging::default(),
        });
        let mut body = serde_json::to_value(&page.findings).expect("findings serialize");
        normalise(&mut body, &path_of(&project));
        findings.extend(
            body.as_array()
                .expect("findings is an array")
                .iter()
                .cloned(),
        );
        match page.page.cursor {
            Some(next) => token = next,
            None => break,
        }
    }
    let text = std::fs::read_to_string(fixture_dir().join("finding_read_offset1.json"))
        .expect("read fixture");
    let fixture: Value = serde_json::from_str(&text).expect("fixture is JSON");
    let expected = fixture["response"]["findings"]
        .as_array()
        .expect("fixture findings is an array");
    assert_eq!(&findings, expected, "finding_read_offset1 findings drifted");
}
