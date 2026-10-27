//! Phase 9 golden responses for `mechanic` add and list; see `golden_phase9_support`.
use super::golden_phase9_support::{check_case, path_of, rust_project};
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::{MechanicAction, MechanicRequest, mechanic};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::Value;

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
fn golden_phase9_mechanic_add_and_list() {
    let project = rust_project();
    let manager = SourceSyncManager::default();

    let (add, added) = add_println_mechanic(&project, &manager);
    check_case("mechanic_add", &project, &add.id, &added);
    let list = mechanic_request(&project, MechanicAction::List);
    let listed = mechanic(
        &Parameters(clone_mechanic(&list)),
        &NoopProgressSink,
        &manager,
    )
    .expect("mechanic list succeeds")
    .0;
    check_case("mechanic_list", &project, &"list", &listed);
}
