//! Phase 9 golden responses for `leads`; see `golden_phase9_support`.
use super::golden_phase9_support::{check_case, path_of, rust_project};
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::{
    LeadMode, LeadsRequest, LegacyPaging, MechanicAction, MechanicRequest, leads, mechanic,
};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};

fn leads_request(project: &tempfile::TempDir, mechanic: Option<&str>) -> LeadsRequest {
    LeadsRequest {
        path: Some(path_of(project)),
        mechanic: mechanic.map(str::to_owned),
        pattern: None,
        limit: None,
        cursor: None,
        mode: LeadMode::Full,
        page_bytes: None,
        legacy_paging: LegacyPaging::default(),
    }
}

fn run_leads(request: &LeadsRequest, manager: &SourceSyncManager) -> Value {
    let response = leads(
        &Parameters(LeadsRequest {
            path: request.path.clone(),
            mechanic: request.mechanic.clone(),
            pattern: request.pattern.clone(),
            limit: request.limit,
            cursor: request.cursor.clone(),
            mode: request.mode,
            page_bytes: request.page_bytes,
            legacy_paging: LegacyPaging::default(),
        }),
        &NoopProgressSink,
        manager,
    )
    .expect("leads succeeds")
    .0;
    serde_json::to_value(response).expect("response serializes")
}

fn leads_case(
    name: &str,
    project: &tempfile::TempDir,
    request: &LeadsRequest,
    manager: &SourceSyncManager,
) -> Value {
    let response = run_leads(request, manager);
    let echoed = json!({
        "mechanic": request.mechanic, "limit": request.limit, "cursor": request.cursor,
        "compact": request.mode == LeadMode::Compact,
    });
    check_case(name, project, &echoed, &response);
    response
}

#[test]
fn golden_phase9_leads_counts_rows_pages_and_compact() {
    let project = rust_project();
    let manager = SourceSyncManager::default();

    leads_case(
        "leads_counts_only",
        &project,
        &leads_request(&project, None),
        &manager,
    );
    let mut first = leads_request(&project, Some("unwrap_calls"));
    first.limit = Some(5);
    let page_one = leads_case("leads_rows_page1", &project, &first, &manager);
    let mut second = leads_request(&project, Some("unwrap_calls"));
    second.limit = Some(5);
    second.path = None;
    second.cursor = page_one["page"]["cursor"].as_str().map(str::to_owned);
    leads_case("leads_rows_page2", &project, &second, &manager);
    let mut compact = leads_request(&project, None);
    compact.mode = LeadMode::Compact;
    compact.limit = Some(8);
    leads_case("leads_compact", &project, &compact, &manager);
}

fn mechanic_request(project: &tempfile::TempDir, action: MechanicAction) -> MechanicRequest {
    MechanicRequest {
        path: path_of(project),
        action,
        id: None,
        index: None,
        language: None,
        query: None,
        note: None,
        persist: false,
        limit: None,
        page_bytes: None,
    }
}

fn add_println_mechanic(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
) -> (MechanicRequest, Value) {
    let mut add = mechanic_request(project, MechanicAction::Add);
    add.id = Some("println_calls".to_owned());
    add.language = Some("rust".to_owned());
    add.query = Some(r#"(macro_invocation macro: (identifier) @m (#eq? @m "println"))"#.to_owned());
    add.note = Some("println macro calls".to_owned());
    add.limit = Some(2);
    let response = mechanic(
        &Parameters(clone_mechanic(&add)),
        &NoopProgressSink,
        manager,
    )
    .expect("mechanic add succeeds")
    .0;
    (
        add,
        serde_json::to_value(response).expect("response serializes"),
    )
}

fn clone_mechanic(request: &MechanicRequest) -> MechanicRequest {
    MechanicRequest {
        path: request.path.clone(),
        action: request.action,
        id: request.id.clone(),
        index: request.index,
        language: request.language.clone(),
        query: request.query.clone(),
        note: request.note.clone(),
        persist: request.persist,
        limit: request.limit,
        page_bytes: request.page_bytes,
    }
}

#[test]
fn golden_phase9_leads_custom_mechanic_rows() {
    let project = rust_project();
    let manager = SourceSyncManager::default();
    add_println_mechanic(&project, &manager);

    let mut request = leads_request(&project, Some("println_calls"));
    request.limit = Some(2);
    leads_case("leads_custom_mechanic", &project, &request, &manager);
}
